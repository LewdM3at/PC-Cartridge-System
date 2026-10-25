//! Copying a game that carries its own files.
//!
//! Steam games get onto a cartridge through `steamlib`, which registers the
//! drive as a Steam library. Everything else — GOG, itch, emulator builds,
//! anything installed into a folder — just needs the folder copied and Play
//! pointed at a file inside it. That is the case this module handles.
//!
//! The hard part is not the copy, it is knowing *which* file to run. A game
//! directory routinely holds a dozen executables: the game, an unwanted
//! launcher, an uninstaller, three redistributables and a crash handler. This
//! ranks the candidates so the obvious one is offered first, and lets the user
//! override it.

use std::path::Path;

use serde::Serialize;

/// How deep to look. Games put their binary at the root or one or two levels
/// down (`bin/`, `Binaries/Win64/`); past that it is libraries and assets.
const MAX_DEPTH: usize = 4;

/// Stop after this many candidates, so a pathological directory cannot make the
/// wizard hang.
const MAX_CANDIDATES: usize = 400;

/// A file on the cartridge that Play could point at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// Path relative to the game folder, using forward slashes.
    pub relative: String,
    /// Just the filename, for the picker.
    pub name: String,
    /// Higher is a better guess. Only meaningful relative to its siblings.
    pub score: i32,
}

/// Filenames that are almost never the game.
const NOISE: [&str; 22] = [
    "unins",
    "uninstall",
    "setup",
    "install",
    "vcredist",
    "vc_redist",
    "dxsetup",
    "dxwebsetup",
    "directx",
    "dotnetfx",
    "ndp",
    "oalinst",
    "openal",
    "physx",
    "crashhandler",
    "crashreport",
    "crashpad",
    "unitycrashhandler",
    "ueprereqsetup",
    "epicgameslauncher",
    "touchup",
    "quicksfv",
];

/// Directories whose contents are support files, not the game.
const NOISE_DIRS: [&str; 8] = [
    "_commonredist",
    "commonredist",
    "redist",
    "directx",
    "dotnet",
    "vcredist",
    "__installer",
    "support",
];

/// Extensions worth offering, per platform convention.
fn is_launchable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();

    if lower.ends_with(".exe") || lower.ends_with(".bat") || lower.ends_with(".cmd") {
        return true;
    }
    // Linux builds: a shell script, a Unity/Unreal binary, or an AppImage.
    if lower.ends_with(".sh") || lower.ends_with(".appimage") || lower.ends_with(".x86_64") {
        return true;
    }
    // A file with no extension at all is often the Linux binary.
    !lower.contains('.')
}

/// Normalise a name for comparison: lowercase, letters and digits only.
fn normalise(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// Rank one candidate against the game's title and Playnite's play action.
fn score(relative: &str, title: &str, play_action: Option<&str>) -> i32 {
    let path = Path::new(relative);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let lower = relative.to_lowercase();
    let depth = path.components().count().saturating_sub(1);

    let mut score = 0;

    // Playnite already knows what it launches; nothing else comes close.
    if let Some(action) = play_action {
        let action_name = Path::new(&action.replace('\\', "/"))
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(action)
            .to_lowercase();
        if !action_name.is_empty()
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_lowercase() == action_name)
                .unwrap_or(false)
        {
            score += 1000;
        }
    }

    // A binary named after the game is the next best signal.
    let title_key = normalise(title);
    let stem_key = normalise(&stem);
    if !title_key.is_empty() && !stem_key.is_empty() {
        if stem_key == title_key {
            score += 120;
        } else if stem_key.contains(&title_key) || title_key.contains(&stem_key) {
            score += 60;
        }
    }

    // Shallow beats deep: the launcher usually sits at the top.
    score -= (depth as i32) * 12;
    if lower.contains("/bin/") || lower.starts_with("bin/") {
        score += 15;
    }

    // Known noise, by filename and by directory.
    if NOISE
        .iter()
        .any(|n| stem.starts_with(n) || stem.contains(n))
    {
        score -= 400;
    }
    if NOISE_DIRS
        .iter()
        .any(|d| lower.split('/').any(|part| part == *d))
    {
        score -= 400;
    }
    // A "launcher" is plausible but usually not what you want on a cartridge.
    if stem.contains("launcher") {
        score -= 40;
    }
    if stem.contains("server") || stem.contains("editor") || stem.contains("benchmark") {
        score -= 80;
    }

    score
}

