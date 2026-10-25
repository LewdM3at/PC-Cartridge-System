//! Finding installed Steam games, so the wizard can offer a list instead of
//! asking someone to type an app id.
//!
//! Everything here reads files Steam already keeps on disk. Nothing is fetched:
//! the app list comes from `appmanifest_*.acf`, and the cover art comes from
//! Steam's own `librarycache`, which is why a cartridge can be made offline.
//!
//! std only, no Tauri, so it can be tested on its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// An installed game the wizard can put on a cartridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamGame {
    pub app_id: String,
    pub name: String,
    /// Portrait library art, when Steam has cached it.
    pub cover_path: Option<PathBuf>,
    pub size_on_disk: u64,
}

/// Valve's KeyValues format, used by both `.vdf` and `.acf`.
///
/// Children are kept in a `Vec` rather than a map: the format allows repeated
/// keys, and discarding duplicates silently would be a way to lose libraries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kv {
    Str(String),
    Map(Vec<(String, Kv)>),
}

impl Kv {
    /// Direct child by key, case-insensitively — Steam is inconsistent about
    /// capitalising `name`, `AppState` and friends.
    pub fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            Kv::Map(entries) => entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Kv::Str(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Kv::Str(s) => Some(s),
            Kv::Map(_) => None,
        }
    }

    /// Follow a chain of keys.
    pub fn path(&self, keys: &[&str]) -> Option<&Kv> {
        keys.iter().try_fold(self, |node, key| node.get(key))
    }

    pub fn entries(&self) -> &[(String, Kv)] {
        match self {
            Kv::Map(entries) => entries,
            Kv::Str(_) => &[],
        }
    }
}

/// Parse KeyValues text.
///
/// Lenient by design: these files are written by Steam and read by everyone, so
/// a stray token should cost one entry rather than the whole list.
pub fn parse_keyvalues(src: &str) -> Kv {
    let mut chars = src.chars().peekable();
    Kv::Map(parse_entries(&mut chars, 0))
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// Guard against a malformed file nesting deeply enough to blow the stack.
const MAX_DEPTH: usize = 64;

fn parse_entries(chars: &mut Chars, depth: usize) -> Vec<(String, Kv)> {
    let mut out = Vec::new();

    loop {
        skip_trivia(chars);
        match chars.peek() {
            None => break,
            Some('}') => {
                chars.next();
                break;
            }
            _ => {}
        }

        let Some(key) = read_token(chars) else { break };

        skip_trivia(chars);
        match chars.peek() {
            Some('{') => {
                chars.next();
                let children = if depth >= MAX_DEPTH {
                    // Too deep: consume the block and record it as empty.
                    skip_block(chars);
                    Vec::new()
                } else {
                    parse_entries(chars, depth + 1)
                };
                out.push((key, Kv::Map(children)));
            }
            Some(_) => {
                if let Some(value) = read_token(chars) {
                    out.push((key, Kv::Str(value)));
                }
            }
            None => break,
        }
    }

    out
}

/// Skip whitespace and `//` comments.
fn skip_trivia(chars: &mut Chars) {
    loop {
        match chars.peek() {
            Some(c) if c.is_whitespace() => {
                chars.next();
            }
            Some('/') => {
                // Only a doubled slash is a comment; a lone one is data.
                let mut probe = chars.clone();
                probe.next();
                if probe.peek() == Some(&'/') {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                } else {
                    return;
                }
            }
            _ => return,
        }
    }
}

fn read_token(chars: &mut Chars) -> Option<String> {
    skip_trivia(chars);
    match chars.peek()? {
        '"' => {
            chars.next();
            let mut out = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '"' => return Some(out),
                    '\\' => match chars.next() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('\\') => out.push('\\'),
                        Some('"') => out.push('"'),
                        Some(other) => {
                            // Windows paths appear unescaped in these files, so
                            // an unknown escape keeps both characters.
                            out.push('\\');
                            out.push(other);
                        }
                        None => break,
                    },
                    _ => out.push(c),
                }
            }
            Some(out)
        }
        '{' | '}' => None,
        _ => {
            let mut out = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                    break;
                }
                out.push(c);
                chars.next();
            }
            (!out.is_empty()).then_some(out)
        }
    }
}

