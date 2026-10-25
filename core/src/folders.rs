//! Games that are just a folder on a disk.
//!
//! Steam is enumerated from its manifests and Playnite from its export, which
//! between them miss everything installed by hand: a GOG folder, an Epic
//! install, a repack, anything copied off an old drive. Those games are already
//! *buildable* — `CartridgeRequest::source_dir` copies a folder and `portable`
//! works out what to run — but nothing ever **found** them, so the wizard could
//! only reach one at a time through its folder picker.
//!
//! This walks a handful of roots and reports each game folder it finds, so they
//! can sit in the same list as the Steam games and be picked the same way.
//!
//! The whole problem is telling a game apart from a folder that merely
//! *contains* games. `F:\Games\Epic\HogwartsLegacy` is a game; `F:\Games` and
//! `F:\Games\Epic` are buckets, and listing either as a game would put a
//! hundred gigabytes behind one checkbox. The rule used here is that a game
//! folder has something of its own at its root — loose files, or one of the
//! standard engine directories — **and** a launchable binary below it; a bucket
//! holds nothing but more directories.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::portable;

/// How far below a root to keep looking for game folders. Three is enough for
/// the deepest layout seen in practice (`F:\Games\Other\Cracked Games\<game>`)
/// without turning a scan into a walk of the whole disk.
const MAX_DEPTH: usize = 3;

/// How far into a candidate to look for its binary when deciding whether it is
/// a game.
///
/// Three, because that is where Unreal puts it: `<Game>/<Project>/Binaries/
/// Win64/<exe>`. At two, such a game looks like a bucket, the scan descends
/// into it, and its project folder is reported as the game — which is how
/// Split Fiction came out as "Split" and It Takes Two as "Nuts".
///
/// Reaching this deep would make a bucket look like a game too, since its games
/// are one level down and many keep their binary at their own root. What stops
/// that is the second half of `is_game_folder`: a bucket has no files and no
/// engine layout of its own, whoever is below it.
const CLASSIFY_DEPTH: usize = 3;

/// Files that say nothing about what a folder is.
const IGNORED_FILES: &[&str] = &["desktop.ini", "thumbs.db", ".ds_store", "autorun.inf"];

/// Directories that are never a game and never worth descending into.
const SKIP_DIRS: &[&str] = &[
    // Steam's own library layout. These games are already listed from Steam's
    // manifests; walking in here would list every one of them twice.
    "steamapps",
    "$recycle.bin",
    "system volume information",
    "$windows.~tmp",
    "windows",
    "windows.old",
    "recovery",
    "wudownloadcache",
    "config.msi",
    // Redistributables and installers shipped alongside a game. These sit
    // *beside* the game's own files, so without this the folder holding both
    // gets scanned and the installer is reported as a game of its own.
    "__installer",
    "_commonredist",
    "commonredist",
    "redist",
    "directx",
    "dotnet",
    "vcredist",
    "support",
    "installers",
    ".egstore",
    // Launcher runtimes that live in the same tree as the games they run.
    "eos",
    "eosbootstrapper",
    "epic online services",
    "overlay",
    // Exactly "launcher": the Epic launcher's own tree sits beside the games it
    // installs. A game *named* "… Launcher" is untouched by this.
    "launcher",
    // The inside of a game, not a game. An engine puts its binary down here
    // (`Binaries/Win64/`), and without this the folder holding it is reported
    // as a game called "Win64".
    "binaries",
    "bin",
    "win64",
    "win32",
    "x64",
    "engine",
];

/// Directories whose presence means the folder holding them is a game, even
/// though it has no loose files of its own — the standard engine layouts.
const STRUCTURAL_DIRS: &[&str] = &[
    "binaries", "engine", "content", "data", "bin", "game", "assets",
];

/// Prefixes of directories that are launcher machinery, not games. Matched as
/// prefixes because they carry a version: `Battle.net.15267`, and a second copy
/// beside it after every update.
const SKIP_PREFIXES: &[&str] = &["battle.net", "epic games launcher", "uplay"];

/// Parent folder names worth reporting as the game's source, and how to write
/// each one out.
const LAUNCHER_NAMES: &[(&str, &str)] = &[
    ("epic", "Epic"),
    ("gog", "GOG"),
    ("origin", "Origin"),
    ("ubisoft", "Ubisoft"),
    ("blizzard", "Blizzard"),
    ("battle.net", "Blizzard"),
    ("ea", "EA"),
    ("itch", "itch.io"),
    ("xboxgames", "Xbox"),
];

/// One game found by scanning a folder root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderGame {
    /// The folder's own name, which is the best available title.
    pub name: String,
    pub path: PathBuf,
    /// A launcher name when the layout implies one, else "Folder".
    pub source: String,
}

