//! Putting a Steam game on the cartridge so Steam plays it *from* the cartridge.
//!
//! Copying `steamapps/common/<game>` onto a drive achieves nothing on its own:
//! `steam://rungameid/<appid>` asks Steam to launch whatever copy Steam knows
//! about, and Steam only knows about folders listed in its own
//! `config/libraryfolders.vdf`. So a cartridge that really holds the game needs
//! three things:
//!
//!   1. `<drive>/steamapps/common/<installdir>` — the game files;
//!   2. `<drive>/steamapps/appmanifest_<appid>.acf` — how Steam recognises it;
//!   3. the drive listed in Steam's `libraryfolders.vdf`.
//!
//! Step 3 edits Steam's own configuration, so the original is copied to
//! `libraryfolders.vdf.bak-cartridge` first and Steam must be closed — it
//! rewrites that file from memory on exit and would undo the change.

use std::path::{Path, PathBuf};

use crate::steam::{self, Kv};

#[derive(Debug)]
pub enum LibraryError {
    SteamNotFound,
    SteamRunning,
    GameNotFound(String),
    NotEnoughSpace { needed: u64, free: u64 },
    Io(String),
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LibraryError::SteamNotFound => write!(f, "Could not find the Steam installation."),
            LibraryError::SteamRunning => write!(
                f,
                "Close Steam first. It rewrites libraryfolders.vdf when it exits, \
                 which would undo the cartridge being registered."
            ),
            LibraryError::GameNotFound(name) => {
                write!(f, "Could not find the installed files for {name}.")
            }
            LibraryError::NotEnoughSpace { needed, free } => write!(
                f,
                "The game needs {} but the cartridge only has {} free.",
                crate::format::human_bytes(*needed),
                crate::format::human_bytes(*free)
            ),
            LibraryError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Where a game's files live, and how big they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledGame {
    pub app_id: String,
    pub name: String,
    /// `…/steamapps/common/<installdir>`
    pub install_path: PathBuf,
    /// `…/steamapps/appmanifest_<appid>.acf`
    pub manifest_path: PathBuf,
    pub size_on_disk: u64,
}

/// Find where Steam actually installed an app, across every library folder.
pub fn locate(steam_root: &Path, app_id: &str) -> Option<InstalledGame> {
    locate_in(steam::library_paths(steam_root), app_id)
}

/// Find an install of an app that is not on `drive`.
///
/// The copy source when writing a cartridge. A cartridge is itself a Steam
/// library, so rewriting one whose game Steam already lists there would find
/// the cartridge's own copy — and copy it onto itself — whenever that library
/// comes first in Steam's list. `None` when the cartridge holds the only copy.
pub fn locate_off(steam_root: &Path, app_id: &str, drive: &Path) -> Option<InstalledGame> {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let drive = canonical(drive);
    locate_in(
        steam::library_paths(steam_root)
            .into_iter()
            .filter(|library| !canonical(library).starts_with(&drive)),
        app_id,
    )
}

fn locate_in(libraries: impl IntoIterator<Item = PathBuf>, app_id: &str) -> Option<InstalledGame> {
    for library in libraries {
        let manifest_path = library.join(format!("appmanifest_{app_id}.acf"));
        let Ok(text) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        let kv = steam::parse_keyvalues(&text);
        let Some(state) = kv.get("AppState") else {
            continue;
        };

        let install_dir = state
            .get("installdir")
            .and_then(Kv::as_str)
            .map(str::to_string)?;
        let install_path = library.join("common").join(&install_dir);
        if !install_path.is_dir() {
            continue;
        }

        return Some(InstalledGame {
            app_id: app_id.to_string(),
            name: state
                .get("name")
                .and_then(Kv::as_str)
                .unwrap_or(&install_dir)
                .to_string(),
            install_path,
            manifest_path,
            size_on_disk: state
                .get("SizeOnDisk")
                .and_then(Kv::as_str)
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0),
        });
    }
    None
}

/// Add a path to Steam's `libraryfolders.vdf`, if it is not already there.
///
/// Returns the file that was written, or `None` when the drive was already
/// registered.
pub fn register_library(steam_root: &Path, drive: &Path) -> Result<Option<PathBuf>, LibraryError> {
    let drive_str = drive.to_string_lossy().to_string();
    let mut written = None;

    for vdf in library_files(steam_root) {
        let text = std::fs::read_to_string(&vdf)
            .map_err(|e| LibraryError::Io(format!("{}: {e}", vdf.display())))?;

        if library_paths_in(&text)
            .iter()
            .any(|p| paths_match(p, &drive_str))
        {
            continue;
        }

        // Keep a copy before touching Steam's own configuration.
        let backup = vdf.with_extension("vdf.bak-cartridge");
        if !backup.exists() {
            std::fs::copy(&vdf, &backup).map_err(|e| {
                LibraryError::Io(format!("could not back up {}: {e}", vdf.display()))
            })?;
        }

        let updated = append_library_entry(&text, &drive_str);
        std::fs::write(&vdf, updated)
            .map_err(|e| LibraryError::Io(format!("could not write {}: {e}", vdf.display())))?;

        written = written.or(Some(vdf));
    }

    Ok(written)
}

/// The folder on a cartridge that holds its Steam library.
///
/// Steam will not accept a drive root as a library — the client says so
/// outright: "New Steam library folder can't be the drive root." Putting
/// `steamapps` at the root of a cartridge therefore produced a library Steam
/// would write into `libraryfolders.vdf` when asked directly, and then scan and
/// find nothing in, leaving the copied game invisible and the original still
/// claiming it.
///
/// Named the way Steam names its own, because it is the same thing and someone
/// looking at the drive should recognise it.
pub const LIBRARY_DIR: &str = "SteamLibrary";

/// Where this cartridge's Steam library lives.
pub fn library_root(drive: &Path) -> PathBuf {
    drive.join(LIBRARY_DIR)
}