/// Find the files in `dir` that could be the game, best guess first.
pub fn find_executables(dir: &Path, title: &str, play_action: Option<&str>) -> Vec<Candidate> {
    let mut found = Vec::new();
    walk(dir, dir, 0, &mut found);

    let mut candidates: Vec<Candidate> = found
        .into_iter()
        .map(|relative| Candidate {
            score: score(&relative, title, play_action),
            name: Path::new(&relative)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&relative)
                .to_string(),
            relative,
        })
        .collect();

    // Best first; ties broken by path so the order is stable between runs.
    candidates.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.relative.cmp(&b.relative))
    });
    candidates
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_DEPTH || out.len() >= MAX_CANDIDATES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if out.len() >= MAX_CANDIDATES {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };

        if kind.is_dir() {
            walk(root, &path, depth + 1, out);
        } else if kind.is_file() && is_launchable(&path) {
            if let Ok(relative) = path.strip_prefix(root) {
                // Forward slashes throughout: this ends up in cartridge.conf,
                // which is read on both platforms.
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// Whether `dir` holds something that could be a game, looking no more than
/// `max_depth` levels down and stopping at the first hit.
///
/// `find_executables` answers this properly, but it walks four levels and
/// ranks everything it finds. A folder scan asks this question of every
/// directory on a disk, so it needs an answer that costs a few `read_dir`
/// calls rather than a full walk of each one.
pub fn holds_launchable(dir: &Path, max_depth: usize) -> bool {
    probe(dir, 0, max_depth)
}

fn probe(dir: &Path, depth: usize, max_depth: usize) -> bool {
    if depth > max_depth {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };

    // Files first, directories after: a game's binary is far more often at the
    // top of its folder than buried, so checking files as they come avoids
    // descending at all in the common case.
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };

        if kind.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if !NOISE_DIRS.contains(&name.as_str()) {
                subdirs.push(path);
            }
            continue;
        }
        if !kind.is_file() || !is_launchable(&path) {
            continue;
        }
        // `is_launchable` accepts an extensionless file because that is how a
        // Linux build ships. On Windows that would make every folder holding a
        // LICENSE or a README look like a game, so there it must have one.
        if cfg!(windows) && path.extension().is_none() {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if !NOISE.iter().any(|noise| stem.starts_with(noise)) {
            return true;
        }
    }

    subdirs.iter().any(|sub| probe(sub, depth + 1, max_depth))
}

/// Total size of a game folder, for the space check before copying.
pub fn tree_size_of(path: &Path) -> u64 {
    crate::steamlib::tree_size(path)
}

