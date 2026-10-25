//! Reading a cartridge: the manifest at its root, and the cover art beside it.
//!
//! Split out of the Tauri binary so it can be tested without a webview. Pure
//! std + serde.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// How long a game takes to beat, as HowLongToBeat said when the cartridge was
/// written: the `hltb_*` keys, in seconds. Zero where it had no figure.
#[derive(Serialize, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct HowLong {
    pub main: u64,
    pub extra: u64,
    pub complete: u64,
}

impl HowLong {
    fn from_keys(map: Option<&HashMap<String, String>>) -> Self {
        let seconds = |key: &str| {
            map.and_then(|m| m.get(key))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0)
        };
        Self {
            main: seconds("hltb_main"),
            extra: seconds("hltb_extra"),
            complete: seconds("hltb_complete"),
        }
    }
}

/// One game entry inside a multi-game bundle cartridge.
#[derive(Serialize, Clone, Debug)]
pub struct GameEntry {
    pub title: String,
    pub executable: String,
    /// Cover as a `data:` URI, or empty string if none.
    pub cover: String,
    /// Absolute path to the cover image, or empty string.
    pub cover_path: String,
    /// The same four pictures a single-game cartridge has, per game.
    ///
    /// A collection used to carry one picture per game and three for itself,
    /// which was right while the launcher only ever showed a cover. A skin can
    /// ask for the hero now, and a collection under one of those needs a hero
    /// for each game rather than one for the shelf — so every game gets the
    /// full set, and any of them may be empty.
    pub background: String,
    pub background_path: String,
    pub logo: String,
    pub logo_path: String,
    pub icon: String,
    pub icon_path: String,
    pub how_long: HowLong,
    /// The game's own `platform=`, else the collection's. See [`platform`].
    pub platform: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CartridgeInfo {
    /// Display title of the game / collection.
    pub title: String,
    /// Absolute path to the cover image, or empty string if none. Shown in the
    /// details sheet; never sent back to the backend.
    pub cover_path: String,
    /// The cover as a `data:` URI, or empty string if there is none.
    ///
    /// Inlined rather than served over the asset protocol: cartridge mount
    /// points are arbitrary, so a scope wide enough to serve them would be wide
    /// enough to serve anything on the machine.
    pub cover: String,
    /// Optional hero/background as a `data:` URI.
    pub background: String,
    /// Absolute path to the background image, or empty string if none. Same
    /// role as `cover_path`: editing needs the real path back, the launcher
    /// only ever wants the `data:` URI above.
    pub background_path: String,
    /// Optional title logo as a `data:` URI.
    pub logo: String,
    /// Absolute path to the logo image, or empty string if none.
    pub logo_path: String,
    /// The picture the drive's icon was made from, as a `data:` URI. The
    /// launcher never shows it — Explorer does — but the editor has to, or its
    /// icon slot has nothing to display and falls back to the cover.
    pub icon: String,
    /// Absolute path to the icon source, or empty string if none.
    pub icon_path: String,
    /// The value from the `executable` / `open` key — either a URI or a
    /// relative path on the cartridge. For bundles this is the first game's
    /// executable so keyboard shortcuts (Enter = play) still work.
    pub executable: String,
    /// The root path of the cartridge drive as supplied by the caller.
    pub drive_path: String,
    /// True when the game's files live on the cartridge itself, rather than the
    /// cartridge being a key that points at an installed copy.
    ///
    /// The launcher uses this to make Eject ask twice: pulling a drive that a
    /// running game is reading from is a different mistake to pulling one that
    /// only holds a text file.
    pub holds_game: bool,
    /// True when this is a multi-game bundle with a `[collection]` + `[game]`
    /// format. Single-game cartridges have `false` here and an empty `games`
    /// vec for full backwards compatibility.
    pub is_bundle: bool,
    /// Individual games, populated only for bundles (`is_bundle == true`).
    pub games: Vec<GameEntry>,
    /// The stylesheet the cartridge carries, or empty.
    ///
    /// The only way a cartridge changes how the launcher looks. Read here and
    /// handed over as text, the same way the artwork is read here and handed
    /// over as a `data:` URI — the window never opens a path on the drive, so
    /// there is one place that decides what a cartridge may be read for.
    pub skin_css: String,
    /// A single game's HowLongToBeat figures. A collection's are per game.
    pub how_long: HowLong,
    /// A combo drive, which the launcher can also show as a memory card.
    pub memory_card: bool,
    /// What the cartridge is for: `PC`, `SNES`, `GBA`... See [`platform`].
    pub platform: String,
}

impl CartridgeInfo {
    /// What the launcher shows for a game on another PC (see
    /// [`crate::remote`]): the host's title and cover, Play standing for the
    /// stream, and `drive_path` the marker every drive command recognises.
    pub fn remote(marker: &str, title: &str, cover: &str) -> CartridgeInfo {
        CartridgeInfo {
            title: title.to_string(),
            cover_path: String::new(),
            cover: cover.to_string(),
            background: String::new(),
            background_path: String::new(),
            logo: String::new(),
            logo_path: String::new(),
            icon: String::new(),
            icon_path: String::new(),
            executable: marker.to_string(),
            drive_path: marker.to_string(),
            holds_game: false,
            is_bundle: false,
            games: Vec::new(),
            skin_css: String::new(),
            how_long: HowLong::default(),
            memory_card: false,
            platform: "PC".to_string(),
        }
    }
}

/// A cartridge's `platform=`, tidied: trimmed, and empty meaning a PC game.
///
/// What the cartridge is *for*, so a front-end that draws the physical thing —
/// a 3D shelf, an Android "cartridge slot", a socket-style console UI — can
/// pick the right shell: a SNES cartridge, a Game Boy one, a PC game box.
/// Free text, compared without regard to case. The names front-ends should
/// recognise are listed in `cartridge.conf.example`; an unknown one is a
/// front-end's to draw as it likes, never an error.
pub fn platform(value: Option<&String>) -> String {
    match value.map(|v| v.trim()).filter(|v| !v.is_empty()) {
        Some(v) => v.to_string(),
        None => "PC".to_string(),
    }
}

/// Set the cartridge's own `platform=` in a conf's text: the single game's, or
/// the collection's in its `[collection]` section. `PC`, or nothing, removes it,
/// since that is what no line means. A game's own line inside a `[game]` is
/// left as it is.
pub fn set_platform(conf: &str, value: &str) -> String {
    let value = value.trim().replace(['\n', '\r'], "");
    let mut out: Vec<String> = Vec::new();
    let mut section = String::new();
    let mut placed = false;
    let wanted = !value.is_empty() && !value.eq_ignore_ascii_case("pc");
    let line = format!("platform={value}");
    for raw in conf.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') {
            section = trimmed
                .trim_matches(|c| c == '[' || c == ']')
                .trim()
                .to_lowercase();
            out.push(raw.to_string());
            if section == "collection" && wanted && !placed {
                out.push(line.clone());
                placed = true;
            }
            continue;
        }
        let is_head = section.is_empty() || section == "collection";
        let is_platform = trimmed
            .split_once('=')
            .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("platform"));
        if is_head && is_platform {
            continue;
        }
        out.push(raw.to_string());
    }
    if wanted && !placed {
        // A single game: its keys are at the top, before any section.
        out.insert(0, line);
    }
    let mut text = out.join("\n");
    if conf.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// A stylesheet the cartridge carries, if it has one and it is not absurd.