/// Move a cartridge written before the library had a folder of its own.
///
/// A rename inside one volume, so it costs nothing however large the game is.
/// Returns whether anything moved.
pub fn migrate_root_library(drive: &Path) -> bool {
    let old = drive.join("steamapps");
    let new = library_root(drive);
    if !old.is_dir() || new.exists() {
        return false;
    }
    std::fs::create_dir_all(&new).is_ok() && std::fs::rename(&old, new.join("steamapps")).is_ok()
}

/// Manifests in other libraries for games this cartridge carries, whose files
/// are not there any more.
///
/// Steam records an app as installed in exactly one library. A game copied to a
/// cartridge is therefore invisible until the library it came from stops
/// claiming it — and deleting the local files does not do that, because the
/// `appmanifest` stays behind. Steam reads it, sees `FullyInstalled` with
/// `FilesMissing`, and offers to download the game again into the library that
/// no longer has it, right past the cartridge holding a perfect copy.
///
/// Only manifests whose `common/<installdir>` is gone are returned. One whose
/// files are still there is a real second install, and choosing between two
/// real copies is not this function's business.
pub fn dead_manifests_elsewhere(steam_root: &Path, drive: &Path) -> Vec<PathBuf> {
    let cartridge = library_root(drive).join("steamapps");
    let ours: Vec<String> = manifest_ids(&cartridge);
    if ours.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for library in steam::library_paths(steam_root) {
        if paths_match(&library.to_string_lossy(), &cartridge.to_string_lossy()) {
            continue;
        }
        for app_id in &ours {
            let manifest = library.join(format!("appmanifest_{app_id}.acf"));
            let Ok(text) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let Some(install_dir) = install_dir_in(&text) else {
                continue;
            };
            if !library.join("common").join(install_dir).is_dir() {
                out.push(manifest);
            }
        }
    }
    out
}

/// Move those manifests aside, and return the ones that moved.
///
/// Renamed rather than deleted: Steam globs `appmanifest_*.acf`, so a file
/// ending in anything else is already invisible to it, and a rename is undone
/// with a rename. Nothing here should be hard to take back — it is somebody's
/// game library.
pub fn retire_manifests(manifests: &[PathBuf]) -> Vec<PathBuf> {
    manifests
        .iter()
        .filter(|manifest| {
            let aside = manifest.with_extension("acf.bak-cartridge");
            std::fs::rename(manifest, &aside).is_ok()
        })
        .cloned()
        .collect()
}

/// The app ids of every manifest in a `steamapps` directory.
fn manifest_ids(steamapps: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(steamapps) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let id = name
                .strip_prefix("appmanifest_")?
                .strip_suffix(".acf")?
                .to_string();
            id.bytes().all(|b| b.is_ascii_digit()).then_some(id)
        })
        .collect()
}

/// The `installdir` a manifest names, which is relative to `common`.
pub(crate) fn install_dir_in(text: &str) -> Option<String> {
    steam::parse_keyvalues(text)
        .get("AppState")?
        .get("installdir")?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.contains(['/', '\\']))
        .map(str::to_string)
}

/// Every `libraryfolders.vdf` this Steam install keeps.
///
/// Steam has moved the file between `steamapps/` and `config/` over the years
/// and now writes both, holding them identical. The one it *reads* at start-up
/// is the modern one under `steamapps/`, so writing only `config/` — which is
/// what this used to do — registered a cartridge in a file Steam does not
/// consult, and the edit was then erased the next time Steam exited and rewrote
/// both from memory. A cartridge could be copied, reported as registered, and
/// still be invisible to Steam, which would go on believing the game lived
/// wherever it was copied from and re-download it if that copy was deleted.
///
/// Both are written now rather than guessing which this client reads.
pub fn library_files(steam_root: &Path) -> Vec<PathBuf> {
    ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"]
        .iter()
        .map(|rel| steam_root.join(rel))
        .filter(|path| path.is_file())
        .collect()
}

/// The `path` of every entry in a `libraryfolders.vdf`.
pub fn library_paths_in(text: &str) -> Vec<String> {
    let kv = steam::parse_keyvalues(text);
    let Some(folders) = kv.get("libraryfolders") else {
        return Vec::new();
    };

    folders
        .entries()
        .iter()
        .filter_map(|(key, value)| match value {
            Kv::Map(_) => value.get("path").and_then(Kv::as_str).map(str::to_string),
            Kv::Str(s) if key.chars().all(|c| c.is_ascii_digit()) => Some(s.clone()),
            Kv::Str(_) => None,
        })
        .collect()
}