/// Roots worth scanning on this machine, before the user edits the list.
///
/// Deliberately generous: finding a folder that holds no games costs one
/// `read_dir`, while missing a root means the user has to know to add it.
pub fn default_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for base in drive_roots() {
        // A cartridge is full of games by definition, and they are copies of
        // games this scan is already finding on the drives they came from.
        // Scanning one would list every game on it a second time — and offer
        // the cartridge in the wizard as a source for building itself.
        if base.join("cartridge.conf").is_file() {
            continue;
        }
        for relative in LAUNCHER_DIRS {
            let candidate = base.join(relative);
            if candidate.is_dir() {
                out.push(candidate);
            }
        }
    }
    out
}

#[cfg(windows)]
const LAUNCHER_DIRS: &[&str] = &[
    "Games",
    "XboxGames",
    "GOG Games",
    "Program Files/Epic Games",
    "Program Files (x86)/Epic Games",
    "Program Files (x86)/GOG Galaxy/Games",
    "Program Files/Origin Games",
    "Program Files (x86)/Origin Games",
    "Program Files/EA Games",
    "Program Files (x86)/EA Games",
    "Program Files (x86)/Ubisoft/Ubisoft Game Launcher/games",
    "Program Files (x86)/Battle.net",
];

#[cfg(not(windows))]
const LAUNCHER_DIRS: &[&str] = &[
    "Games",
    "games",
    "GOG Games",
    ".local/share/Steam/compatdata",
];

/// Every place a `LAUNCHER_DIRS` entry could hang off.
#[cfg(windows)]
fn drive_roots() -> Vec<PathBuf> {
    // 26 `is_dir` calls, against enumerating volumes through the Win32 API for
    // a list that is then thrown away. The empty slots answer immediately.
    (b'A'..=b'Z')
        .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
        .filter(|path| path.is_dir())
        .collect()
}

#[cfg(not(windows))]
fn drive_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        out.push(PathBuf::from(home));
    }
    for shared in ["/mnt", "/media", "/run/media"] {
        let path = PathBuf::from(shared);
        if path.is_dir() {
            // One level down: /mnt itself holds no games, /mnt/games might.
            if let Ok(entries) = std::fs::read_dir(&path) {
                out.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
            }
            out.push(path);
        }
    }
    out
}

/// Every game folder under `roots`, sorted by name.
///
/// A folder reached through two different roots is reported once: the roots are
/// allowed to overlap, since the defaults and whatever the user added are
/// scanned together.
pub fn scan(roots: &[PathBuf]) -> Vec<FolderGame> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for root in roots {
        collect(root, 0, &mut out, &mut seen);
    }

    out.sort_by_key(|game| game.name.to_lowercase());
    out
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<FolderGame>, seen: &mut HashSet<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_skipped(name) {
            continue;
        }

        if is_game_folder(&path) {
            // Canonicalise only here, not while walking: it resolves links and
            // hits the disk, and the answer is only needed to deduplicate.
            let key = path.canonicalize().unwrap_or_else(|_| path.clone());
            if seen.insert(key) {
                out.push(FolderGame {
                    name: name.to_string(),
                    source: source_for(&path),
                    path,
                });
            }
        } else {
            collect(&path, depth + 1, out, seen);
        }
    }
}

fn is_skipped(name: &str) -> bool {
    let lower = name.to_lowercase();
    SKIP_DIRS.contains(&lower.as_str())
        || SKIP_PREFIXES.iter().any(|prefix| lower.starts_with(prefix))
}

/// A game folder has files of its own **and** something launchable below it.
///
/// Either test alone gets it wrong. Files alone would call a bucket with a
/// stray `readme.txt` a game; a launchable alone would call `F:\Games\Epic` a
/// game, because one exists two levels down inside every game it holds.
///
/// A game that keeps nothing at its root is still a game, so a standard engine
/// layout counts in place of loose files. Without that, such a folder reads as
/// a bucket and the scan descends into it and reports its `Binaries` directory
/// as a game called "Win64".
fn is_game_folder(dir: &Path) -> bool {
    // A Steam library root passes both tests below — `libraryfolder.vdf` is a
    // file of its own, and every game it holds is launchable inside it — so it
    // has to be ruled out by name. `SKIP_DIRS` only stops the walk descending
    // into `steamapps`; it says nothing about the folder holding it.
    if dir.join("steamapps").is_dir() {
        return false;
    }
    portable::holds_launchable(dir, CLASSIFY_DEPTH)
        && (has_own_files(dir) || has_structural_child(dir))
}

fn has_structural_child(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_type().is_ok_and(|kind| kind.is_dir())
            && STRUCTURAL_DIRS
                .contains(&entry.file_name().to_string_lossy().to_lowercase().as_str())
    })
}