///
/// `.gamepak/skin.css`, beside the artwork, because that is where a cartridge's
/// own files already live. Capped: this ends up inline in the window, and a
/// stylesheet large enough to matter is a mistake rather than a design.
///
/// The launcher used to ship a list of looks that a cartridge picked from by
/// name. It does not any more: a cartridge that wants to look like something
/// brings the stylesheet, and one that does not gets the stock window. The
/// examples that used to ship are in `docs/skins/`.
pub fn skin_css(root: &Path) -> String {
    let path = root.join(crate::create::ASSET_DIR).join("skin.css");
    match std::fs::metadata(&path) {
        Ok(meta) if meta.len() <= MAX_SKIN_BYTES => {
            std::fs::read_to_string(&path).unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// Does this cartridge carry the game, or just point at it?
pub fn holds_game(root: &Path) -> bool {
    // Written by the wizard's "copy the game" step: the Steam library layout
    // for a Steam game, the wizard's own Games/ folder for a portable one, or
    // a ROM in its system's folder for an emulated one.
    root.join("steamapps").join("common").is_dir()
        || root
            .join(crate::steamlib::LIBRARY_DIR)
            .join("steamapps/common")
            .is_dir()
        || root.join("Games").is_dir()
        || crate::emulated::carries_rom(root)
}

/// Largest cover we will base64 into the webview. A cartridge is not a trusted
/// input, and a 200 MB "cover" should fail rather than be inlined.
pub const MAX_COVER_BYTES: u64 = 8 * 1024 * 1024;

/// A cartridge's own stylesheet is inlined into the window, so it is held to a
/// size that stays a stylesheet. The launcher's own five are 4 KB each.
pub const MAX_SKIN_BYTES: u64 = 256 * 1024;

// --------------------------------------------------------------------------
// Inline INI / conf file parser
//
// Handles both Windows autorun.inf ([section] key=value) and our flat
// cartridge.conf (key=value, no sections required).
// --------------------------------------------------------------------------

pub type IniMap = HashMap<String, HashMap<String, String>>;

pub fn parse_ini(content: &str) -> IniMap {
    let mut map: IniMap = HashMap::new();
    let mut current_section = String::from("general");

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                current_section = line[1..end].trim().to_lowercase();
            }
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_lowercase();
            let val = line[eq + 1..].trim().to_string();
            map.entry(current_section.clone())
                .or_default()
                .insert(key, val);
        }
    }
    map
}

