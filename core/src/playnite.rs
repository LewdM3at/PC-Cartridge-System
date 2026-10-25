//! Importing a Playnite library.
//!
//! Playnite aggregates Steam, GOG, Epic, Xbox, Ubisoft, itch, emulators and
//! anything added by hand, which is a far wider net than reading Steam alone.
//!
//! It keeps its library in `games.db`, a LiteDB file — a binary .NET database
//! with no usable Rust reader — so this reads a JSON export instead. Any of the
//! community exporter extensions will do; they serialise Playnite's own `Game`
//! model, so the field names below follow that model.
//!
//! The parsing is deliberately forgiving. There is no single blessed export
//! format, so every field is optional, `PascalCase` and `camelCase` are both
//! accepted, and objects that might be a bare string are handled either way. A
//! game missing a field should cost that field, not the whole import.
//!
//! std + serde only, no Tauri, so it can be tested on its own.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A game as imported from Playnite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayniteGame {
    /// Playnite's GUID. This is what `playnite://playnite/start/<id>` takes.
    pub id: String,
    pub name: String,
    /// "Steam", "GOG", "Epic" … blank when Playnite does not say.
    pub source: String,
    /// Where the game is installed, when Playnite knows.
    pub install_dir: Option<PathBuf>,
    /// Cover art, relative to Playnite's `library/files`, or absolute.
    pub cover: Option<String>,
    pub is_installed: bool,
    /// Seconds, as Playnite records it.
    pub playtime: u64,
    /// The `Path` of the game's play action, when the export carries one.
    ///
    /// Playnite knows exactly which file it launches, so this beats any guess
    /// made by scanning the install directory. It may be relative to
    /// `install_dir` or absolute, and is often a bare filename.
    pub play_action: Option<String>,
}

impl PlayniteGame {
    /// The URI that launches this game through Playnite.
    pub fn launch_uri(&self) -> String {
        format!("playnite://playnite/start/{}", self.id)
    }
}

/// Either `[ … ]` or `{ "Games": [ … ] }` — exporters differ.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Export {
    Bare(Vec<RawGame>),
    Wrapped {
        #[serde(alias = "games", alias = "Games", alias = "library", alias = "Library")]
        games: Vec<RawGame>,
    },
}

/// A value that may be a plain string or an object with a name, which is how
/// Playnite's `Source` and `Platform` come out depending on the exporter.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Named {
    Text(String),
    Object {
        #[serde(alias = "name", alias = "Name")]
        name: Option<String>,
    },
    /// Anything else (numbers, null, nested junk) is simply unnamed.
    /// `IgnoredAny` accepts any shape and keeps nothing, which is exactly the
    /// catch-all an untagged enum needs to stop one odd field failing the parse.
    Other(serde::de::IgnoredAny),
}

impl Named {
    fn name(&self) -> Option<&str> {
        match self {
            Named::Text(s) => Some(s.as_str()),
            Named::Object { name } => name.as_deref(),
            Named::Other(_) => None,
        }
    }
}

/// Playtime may be a number or a stringified number.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Number {
    Int(u64),
    Text(String),
    Other(serde::de::IgnoredAny),
}

impl Number {
    fn value(&self) -> u64 {
        match self {
            Number::Int(n) => *n,
            Number::Text(s) => s.trim().parse().unwrap_or(0),
            Number::Other(_) => 0,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawGame {
    #[serde(
        default,
        alias = "id",
        alias = "Id",
        alias = "gameId",
        alias = "GameId"
    )]
    id: Option<String>,
    #[serde(
        default,
        alias = "name",
        alias = "Name",
        alias = "title",
        alias = "Title"
    )]
    name: Option<String>,
    #[serde(default, alias = "source", alias = "Source")]
    source: Option<Named>,
    #[serde(default, alias = "platform", alias = "Platform")]
    platform: Option<Named>,
    #[serde(default, alias = "platforms", alias = "Platforms")]
    platforms: Option<Vec<Named>>,
    #[serde(
        default,
        alias = "installDirectory",
        alias = "InstallDirectory",
        alias = "installDir",
        alias = "InstallDir"
    )]
    install_directory: Option<String>,
    #[serde(
        default,
        alias = "coverImage",
        alias = "CoverImage",
        alias = "cover",
        alias = "Cover"
    )]
    cover_image: Option<String>,
    #[serde(default, alias = "icon", alias = "Icon")]
    icon: Option<String>,
    #[serde(
        default,
        alias = "isInstalled",
        alias = "IsInstalled",
        alias = "installed"
    )]
    is_installed: Option<bool>,
    #[serde(default, alias = "playtime", alias = "Playtime")]
    playtime: Option<Number>,
    /// Playnite marks DLC and similar as hidden; they are not cartridges.
    #[serde(default, alias = "hidden", alias = "Hidden")]
    hidden: Option<bool>,
    #[serde(default, alias = "gameActions", alias = "GameActions")]
    game_actions: Option<Vec<RawAction>>,
}