/// Consume a `{ … }` block whose contents we are discarding.
fn skip_block(chars: &mut Chars) {
    let mut depth = 1;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                // Braces inside strings do not count.
                while let Some(s) = chars.next() {
                    if s == '\\' {
                        chars.next();
                    } else if s == '"' {
                        break;
                    }
                }
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Locate the Steam installation.
pub fn steam_root() -> Option<PathBuf> {
    let candidates = steam_root_candidates();
    candidates
        .into_iter()
        .find(|p| p.join("steamapps").is_dir() || p.join("config").is_dir())
}

fn steam_root_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();

    // An explicit override wins, for unusual installs.
    if let Some(dir) = std::env::var_os("STEAM_ROOT") {
        out.push(PathBuf::from(dir));
    }

    #[cfg(windows)]
    {
        // Steam's default 32-bit Program Files location, plus the common
        // "installed on the big disk" case.
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(base) = std::env::var_os(var) {
                out.push(PathBuf::from(base).join("Steam"));
            }
        }
        for letter in ['C', 'D', 'E'] {
            out.push(PathBuf::from(format!("{letter}:\\Steam")));
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            out.push(home.join(".local/share/Steam"));
            out.push(home.join(".steam/steam"));
            out.push(home.join(".steam/root"));
            // Flatpak
            out.push(home.join(".var/app/com.valvesoftware.Steam/data/Steam"));
            // macOS
            out.push(home.join("Library/Application Support/Steam"));
        }
    }

    out
}

/// Every Steam library folder, including the one inside the Steam install.
///
/// Games are frequently on a different disk from Steam itself, so reading only
/// `steamapps` under the install would miss most of a real library.
pub fn library_paths(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.join("steamapps")];

    // Steam has kept this file in both places over the years.
    for rel in ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"] {
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        let kv = parse_keyvalues(&text);
        let Some(folders) = kv.get("libraryfolders") else {
            continue;
        };

        for (key, value) in folders.entries() {
            // Modern layout: each numbered entry is a map with a "path".
            // Ancient layout: the numbered key maps straight to the path string.
            let path = match value {
                Kv::Map(_) => value.get("path").and_then(Kv::as_str),
                Kv::Str(s) if key.chars().all(|c| c.is_ascii_digit()) => Some(s.as_str()),
                Kv::Str(_) => None,
            };
            if let Some(path) = path {
                let steamapps = PathBuf::from(path).join("steamapps");
                if !out.contains(&steamapps) {
                    out.push(steamapps);
                }
            }
        }
        break;
    }

    out
}

/// `StateFlags` bit meaning "fully installed".
const STATE_FULLY_INSTALLED: u64 = 4;

/// Read every installed game across all libraries, sorted by name.
pub fn installed_games(root: &Path) -> Vec<SteamGame> {
    // Keyed by app id so a game present in two libraries appears once.
    let mut games: BTreeMap<String, SteamGame> = BTreeMap::new();

    for library in library_paths(root) {
        let Ok(entries) = std::fs::read_dir(&library) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            if let Some(game) = game_from_manifest(&text, root) {
                games.entry(game.app_id.clone()).or_insert(game);
            }
        }
    }

    let mut out: Vec<SteamGame> = games.into_values().collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Apps Steam installs beside games that are not games.
///
/// Steam writes an `appmanifest` for these exactly as it does for a game, and
/// they are fully installed, so nothing else here distinguishes them. They are
/// listed by id because the names are localised and change between releases.
///
/// Everything below is a runtime or a redistributable bundle: putting one on a
/// cartridge would produce something with a Play button that starts nothing.
const NOT_GAMES: [&str; 4] = [
    "228980",  // Steamworks Common Redistributables
    "1070560", // Steam Linux Runtime 1.0 (scout)
    "1391110", // Steam Linux Runtime 2.0 (soldier)
    "1628350", // Steam Linux Runtime 3.0 (sniper)
];