pub fn ini_get<'a>(map: &'a IniMap, section: &str, key: &str) -> Option<&'a String> {
    map.get(section)?.get(key)
}

/// Extract every `[game]` section in order, preserving duplicates.
///
/// A regular `IniMap` / `HashMap` collapses repeated sections; this returns
/// each `[game]` block as its own key→value map so a multi-game bundle is
/// parsed correctly.
pub fn parse_game_sections(content: &str) -> Vec<HashMap<String, String>> {
    let mut result: Vec<HashMap<String, String>> = Vec::new();
    let mut in_game = false;
    let mut current: HashMap<String, String> = HashMap::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                let section = line[1..end].trim().to_lowercase();
                if in_game && !current.is_empty() {
                    result.push(current.clone());
                    current.clear();
                } else if in_game {
                    current.clear();
                }
                in_game = section == "game";
            }
            continue;
        }
        if in_game {
            if let Some(eq) = line.find('=') {
                let key = line[..eq].trim().to_lowercase();
                let val = line[eq + 1..].trim().to_string();
                current.insert(key, val);
            }
        }
    }
    if in_game && !current.is_empty() {
        result.push(current);
    }
    result
}

// --------------------------------------------------------------------------
// Helper: read cartridge metadata
//
// Priority:
//   1. cartridge.conf  (our own flat format — either the legacy single-game
//      key=value style, or the new [collection] + [game] bundle format)
//   2. autorun.inf     (classic Windows autorun, section [autorun])
// --------------------------------------------------------------------------