/// One entry of Playnite's `GameActions`.
#[derive(Debug, Deserialize)]
struct RawAction {
    #[serde(default, alias = "path", alias = "Path")]
    path: Option<String>,
    #[serde(default, alias = "isPlayAction", alias = "IsPlayAction")]
    is_play_action: Option<bool>,
    #[serde(default, alias = "type", alias = "Type")]
    kind: Option<Named>,
}

#[derive(Debug)]
pub enum ImportError {
    NotFound,
    Io(String),
    Json(String),
    Empty,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NotFound => write!(
                f,
                "No Playnite library export found. Install a JSON library exporter \
                 extension in Playnite, run it, then point the wizard at the \
                 library.json it produces."
            ),
            ImportError::Io(e) => write!(f, "Could not read the export: {e}"),
            ImportError::Json(e) => write!(f, "That file is not a Playnite JSON export: {e}"),
            ImportError::Empty => write!(f, "The export parsed but contained no installed games."),
        }
    }
}

/// Parse an export's contents.
pub fn parse_export(json: &str) -> Result<Vec<PlayniteGame>, ImportError> {
    let export: Export =
        serde_json::from_str(json).map_err(|e| ImportError::Json(e.to_string()))?;

    let raw = match export {
        Export::Bare(games) => games,
        Export::Wrapped { games } => games,
    };

    let mut out = Vec::new();
    for game in raw {
        // Without an id there is nothing to launch, and without a name there is
        // nothing to show, so those two are the only hard requirements.
        let Some(id) = game
            .id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(name) = game
            .name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if game.hidden.unwrap_or(false) {
            continue;
        }
        // A game Playnite has not installed cannot be copied to a cartridge.
        if !game.is_installed.unwrap_or(false) {
            continue;
        }

        let source = game
            .source
            .as_ref()
            .and_then(Named::name)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                // Fall back to the platform when the source is missing.
                game.platform
                    .as_ref()
                    .and_then(Named::name)
                    .or_else(|| {
                        game.platforms
                            .as_ref()
                            .and_then(|p| p.first())
                            .and_then(Named::name)
                    })
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_default();

        // Prefer the action Playnite marks as the play action; failing that, one
        // typed as a file; failing that, the first with a path at all.
        let actions = game.game_actions.unwrap_or_default();
        let play_action = actions
            .iter()
            .find(|a| a.is_play_action.unwrap_or(false) && a.path.is_some())
            .or_else(|| {
                actions.iter().find(|a| {
                    a.kind
                        .as_ref()
                        .and_then(Named::name)
                        .map(|k| k.eq_ignore_ascii_case("File"))
                        .unwrap_or(false)
                        && a.path.is_some()
                })
            })
            .or_else(|| actions.iter().find(|a| a.path.is_some()))
            .and_then(|a| a.path.clone())
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());

        out.push(PlayniteGame {
            id,
            name,
            play_action,
            source,
            install_dir: game
                .install_directory
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from),
            // Prefer the cover; an icon is better than nothing.
            cover: game
                .cover_image
                .or(game.icon)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            is_installed: true,
            playtime: game.playtime.as_ref().map(Number::value).unwrap_or(0),
        });
    }

    if out.is_empty() {
        return Err(ImportError::Empty);
    }

    out.sort_by_key(|a| a.name.to_lowercase());
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

