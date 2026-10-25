//! Did the bytes actually arrive?
//!
//! A cartridge is written over USB, often 60 GB of it, sometimes through a
//! bridge chip in a £12 enclosure that is also getting warm. Copies mostly work.
//! When they do not, the failure is silent: `std::fs::copy` reports success for
//! every byte the kernel accepted, and a truncated or flipped-bit game shows up
//! later as a crash on a level you have not played yet.
//!
//! So the wizard checks its own work. Each file is summed as it is copied —
//! CRC-32 costs nothing next to a USB write — and then the cartridge is read
//! back and the sums compared. That is one extra pass over the drive, and it is
//! on by default, because the first cartridge ever checked on real hardware
//! failed: two 2 GB archives out of 107 GB, both the right length, both with
//! different contents, written through a bridge that was resetting the bus
//! every twenty seconds. The copy reported success for all of it.
//!
//! **This is an integrity check, not a signature.** CRC-32 is the right tool for
//! "did this survive the cable", the same job it does in zip and gzip, and the
//! wrong tool for "did somebody change this on purpose". Nothing here defends
//! against a person; it defends against a cable, a bridge and a hot drive.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where the record of a copy lives on the cartridge, so it can be checked
/// again later without the machine that wrote it.
pub const MANIFEST_PATH: &str = ".gamepak/manifest.json";

/// Read in chunks this size. Large enough that the syscall overhead disappears,
/// small enough to stay out of the way on a modest machine.
const CHUNK: usize = 1024 * 1024;

/// One file, as it was when it was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDigest {
    /// Relative to the cartridge root, with forward slashes so a cartridge
    /// written on Windows verifies on Linux and the other way round.
    pub path: String,
    pub bytes: u64,
    pub crc: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub files: Vec<FileDigest>,
}

impl Manifest {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }
}

/// What was wrong with a file, in the words the user will see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Problem {
    /// The file is not on the cartridge at all.
    Missing(String),
    /// It is there but the wrong length — the classic half-written file.
    Truncated {
        path: String,
        expected: u64,
        found: u64,
    },
    /// Right length, wrong contents.
    Corrupt(String),
    /// It could not be read back to check.
    Unreadable { path: String, why: String },
}

impl Problem {
    pub fn path(&self) -> &str {
        match self {
            Problem::Missing(path) | Problem::Corrupt(path) => path,
            Problem::Truncated { path, .. } | Problem::Unreadable { path, .. } => path,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Problem::Missing(path) => format!("{path} did not make it across"),
            Problem::Truncated {
                path,
                expected,
                found,
            } => format!("{path} is {found} bytes, not {expected}"),
            Problem::Corrupt(path) => format!("{path} arrived with different contents"),
            Problem::Unreadable { path, why } => format!("{path} could not be read back: {why}"),
        }
    }
}

/// Copy one file, summing it on the way past.
///
/// This is the reason the verified path does not use `std::fs::copy`: the bytes
/// have to pass through here to be summed, and reading the source a second time
/// afterwards would double the cost of the check.
pub fn copy_and_digest(from: &Path, to: &Path) -> std::io::Result<(u64, u32)> {
    let mut source = std::fs::File::open(from)?;
    let mut destination = std::fs::File::create(to)?;
    let mut buffer = vec![0u8; CHUNK];
    let mut crc = Crc32::new();
    let mut bytes = 0u64;
    // Paced across this file rather than the whole cartridge: a fresh clock per
    // file keeps one slow file from earning the next one a burst.
    let started = std::time::Instant::now();

    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        crc.update(&buffer[..read]);
        bytes += read as u64;
        std::io::Write::write_all(&mut destination, &buffer[..read])?;
        crate::throttle::pace(bytes, started);
    }

    // Without this the check could pass against a file the kernel has not
    // finished writing, which is exactly the failure it is meant to catch.
    destination.sync_all()?;
    Ok((bytes, crc.finish()))
}