pub fn read_cartridge_info(drive_path: &str) -> Result<CartridgeInfo, String> {
    let root = Path::new(drive_path);

    // ---- Try cartridge.conf first ----
    let conf_path = root.join("cartridge.conf");
    if conf_path.exists() {
        let content = std::fs::read_to_string(&conf_path)
            .map_err(|e| format!("Failed to read cartridge.conf: {e}"))?;

        let ini = parse_ini(&content);
        let game_sections = parse_game_sections(&content);

        // ---- Bundle format: one or more [game] sections ----
        if !game_sections.is_empty() {
            let title = ini_get(&ini, "collection", "title")
                .cloned()
                .unwrap_or_else(|| "Game Collection".to_string());

            let cover_rel = ini_get(&ini, "collection", "cover")
                .cloned()
                .unwrap_or_default();
            let cover_path = resolve_cover(root, &cover_rel);
            let background_path = ini_get(&ini, "collection", "background")
                .map(|rel| resolve_cover(root, rel))
                .unwrap_or_default();
            let logo_path = ini_get(&ini, "collection", "logo")
                .map(|rel| resolve_cover(root, rel))
                .unwrap_or_default();
            let icon_path = ini_get(&ini, "collection", "icon")
                .map(|rel| resolve_cover(root, rel))
                .unwrap_or_default();

            let collection_platform = ini_get(&ini, "collection", "platform").cloned();
            let games: Vec<GameEntry> = game_sections
                .iter()
                .map(|g| {
                    let game_title = g
                        .get("title")
                        .cloned()
                        .unwrap_or_else(|| "Unknown Game".to_string());
                    let game_exec = g.get("executable").cloned().unwrap_or_default();
                    // Only the cover falls back to guessing at some image on
                    // the drive when the key is absent, exactly as it does for
                    // the cartridge itself. An absent hero is an absent hero.
                    let cover_path =
                        resolve_cover(root, &g.get("cover").cloned().unwrap_or_default());
                    let art = |key: &str| {
                        g.get(key)
                            .map(|rel| resolve_cover(root, rel))
                            .unwrap_or_default()
                    };
                    let background_path = art("background");
                    let logo_path = art("logo");
                    let icon_path = art("icon");

                    GameEntry {
                        title: game_title,
                        executable: game_exec,
                        cover: cover_as_data_uri(&cover_path),
                        cover_path,
                        background: cover_as_data_uri(&background_path),
                        background_path,
                        logo: cover_as_data_uri(&logo_path),
                        logo_path,
                        icon: cover_as_data_uri(&icon_path),
                        icon_path,
                        how_long: HowLong::from_keys(Some(g)),
                        platform: platform(g.get("platform").or(collection_platform.as_ref())),
                    }
                })
                .collect();

            // Expose the first game's executable as the primary so keyboard
            // shortcuts (Enter = play) still work for bundles.
            let executable = games
                .first()
                .map(|g| g.executable.clone())
                .unwrap_or_default();

            return Ok(CartridgeInfo {
                title,
                cover: cover_as_data_uri(&cover_path),
                background: cover_as_data_uri(&background_path),
                background_path,
                logo: cover_as_data_uri(&logo_path),
                logo_path,
                icon: cover_as_data_uri(&icon_path),
                icon_path,
                cover_path,
                executable,
                drive_path: drive_path.to_string(),
                holds_game: holds_game(root),
                is_bundle: true,
                games,
                skin_css: skin_css(root),
                how_long: HowLong::default(),
                memory_card: crate::memcard::is_combo_drive(root, &content),
                platform: platform(collection_platform.as_ref()),
            });
        }

        // ---- Single-game format: flat key=value (backwards compatible) ----
        let executable = ini_get(&ini, "general", "executable")
            .cloned()
            .unwrap_or_default();

        let title = ini_get(&ini, "general", "title")
            .cloned()
            .unwrap_or_else(|| "Unknown Game".to_string());

        let cover_rel = ini_get(&ini, "general", "cover")
            .cloned()
            .unwrap_or_default();
        let cover_path = resolve_cover(root, &cover_rel);

        // Unlike `cover`, an absent background or logo must stay absent: both
        // go through `resolve_cover`, but only when a key was actually given —
        // otherwise it falls back to guessing at *some* image file on the
        // drive, which is the right behaviour for the primary cover and the
        // wrong one for these.
        let background_path = ini_get(&ini, "general", "background")
            .map(|rel| resolve_cover(root, rel))
            .unwrap_or_default();
        let logo_path = ini_get(&ini, "general", "logo")
            .map(|rel| resolve_cover(root, rel))
            .unwrap_or_default();
        let icon_path = ini_get(&ini, "general", "icon")
            .map(|rel| resolve_cover(root, rel))
            .unwrap_or_default();

        return Ok(CartridgeInfo {
            title,
            cover: cover_as_data_uri(&cover_path),
            background: cover_as_data_uri(&background_path),
            background_path,
            logo: cover_as_data_uri(&logo_path),
            logo_path,
            icon: cover_as_data_uri(&icon_path),
            icon_path,
            cover_path,
            executable,
            drive_path: drive_path.to_string(),
            holds_game: holds_game(root),
            is_bundle: false,
            games: Vec::new(),
            skin_css: skin_css(root),
            how_long: HowLong::from_keys(ini.get("general")),
            memory_card: crate::memcard::is_combo_drive(root, &content),
            platform: platform(ini_get(&ini, "general", "platform")),
        });
    }

    // ---- Fall back to autorun.inf ----
    let autorun_path = root.join("autorun.inf");
    if autorun_path.exists() {
        let content = std::fs::read_to_string(&autorun_path)
            .map_err(|e| format!("Failed to read autorun.inf: {e}"))?;

        let ini = parse_ini(&content);

        // `open=` and `shellexecute=` are deliberately not used as the Play
        // target. Windows has ignored them on non-optical media since Windows 7
        // — they are the original autorun malware vector. A drive that has only
        // an autorun.inf can still be shown in the launcher (label + cover art),
        // but Play will be disabled because there is no executable to run.
        let title = ini_get(&ini, "autorun", "label")
            .cloned()
            .unwrap_or_else(|| "Unknown Game".to_string());

        let icon_rel = ini_get(&ini, "autorun", "icon")
            .cloned()
            .unwrap_or_default();
        let cover_path = resolve_cover(root, &icon_rel);

        return Ok(CartridgeInfo {
            title,
            cover: cover_as_data_uri(&cover_path),
            background: String::new(),
            background_path: String::new(),
            logo: String::new(),
            logo_path: String::new(),
            // An autorun-only drive has one picture and it is the icon, which
            // is already standing in as the cover above.
            icon: String::new(),
            icon_path: String::new(),
            cover_path,
            // Empty: autorun.inf's open= and shellexecute= are not used as the
            // play target; Play is intentionally disabled for autorun-only drives.
            executable: String::new(),
            drive_path: drive_path.to_string(),
            holds_game: holds_game(root),
            is_bundle: false,
            skin_css: skin_css(root),
            how_long: HowLong::default(),
            games: Vec::new(),
            memory_card: false,
            platform: platform(None),
        });
    }

    Err(format!(
        "No cartridge.conf or autorun.inf found in {drive_path}"
    ))
}