/// Read an export from a specific file.
pub fn import_from(path: &Path) -> Result<Vec<PlayniteGame>, ImportError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_export(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ImportError::NotFound),
        Err(e) => Err(ImportError::Io(e.to_string())),
    }
}

/// Playnite's data directory, for both installed and portable setups.
pub fn playnite_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(dir) = std::env::var_os("PLAYNITE_ROOT") {
        candidates.push(PathBuf::from(dir));
    }

    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            candidates.push(PathBuf::from(appdata).join("Playnite"));
        }
        // Portable installs keep their data next to the executable.
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var) {
                candidates.push(PathBuf::from(base).join("Playnite"));
            }
        }
    }

    #[cfg(not(windows))]
    {
        // Playnite is a Windows application, but it is commonly run under Proton,
        // so look inside the usual prefixes before giving up.
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            // Wine default prefix.
            candidates.push(home.join(".wine/drive_c/users/steamuser/AppData/Roaming/Playnite"));

            // Scan every Proton compatdata prefix rather than hard-coding one
            // app ID. The Playnite Steam app (975330) is the common case, but
            // users may have it in a custom non-Steam game entry or a different
            // Proton prefix, and scanning is cheap because only the directories
            // that exist under compatdata are checked.
            let compatdata = home.join(".steam/steam/steamapps/compatdata");
            if let Ok(entries) = std::fs::read_dir(&compatdata) {
                for entry in entries.flatten() {
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let candidate = entry
                        .path()
                        .join("pfx/drive_c/users/steamuser/AppData/Roaming/Playnite");
                    candidates.push(candidate);
                }
            }
        }
    }

    candidates
        .into_iter()
        .find(|p| p.join("library").is_dir() || p.is_dir())
}

/// Where a library export is likely to be.
///
/// The exporter extensions write into their own `ExtensionsData` folder, whose
/// name is the extension's GUID, so this scans rather than hard-coding one.
pub fn find_exports(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();

    // Anywhere obvious first.
    for rel in ["library.json", "games.json", "playnite-library.json"] {
        let direct = root.join(rel);
        if direct.is_file() {
            found.push(direct);
        }
    }

    // Then one level inside each extension's data folder.
    if let Ok(entries) = std::fs::read_dir(root.join("ExtensionsData")) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            if let Ok(files) = std::fs::read_dir(entry.path()) {
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("json") {
                        continue;
                    }
                    // Every extension keeps its settings in `config.json`, so on
                    // a real Playnite install these outnumber the exports and
                    // none of them is one. Taking them as candidates is how a
                    // machine with no exporter installed ends up reporting a
                    // parse error against an unrelated file instead of saying
                    // there is no export.
                    if is_extension_settings(&path) {
                        continue;
                    }
                    if !found.contains(&path) {
                        found.push(path);
                    }
                }
            }
        }
    }

    found
}

/// A file that is an extension's own settings rather than a library export.
fn is_extension_settings(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("config.json"))
}

/// Import from whichever candidate under `root` actually parses, newest first.
///
/// An extension data folder holds whatever that extension felt like writing —
/// tag lists, caches, settings under some other name — so the newest JSON is
/// not necessarily an export. Trying them in turn means one piece of unrelated
/// JSON cannot hide a library that is sitting right beside it.
///
/// The error, when every candidate fails, is `NotFound`: from the caller's
/// point of view there was no export here, which is both true and the thing
/// worth telling the user.
pub fn import_newest_in(root: &Path) -> Result<(PathBuf, Vec<PlayniteGame>), ImportError> {
    let mut candidates = find_exports(root);
    if candidates.is_empty() {
        return Err(ImportError::NotFound);
    }

    // Newest first: several extensions may have written one, and the most
    // recently refreshed export is the one that matches the library.
    candidates.sort_by_key(|path| {
        std::cmp::Reverse(
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH),
        )
    });

    for path in candidates {
        if let Ok(games) = import_from(&path) {
            return Ok((path, games));
        }
    }

    Err(ImportError::NotFound)
}