/// Sum a file that is already written.
pub fn digest_file(path: &Path) -> std::io::Result<(u64, u32)> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; CHUNK];
    let mut crc = Crc32::new();
    let mut bytes = 0u64;

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        crc.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok((bytes, crc.finish()))
}

/// Read the cartridge back and compare it with what was written.
///
/// `progress` is called with (bytes checked, bytes total) so a 60 GB read can
/// say something other than "working".
pub fn verify(
    root: &Path,
    manifest: &Manifest,
    progress: &mut dyn FnMut(u64, u64),
) -> Vec<Problem> {
    let total = manifest.total_bytes();
    let mut checked = 0u64;
    let mut problems = Vec::new();

    for file in &manifest.files {
        let path = root.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));

        match std::fs::metadata(&path) {
            Err(_) => {
                problems.push(Problem::Missing(file.path.clone()));
                checked += file.bytes;
                progress(checked, total);
                continue;
            }
            Ok(meta) if meta.len() != file.bytes => {
                problems.push(Problem::Truncated {
                    path: file.path.clone(),
                    expected: file.bytes,
                    found: meta.len(),
                });
                checked += file.bytes;
                progress(checked, total);
                continue;
            }
            Ok(_) => {}
        }

        match digest_file(&path) {
            Ok((_, crc)) if crc == file.crc => {}
            Ok(_) => problems.push(Problem::Corrupt(file.path.clone())),
            Err(e) => problems.push(Problem::Unreadable {
                path: file.path.clone(),
                why: e.to_string(),
            }),
        }
        checked += file.bytes;
        progress(checked, total);
    }

    problems
}