/// Name prefixes for the runtimes that get a new app id per release, which is
/// more of them than a list of ids can keep up with. Both names are Valve's own
/// product names and are not translated.
const NOT_GAME_PREFIXES: [&str; 2] = ["Proton ", "Steam Linux Runtime"];

fn is_not_a_game(app_id: &str, name: &str) -> bool {
    if NOT_GAMES.contains(&app_id) {
        return true;
    }
    let name = name.trim();
    NOT_GAME_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Build a game from one `appmanifest_*.acf`, or `None` if it is not a usable
/// installed game.
pub fn game_from_manifest(text: &str, steam_root: &Path) -> Option<SteamGame> {
    let kv = parse_keyvalues(text);
    let state = kv.get("AppState")?;

    let app_id = state.get("appid")?.as_str()?.trim().to_string();
    if app_id.is_empty() || !app_id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    let name = state
        .get("name")
        .and_then(Kv::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Unknown game")
        .to_string();

    if is_not_a_game(&app_id, &name) {
        return None;
    }

    // Skip games that are only partially there — a download in progress would
    // make a cartridge that cannot play.
    let flags: u64 = state
        .get("StateFlags")
        .and_then(Kv::as_str)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    if flags & STATE_FULLY_INSTALLED == 0 {
        return None;
    }

    let size_on_disk = state
        .get("SizeOnDisk")
        .and_then(Kv::as_str)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    Some(SteamGame {
        cover_path: find_cover(steam_root, &app_id),
        app_id,
        name,
        size_on_disk,
    })
}

/// Steam's cached portrait art for an app, if present.
///
/// The layout has moved between releases, so several shapes are tried. The
/// 600x900 portrait is what the launcher wants; the 2x variant is preferred
/// because the window is drawn on high-DPI displays.
pub fn find_cover(steam_root: &Path, app_id: &str) -> Option<PathBuf> {
    let cache = steam_root.join("appcache/librarycache");

    let candidates = [
        // 2023+ : one directory per app
        cache.join(app_id).join("library_600x900_2x.jpg"),
        cache.join(app_id).join("library_600x900.jpg"),
        // older flat layout
        cache.join(format!("{app_id}_library_600x900_2x.jpg")),
        cache.join(format!("{app_id}_library_600x900.jpg")),
        // last resort: the landscape header, better than no art
        cache.join(app_id).join("header.jpg"),
        cache.join(format!("{app_id}_header.jpg")),
    ];

    candidates.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_flat_manifest() {
        let kv = parse_keyvalues(
            r#"
"AppState"
{
	"appid"		"367520"
	"name"		"Hollow Knight"
	"StateFlags"		"4"
	"SizeOnDisk"		"9106886656"
}
"#,
        );
        let state = kv.get("AppState").unwrap();
        assert_eq!(state.get("appid").unwrap().as_str(), Some("367520"));
        assert_eq!(state.get("name").unwrap().as_str(), Some("Hollow Knight"));
    }

    #[test]
    fn key_lookup_ignores_case() {
        let kv = parse_keyvalues("\"AppState\" { \"AppID\" \"1\" \"StateFlags\" \"4\" }");
        assert!(kv.get("appstate").is_some());
        assert_eq!(
            kv.path(&["APPSTATE", "appid"]).and_then(Kv::as_str),
            Some("1")
        );
    }

    #[test]
    fn builds_a_game_from_a_manifest() {
        let game = game_from_manifest(
            r#"
"AppState"
{
	"appid"		"367520"
	"name"		"Hollow Knight"
	"installdir"		"Hollow Knight"
	"StateFlags"		"4"
	"SizeOnDisk"		"9106886656"
}
"#,
            Path::new("/nonexistent"),
        )
        .expect("a fully installed game");

        assert_eq!(game.app_id, "367520");
        assert_eq!(game.name, "Hollow Knight");
        assert_eq!(game.size_on_disk, 9_106_886_656);
        assert_eq!(game.cover_path, None);
    }

    #[test]
    fn skips_games_that_are_not_fully_installed() {
        // StateFlags 1026 = update queued + downloading, bit 4 clear.
        let partial = r#""AppState" { "appid" "1" "name" "Half Downloaded" "StateFlags" "1026" }"#;
        assert_eq!(game_from_manifest(partial, Path::new("/x")), None);

        // Missing StateFlags entirely is also not a confirmed install.
        let unknown = r#""AppState" { "appid" "1" "name" "No Flags" }"#;
        assert_eq!(game_from_manifest(unknown, Path::new("/x")), None);
    }

    #[test]
    fn rejects_manifests_without_a_numeric_appid() {
        for bad in [
            r#""AppState" { "name" "No id" "StateFlags" "4" }"#,
            r#""AppState" { "appid" "abc" "name" "Bad id" "StateFlags" "4" }"#,
            r#""AppState" { "appid" "" "name" "Empty" "StateFlags" "4" }"#,
            r#""Nonsense" { "appid" "1" }"#,
        ] {
            assert_eq!(game_from_manifest(bad, Path::new("/x")), None, "{bad}");
        }
    }

    #[test]
    fn a_missing_name_still_yields_a_usable_entry() {
        let game = game_from_manifest(
            r#""AppState" { "appid" "7" "StateFlags" "4" }"#,
            Path::new("/x"),
        )
        .unwrap();
        assert_eq!(game.name, "Unknown game");
    }

    #[test]
    fn reads_modern_library_folders() {
        let scratch = crate::testutil::Scratch::new("libs-modern");
        std::fs::create_dir_all(scratch.join("steamapps")).unwrap();
        std::fs::write(
            scratch.join("steamapps/libraryfolders.vdf"),
            r#"
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
		"path"		"/mnt/games/SteamLibrary"
		"label"		"games"
	}
}
"#,
        )
        .unwrap();

        let libs = library_paths(scratch.path());
        assert!(libs.contains(&scratch.join("steamapps")));
        assert!(libs.contains(&PathBuf::from("/home/harry/.local/share/Steam/steamapps")));
        assert!(libs.contains(&PathBuf::from("/mnt/games/SteamLibrary/steamapps")));
    }

    #[test]
    fn reads_the_ancient_flat_library_format() {
        let scratch = crate::testutil::Scratch::new("libs-old");
        std::fs::create_dir_all(scratch.join("steamapps")).unwrap();
        std::fs::write(
            scratch.join("steamapps/libraryfolders.vdf"),
            "\"LibraryFolders\"\n{\n\t\"TimeNextStatsReport\"\t\"123\"\n\t\"1\"\t\"/mnt/old/Library\"\n}\n",
        )
        .unwrap();

        // Asserted exactly rather than by substring: the scratch directory's
        // own name carries a pid and a timestamp, so "does any path contain
        // 123" was a coin flip on the bookkeeping value below.
        assert_eq!(
            library_paths(scratch.path()),
            vec![
                scratch.join("steamapps"),
                PathBuf::from("/mnt/old/Library/steamapps"),
            ],
            "the non-numeric bookkeeping key must not become a library path"
        );
    }

    #[test]
    fn windows_paths_survive_parsing() {
        // These appear with doubled backslashes in real vdf files.
        let kv =
            parse_keyvalues("\"libraryfolders\" { \"0\" { \"path\" \"D:\\\\SteamLibrary\" } }");
        assert_eq!(
            kv.path(&["libraryfolders", "0", "path"])
                .and_then(Kv::as_str),
            Some("D:\\SteamLibrary")
        );
    }

    #[test]
    fn enumerates_installed_games_across_libraries() {
        let scratch = crate::testutil::Scratch::new("games");
        let second = scratch.join("second");
        std::fs::create_dir_all(scratch.join("steamapps")).unwrap();
        std::fs::create_dir_all(second.join("steamapps")).unwrap();

        std::fs::write(
            scratch.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
                second.display()
            ),
        )
        .unwrap();

        std::fs::write(
            scratch.join("steamapps/appmanifest_2.acf"),
            r#""AppState" { "appid" "2" "name" "Zebra" "StateFlags" "4" }"#,
        )
        .unwrap();
        std::fs::write(
            second.join("steamapps/appmanifest_1.acf"),
            r#""AppState" { "appid" "1" "name" "apple" "StateFlags" "4" }"#,
        )
        .unwrap();
        std::fs::write(
            second.join("steamapps/appmanifest_3.acf"),
            r#""AppState" { "appid" "3" "name" "Downloading" "StateFlags" "2" }"#,
        )
        .unwrap();
        // Not a manifest; must be ignored.
        std::fs::write(scratch.join("steamapps/readme.txt"), "hello").unwrap();

        let games = installed_games(scratch.path());
        // Sorted case-insensitively by name, incomplete install excluded.
        assert_eq!(
            games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["apple", "Zebra"]
        );
    }

    #[test]
    fn runtimes_and_redistributables_are_not_offered_as_games() {
        // Steam writes these an appmanifest exactly as it does a game, and they
        // are fully installed, so nothing else tells them apart. A cartridge
        // made from one would have a Play button that starts nothing.
        assert!(game_from_manifest(
            r#""AppState" { "appid" "228980" "name" "Steamworks Common Redistributables" "StateFlags" "4" }"#,
            Path::new("/steam"),
        )
        .is_none());

        // Proton and the Linux runtimes take a new app id per release, so the
        // name is what catches the ones no list can keep up with.
        assert!(game_from_manifest(
            r#""AppState" { "appid" "2805730" "name" "Proton 9.0" "StateFlags" "4" }"#,
            Path::new("/steam"),
        )
        .is_none());
        assert!(game_from_manifest(
            r#""AppState" { "appid" "1628350" "name" "Steam Linux Runtime 3.0 (sniper)" "StateFlags" "4" }"#,
            Path::new("/steam"),
        )
        .is_none());

        // A game whose name merely starts similarly is still a game.
        let game = game_from_manifest(
            r#""AppState" { "appid" "42" "name" "Protonaut" "StateFlags" "4" }"#,
            Path::new("/steam"),
        )
        .expect("Protonaut is a game");
        assert_eq!(game.name, "Protonaut");
    }

    #[test]
    fn finds_cover_art_in_both_cache_layouts() {
        let scratch = crate::testutil::Scratch::new("covers");
        let flat = scratch.join("appcache/librarycache");
        std::fs::create_dir_all(flat.join("367520")).unwrap();
        std::fs::write(flat.join("367520/library_600x900.jpg"), b"x").unwrap();
        assert_eq!(
            find_cover(scratch.path(), "367520"),
            Some(flat.join("367520/library_600x900.jpg"))
        );

        // The 2x variant wins when both exist.
        std::fs::write(flat.join("367520/library_600x900_2x.jpg"), b"x").unwrap();
        assert_eq!(
            find_cover(scratch.path(), "367520"),
            Some(flat.join("367520/library_600x900_2x.jpg"))
        );

        // Older flat naming.
        std::fs::write(flat.join("99_library_600x900.jpg"), b"x").unwrap();
        assert_eq!(
            find_cover(scratch.path(), "99"),
            Some(flat.join("99_library_600x900.jpg"))
        );

        assert_eq!(find_cover(scratch.path(), "12345"), None);
    }

    #[test]
    fn tolerates_comments_and_junk() {
        let kv = parse_keyvalues(
            "// leading comment\n\"AppState\"\n{\n\t\"appid\" \"1\" // trailing\n\t\"name\" \"A/B\"\n}\n",
        );
        assert_eq!(
            kv.path(&["AppState", "appid"]).and_then(Kv::as_str),
            Some("1")
        );
        // A single slash is data, not a comment.
        assert_eq!(
            kv.path(&["AppState", "name"]).and_then(Kv::as_str),
            Some("A/B")
        );
    }

    #[test]
    fn does_not_hang_or_panic_on_malformed_input() {
        for junk in [
            "",
            "{",
            "}",
            "\"unclosed",
            "\"a\" {",
            "\"a\" { \"b\"",
            "{{{{{{{{",
            "\"a\" \"b\" }}}} \"c\" \"d\"",
        ] {
            let _ = parse_keyvalues(junk);
        }
        // Deep nesting must not blow the stack.
        let deep = "\"a\" {".repeat(500) + &"}".repeat(500);
        let _ = parse_keyvalues(&deep);
    }

    // ---- helpers ----
}