/// Compare library paths the way Steam does: separators and trailing slashes
/// are noise, and Windows paths are case-insensitive.
fn paths_match(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

/// Insert a new numbered entry into `libraryfolders.vdf`.
///
/// Written as a text edit rather than by re-serialising the parsed tree: Steam's
/// file carries per-app size bookkeeping that we have no business rewriting, so
/// the safest change is the smallest one.
pub fn append_library_entry(text: &str, drive: &str) -> String {
    let next_index = library_paths_in(text).len();
    // Steam escapes backslashes in this file.
    let escaped = drive.replace('\\', "\\\\");

    let entry = format!(
        "\t\"{next_index}\"\n\t{{\n\t\t\"path\"\t\t\"{escaped}\"\n\t\t\"label\"\t\t\"{CARTRIDGE_LABEL}\"\n\t\t\"contentid\"\t\t\"0\"\n\t\t\"totalsize\"\t\t\"0\"\n\t\t\"apps\"\n\t\t{{\n\t\t}}\n\t}}\n"
    );

    // Insert before the final closing brace of the libraryfolders block.
    match text.rfind('}') {
        Some(index) => {
            let mut out = String::with_capacity(text.len() + entry.len());
            out.push_str(&text[..index]);
            out.push_str(&entry);
            out.push_str(&text[index..]);
            out
        }
        // No structure to preserve: write a whole file.
        None => format!("\"libraryfolders\"\n{{\n{entry}}}\n"),
    }
}

/// Stamped on the library entries this tool adds.
///
/// Steam shows it in the library-folder list, and it is how `unregister_library`
/// knows which entries are ours. Entries are never removed automatically: a
/// cartridge is *supposed* to be unplugged most of the time, so a missing folder
/// is the normal state, not stale cruft.
pub const CARTRIDGE_LABEL: &str = "PC GamePak";

/// Whether `drive` is listed in *every* one of Steam's library files.
///
/// Every, not any: a cartridge listed in one file and missing from the other is
/// a cartridge Steam may or may not see depending on which it reads, and
/// answering "registered" to that is how a game ends up copied to a drive
/// nothing ever launches from. Saying no here means the next registration puts
/// the missing entry back.
pub fn is_registered(steam_root: &Path, drive: &Path) -> bool {
    let files = library_files(steam_root);
    if files.is_empty() {
        return false;
    }

    let drive_str = drive.to_string_lossy().to_string();
    files.iter().all(|vdf| {
        std::fs::read_to_string(vdf).is_ok_and(|text| {
            library_paths_in(&text)
                .iter()
                .any(|p| paths_match(p, &drive_str))
        })
    })
}

/// Remove a cartridge from Steam's library folders.
///
/// Only ever called from an explicit "unregister" action — this is for a
/// cartridge that has been reformatted or repurposed, where the entry would
/// otherwise point at something that is never coming back.
pub fn unregister_library(steam_root: &Path, drive: &Path) -> Result<bool, LibraryError> {
    if steam_is_running() {
        return Err(LibraryError::SteamRunning);
    }

    let drive_str = drive.to_string_lossy().to_string();
    let mut removed = false;

    // Every file it was written to, for the same reason it is written to every
    // file: leaving the entry in one of them puts the cartridge back the next
    // time Steam reads that one.
    for vdf in library_files(steam_root) {
        let text = std::fs::read_to_string(&vdf)
            .map_err(|e| LibraryError::Io(format!("{}: {e}", vdf.display())))?;

        let Some(updated) = remove_library_entry(&text, &drive_str) else {
            continue;
        };

        let backup = vdf.with_extension("vdf.bak-cartridge");
        if !backup.exists() {
            std::fs::copy(&vdf, &backup).map_err(|e| {
                LibraryError::Io(format!("could not back up {}: {e}", vdf.display()))
            })?;
        }
        std::fs::write(&vdf, updated)
            .map_err(|e| LibraryError::Io(format!("could not write {}: {e}", vdf.display())))?;
        removed = true;
    }

    Ok(removed)
}

/// Cut the entry whose `path` matches, and renumber the ones after it.
///
/// Text surgery rather than re-serialising the parsed tree, for the same reason
/// as `append_library_entry`: Steam keeps per-app bookkeeping in this file that
/// we have no business rewriting.
pub fn remove_library_entry(text: &str, drive: &str) -> Option<String> {
    let paths = library_paths_in(text);
    let index = paths.iter().position(|p| paths_match(p, drive))?;

    // Find the `"<index>"` key at entry level, then its brace-matched block.
    let key = format!("\"{index}\"");
    let key_at = text.find(&key)?;
    let open = text[key_at..].find('{')? + key_at;

    let mut depth = 0;
    let mut close = None;
    let bytes = text.as_bytes();
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                // Skip string contents so braces inside them do not count.
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let close = close?;

    // Take the whole line the key sits on, through the closing brace's line.
    let start = text[..key_at].rfind('\n').map(|n| n + 1).unwrap_or(0);
    let end = text[close..]
        .find('\n')
        .map(|n| close + n + 1)
        .unwrap_or(text.len());

    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..start]);
    out.push_str(&text[end..]);

    // Renumber the entries after the hole so the keys stay sequential.
    for old in (index + 1)..paths.len() {
        out = out.replacen(&format!("\"{old}\"\n"), &format!("\"{}\"\n", old - 1), 1);
    }
    Some(out)
}