/// Resolve a game's cover to a real file.
///
/// Playnite stores metadata images under `library/files`, and `CoverImage` is
/// usually relative to that. Absolute paths are taken as given.
pub fn resolve_cover(root: &Path, cover: &str) -> Option<PathBuf> {
    if cover.is_empty() {
        return None;
    }

    let as_given = Path::new(cover);
    if as_given.is_absolute() && as_given.is_file() {
        return Some(as_given.to_path_buf());
    }

    // Playnite writes these with Windows separators; normalise so they resolve
    // when the wizard is run on Linux against a Proton prefix.
    let normalised: PathBuf = cover.split(['\\', '/']).filter(|s| !s.is_empty()).collect();

    for base in [
        root.join("library/files"),
        root.join("library"),
        root.to_path_buf(),
    ] {
        let candidate = base.join(&normalised);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_playnite_style_export() {
        let games = parse_export(
            r#"[
              {
                "Id": "2b3c4d5e-1111-2222-3333-444455556666",
                "Name": "Hollow Knight",
                "Source": { "Name": "Steam" },
                "Platforms": [ { "Name": "PC (Windows)" } ],
                "InstallDirectory": "D:\\SteamLibrary\\steamapps\\common\\Hollow Knight",
                "CoverImage": "2b3c4d5e-1111-2222-3333-444455556666\\cover.jpg",
                "IsInstalled": true,
                "Playtime": 46800
              }
            ]"#,
        )
        .expect("valid export");

        assert_eq!(games.len(), 1);
        let g = &games[0];
        assert_eq!(g.name, "Hollow Knight");
        assert_eq!(g.source, "Steam");
        assert_eq!(g.playtime, 46800);
        assert_eq!(
            g.launch_uri(),
            "playnite://playnite/start/2b3c4d5e-1111-2222-3333-444455556666"
        );
    }

    #[test]
    fn accepts_camel_case_and_a_wrapped_array() {
        let games = parse_export(
            r#"{ "games": [
                 { "id": "a", "name": "Alpha", "source": "GOG", "isInstalled": true },
                 { "id": "b", "name": "Beta", "source": "Epic", "isInstalled": true }
               ] }"#,
        )
        .unwrap();
        assert_eq!(
            games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "Beta"]
        );
        assert_eq!(games[0].source, "GOG");
    }

    #[test]
    fn source_may_be_a_bare_string_or_an_object() {
        let games = parse_export(
            r#"[
                {"Id":"1","Name":"A","Source":"Xbox","IsInstalled":true},
                {"Id":"2","Name":"B","Source":{"Name":"Ubisoft Connect"},"IsInstalled":true}
            ]"#,
        )
        .unwrap();
        assert_eq!(games[0].source, "Xbox");
        assert_eq!(games[1].source, "Ubisoft Connect");
    }

    #[test]
    fn falls_back_to_platform_when_there_is_no_source() {
        let games = parse_export(
            r#"[{"Id":"1","Name":"Emulated","Platforms":[{"Name":"Nintendo 64"}],"IsInstalled":true}]"#,
        )
        .unwrap();
        assert_eq!(games[0].source, "Nintendo 64");
    }

    #[test]
    fn skips_games_that_cannot_become_cartridges() {
        // Not installed, hidden, or missing an id or name.
        let games = parse_export(
            r#"[
                {"Id":"1","Name":"Not installed","IsInstalled":false},
                {"Id":"2","Name":"Hidden","IsInstalled":true,"Hidden":true},
                {"Name":"No id","IsInstalled":true},
                {"Id":"4","IsInstalled":true},
                {"Id":"  ","Name":"Blank id","IsInstalled":true},
                {"Id":"6","Name":"Keeper","IsInstalled":true}
            ]"#,
        )
        .unwrap();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Keeper");
    }

    #[test]
    fn a_playtime_string_is_still_a_number() {
        let games = parse_export(r#"[{"Id":"1","Name":"A","IsInstalled":true,"Playtime":"3600"}]"#)
            .unwrap();
        assert_eq!(games[0].playtime, 3600);
    }

    #[test]
    fn unexpected_shapes_do_not_sink_the_import() {
        // Source as a number, platforms as junk, playtime as an object.
        let games = parse_export(
            r#"[{"Id":"1","Name":"Odd","IsInstalled":true,
                 "Source":42,"Platforms":[7],"Playtime":{"nope":1}}]"#,
        )
        .unwrap();
        assert_eq!(games[0].name, "Odd");
        assert_eq!(games[0].source, "");
        assert_eq!(games[0].playtime, 0);
    }

    #[test]
    fn an_icon_stands_in_for_a_missing_cover() {
        let games =
            parse_export(r#"[{"Id":"1","Name":"A","IsInstalled":true,"Icon":"1\\icon.png"}]"#)
                .unwrap();
        assert_eq!(games[0].cover.as_deref(), Some("1\\icon.png"));
    }

    #[test]
    fn reports_useful_errors() {
        assert!(matches!(
            parse_export("not json"),
            Err(ImportError::Json(_))
        ));
        assert!(matches!(parse_export("[]"), Err(ImportError::Empty)));
        assert!(matches!(
            parse_export(r#"[{"Id":"1","Name":"A","IsInstalled":false}]"#),
            Err(ImportError::Empty)
        ));
    }

    #[test]
    fn resolves_covers_including_windows_separators() {
        let scratch = crate::testutil::Scratch::new("playnite");
        let files = scratch.join("library/files/abc");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("cover.jpg"), b"x").unwrap();

        assert_eq!(
            resolve_cover(scratch.path(), "abc\\cover.jpg"),
            Some(files.join("cover.jpg"))
        );
        assert_eq!(
            resolve_cover(scratch.path(), "abc/cover.jpg"),
            Some(files.join("cover.jpg"))
        );
        assert_eq!(resolve_cover(scratch.path(), "abc\\missing.jpg"), None);
        assert_eq!(resolve_cover(scratch.path(), ""), None);
    }

    #[test]
    fn finds_exports_inside_extension_data_folders() {
        let scratch = crate::testutil::Scratch::new("playnite-exp");
        let ext = scratch.join("ExtensionsData/66b8eca4-3f39-4b79-a359-3cb98d5b18fd");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(ext.join("library.json"), b"[]").unwrap();
        std::fs::write(ext.join("settings.dat"), b"x").unwrap();
        std::fs::write(scratch.join("library.json"), b"[]").unwrap();

        let found = find_exports(scratch.path());
        assert!(found.contains(&scratch.join("library.json")));
        assert!(found.contains(&ext.join("library.json")));
        // Non-JSON files are not exports.
        assert!(!found.iter().any(|p| p.ends_with("settings.dat")));
    }

    #[test]
    fn an_extensions_own_settings_are_not_a_library_export() {
        // Every installed extension writes a config.json, so on a real Playnite
        // install these outnumber the exports and none of them is one. Counting
        // them meant a machine with no exporter reported a parse failure against
        // an unrelated file instead of saying there was no export.
        let scratch = crate::testutil::Scratch::new("playnite-conf");
        let ext = scratch.join("ExtensionsData/cb91dfc9-b977-43bf-8e70-55f46e410fab");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(ext.join("config.json"), br#"{"Enabled":true}"#).unwrap();

        assert!(find_exports(scratch.path()).is_empty());
        assert!(matches!(
            import_newest_in(scratch.path()),
            Err(ImportError::NotFound)
        ));
    }

    #[test]
    fn unrelated_json_beside_an_export_does_not_hide_it() {
        // An extension data folder holds whatever that extension felt like
        // writing. The newest file there is not necessarily the export, and
        // when it is not, the export must still be found.
        let scratch = crate::testutil::Scratch::new("playnite-junk");
        let ext = scratch.join("ExtensionsData/cb91dfc9-b977-43bf-8e70-55f46e410fab");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("library.json"),
            br#"[{"Id":"1","Name":"Tunic","IsInstalled":true}]"#,
        )
        .unwrap();
        // Written second, so it is the newer of the two.
        std::fs::write(ext.join("tagnames-swedish.json"), br#"{"rollspel":"RPG"}"#).unwrap();

        let (path, games) = import_newest_in(scratch.path()).expect("the real export");
        assert_eq!(path, ext.join("library.json"));
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Tunic");
    }

    #[test]
    fn no_playnite_data_at_all_is_simply_not_found() {
        let scratch = crate::testutil::Scratch::new("playnite-bare");
        assert!(matches!(
            import_newest_in(scratch.path()),
            Err(ImportError::NotFound)
        ));
    }
}