pub fn write_manifest(root: &Path, manifest: &Manifest) -> Result<(), String> {
    let path = root.join(MANIFEST_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("could not encode the manifest: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// The record left on a cartridge by whichever machine wrote it.
pub fn read_manifest(root: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(root.join(MANIFEST_PATH)).ok()?;
    serde_json::from_str(&text).ok()
}

/// A path relative to the cartridge root, in the manifest's own spelling.
pub fn relative_path(root: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(root).ok()?;
    let mut out = String::new();
    for part in relative.components() {
        if let std::path::Component::Normal(name) = part {
            if !out.is_empty() {
                out.push('/');
            }
            out.push_str(&name.to_string_lossy());
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Somewhere to put a digest as a tree is copied.
#[derive(Debug, Default)]
pub struct Digests {
    root: PathBuf,
    files: Vec<FileDigest>,
}

impl Digests {
    /// `root` is the cartridge root, so recorded paths are relative to it
    /// rather than to the folder being copied.
    pub fn new(root: &Path) -> Self {
        Digests {
            root: root.to_path_buf(),
            files: Vec::new(),
        }
    }

    pub fn record(&mut self, destination: &Path, bytes: u64, crc: u32) {
        if let Some(path) = relative_path(&self.root, destination) {
            self.files.push(FileDigest { path, bytes, crc });
        }
    }

    pub fn into_manifest(self) -> Manifest {
        Manifest { files: self.files }
    }
}

// --------------------------------------------------------------------------
// One value for a whole cartridge
// --------------------------------------------------------------------------

/// The name of a cartridge's contents, as one string.
///
/// `verify` already answers "did these bytes survive the trip" against a
/// manifest written by the machine that made the copy. It cannot answer the
/// other question: **did you and I build the same cartridge?** Two people
/// building from the same `CartridgeRequest` have two manifests, and comparing
/// them by eye is not a thing anyone does.
///
/// Kazeta's cartridge creator does this properly — a recipe declares the hash
/// its finished cart must have, so everybody builds byte-identical carts and
/// the recipe file doubles as a compatibility list. This is the same idea at
/// the scale this project works at.
///
/// Over the manifest rather than over the drive, deliberately:
///
/// * The manifest already names every copied file with its length and CRC-32,
///   so this costs no extra reading. Re-hashing a 107 GB cartridge to produce
///   one line would be a second verify pass.
/// * Sorted by path first, so the order files happened to be copied in does
///   not change the answer.
/// * The artwork, `cartridge.conf` and `.gamepak/` are not in the manifest and
///   so are not in this. That is the useful scope: two people who chose
///   different cover art but copied the same game should agree, because the
///   game is what a recipe pins.
///
/// The input is one line per file, `path\0bytes\0crc\n`, which is written out
/// rather than derived from the JSON — a serialiser that changes its key order
/// or its number formatting would otherwise change every hash ever published.
pub fn cartridge_digest(manifest: &Manifest) -> String {
    let mut lines: Vec<String> = manifest
        .files
        .iter()
        .map(|file| format!("{}\0{}\0{}\n", file.path, file.bytes, file.crc))
        .collect();
    lines.sort();

    let mut hash = Sha256::new();
    for line in &lines {
        hash.update(line.as_bytes());
    }
    hash.hex()
}

/// Whether a cartridge came out the way a request said it would.
///
/// `None` when the request declared nothing, which is the ordinary case and not
/// a failure. Case-insensitive, and whitespace-tolerant, because the expected
/// value arrives from a hand-edited JSON file and a pasted hash often brings a
/// stray space or a capital letter with it. A short prefix is accepted too: a
/// recipe that only quotes the first twelve characters is still making a
/// checkable claim, and rejecting it for being short would push people towards
/// declaring nothing at all.
pub fn digest_matches(expected: &str, actual: &str) -> Option<bool> {
    let expected = expected.trim();
    if expected.is_empty() {
        return None;
    }
    // A prefix has to be long enough to mean something. Four hex characters is
    // one chance in 65,536 of agreeing by accident, which is not a check.
    if expected.len() < 8 || expected.len() > actual.len() {
        return Some(false);
    }
    Some(actual[..expected.len()].eq_ignore_ascii_case(expected))
}

/// The first twelve characters of a digest, for showing a person.
///
/// Long enough that two cartridges on one desk will not collide, short enough
/// to read out loud or paste into a recipe. The whole value is what gets
/// compared; this is only ever for display.
pub fn short_digest(digest: &str) -> String {
    digest.chars().take(12).collect()
}

// --------------------------------------------------------------------------
// SHA-256 (FIPS 180-4)
// --------------------------------------------------------------------------

/// Hand-written, like the CRC-32 below and the KeyValues parser in `steam`.
///
/// A dependency would be one more crate in the tree of a program whose whole
/// pitch is that it reads a text file off a drive, and this is sixty lines of
/// arithmetic with published test vectors to check it against. CRC-32 would not
/// have done: thirty-two bits is fine for catching a bus reset mid-copy, and
/// far too few for a value people publish and compare.
#[derive(Debug, Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    bytes: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0; 64],
            buffered: 0,
            bytes: 0,
        }
    }

    pub fn update(&mut self, mut input: &[u8]) {
        self.bytes = self.bytes.wrapping_add(input.len() as u64);
        // Fill the partial block first, if there is one.
        if self.buffered > 0 {
            let want = (64 - self.buffered).min(input.len());
            self.buffer[self.buffered..self.buffered + want].copy_from_slice(&input[..want]);
            self.buffered += want;
            input = &input[want..];
            if self.buffered < 64 {
                // The input ran out inside the partial block. Returning here is
                // load-bearing: the tail below writes from index 0 and sets
                // `buffered` to what it wrote, which would throw away the
                // bytes just buffered. Every published test vector passes in
                // one call and so never reaches this.
                return;
            }
            let block = self.buffer;
            self.compress(&block);
            self.buffered = 0;
        }
        // Then whole blocks straight out of the input.
        while input.len() >= 64 {
            let (block, rest) = input.split_at(64);
            let mut owned = [0u8; 64];
            owned.copy_from_slice(block);
            self.compress(&owned);
            input = rest;
        }
        // Whatever is left waits for the next call, or for the padding.
        self.buffer[..input.len()].copy_from_slice(input);
        self.buffered = input.len();
    }

    /// The digest as lowercase hex, which is the form anyone will paste.
    pub fn hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.digest() {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    /// The digest as its 32 bytes, for HMAC (see [`crate::remote`]).
    pub fn digest(mut self) -> [u8; 32] {
        // Padding: a 1 bit, zeroes, then the length in bits as a big-endian
        // u64 — so the final block always has eight bytes free for it.
        let bits = self.bytes.wrapping_mul(8);
        self.update_raw(&[0x80]);
        while self.buffered != 56 {
            self.update_raw(&[0]);
        }
        self.update_raw(&bits.to_be_bytes());

        let mut out = [0u8; 32];
        for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// `update` without counting the bytes, for the padding itself.
    fn update_raw(&mut self, input: &[u8]) {
        for byte in input {
            self.buffer[self.buffered] = *byte;
            self.buffered += 1;
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (index, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }

        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
}

// --------------------------------------------------------------------------
// CRC-32 (IEEE 802.3), the one zip and gzip use
// --------------------------------------------------------------------------

/// Built at compile time, so there is no lazy initialisation to synchronise.
const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

#[derive(Debug, Clone)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        let mut crc = self.0;
        for byte in bytes {
            crc = TABLE[((crc ^ *byte as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        self.0 = crc;
    }

    pub fn finish(&self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc(bytes: &[u8]) -> u32 {
        let mut c = Crc32::new();
        c.update(bytes);
        c.finish()
    }

    /// A manifest entry whose numbers are made up but consistent.
    fn digest(path: &str, bytes: u64) -> FileDigest {
        FileDigest {
            path: path.to_string(),
            bytes,
            crc: crc(path.as_bytes()),
        }
    }

    #[test]
    fn crc32_matches_the_reference_vectors() {
        // The check value every CRC-32/ISO-HDLC implementation is tested with.
        assert_eq!(crc(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc(b""), 0x0000_0000);
        assert_eq!(crc(b"a"), 0xE8B7_BE43);
        assert_eq!(
            crc(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn feeding_it_in_pieces_gives_the_same_answer() {
        // It is fed a megabyte at a time in use, so this is the property that
        // actually matters.
        let mut whole = Crc32::new();
        whole.update(b"123456789");

        let mut pieces = Crc32::new();
        pieces.update(b"1234");
        pieces.update(b"");
        pieces.update(b"56789");

        assert_eq!(whole.finish(), pieces.finish());
    }

    #[test]
    fn a_copy_is_summed_as_it_is_made() {
        let scratch = crate::testutil::Scratch::new("copy-digest");
        scratch.write("source.bin", b"123456789");
        let destination = scratch.join("copy.bin");

        let (bytes, sum) = copy_and_digest(&scratch.join("source.bin"), &destination).unwrap();
        assert_eq!(bytes, 9);
        assert_eq!(sum, 0xCBF4_3926);
        assert_eq!(std::fs::read(&destination).unwrap(), b"123456789");

        // And reading it back afterwards agrees with what the copy reported.
        assert_eq!(digest_file(&destination).unwrap(), (bytes, sum));
    }

    #[test]
    fn a_good_cartridge_reports_nothing() {
        let scratch = crate::testutil::Scratch::new("verify-good");
        scratch.write("Games/Tunic/TUNIC.exe", b"pretend a game");
        scratch.write("cover.jpg", b"pretend art");

        let manifest = manifest_of(&scratch, &["Games/Tunic/TUNIC.exe", "cover.jpg"]);
        let mut seen = Vec::new();
        let problems = verify(scratch.path(), &manifest, &mut |done, total| {
            seen.push((done, total))
        });

        assert!(problems.is_empty(), "{problems:?}");
        // Progress runs to the end, so a bar can reach 100%.
        assert_eq!(seen.last().unwrap().0, manifest.total_bytes());
    }

    #[test]
    fn every_way_a_file_can_be_wrong_is_named() {
        let scratch = crate::testutil::Scratch::new("verify-bad");
        scratch.write("intact.bin", b"123456789");
        scratch.write("short.bin", b"123456789");
        scratch.write("flipped.bin", b"123456789");

        let mut manifest = manifest_of(
            &scratch,
            &["intact.bin", "short.bin", "flipped.bin", "gone.bin"],
        );
        // "gone.bin" was never written; the other two are damaged after the
        // fact, the way a bad cable would.
        manifest.files.push(FileDigest {
            path: "gone.bin".to_string(),
            bytes: 9,
            crc: 0,
        });
        scratch.write("short.bin", b"1234");
        scratch.write("flipped.bin", b"12345678X");

        let problems = verify(scratch.path(), &manifest, &mut |_, _| {});

        let by_path: Vec<&str> = problems.iter().map(Problem::path).collect();
        assert!(!by_path.contains(&"intact.bin"), "{problems:?}");
        assert!(matches!(
            problems.iter().find(|p| p.path() == "short.bin"),
            Some(Problem::Truncated {
                expected: 9,
                found: 4,
                ..
            })
        ));
        assert!(matches!(
            problems.iter().find(|p| p.path() == "flipped.bin"),
            Some(Problem::Corrupt(_))
        ));
        assert!(matches!(
            problems.iter().find(|p| p.path() == "gone.bin"),
            Some(Problem::Missing(_))
        ));

        // A truncated file says both numbers, since "wrong size" alone does not
        // tell you whether it is half-written or a different file entirely.
        let said = problems
            .iter()
            .find(|p| p.path() == "short.bin")
            .unwrap()
            .describe();
        assert!(said.contains('4') && said.contains('9'), "{said}");
    }

    #[test]
    fn a_manifest_survives_the_round_trip() {
        let scratch = crate::testutil::Scratch::new("manifest");
        let manifest = Manifest {
            files: vec![FileDigest {
                path: "Games/Tunic/TUNIC.exe".to_string(),
                bytes: 42,
                crc: 0xDEAD_BEEF,
            }],
        };

        assert!(read_manifest(scratch.path()).is_none());
        write_manifest(scratch.path(), &manifest).unwrap();
        assert_eq!(read_manifest(scratch.path()).unwrap(), manifest);
    }

    #[test]
    fn recorded_paths_are_relative_and_use_forward_slashes() {
        let root = Path::new("/media/CART");
        assert_eq!(
            relative_path(root, &root.join("Games").join("Tunic").join("TUNIC.exe")).unwrap(),
            "Games/Tunic/TUNIC.exe"
        );
        // A file outside the cartridge has no place in its manifest.
        assert!(relative_path(root, Path::new("/etc/passwd")).is_none());
        assert!(relative_path(root, root).is_none());
    }

    /// Build a manifest from files that are already on disk.
    fn manifest_of(scratch: &crate::testutil::Scratch, paths: &[&str]) -> Manifest {
        let mut digests = Digests::new(scratch.path());
        for path in paths {
            let full = scratch.join(path);
            if let Ok((bytes, crc)) = digest_file(&full) {
                digests.record(&full, bytes, crc);
            }
        }
        digests.into_manifest()
    }

    // ---- SHA-256 and the cartridge digest --------------------------------

    #[test]
    fn sha256_matches_the_published_vectors() {
        // FIPS 180-4 / NIST examples. A hand-written hash that is not checked
        // against these is a hand-written hash that is probably wrong.
        let cases: &[(&[u8], &str)] = &[
            (
                b"",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ];
        for (input, expected) in cases {
            let mut hash = Sha256::new();
            hash.update(input);
            assert_eq!(
                hash.hex(),
                *expected,
                "input {:?}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn sha256_does_not_care_how_the_input_was_split() {
        // The buffering is the part most likely to be wrong: a block boundary
        // landing inside one `update` call must give the same answer.
        let whole = {
            let mut hash = Sha256::new();
            hash.update(&[7u8; 200]);
            hash.hex()
        };
        let piecemeal = {
            let mut hash = Sha256::new();
            for chunk in [1usize, 62, 1, 70, 66].iter() {
                hash.update(&vec![7u8; *chunk]);
            }
            hash.hex()
        };
        assert_eq!(whole, piecemeal);
    }

    #[test]
    fn sha256_matches_an_independent_implementation_over_many_blocks() {
        // `python3 -c "import hashlib; print(hashlib.sha256(b'a'*1000).hexdigest())"`.
        // The vectors above are all shorter than three blocks; this is the one
        // that would catch a message-schedule mistake that only shows up later.
        let mut hash = Sha256::new();
        hash.update(&[b'a'; 1000]);
        assert_eq!(
            hash.hex(),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn sha256_handles_a_length_that_lands_exactly_on_a_block() {
        // 64 bytes in means the padding needs a whole extra block, which is
        // the case an off-by-one in the padding loop gets wrong.
        let mut hash = Sha256::new();
        hash.update(&[0u8; 64]);
        assert_eq!(
            hash.hex(),
            "f5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea9831a92759fb4b"
        );
    }

    #[test]
    fn the_same_files_give_the_same_cartridge_digest() {
        let one = Manifest {
            files: vec![digest("a/x", 1), digest("b/y", 2)],
        };
        // The same cartridge, copied in the other order.
        let two = Manifest {
            files: vec![digest("b/y", 2), digest("a/x", 1)],
        };
        assert_eq!(cartridge_digest(&one), cartridge_digest(&two));
        assert_eq!(cartridge_digest(&one).len(), 64);
    }

    #[test]
    fn one_wrong_byte_gives_a_different_cartridge_digest() {
        let good = Manifest {
            files: vec![digest("a/x", 10)],
        };
        let mut bad = good.clone();
        bad.files[0].crc ^= 1;
        assert_ne!(cartridge_digest(&good), cartridge_digest(&bad));

        let mut shorter = good.clone();
        shorter.files[0].bytes -= 1;
        assert_ne!(cartridge_digest(&good), cartridge_digest(&shorter));

        let mut renamed = good.clone();
        renamed.files[0].path = "a/z".to_string();
        assert_ne!(cartridge_digest(&good), cartridge_digest(&renamed));
    }

    #[test]
    fn an_empty_cartridge_still_has_a_digest() {
        // Not a special case anywhere: a cartridge that carries no files is a
        // key pointing at an installed game, and comparing two of those should
        // work rather than panic.
        assert_eq!(cartridge_digest(&Manifest::default()).len(), 64);
    }

    #[test]
    fn a_declared_digest_is_compared_forgivingly_but_not_loosely() {
        let actual = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

        // Nothing declared is not a failure.
        assert_eq!(digest_matches("", actual), None);
        assert_eq!(digest_matches("   ", actual), None);

        // The whole thing, however it was pasted.
        assert_eq!(digest_matches(actual, actual), Some(true));
        assert_eq!(digest_matches(&actual.to_uppercase(), actual), Some(true));
        assert_eq!(digest_matches(&format!("  {actual}  "), actual), Some(true));

        // A prefix long enough to mean something.
        assert_eq!(digest_matches("abcdef0123", actual), Some(true));
        assert_eq!(digest_matches("abcdef0124", actual), Some(false));

        // Too short to be a claim, and longer than the answer.
        assert_eq!(digest_matches("abcd", actual), Some(false));
        assert_eq!(digest_matches(&format!("{actual}00"), actual), Some(false));
    }

    #[test]
    fn the_short_form_is_a_prefix_of_the_whole() {
        let digest = cartridge_digest(&Manifest {
            files: vec![digest("a/x", 1)],
        });
        let short = short_digest(&digest);
        assert_eq!(short.len(), 12);
        assert!(digest.starts_with(&short));
    }
}