/// Resolve a relative cover image path, falling back to common filenames.
pub fn resolve_cover(root: &Path, rel: &str) -> String {
    if !rel.is_empty() {
        if let Some(p) = join_within(root, rel) {
            if p.is_file() {
                return p.to_string_lossy().to_string();
            }
        }
    }
    find_cover_image(root)
}

/// Join a cartridge-supplied relative path onto the drive root, refusing
/// anything that would leave the drive.
///
/// `cover=` comes out of a file on a volume someone else may have written, so
/// `..\..\Users\me\.ssh\id_rsa` has to be rejected rather than read and handed
/// to the webview.
pub(crate) fn join_within(root: &Path, rel: &str) -> Option<PathBuf> {
    use std::path::Component;

    let candidate = Path::new(rel);
    // Absolute paths and drive-qualified paths (`C:\…`) are never relative to
    // this cartridge.
    if candidate.is_absolute() || rel.contains(':') {
        return None;
    }

    let mut out = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            // Any climb out, and any root or drive prefix, disqualifies it.
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out.as_os_str().is_empty() {
        return None;
    }
    Some(root.join(out))
}

/// Read a cover image and encode it as a `data:` URI. Any failure yields an
/// empty string: a missing or oversized cover is not a reason to refuse the
/// cartridge, the placeholder just stays.
pub fn cover_as_data_uri(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let p = Path::new(path);
    match std::fs::metadata(p) {
        Ok(meta) if meta.len() > MAX_COVER_BYTES => return String::new(),
        Ok(_) => {}
        Err(_) => return String::new(),
    }
    let Ok(bytes) = std::fs::read(p) else {
        return String::new();
    };

    let mime = match p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        _ => "image/png",
    };

    format!("data:{mime};base64,{}", base64_encode(&bytes))
}

/// Look for common cover image filenames in the root of the cartridge.
pub fn find_cover_image(root: &Path) -> String {
    let candidates = [
        "cover.png",
        "cover.jpg",
        "cover.jpeg",
        "cover.webp",
        "poster.png",
        "poster.jpg",
        "box.png",
        "box.jpg",
    ];
    for name in &candidates {
        let p = root.join(name);
        if p.is_file() {
            return p.to_string_lossy().to_string();
        }
    }
    String::new()
}

