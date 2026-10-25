//! Check a cartridge you already have.
//!
//! The manifest is written and checked at copy time, but until now nothing
//! re-checked a cartridge on demand — so a drive that was written over a
//! failing cable had no way to answer the only question that matters, which is
//! whether the bytes on it are still the bytes that were meant to be there.
//!
//! Everything here is read-only. It reads `.gamepak/manifest.json`, reads every
//! file it names, and compares. It never writes to the cartridge.
//!
//!     verify-cart D:\        check every file against the manifest
//!
//! Exit status is 0 when the cartridge is intact and 1 when it is not, so this
//! can sit in a script.
//!
//! It also prints the cartridge's digest — the same value `build-cart` reports —
//! which answers the question verifying cannot: not "did these bytes survive"
//! but "is this the same cartridge somebody else built".

use std::path::Path;
use std::time::Instant;

use gamepak_core::verify;

fn main() {
    let root = match std::env::args().nth(1) {
        Some(arg) => arg,
        None => {
            eprintln!("usage: verify-cart <cartridge root>");
            std::process::exit(2);
        }
    };
    let root = Path::new(&root);

    let manifest = match verify::read_manifest(root) {
        Some(manifest) => manifest,
        None => {
            eprintln!(
                "no manifest at {}",
                root.join(verify::MANIFEST_PATH).display()
            );
            eprintln!("this cartridge was written without the verify option, so there is nothing to check it against.");
            std::process::exit(2);
        }
    };

    let total = manifest.total_bytes();
    println!(
        "{} files, {:.2} GB, from {}",
        manifest.files.len(),
        total as f64 / 1e9,
        root.display()
    );

    // Printed before the read starts, because it costs nothing and answers a
    // different question to the one below: the check that follows says whether
    // this cartridge still matches its own manifest, and this says which
    // cartridge it is. Two people comparing this line know whether they built
    // the same thing, which no amount of local verifying can tell them.
    println!("digest: {}", verify::cartridge_digest(&manifest));

    // Progress is reported per file, which is bursty on a cartridge holding one
    // 2 GB archive and a hundred small DLLs. Rate-limiting to once a second
    // keeps it readable without hiding a stall.
    let started = Instant::now();
    let mut last_print = Instant::now();
    let problems = verify::verify(root, &manifest, &mut |done, total| {
        if last_print.elapsed().as_millis() < 1000 && done < total {
            return;
        }
        last_print = Instant::now();
        let seconds = started.elapsed().as_secs_f64();
        let rate = if seconds > 0.0 {
            done as f64 / seconds / 1e6
        } else {
            0.0
        };
        let percent = if total > 0 {
            done as f64 / total as f64 * 100.0
        } else {
            100.0
        };
        println!(
            "  {percent:5.1}%  {:.2} / {:.2} GB  {rate:.0} MB/s",
            done as f64 / 1e9,
            total as f64 / 1e9,
        );
    });

    let seconds = started.elapsed().as_secs_f64();
    println!(
        "read {:.2} GB in {seconds:.0}s ({:.0} MB/s)",
        total as f64 / 1e9,
        total as f64 / seconds / 1e6
    );

    if problems.is_empty() {
        println!("\nintact: every file matches the manifest.");
        return;
    }

    // Grouped, because a cable that dropped out mid-copy produces hundreds of
    // problems and the shape of them says more than the list does.
    let mut missing = 0;
    let mut truncated = 0;
    let mut corrupt = 0;
    let mut unreadable = 0;
    for problem in &problems {
        match problem {
            verify::Problem::Missing(_) => missing += 1,
            verify::Problem::Truncated { .. } => truncated += 1,
            verify::Problem::Corrupt(_) => corrupt += 1,
            verify::Problem::Unreadable { .. } => unreadable += 1,
        }
    }

    println!("\n{} problems:", problems.len());
    if missing > 0 {
        println!("  {missing} missing");
    }
    if truncated > 0 {
        println!("  {truncated} truncated");
    }
    if corrupt > 0 {
        println!("  {corrupt} corrupt");
    }
    if unreadable > 0 {
        println!("  {unreadable} unreadable");
    }
    println!();
    for problem in problems.iter().take(50) {
        println!("  {}", problem.describe());
    }
    if problems.len() > 50 {
        println!("  ... and {} more", problems.len() - 50);
    }

    std::process::exit(1);
}