/// The `"key" { … }` blocks directly inside `text[start..end]`.
///
/// Returned as `(key, open, close)`, the byte offsets of the braces. A real walk
/// of the structure rather than a search for `"3"`: in `libraryfolders.vdf` the
/// same quoted digits are both entry keys and values (`"totalsize" "0"`), so a
/// search finds whichever comes first. Braces inside strings do not count.
fn child_blocks(text: &str, start: usize, end: usize) -> Vec<(String, usize, usize)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0usize;
    // The string most recently seen at this level that has not had a value yet.
    let mut pending_key: Option<String> = None;
    let mut open: Option<(String, usize)> = None;

    let mut i = start;
    while i < end {
        match bytes[i] {
            b'"' => {
                let from = i + 1;
                i += 1;
                while i < end && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if depth == 0 {
                    pending_key = match pending_key.take() {
                        // That was a key and this is its value.
                        Some(_) => None,
                        None => Some(text[from..i.min(end)].to_string()),
                    };
                }
            }
            b'{' => {
                if depth == 0 {
                    open = pending_key.take().map(|key| (key, i));
                }
                depth += 1;
            }
            b'}' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some((key, at)) = open.take() {
                        out.push((key, at, i));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The braces of the `apps` block inside the entry for `library`.
fn apps_block(text: &str, library: &str) -> Option<(usize, usize)> {
    let (_, root_open, root_close) = child_blocks(text, 0, text.len())
        .into_iter()
        .find(|(key, ..)| key == "libraryfolders")?;

    child_blocks(text, root_open + 1, root_close)
        .into_iter()
        .find(|(_, open, close)| {
            let entry = steam::parse_keyvalues(&format!("\"entry\"\n{}", &text[*open..=*close]));
            entry
                .get("entry")
                .and_then(|e| e.get("path"))
                .and_then(Kv::as_str)
                .is_some_and(|path| paths_match(path, library))
        })
        .and_then(|(_, open, close)| {
            child_blocks(text, open + 1, close)
                .into_iter()
                .find(|(key, ..)| key == "apps")
                .map(|(_, open, close)| (open, close))
        })
}

/// Say that `app_id` now lives in the library at `to`, not the one at `from`.
///
/// Steam keeps, per library, the ids it believes are installed there and their
/// sizes. It rebuilds that list when it scans, but a list naming the game in the
/// library it has just been deleted from is exactly the belief a move exists to
/// end, so it is corrected here too rather than left for Steam to notice.
///
/// Line surgery, like the rest of this file. `None` when nothing changed.
pub fn move_app_entry(text: &str, app_id: &str, from: &str, to: &str, size: u64) -> Option<String> {
    let key = format!("\"{app_id}\"");
    let mut out = text.to_string();
    let mut changed = false;

    if let Some((open, close)) = apps_block(&out, from) {
        let inner = &out[open + 1..close];
        if let Some(at) = inner
            .match_indices(&key)
            .map(|(at, _)| open + 1 + at)
            .find(|at| {
                out[..*at]
                    .rsplit('\n')
                    .next()
                    .is_some_and(|before| before.trim().is_empty())
            })
        {
            let start = out[..at].rfind('\n').map(|n| n + 1).unwrap_or(0);
            let end = out[at..].find('\n').map(|n| at + n + 1).unwrap_or(close);
            out.replace_range(start..end, "");
            changed = true;
        }
    }

    if let Some((open, close)) = apps_block(&out, to) {
        let already = out[open + 1..close]
            .lines()
            .any(|line| line.trim_start().starts_with(&key));
        if !already {
            // Before the line holding the closing brace, indented one level in.
            let line_start = out[..close].rfind('\n').map(|n| n + 1).unwrap_or(close);
            let indent: String = out[line_start..close]
                .chars()
                .take_while(|c| c.is_whitespace())
                .collect();
            out.insert_str(line_start, &format!("{indent}\t{key}\t\t\"{size}\"\n"));
            changed = true;
        }
    }

    changed.then_some(out)
}

/// Apply [`move_app_entry`] to every `libraryfolders.vdf`, backing each up first.
pub fn record_moved_app(
    steam_root: &Path,
    app_id: &str,
    from: &Path,
    to: &Path,
    size: u64,
) -> Result<bool, LibraryError> {
    if steam_is_running() {
        return Err(LibraryError::SteamRunning);
    }
    let (from, to) = (from.to_string_lossy(), to.to_string_lossy());
    let mut changed = false;
    for vdf in library_files(steam_root) {
        let text = std::fs::read_to_string(&vdf)
            .map_err(|e| LibraryError::Io(format!("{}: {e}", vdf.display())))?;
        let Some(updated) = move_app_entry(&text, app_id, &from, &to, size) else {
            continue;
        };
        let backup = vdf.with_extension("vdf.bak-cartridge");
        if !backup.exists() {
            std::fs::copy(&vdf, &backup).map_err(|e| {
                LibraryError::Io(format!("could not back up {}: {e}", vdf.display()))
            })?;
        }
        std::fs::write(&vdf, updated)
            .map_err(|e| LibraryError::Io(format!("could not write {}: {e}", vdf.display())))?;
        changed = true;
    }
    Ok(changed)
}

/// The library folder a Steam install belongs to: `…/steamapps/common/<dir>`
/// gives `…`, and anything not shaped like that gives `None`.
///
/// This is the guard in front of a delete, so it is strict. The manifest has to
/// sit in the same `steamapps`, and the install directory has to be a single
/// component directly under `common` — a manifest whose `installdir` walked
/// anywhere else would otherwise name something that is not a game.
pub fn library_of_install(game: &InstalledGame) -> Option<PathBuf> {
    let dir = game.install_path.file_name()?.to_str()?;
    if dir.is_empty() || dir == "." || dir == ".." {
        return None;
    }
    let common = game.install_path.parent()?;
    if !common.file_name()?.eq_ignore_ascii_case("common") {
        return None;
    }
    let steamapps = common.parent()?;
    if !steamapps.file_name()?.eq_ignore_ascii_case("steamapps") {
        return None;
    }
    if game.manifest_path.parent()? != steamapps {
        return None;
    }
    steamapps.parent().map(Path::to_path_buf)
}

/// Whether Steam is currently running.
pub fn steam_is_running() -> bool {
    #[cfg(windows)]
    {
        crate::proc::command("tasklist")
            .args(["/FI", "IMAGENAME eq steam.exe", "/NH"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .to_lowercase()
                    .contains("steam.exe")
            })
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        crate::proc::command("pgrep")
            .args(["-x", "steam"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

/// What came of asking Steam to close.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Shutdown {
    /// It was not running in the first place.
    NotRunning,
    /// It was asked, and it went.
    Exited,
    /// It is still there. The message says what was tried.
    StillRunning(String),
}

/// How long to wait for Steam to finish what it was doing.
///
/// Generous on purpose: Steam flushes downloads, cloud saves and its own
/// configuration on the way out, and hurrying that is how the file we are about
/// to edit gets corrupted.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(25);
const SHUTDOWN_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// Ask Steam to close, and wait for it.
///
/// Steam holds `libraryfolders.vdf` in memory and writes it out when it exits,
/// so any edit made while it runs is silently reverted a few minutes later. The
/// only way to change that file reliably is to have Steam gone first.
///
/// Deliberately not a kill. `-shutdown` is Steam's own "close yourself"
/// argument, which lets it write its state and *then* leave — after which our
/// edit is the last word. Killing it outright would leave whatever it was doing
/// half-finished, and killing it mid-write to its own config is precisely how
/// the file we came to fix gets damaged. If it will not go, that is reported
/// rather than forced.
pub fn shutdown_steam(steam_root: &Path) -> Shutdown {
    if !steam_is_running() {
        return Shutdown::NotRunning;
    }

    let mut asked = false;
    for (program, args) in shutdown_commands(steam_root) {
        if crate::proc::command(&program)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            asked = true;
            break;
        }
    }

    if !asked {
        return Shutdown::StillRunning(
            "Steam is running and its own shutdown command could not be found. \
             Close Steam and try again."
                .to_string(),
        );
    }

    let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(SHUTDOWN_POLL);
        if !steam_is_running() {
            // It writes its configuration on the way out; give the last write a
            // moment to land before reading the file back.
            std::thread::sleep(std::time::Duration::from_millis(750));
            return Shutdown::Exited;
        }
    }

    Shutdown::StillRunning(format!(
        "Steam was asked to close but was still running {} seconds later — it may be mid-download, \
         or waiting on a dialog. Close it yourself and try again.",
        SHUTDOWN_GRACE.as_secs()
    ))
}

/// The ways to ask Steam to close, best first.
///
/// Split out so the argument order can be read and tested without a Steam
/// installation to point at.
pub fn shutdown_commands(steam_root: &Path) -> Vec<(String, Vec<String>)> {
    let shutdown = vec!["-shutdown".to_string()];

    #[cfg(windows)]
    {
        vec![
            (
                steam_root.join("steam.exe").to_string_lossy().into_owned(),
                shutdown.clone(),
            ),
            ("steam.exe".to_string(), shutdown),
        ]
    }
    #[cfg(not(windows))]
    {
        let _ = steam_root;
        vec![
            ("steam".to_string(), shutdown.clone()),
            (
                "flatpak".to_string(),
                vec![
                    "run".to_string(),
                    "com.valvesoftware.Steam".to_string(),
                    "-shutdown".to_string(),
                ],
            ),
        ]
    }
}

/// Copy a directory tree, reporting bytes copied as it goes.
///
/// A game is tens of gigabytes, so the callback lets the window show progress
/// instead of appearing to hang.
pub fn copy_tree(from: &Path, to: &Path, progress: &mut dyn FnMut(u64)) -> std::io::Result<u64> {
    copy_tree_digesting(from, to, None, progress)
}

/// Copy a tree, optionally summing every file on the way past.
///
/// With `digests` the copy goes through a buffer so each file can be summed as
/// it is written; without it, `std::fs::copy` does the work, which lets the
/// platform use whatever fast path it has. The check is opt-in, so the fast
/// path stays the default one.
pub fn copy_tree_digesting(
    from: &Path,
    to: &Path,
    digests: Option<&mut crate::verify::Digests>,
    progress: &mut dyn FnMut(u64),
) -> std::io::Result<u64> {
    // The running total is threaded through the recursion rather than
    // accumulated per level, so `progress` always receives the total copied so
    // far. Reporting a per-directory subtotal would make a progress bar jump
    // backwards every time the walk entered a new folder.
    let mut total = 0u64;
    let mut digests = digests;
    copy_into(from, to, &mut total, &mut digests, progress)?;
    Ok(total)
}

fn copy_into(
    from: &Path,
    to: &Path,
    total: &mut u64,
    digests: &mut Option<&mut crate::verify::Digests>,
    progress: &mut dyn FnMut(u64),
) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;

    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let source = entry.path();
        let destination = to.join(entry.file_name());

        if file_type.is_dir() {
            copy_into(&source, &destination, total, digests, progress)?;
        } else if file_type.is_file() {
            let bytes = match digests {
                Some(digests) => {
                    let (bytes, crc) = crate::verify::copy_and_digest(&source, &destination)?;
                    digests.record(&destination, bytes, crc);
                    bytes
                }
                None => std::fs::copy(&source, &destination)?,
            };
            *total += bytes;
            progress(*total);
        }
        // Symlinks are skipped: WinBtrfs on Windows does not reliably support
        // them, and Steam games on Windows do not rely on them.
    }

    Ok(())
}

/// Total size of a directory tree.
pub fn tree_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(t) if t.is_dir() => tree_size(&entry.path()),
            Ok(t) if t.is_file() => entry.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_is_asked_to_close_itself_rather_than_killed() {
        let commands = shutdown_commands(Path::new("/home/harry/.local/share/Steam"));
        assert!(!commands.is_empty());

        // Every candidate is Steam's own shutdown argument. Nothing in here
        // kills a process: Steam writes its configuration on the way out, and
        // that write is the one that must happen before ours.
        for (program, args) in &commands {
            assert!(
                args.iter().any(|a| a == "-shutdown"),
                "{program} {args:?} is not a shutdown request"
            );
            let lowered = program.to_lowercase();
            assert!(
                !lowered.contains("kill") && !lowered.contains("taskkill"),
                "{program} kills rather than asks"
            );
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn a_flatpak_steam_is_reachable_too() {
        let commands = shutdown_commands(Path::new("/home/harry/.local/share/Steam"));
        assert_eq!(commands[0].0, "steam");
        assert!(
            commands.iter().any(|(program, args)| program == "flatpak"
                && args.iter().any(|a| a == "com.valvesoftware.Steam")),
            "{commands:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_installed_steam_is_preferred_over_whatever_is_on_the_path() {
        let commands = shutdown_commands(Path::new("C:\\Program Files (x86)\\Steam"));
        assert!(commands[0].0.ends_with("Steam\\steam.exe"), "{commands:?}");
    }

    const SAMPLE: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/harry/.local/share/Steam"
		"label"		""
		"apps"
		{
			"367520"		"9106886656"
		}
	}
}
"#;

    #[test]
    fn reads_existing_library_paths() {
        assert_eq!(
            library_paths_in(SAMPLE),
            vec!["/home/harry/.local/share/Steam"]
        );
    }

    #[test]
    fn appends_a_new_entry_and_keeps_the_old_one() {
        let updated = append_library_entry(SAMPLE, "/run/media/harry/CINDER");
        let paths = library_paths_in(&updated);
        assert_eq!(
            paths,
            vec!["/home/harry/.local/share/Steam", "/run/media/harry/CINDER"]
        );
        // The existing app bookkeeping must survive the edit untouched.
        assert!(updated.contains("\"367520\"\t\t\"9106886656\""));
        // And the result must still parse as one libraryfolders block.
        assert_eq!(
            steam::parse_keyvalues(&updated)
                .get("libraryfolders")
                .map(|f| f.entries().len()),
            Some(2)
        );
    }

    #[test]
    fn appends_windows_paths_with_escaped_separators() {
        let updated = append_library_entry(SAMPLE, "D:\\SteamLibrary");
        assert!(updated.contains("D:\\\\SteamLibrary"), "{updated}");
        // Round-trips back to the unescaped form.
        assert!(library_paths_in(&updated).contains(&"D:\\SteamLibrary".to_string()));
    }

    #[test]
    fn only_a_manifest_whose_files_are_gone_is_moved_aside() {
        let scratch = crate::testutil::Scratch::new("steam-dead-manifests");
        let root = scratch.path();

        let manifest = |app_id: &str, install_dir: &str| {
            format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"name\"\t\t\"Game\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"{install_dir}\"\n}}\n"
            )
        };

        // Steam itself, plus a second library on F:, plus the cartridge.
        let library = root.join("F/Steam/steamapps");
        std::fs::create_dir_all(library.join("common")).unwrap();
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();

        let vdf = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            root.join("F/Steam")
                .display()
                .to_string()
                .replace('\\', "\\\\")
        );
        std::fs::write(root.join("steamapps/libraryfolders.vdf"), &vdf).unwrap();
        std::fs::write(root.join("config/libraryfolders.vdf"), &vdf).unwrap();

        // 413150 was copied to the cartridge and its local files deleted; the
        // manifest F: left behind is what keeps Steam pointed at F:.
        std::fs::write(
            library.join("appmanifest_413150.acf"),
            manifest("413150", "Stardew Valley"),
        )
        .unwrap();

        // 220 is a second, real install on F: that nobody has touched.
        std::fs::write(
            library.join("appmanifest_220.acf"),
            manifest("220", "Half-Life 2"),
        )
        .unwrap();
        std::fs::create_dir_all(library.join("common/Half-Life 2")).unwrap();

        // The cartridge's library lives in a folder of its own, because Steam
        // will not read one at a drive root.
        let cart = scratch.path().join("cart");
        let cart_apps = library_root(&cart).join("steamapps");
        std::fs::create_dir_all(cart_apps.join("common/Stardew Valley")).unwrap();
        for id in ["413150", "220"] {
            std::fs::write(
                cart_apps.join(format!("appmanifest_{id}.acf")),
                manifest(id, "whatever"),
            )
            .unwrap();
        }

        let dead = dead_manifests_elsewhere(root, &cart);
        assert_eq!(
            dead,
            vec![library.join("appmanifest_413150.acf")],
            "{dead:?}"
        );

        let moved = retire_manifests(&dead);
        assert_eq!(moved.len(), 1);
        assert!(!library.join("appmanifest_413150.acf").exists());
        // Renamed, not destroyed: Steam only globs .acf, and a rename undoes.
        assert!(library
            .join("appmanifest_413150.acf.bak-cartridge")
            .is_file());
        // The one with its files still there was never in question.
        assert!(library.join("appmanifest_220.acf").is_file());
    }

    #[test]
    fn an_old_cartridge_moves_its_library_off_the_drive_root() {
        let scratch = crate::testutil::Scratch::new("steam-migrate-root");
        let drive = scratch.path();

        // How every cartridge written before this was laid out: steamapps at
        // the root, which is the one place Steam refuses to read a library.
        std::fs::create_dir_all(drive.join("steamapps/common/Stardew Valley")).unwrap();
        std::fs::write(drive.join("steamapps/appmanifest_413150.acf"), "x").unwrap();

        assert!(migrate_root_library(drive));

        let moved = library_root(drive).join("steamapps");
        assert!(moved.join("common/Stardew Valley").is_dir());
        assert!(moved.join("appmanifest_413150.acf").is_file());
        assert!(!drive.join("steamapps").exists());

        // Nothing left to move, and nothing broken by asking twice.
        assert!(!migrate_root_library(drive));
    }

    #[test]
    fn a_cartridge_already_in_the_new_shape_is_left_alone() {
        let scratch = crate::testutil::Scratch::new("steam-migrate-noop");
        let drive = scratch.path();
        std::fs::create_dir_all(library_root(drive).join("steamapps/common")).unwrap();

        assert!(!migrate_root_library(drive));
        assert!(library_root(drive).join("steamapps/common").is_dir());
    }

    #[test]
    fn registration_reaches_the_file_steam_actually_reads() {
        let scratch = crate::testutil::Scratch::new("steam-both-vdfs");
        let root = scratch.path();

        // A modern Steam install keeps the list in both places and holds them
        // identical. It reads the one under steamapps.
        let original =
            "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Steam\"\n\t}\n}\n";
        for rel in ["steamapps", "config"] {
            std::fs::create_dir_all(root.join(rel)).unwrap();
            std::fs::write(root.join(rel).join("libraryfolders.vdf"), original).unwrap();
        }

        let drive = std::path::Path::new("H:\\");
        assert!(!is_registered(root, drive));
        assert!(register_library(root, drive).unwrap().is_some());

        // Both, not just config: writing only config registered the cartridge
        // in a file Steam does not read, so the game was copied and then never
        // launched from — and Steam erased the entry the next time it exited.
        for rel in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
            let text = std::fs::read_to_string(root.join(rel)).unwrap();
            assert!(
                library_paths_in(&text)
                    .iter()
                    .any(|p| paths_match(p, "H:\\")),
                "{rel} was left without the cartridge"
            );
        }
        assert!(is_registered(root, drive));

        // A second pass has nothing left to do.
        assert!(register_library(root, drive).unwrap().is_none());
    }

    #[test]
    fn a_drive_in_only_one_of_the_two_files_is_not_registered() {
        let scratch = crate::testutil::Scratch::new("steam-half-registered");
        let root = scratch.path();

        let bare = "\"libraryfolders\"\n{\n}\n";
        let with_drive =
            "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"H:\\\\\"\n\t}\n}\n";

        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/libraryfolders.vdf"), with_drive).unwrap();
        std::fs::write(root.join("steamapps/libraryfolders.vdf"), bare).unwrap();

        // Listed in one file and missing from the other is not registered: it
        // depends on which file this client reads, and saying yes to that is
        // how a cartridge gets copied to and never launched from.
        let drive = std::path::Path::new("H:\\");
        assert!(!is_registered(root, drive));

        register_library(root, drive).unwrap();
        assert!(is_registered(root, drive));
    }

    #[test]
    fn recognises_an_already_registered_drive() {
        let updated = append_library_entry(SAMPLE, "/run/media/harry/CINDER");
        let existing = library_paths_in(&updated);
        assert!(existing
            .iter()
            .any(|p| paths_match(p, "/run/media/harry/CINDER")));
        // Trailing slash and separator style must not fool it.
        assert!(existing
            .iter()
            .any(|p| paths_match(p, "/run/media/harry/CINDER/")));
        assert!(!existing
            .iter()
            .any(|p| paths_match(p, "/run/media/harry/OTHER")));
    }

    #[test]
    fn windows_path_comparison_ignores_case_and_separators() {
        assert!(paths_match("D:\\SteamLibrary", "d:/steamlibrary"));
        assert!(paths_match("D:\\SteamLibrary\\", "D:\\SteamLibrary"));
        assert!(!paths_match("D:\\SteamLibrary", "E:\\SteamLibrary"));
    }

    #[test]
    fn writes_a_whole_file_when_there_is_nothing_to_preserve() {
        let out = append_library_entry("", "/mnt/cart");
        assert_eq!(library_paths_in(&out), vec!["/mnt/cart"]);
    }

    #[test]
    fn locates_an_installed_game_across_libraries() {
        let scratch = crate::testutil::Scratch::new("locate");
        let second = scratch.join("second");
        std::fs::create_dir_all(scratch.join("steamapps/common")).unwrap();
        std::fs::create_dir_all(second.join("steamapps/common/Hollow Knight")).unwrap();

        std::fs::write(
            scratch.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
                second.display()
            ),
        )
        .unwrap();
        std::fs::write(
            second.join("steamapps/appmanifest_367520.acf"),
            r#""AppState" { "appid" "367520" "name" "Hollow Knight"
                "installdir" "Hollow Knight" "StateFlags" "4" "SizeOnDisk" "9106886656" }"#,
        )
        .unwrap();

        let game = locate(scratch.path(), "367520").expect("found in the second library");
        assert_eq!(game.name, "Hollow Knight");
        assert_eq!(game.size_on_disk, 9_106_886_656);
        assert_eq!(
            game.install_path,
            second.join("steamapps/common/Hollow Knight")
        );

        assert_eq!(locate(scratch.path(), "999999"), None);
    }

    #[test]
    fn the_copy_source_is_never_the_cartridge_it_is_copied_to() {
        // The cartridge is listed first, the way a freshly registered one can
        // be, and both it and the PC hold the game.
        let scratch = crate::testutil::Scratch::new("locate-off");
        let cart = scratch.join("CART");
        let pc = scratch.join("pc");
        let manifest = r#""AppState" { "appid" "212680" "name" "FTL"
            "installdir" "FTL" "StateFlags" "4" }"#;
        for library in [cart.join("SteamLibrary"), pc.clone()] {
            std::fs::create_dir_all(library.join("steamapps/common/FTL")).unwrap();
            std::fs::write(library.join("steamapps/appmanifest_212680.acf"), manifest).unwrap();
        }
        std::fs::create_dir_all(scratch.join("steamapps")).unwrap();
        std::fs::write(
            scratch.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
                cart.join("SteamLibrary").display(),
                pc.display()
            ),
        )
        .unwrap();

        assert!(locate(scratch.path(), "212680")
            .unwrap()
            .install_path
            .starts_with(&cart));
        let source = locate_off(scratch.path(), "212680", &cart).expect("the PC's copy");
        assert_eq!(source.install_path, pc.join("steamapps/common/FTL"));

        // With the PC's copy gone, the cartridge is all there is: no source.
        std::fs::remove_dir_all(pc.join("steamapps/common/FTL")).unwrap();
        assert_eq!(locate_off(scratch.path(), "212680", &cart), None);
    }

    #[test]
    fn does_not_locate_a_game_whose_files_are_gone() {
        let scratch = crate::testutil::Scratch::new("missing-files");
        std::fs::create_dir_all(scratch.join("steamapps")).unwrap();
        std::fs::write(
            scratch.join("steamapps/appmanifest_1.acf"),
            r#""AppState" { "appid" "1" "name" "Ghost" "installdir" "Ghost" "StateFlags" "4" }"#,
        )
        .unwrap();
        // The manifest exists but common/Ghost does not.
        assert_eq!(locate(scratch.path(), "1"), None);
    }

    #[test]
    fn copies_a_tree_and_reports_progress() {
        let scratch = crate::testutil::Scratch::new("copy");
        let from = scratch.join("from");
        let to = scratch.join("to");
        std::fs::create_dir_all(from.join("bin/data")).unwrap();
        std::fs::write(from.join("game.exe"), vec![b'x'; 100]).unwrap();
        std::fs::write(from.join("bin/lib.dll"), vec![b'y'; 250]).unwrap();
        std::fs::write(from.join("bin/data/pack.bin"), vec![b'z'; 650]).unwrap();

        let mut reports = Vec::new();
        let copied = copy_tree(&from, &to, &mut |n| reports.push(n)).unwrap();

        assert_eq!(copied, 1000);
        assert_eq!(tree_size(&to), 1000);
        assert!(to.join("bin/data/pack.bin").is_file());
        // Progress is reported per file and never goes backwards.
        assert_eq!(reports.len(), 3);
        assert!(reports.windows(2).all(|w| w[1] >= w[0]), "{reports:?}");
        assert_eq!(*reports.last().unwrap(), 1000);
    }

    #[test]
    fn tree_size_of_a_missing_directory_is_zero() {
        assert_eq!(tree_size(Path::new("/definitely/not/here")), 0);
    }
}