/// Pull the value of `--drive` out of an argument list.
///
/// Accepts `--drive X` and `--drive=X`. Returns an empty string when absent,
/// which the frontend reports as "no cartridge" rather than guessing.
pub fn drive_from_args<I: Iterator<Item = String>>(args: I) -> String {
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "--drive" {
            return args.next().unwrap_or_default();
        }
        if let Some(value) = arg.strip_prefix("--drive=") {
            return value.to_string();
        }
    }
    String::new()
}

/// Minimal base64 encoder — avoids adding a `base64` crate dependency.
pub fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };
        out.push(CHARS[(b0 >> 2) & 0x3F] as char);
        out.push(CHARS[((b0 << 4) | (b1 >> 4)) & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            CHARS[((b1 << 2) | (b2 >> 6)) & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[b2 & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    // Artwork lives under .gamepak now, so the reader has to resolve a path with
    // a separator in it — and still refuse one that climbs out of the drive.
    #[test]
    fn how_long_to_beat_is_read_per_game_and_for_a_single_game() {
        let scratch = crate::testutil::Scratch::new("cart-hltb");
        std::fs::write(
            scratch.path().join("cartridge.conf"),
            "title=Hollow Knight\nexecutable=steam://rungameid/367520\nhltb_main=97200\nhltb_complete=226800\n",
        )
        .unwrap();
        let info = read_cartridge_info(&scratch.path().to_string_lossy()).unwrap();
        assert_eq!(
            info.how_long,
            HowLong {
                main: 97200,
                extra: 0,
                complete: 226800
            }
        );

        std::fs::write(
            scratch.path().join("cartridge.conf"),
            "[collection]\ntitle=C\n\n[game]\ntitle=A\nexecutable=x://1\nhltb_main=3600\n\n[game]\ntitle=B\nexecutable=x://2\n",
        )
        .unwrap();
        let info = read_cartridge_info(&scratch.path().to_string_lossy()).unwrap();
        assert_eq!(info.games[0].how_long.main, 3600);
        assert_eq!(info.games[1].how_long, HowLong::default());
    }

    #[test]
    fn art_in_the_asset_folder_is_found() {
        let scratch = crate::testutil::Scratch::new("asset-dir");
        std::fs::create_dir_all(scratch.path().join(".gamepak")).unwrap();
        std::fs::write(scratch.path().join(".gamepak").join("cover.png"), b"art").unwrap();
        scratch.write(
            "cartridge.conf",
            b"title=Tunic
executable=steam://rungameid/553420
cover=.gamepak/cover.png
",
        );

        let info = super::read_cartridge_info(&scratch.path().to_string_lossy()).unwrap();
        assert!(
            info.cover_path.ends_with("cover.png"),
            "expected the asset folder's cover, got {:?}",
            info.cover_path
        );
        assert!(
            !info.cover.is_empty(),
            "it should have been read and inlined"
        );
    }

    use super::*;

    #[test]
    fn reads_a_cartridge_conf() {
        let scratch = crate::testutil::Scratch::new("conf");
        std::fs::write(
            scratch.join("cartridge.conf"),
            "title=Hollow Knight\nexecutable=steam://rungameid/367520\n",
        )
        .unwrap();

        let info = read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert_eq!(info.title, "Hollow Knight");
        assert_eq!(info.executable, "steam://rungameid/367520");
        assert!(!info.holds_game);
        assert!(!info.is_bundle);
        assert!(info.games.is_empty());
    }

    #[test]
    fn the_cartridge_s_platform_is_set_where_it_belongs() {
        // A single game: at the top, replacing any old line.
        assert_eq!(
            set_platform("title=Zelda\nplatform=NES\nexecutable=z\n", "SNES"),
            "platform=SNES\ntitle=Zelda\nexecutable=z\n"
        );
        // PC, or nothing, is no line at all.
        assert_eq!(set_platform("title=X\nplatform=GBA\n", "PC"), "title=X\n");
        assert_eq!(set_platform("title=X\n", ""), "title=X\n");
        // A collection: in its own section, and a game's own line is left alone.
        let conf = "memory_card=yes\n[collection]\ntitle=N\nplatform=NES\n\n\
                    [game]\ntitle=P\nplatform=GBA\nexecutable=p\n";
        assert_eq!(
            set_platform(conf, "SNES"),
            "memory_card=yes\n[collection]\nplatform=SNES\ntitle=N\n\n\
             [game]\ntitle=P\nplatform=GBA\nexecutable=p\n"
        );
    }

    #[test]
    fn a_cartridge_says_what_platform_it_is_for() {
        let scratch = crate::testutil::Scratch::new("platform");
        let read = |conf: &str| {
            std::fs::write(scratch.join("cartridge.conf"), conf).unwrap();
            read_cartridge_info(scratch.path().to_str().unwrap()).unwrap()
        };
        assert_eq!(read("title=Hades\nexecutable=x.exe\n").platform, "PC");
        assert_eq!(
            read("title=Zelda\nplatform= SNES \nexecutable=z.sfc\n").platform,
            "SNES"
        );

        // A collection's platform is each game's unless the game says otherwise.
        let info = read(
            "[collection]\ntitle=Nintendo\nplatform=SNES\n\n\
             [game]\ntitle=Zelda\nexecutable=z.sfc\n\n\
             [game]\ntitle=Pokemon\nplatform=GBA\nexecutable=p.gba\n",
        );
        assert_eq!(info.platform, "SNES");
        let games: Vec<&str> = info.games.iter().map(|g| g.platform.as_str()).collect();
        assert_eq!(games, ["SNES", "GBA"]);
    }

    #[test]
    fn a_single_game_can_carry_a_background_and_logo_too() {
        let scratch = crate::testutil::Scratch::new("art");
        std::fs::write(scratch.join("cover.jpg"), b"cover").unwrap();
        std::fs::write(scratch.join("background.jpg"), b"background").unwrap();
        std::fs::write(scratch.join("logo.png"), b"logo").unwrap();
        std::fs::write(
            scratch.join("cartridge.conf"),
            "title=Hollow Knight\nexecutable=steam://rungameid/367520\n\
             cover=cover.jpg\nbackground=background.jpg\nlogo=logo.png\n",
        )
        .unwrap();

        let info = read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert!(info.cover.starts_with("data:"));
        assert!(info.background.starts_with("data:"));
        assert!(info.background_path.ends_with("background.jpg"));
        assert!(info.logo.starts_with("data:"));
        assert!(info.logo_path.ends_with("logo.png"));
    }

    #[test]
    fn a_single_game_with_no_background_or_logo_key_gets_neither() {
        // Critically, this must not fall back to guessing at *some* image on
        // the drive the way the bare cover does — an unset background/logo
        // has to stay unset.
        let scratch = crate::testutil::Scratch::new("no-art");
        std::fs::write(scratch.join("cover.jpg"), b"cover").unwrap();
        std::fs::write(
            scratch.join("cartridge.conf"),
            "title=Hollow Knight\nexecutable=steam://rungameid/367520\ncover=cover.jpg\n",
        )
        .unwrap();

        let info = read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert!(info.background.is_empty());
        assert!(info.background_path.is_empty());
        assert!(info.logo.is_empty());
        assert!(info.logo_path.is_empty());
    }

    #[test]
    fn reads_a_bundle_cartridge_conf() {
        let scratch = crate::testutil::Scratch::new("bundle");
        std::fs::write(
            scratch.join("cartridge.conf"),
            "[collection]\ntitle=God of War Collection\ncover=collection.jpg\n\n\
             [game]\ntitle=God of War 2018\nexecutable=steam://rungameid/310970\n\n\
             [game]\ntitle=God of War Ragnarök\nexecutable=steam://rungameid/1476670\n",
        )
        .unwrap();

        let info = read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert!(info.is_bundle);
        assert_eq!(info.title, "God of War Collection");
        assert_eq!(info.games.len(), 2);
        assert_eq!(info.games[0].title, "God of War 2018");
        assert_eq!(info.games[0].executable, "steam://rungameid/310970");
        assert_eq!(info.games[1].title, "God of War Ragnarök");
        assert_eq!(info.games[1].executable, "steam://rungameid/1476670");
        // Primary executable is the first game's for keyboard shortcuts.
        assert_eq!(info.executable, "steam://rungameid/310970");
    }

    #[test]
    fn parse_game_sections_preserves_order() {
        let content = "[collection]\ntitle=Test\n\n\
                       [game]\ntitle=A\nexecutable=steam://1\n\n\
                       [game]\ntitle=B\nexecutable=steam://2\n";
        let sections = parse_game_sections(content);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].get("title").unwrap(), "A");
        assert_eq!(sections[1].get("title").unwrap(), "B");
    }

    #[test]
    fn single_game_conf_is_not_a_bundle() {
        let scratch = crate::testutil::Scratch::new("single");
        std::fs::write(
            scratch.join("cartridge.conf"),
            "title=Cyberpunk 2077\nexecutable=steam://rungameid/1091500\n",
        )
        .unwrap();

        let info = read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert!(!info.is_bundle);
        assert!(info.games.is_empty());
    }

    #[test]
    fn a_cartridge_carrying_the_game_says_so() {
        let scratch = crate::testutil::Scratch::new("holds");
        std::fs::write(
            scratch.join("cartridge.conf"),
            "title=X\nexecutable=steam://rungameid/1\n",
        )
        .unwrap();
        assert!(
            !read_cartridge_info(scratch.path().to_str().unwrap())
                .unwrap()
                .holds_game
        );

        std::fs::create_dir_all(scratch.join("steamapps/common/X")).unwrap();
        assert!(
            read_cartridge_info(scratch.path().to_str().unwrap())
                .unwrap()
                .holds_game
        );
    }

    #[test]
    fn a_portable_copy_also_counts_as_carrying_the_game() {
        let scratch = crate::testutil::Scratch::new("holds-portable");
        std::fs::write(
            scratch.join("cartridge.conf"),
            "title=X\nexecutable=Games/X/X.exe\n",
        )
        .unwrap();
        assert!(
            !read_cartridge_info(scratch.path().to_str().unwrap())
                .unwrap()
                .holds_game
        );

        std::fs::create_dir_all(scratch.join("Games/X")).unwrap();
        assert!(
            read_cartridge_info(scratch.path().to_str().unwrap())
                .unwrap()
                .holds_game
        );
    }

    #[test]
    fn autorun_supplies_a_label_but_never_an_executable() {
        let scratch = crate::testutil::Scratch::new("autorun");
        std::fs::write(
            scratch.join("autorun.inf"),
            "[autorun]\r\nlabel=Legacy Disc\r\nicon=cover.ico\r\nopen=evil.exe\r\n",
        )
        .unwrap();
        let info = read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert_eq!(info.title, "Legacy Disc");
        // open= and shellexecute= must never reach the launcher as a play target.
        assert!(
            info.executable.is_empty(),
            "executable should be empty, got {:?}",
            info.executable
        );
    }

    #[test]
    fn a_volume_with_neither_file_is_not_a_cartridge() {
        let scratch = crate::testutil::Scratch::new("empty");
        assert!(read_cartridge_info(scratch.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn cover_paths_stay_on_the_cartridge() {
        let root = Path::new("/media/x");
        assert_eq!(
            join_within(root, "art/cover.png"),
            Some(root.join("art/cover.png"))
        );
        for bad in [
            "../../../etc/passwd",
            "/etc/passwd",
            "C:\\Windows\\SAM",
            "..",
        ] {
            assert_eq!(join_within(root, bad), None, "{bad}");
        }
    }

    #[test]
    fn base64_matches_the_reference_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn drive_from_args_reads_both_forms() {
        let v = |a: &[&str]| drive_from_args(a.iter().map(|s| s.to_string()));
        assert_eq!(v(&["--drive", "/run/media/h/CART"]), "/run/media/h/CART");
        assert_eq!(v(&["--drive=D:\\"]), "D:\\");
        assert_eq!(v(&["--create"]), "");
        assert_eq!(v(&["--drive"]), "");
    }
}
