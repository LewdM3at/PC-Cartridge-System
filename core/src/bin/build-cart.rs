//! Build a cartridge from a JSON `CartridgeRequest`, with no wizard.
//!
//! The wizard is the only way to make a cartridge, which is fine for one and
//! tedious for eleven. This runs the same `create::create_cartridge` the
//! wizard's Tauri command runs, against a request read from a file, so a batch
//! of cartridges is a batch of JSON files rather than an evening of clicking.
//!
//!     build-cart plan  cart.json     what formatting would destroy, then stop
//!     build-cart build cart.json     do it
//!
//! `plan` is the default: building is opt-in because formatting is not undoable.
//!
//! A finished build prints its `digest` — one value naming everything that was
//! copied. Put it back into the request as `expectDigest` and every later build
//! of that request is checked against it, so a batch of eleven cartridges is
//! eleven cartridges somebody else can reproduce rather than eleven that merely
//! did not error. The idea is Kazeta's cartridge creator's, which has done this
//! from the start; see `docs/STATUS.md`.

use std::io::Write;

use gamepak_core::create;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut args = {
        // `drives` and `games` take no request file: they report what the
        // wizard would have to choose from, which is the fastest way to see
        // why something is missing from it.
        let peeked: Vec<String> = args.by_ref().collect();
        match peeked.first().map(String::as_str) {
            Some("drives") => {
                for drive in create::target_drives() {
                    println!("{}", serde_json::to_string(&drive).unwrap());
                }
                return;
            }
            Some("games") => {
                match create::list_games(None) {
                    Ok(list) => {
                        for problem in &list.problems {
                            eprintln!("problem: {problem}");
                        }
                        println!("{} games", list.games.len());
                        for game in &list.games {
                            println!("{:?}\t{}\t{}", game.library, game.source, game.name);
                        }
                    }
                    Err(e) => eprintln!("{e}"),
                }
                return;
            }
            Some("health") => {
                // What the launcher's details panel would say about a drive,
                // without opening the launcher to find out.
                for mount in peeked.iter().skip(1) {
                    let health = gamepak_core::health::inspect(mount);
                    println!("{}", serde_json::to_string_pretty(&health).unwrap());
                }
                return;
            }
            Some("query") => {
                for name in peeked.iter().skip(1) {
                    println!("{name}\t->\t{}", create::search_query_for(name));
                }
                return;
            }
            Some("cover") => {
                // Exercises the SteamGridDB fetch the wizard triggers when a
                // folder game with no local art is picked.
                match peeked.get(1) {
                    Some(path) => {
                        let art = create::game_cover(create::Library::Folder, path);
                        match art.split_once(',') {
                            Some((header, data)) => {
                                println!("{header} — {} bytes of base64", data.len())
                            }
                            None => println!("no cover"),
                        }
                    }
                    None => eprintln!("usage: build-cart cover <game folder path>"),
                }
                return;
            }
            _ => peeked.into_iter(),
        }
    };
    let (mode, path) = match (args.next(), args.next()) {
        (Some(mode), Some(path)) => (mode, path),
        (Some(path), None) => ("plan".to_string(), path),
        _ => {
            eprintln!("usage: build-cart [plan|build] <request.json>");
            std::process::exit(2);
        }
    };

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };
    let request: create::CartridgeRequest = match serde_json::from_str(&text) {
        Ok(request) => request,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };

    // What is on the drive now, named before anything touches it. Printed for
    // both modes: the point of `plan` is to read it, and the point of printing
    // it on `build` is that the log says what was overwritten.
    if request.format_drive {
        match create::format_plan(&request.drive_path) {
            Ok(plan) => println!("format plan: {}", serde_json::to_string(&plan).unwrap()),
            Err(e) => {
                eprintln!("cannot read {}: {e}", request.drive_path);
                std::process::exit(1);
            }
        }
    }

    // The other thing a build can destroy, named just as plainly.
    if request.move_game {
        for line in create::move_plan(&request) {
            println!("move plan: {line}");
        }
    }

    if mode != "build" {
        println!("plan only; nothing written. Re-run with `build` to write it.");
        return;
    }

    let mut last = String::new();
    let result = create::create_cartridge(&request, &mut |progress| {
        // One line per step, plus a percentage that overwrites itself, so a
        // 100 GiB copy is watchable without scrolling a terminal off its buffer.
        if progress.step != last {
            println!();
            print!("{}: {}", progress.step, progress.message);
            last = progress.step.to_string();
        }
        if progress.total_bytes > 0 {
            // Capped the way the wizard caps its bar: a game that grows a file
            // while it is being copied can still pass its measured size.
            let percent =
                (progress.done_bytes as f64 / progress.total_bytes as f64 * 100.0).min(100.0);
            print!("\r{}: {} {percent:.1}%", progress.step, progress.message);
        }
        let _ = std::io::stdout().flush();
    });
    println!();

    match result {
        Ok(result) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            // A cartridge that failed its own integrity check is not a
            // cartridge. The library reports this as a warning and still
            // returns the built result, which is right for a wizard showing it
            // to someone — but a script that reads exit status alone would
            // record a good build, so it is an error here.
            if result.verified_ok == Some(false) {
                eprintln!("verification failed: the cartridge is not trustworthy");
                std::process::exit(1);
            }

            // The one value that says whether two people building this request
            // got the same cartridge. Printed on its own line rather than only
            // inside the JSON above, because the point of it is to be read,
            // quoted, and pasted back into the request as `expectDigest`.
            if let Some(digest) = &result.digest {
                println!("digest: {digest}");
                match result.digest_matched {
                    Some(true) => println!("matches the digest this request expected."),
                    Some(false) => {
                        // Same reasoning as the verify failure above: the
                        // warning is in the JSON, and a script reading exit
                        // status alone would otherwise record a good build.
                        eprintln!(
                            "digest mismatch: this is not the cartridge the request describes"
                        );
                        std::process::exit(1);
                    }
                    None => println!(
                        "add \"expectDigest\": \"{digest}\" to the request to have this \
                         checked next time."
                    ),
                }
            }
        }
        Err(e) => {
            eprintln!("failed: {e}");
            std::process::exit(1);
        }
    }
}