#[cfg(test)]
mod unregister_tests {
    use super::*;

    const TWO: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/harry/.local/share/Steam"
		"label"		""
		"apps"
		{
			"367520"		"9106886656"
		}
	}
	"1"
	{
		"path"		"/run/media/harry/CINDER"
		"label"		"PC GamePak"
		"apps"
		{
		}
	}
	"2"
	{
		"path"		"/mnt/games/SteamLibrary"
		"label"		""
		"apps"
		{
			"1145360"		"15204593664"
		}
	}
}
"#;

    #[test]
    fn new_entries_carry_the_cartridge_label() {
        let updated = append_library_entry(TWO, "/run/media/harry/HOLLOW");
        assert!(updated.contains(CARTRIDGE_LABEL), "{updated}");
    }

    #[test]
    fn removes_the_matching_entry_and_keeps_the_others() {
        let out = remove_library_entry(TWO, "/run/media/harry/CINDER").expect("found");
        let paths = library_paths_in(&out);
        assert_eq!(
            paths,
            vec!["/home/harry/.local/share/Steam", "/mnt/games/SteamLibrary"]
        );
        // The surviving libraries keep their app bookkeeping untouched.
        assert!(out.contains("\"367520\"\t\t\"9106886656\""));
        assert!(out.contains("\"1145360\"\t\t\"15204593664\""));
        // And the keys are renumbered, so no gap is left behind.
        assert!(out.contains("\"1\"") && !out.contains("\"2\""), "{out}");
    }

    #[test]
    fn removing_the_last_entry_works() {
        let out = remove_library_entry(TWO, "/mnt/games/SteamLibrary").expect("found");
        assert_eq!(
            library_paths_in(&out),
            vec!["/home/harry/.local/share/Steam", "/run/media/harry/CINDER"]
        );
    }

    #[test]
    fn a_drive_that_is_not_listed_is_left_alone() {
        assert_eq!(remove_library_entry(TWO, "/run/media/harry/NOPE"), None);
    }

    #[test]
    fn the_result_still_parses_as_one_block() {
        let out = remove_library_entry(TWO, "/run/media/harry/CINDER").unwrap();
        let parsed = steam::parse_keyvalues(&out);
        assert_eq!(
            parsed.get("libraryfolders").map(|f| f.entries().len()),
            Some(2)
        );
    }

    #[test]
    fn round_trips_through_add_and_remove() {
        let added = append_library_entry(TWO, "/run/media/harry/HOLLOW");
        assert_eq!(library_paths_in(&added).len(), 4);
        let removed = remove_library_entry(&added, "/run/media/harry/HOLLOW").unwrap();
        assert_eq!(library_paths_in(&removed), library_paths_in(TWO));
    }

    /// Apps named by id under whichever library holds them.
    fn apps_of(text: &str, library: &str) -> Vec<String> {
        let (open, close) = apps_block(text, library).expect("library has an apps block");
        steam::parse_keyvalues(&format!("\"apps\"\n{}", &text[open..=close]))
            .get("apps")
            .map(|apps| apps.entries().iter().map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_moved_app_leaves_its_old_library_and_joins_the_cartridge() {
        let out = move_app_entry(
            TWO,
            "367520",
            "/home/harry/.local/share/Steam",
            "/run/media/harry/CINDER",
            9_106_886_656,
        )
        .expect("changed");
        assert!(
            apps_of(&out, "/home/harry/.local/share/Steam").is_empty(),
            "{out}"
        );
        assert_eq!(apps_of(&out, "/run/media/harry/CINDER"), vec!["367520"]);
        assert_eq!(apps_of(&out, "/mnt/games/SteamLibrary"), vec!["1145360"]);
        // Still one well-formed file with three libraries in it.
        assert_eq!(library_paths_in(&out), library_paths_in(TWO));
        assert!(out.contains("\"367520\"\t\t\"9106886656\""), "{out}");
    }

    #[test]
    fn digits_that_are_values_are_not_mistaken_for_entry_keys() {
        // The shape of this host's real file: "0" and "3" appear as values
        // before the entries that use them as keys.
        // Backslashes escaped the way Steam writes them.
        let text = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"F:\\Games\\Steam"
		"totalsize"		"0"
		"update_clean_bytes_tally"		"3"
		"apps"
		{
			"212680"		"286586003"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"label"		"PC GamePak"
		"apps"
		{
		}
	}
}
"#;
        let (from, to) = (r"F:\Games\Steam", r"D:\SteamLibrary");
        let out = move_app_entry(text, "212680", from, to, 287_269_333).expect("changed");
        assert!(apps_of(&out, from).is_empty(), "{out}");
        assert_eq!(apps_of(&out, to), vec!["212680"]);
        assert!(out.contains("\t\t\t\"212680\"\t\t\"287269333\"\n"), "{out}");
        // Moving it again changes nothing.
        assert_eq!(move_app_entry(&out, "212680", from, to, 1), None);
    }

    #[test]
    fn only_a_real_steam_install_has_a_library_to_delete_from() {
        let game = |install: &str, manifest: &str| InstalledGame {
            app_id: "212680".into(),
            name: "FTL".into(),
            install_path: PathBuf::from(install),
            manifest_path: PathBuf::from(manifest),
            size_on_disk: 0,
        };
        assert_eq!(
            library_of_install(&game(
                "/games/Steam/steamapps/common/FTL",
                "/games/Steam/steamapps/appmanifest_212680.acf"
            )),
            Some(PathBuf::from("/games/Steam"))
        );
        // Not under common.
        assert_eq!(
            library_of_install(&game(
                "/games/Steam/steamapps/FTL",
                "/games/Steam/steamapps/appmanifest_212680.acf"
            )),
            None
        );
        // The manifest belongs to some other library.
        assert_eq!(
            library_of_install(&game(
                "/games/Steam/steamapps/common/FTL",
                "/elsewhere/steamapps/appmanifest_212680.acf"
            )),
            None
        );
        // `common` itself, which would be every game in the library.
        assert_eq!(
            library_of_install(&game(
                "/games/Steam/steamapps/common",
                "/games/Steam/steamapps/appmanifest_212680.acf"
            )),
            None
        );
    }
}