fn has_own_files(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            return false;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        !IGNORED_FILES.contains(&name.as_str())
    })
}

/// The launcher a game's layout implies, from the folder holding it.
fn source_for(path: &Path) -> String {
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_lowercase();

    LAUNCHER_NAMES
        .iter()
        .find(|(prefix, _)| parent.starts_with(prefix))
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| "Folder".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    fn names(found: &[FolderGame]) -> Vec<&str> {
        found.iter().map(|g| g.name.as_str()).collect()
    }

    #[test]
    fn a_folder_with_its_own_binary_is_a_game() {
        let dir = Scratch::new("folders");
        dir.write("Some Game/game.exe", b"x");

        let found = scan(&[dir.path().to_path_buf()]);

        assert_eq!(names(&found), vec!["Some Game"]);
    }

    #[test]
    fn a_bucket_of_games_is_not_itself_a_game() {
        let dir = Scratch::new("folders");
        dir.write("Epic/Hogwarts/hogwarts.exe", b"x");
        dir.write("Epic/Control/control.exe", b"x");

        let found = scan(&[dir.path().to_path_buf()]);

        // The two games, and not "Epic" — which holds a launchable two levels
        // down and would be listed by a naive depth check.
        assert_eq!(names(&found), vec!["Control", "Hogwarts"]);
    }

    #[test]
    fn the_binary_may_be_a_couple_of_levels_down() {
        let dir = Scratch::new("folders");
        dir.write("Deep Game/readme.txt", b"x");
        dir.write("Deep Game/Binaries/Win64/deep.exe", b"x");

        let found = scan(&[dir.path().to_path_buf()]);

        assert_eq!(names(&found), vec!["Deep Game"]);
    }

    #[test]
    fn steam_libraries_are_left_to_steam() {
        let dir = Scratch::new("folders");
        // The marker file matters: it is a file of the library root's own, so
        // without the `steamapps` check the root itself reads as a game.
        dir.write("Steam/libraryfolder.vdf", b"x");
        dir.write("Steam/steamapps/common/Tomb Raider/tr.exe", b"x");

        assert!(scan(&[dir.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn a_stray_file_does_not_make_a_bucket_a_game() {
        let dir = Scratch::new("folders");
        dir.write("Library/desktop.ini", b"x");
        dir.write("Library/A Game/a.exe", b"x");

        let found = scan(&[dir.path().to_path_buf()]);

        assert_eq!(names(&found), vec!["A Game"]);
    }

    #[test]
    fn an_uninstaller_alone_is_not_a_game() {
        let dir = Scratch::new("folders");
        dir.write("Leftovers/unins000.exe", b"x");

        assert!(scan(&[dir.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn the_holding_folder_names_the_source() {
        let dir = Scratch::new("folders");
        dir.write("Ubisoft/Valhalla/acv.exe", b"x");
        dir.write("Whatever/Repack/game.exe", b"x");

        let found = scan(&[dir.path().to_path_buf()]);

        let sources: Vec<&str> = found.iter().map(|g| g.source.as_str()).collect();
        assert_eq!(sources, vec!["Folder", "Ubisoft"]);
    }

    #[test]
    fn an_unreal_game_is_the_game_not_its_project_folder() {
        // The layout that made Split Fiction list as "Split": the binary is
        // three levels down and the game's own root holds only its uninstaller.
        let dir = Scratch::new("folders");
        dir.write("Games/Split Fiction/unins000.exe", b"x");
        dir.write("Games/Split Fiction/Split/Binaries/Win64/Split.exe", b"x");

        let found = scan(&[dir.path().to_path_buf()]);

        assert_eq!(names(&found), vec!["Split Fiction"]);
    }

    #[test]
    fn a_bucket_stays_a_bucket_when_its_games_run_from_their_own_root() {
        // The other side of the depth used above: reaching three levels down
        // means a bucket can see the binaries of the games inside it.
        let dir = Scratch::new("folders");
        dir.write("Cracked Games/God of War/GoW.exe", b"x");
        dir.write("Cracked Games/It Takes Two/unins000.exe", b"x");
        dir.write(
            "Cracked Games/It Takes Two/Nuts/Binaries/Win64/nuts.exe",
            b"x",
        );

        let found = scan(&[dir.path().to_path_buf()]);

        assert_eq!(names(&found), vec!["God of War", "It Takes Two"]);
    }

    #[test]
    fn the_same_folder_reached_twice_is_listed_once() {
        let dir = Scratch::new("folders");
        dir.write("Games/A Game/a.exe", b"x");

        let found = scan(&[dir.path().to_path_buf(), dir.join("Games")]);

        assert_eq!(names(&found), vec!["A Game"]);
    }
}