/// A folder name safe to create on a btrfs cartridge (and readable on Windows).
pub fn safe_folder_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            // Characters Windows refuses.
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // Trailing dots and spaces are legal in the string but not on disk.
    let trimmed = cleaned.trim_end_matches(['.', ' ']).trim();
    let capped: String = trimmed.chars().take(64).collect();
    let capped = capped.trim_end_matches(['.', ' ']).trim().to_string();

    if capped.is_empty() {
        "Game".to_string()
    } else {
        capped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    fn candidates(files: &[&str], title: &str, action: Option<&str>) -> Vec<Candidate> {
        let scratch = Scratch::new("portable");
        for f in files {
            scratch.write(f, b"x");
        }
        find_executables(scratch.path(), title, action)
    }

    #[test]
    fn picks_the_binary_named_after_the_game() {
        let found = candidates(
            &[
                "HollowKnight.exe",
                "UnityCrashHandler64.exe",
                "unins000.exe",
            ],
            "Hollow Knight",
            None,
        );
        assert_eq!(found[0].name, "HollowKnight.exe");
    }

    #[test]
    fn the_play_action_wins_over_everything() {
        // Even when another file matches the title better.
        let found = candidates(
            &["Hollow Knight.exe", "bin/start.exe"],
            "Hollow Knight",
            Some("D:\\Games\\HK\\bin\\start.exe"),
        );
        assert_eq!(found[0].relative, "bin/start.exe");
    }

    #[test]
    fn uninstallers_and_redistributables_sink() {
        let found = candidates(
            &[
                "Game.exe",
                "unins000.exe",
                "_CommonRedist/vcredist_x64.exe",
                "DXSETUP.exe",
                "redist/dotnetfx45.exe",
            ],
            "Game",
            None,
        );
        assert_eq!(found[0].name, "Game.exe");
        // Everything else scores below it, and below zero.
        for c in &found[1..] {
            assert!(c.score < 0, "{} scored {}", c.relative, c.score);
        }
    }

    #[test]
    fn a_launcher_ranks_below_the_game_itself() {
        let found = candidates(&["MyGame.exe", "MyGameLauncher.exe"], "My Game", None);
        assert_eq!(found[0].name, "MyGame.exe");
    }

    #[test]
    fn shallower_wins_when_nothing_else_separates_them() {
        let found = candidates(&["run.exe", "a/b/c/run.exe"], "Nothing Matches", None);
        assert_eq!(found[0].relative, "run.exe");
    }

    #[test]
    fn finds_linux_style_binaries() {
        let found = candidates(&["start.sh", "Game.x86_64", "readme.txt"], "Game", None);
        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"start.sh"));
        assert!(names.contains(&"Game.x86_64"));
        assert!(!names.contains(&"readme.txt"));
    }

    #[test]
    fn ignores_files_that_are_not_launchable() {
        let found = candidates(
            &["data.pak", "art.png", "config.ini", "Game.exe"],
            "Game",
            None,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Game.exe");
    }

    #[test]
    fn does_not_descend_forever() {
        let deep = "a/b/c/d/e/f/g/buried.exe";
        let found = candidates(&["top.exe", deep], "Buried", None);
        assert!(found.iter().all(|c| c.relative != deep), "{found:?}");
    }

    #[test]
    fn an_empty_directory_yields_nothing_rather_than_failing() {
        let scratch = Scratch::new("empty-portable");
        assert!(find_executables(scratch.path(), "Anything", None).is_empty());
        assert!(find_executables(Path::new("/definitely/not/here"), "x", None).is_empty());
    }

    #[test]
    fn ordering_is_stable_between_runs() {
        let files = ["b.exe", "a.exe", "c.exe"];
        let first = candidates(&files, "Nothing", None);
        let second = candidates(&files, "Nothing", None);
        assert_eq!(first, second);
    }

    #[test]
    fn folder_names_are_safe_for_btrfs_and_windows() {
        assert_eq!(safe_folder_name("Hollow Knight"), "Hollow Knight");
        assert_eq!(
            safe_folder_name("Ori: and the Blind Forest"),
            "Ori and the Blind Forest"
        );
        // Only the characters Windows actually refuses are stripped; "!" is legal
        // and stays.
        assert_eq!(safe_folder_name("What?! *really*"), "What ! really");
        assert_eq!(safe_folder_name("A|B*C?D"), "A B C D");
        assert_eq!(safe_folder_name("a/b\\c"), "a b c");
        assert_eq!(safe_folder_name("trailing dots..."), "trailing dots");
        assert_eq!(safe_folder_name("   "), "Game");
        assert_eq!(safe_folder_name(""), "Game");
        assert!(safe_folder_name(&"x".repeat(200)).len() <= 64);
    }
}
