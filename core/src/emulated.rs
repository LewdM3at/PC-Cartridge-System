//! Cartridges for emulated games: the ROM on the drive, and the files other
//! front-ends read to list it.
//!
//! A cartridge that says `platform=SNES` (anything but PC) is for an emulator,
//! and the emulator front-ends people already use — Pegasus, ES-DE, Daijishō,
//! LaunchBox, on a PC or on an Android phone over USB-C — do not read
//! `cartridge.conf`. What they do read is plain files in known places, so the
//! cartridge carries those too:
//!
//! ```text
//! cartridge.conf
//! metadata.pegasus.txt        Pegasus: a collection of what is on the drive
//! snes/Chrono Trigger.sfc     the ROM, in the folder ES-DE calls that system
//! snes/gamelist.xml           EmulationStation's titles and art for the folder
//! .gamepak/cover.png          the art the wizard already wrote
//! ```
//!
//! The folder names are ES-DE's system names. Daijishō and LaunchBox scan any
//! folder they are pointed at, so one layout serves all of them, and none of it
//! needs a program running on the phone.
//!
//! Nothing here picks an emulator. Which one plays SNES is the host's business
//! and differs between a PC, a Deck and a phone; on a PC the launcher hands the
//! ROM to whatever the desktop opens that file type with (see
//! [`crate::launch`]).
//!
//! Every file this writes says so on its first line, and only such files are
//! replaced or removed. A `gamelist.xml` somebody scraped by hand is left alone.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::cartridge::{ini_get, parse_game_sections, parse_ini};

/// The Pegasus metadata file, at the cartridge's root.
pub const PEGASUS_FILE: &str = "metadata.pegasus.txt";
/// EmulationStation's per-system list, inside each system folder.
pub const GAMELIST_FILE: &str = "gamelist.xml";

/// The mark on the first line of every file written here.
const MARK: &str = "Written by PC GamePak";

/// Whether a cartridge with this `platform=` is for an emulator.
pub fn is_emulated(platform: &str) -> bool {
    let platform = platform.trim();
    !platform.is_empty() && !platform.eq_ignore_ascii_case("PC")
}

/// The folder a system's ROMs go in: ES-DE's name for it.
///
/// For the names `cartridge.conf.example` lists. Anything else is lowercased
/// and stripped to letters and digits, which is how ES-DE names most systems
/// anyway, so a platform this table has not heard of still gets a sensible
/// folder rather than an error.
pub fn system_dir(platform: &str) -> String {
    let lower = platform.trim().to_ascii_lowercase();
    let known = match lower.as_str() {
        "gamecube" => "gc",
        "3ds" => "n3ds",
        "32x" => "sega32x",
        "ps1" | "psx" | "playstation" => "psx",
        "vita" => "psvita",
        "lynx" => "atarilynx",
        "jaguar" => "atarijaguar",
        "turbografx16" | "turbografx-16" | "pcengine" => "pcengine",
        "neogeopocket" => "ngp",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let cleaned: String = lower.chars().filter(char::is_ascii_alphanumeric).collect();
    if cleaned.is_empty() {
        "roms".to_string()
    } else {
        cleaned
    }
}

/// A platform guessed from a ROM's file extension, for the wizard to offer.
///
/// Only extensions that belong to one system. A `.zip` or an `.iso` could be
/// anything, so those give no guess and the person picks.
pub fn platform_for_extension(extension: &str) -> Option<&'static str> {
    Some(
        match extension
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "nes" | "fds" => "NES",
            "sfc" | "smc" => "SNES",
            "n64" | "z64" | "v64" => "N64",
            "gb" => "GB",
            "gbc" => "GBC",
            "gba" => "GBA",
            "nds" => "NDS",
            "3ds" | "cia" => "3DS",
            "vb" => "VirtualBoy",
            "sms" => "MasterSystem",
            "md" | "gen" | "smd" => "Genesis",
            "32x" => "32X",
            "gg" => "GameGear",
            "gcm" | "rvz" => "GameCube",
            "wbfs" => "Wii",
            "wua" | "wux" | "rpx" => "WiiU",
            "nsp" | "xci" => "Switch",
            "pbp" => "PSP",
            "vpk" => "Vita",
            "a26" => "Atari2600",
            "a78" => "Atari7800",
            "lnx" => "Lynx",
            "j64" => "Jaguar",
            "pce" => "TurboGrafx16",
            "ngp" | "ngc" => "NeoGeoPocket",
            "ws" | "wsc" => "WonderSwan",
            _ => return None,
        },
    )
}

/// Whether `path` names a ROM or disc image rather than a program.
///
/// The launcher asks this before starting a file on the cartridge: a program
/// is run, a ROM is opened with whatever the desktop plays that file type with.
/// The extensions [`platform_for_extension`] knows, plus the disc and playlist
/// formats several systems share.
pub fn is_rom(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    platform_for_extension(extension).is_some()
        || matches!(
            extension.to_ascii_lowercase().as_str(),
            "iso" | "chd" | "cue" | "m3u" | "cso" | "gdi" | "cdi"
        )
}

/// Copy a ROM into its system's folder on the cartridge.
///
/// Returns the path `executable=` should hold, relative to the root and with
/// forward slashes, and the bytes copied. `digests` records the file when the
/// build is checking its own work.
pub fn copy_rom(
    source: &Path,
    root: &Path,
    platform: &str,
    digests: Option<&mut crate::verify::Digests>,
) -> Result<(String, u64), String> {
    if !source.is_file() {
        return Err(format!("{} is not a file", source.display()));
    }
    if source.starts_with(root) {
        return Err(format!(
            "{} is on the cartridge already, so there is nothing to copy",
            source.display()
        ));
    }
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| format!("{} has no file name", source.display()))?;
    let folder = system_dir(platform);
    let destination = root.join(&folder).join(&name);
    std::fs::create_dir_all(root.join(&folder))
        .map_err(|e| format!("Could not make {folder}/ on the cartridge: {e}"))?;
    let (bytes, crc) = crate::verify::copy_and_digest(source, &destination)
        .map_err(|e| format!("{}: {e}", source.display()))?;
    if let Some(digests) = digests {
        digests.record(&destination, bytes, crc);
    }
    Ok((format!("{folder}/{name}"), bytes))
}

/// One game, as the front-end files need it: paths relative to the root, with
/// forward slashes, exactly as `cartridge.conf` gives them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub title: String,
    /// The ROM, relative to the root.
    pub file: String,
    pub platform: String,
    pub cover: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
}

/// The emulated games a `cartridge.conf` describes, with their own art.
///
/// A game counts when its platform is not PC and its `executable=` is a file on
/// the drive. A URI cannot be listed by a front-end that scans files, and a
/// single game falls back to the cartridge's art the way the launcher does.
pub fn games_in(conf: &str) -> Vec<Game> {
    let ini = parse_ini(conf);
    let sections = parse_game_sections(conf);
    let get = |map: &HashMap<String, String>, key: &str| {
        map.get(key)
            .map(|v| v.trim().replace('\\', "/"))
            .filter(|v| !v.is_empty())
    };

    let raw: Vec<(HashMap<String, String>, Option<String>)> = if sections.is_empty() {
        let general = ini.get("general").cloned().unwrap_or_default();
        let platform = general.get("platform").cloned();
        vec![(general, platform)]
    } else {
        let collection = ini_get(&ini, "collection", "platform").cloned();
        sections
            .into_iter()
            .map(|game| {
                let platform = game.get("platform").cloned().or(collection.clone());
                (game, platform)
            })
            .collect()
    };

    raw.into_iter()
        .filter_map(|(game, platform)| {
            let platform = platform.unwrap_or_default().trim().to_string();
            if !is_emulated(&platform) {
                return None;
            }
            let file = get(&game, "executable")?;
            if crate::launch::is_uri(&file) || file.contains("://") {
                return None;
            }
            Some(Game {
                title: game
                    .get("title")
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "Unknown Game".to_string()),
                file,
                platform,
                cover: get(&game, "cover"),
                background: get(&game, "background"),
                logo: get(&game, "logo"),
            })
        })
        .collect()
}

/// Pegasus's metadata for these games: one collection per system.
///
/// No `launch:` line. Pegasus needs one to start anything, and the right one is
/// an emulator on the machine reading the drive — `am start …` on Android,
/// RetroArch with a core on a PC — which a cartridge cannot know. The comment
/// at the top says where to add it.
pub fn pegasus_text(cartridge_title: &str, games: &[Game]) -> String {
    let mut systems: Vec<String> = Vec::new();
    for game in games {
        let dir = system_dir(&game.platform);
        if !systems.contains(&dir) {
            systems.push(dir);
        }
    }

    let mut out = format!(
        "# {MARK}. Rewritten whenever the cartridge is.\n\
         # Add a `launch:` line under a collection to play from Pegasus: the\n\
         # emulator is the machine's choice, not the cartridge's.\n"
    );
    for dir in &systems {
        let in_system: Vec<&Game> = games
            .iter()
            .filter(|g| &system_dir(&g.platform) == dir)
            .collect();
        let name = if systems.len() == 1 {
            one_line(cartridge_title)
        } else {
            format!("{} ({})", one_line(cartridge_title), in_system[0].platform)
        };
        out.push_str(&format!("\ncollection: {name}\nshortname: {dir}\nfiles:\n"));
        for game in &in_system {
            out.push_str(&format!("  {}\n", game.file));
        }
    }
    for game in games {
        out.push_str(&format!(
            "\ngame: {}\nfile: {}\n",
            one_line(&game.title),
            game.file
        ));
        for (key, value) in [
            ("assets.boxFront", &game.cover),
            ("assets.background", &game.background),
            ("assets.logo", &game.logo),
        ] {
            if let Some(value) = value {
                out.push_str(&format!("{key}: {value}\n"));
            }
        }
    }
    out
}

/// EmulationStation's `gamelist.xml` for the games in one system folder.
///
/// Paths are relative to that folder, as EmulationStation reads them. ES-DE
/// takes the names from it and finds its own media; Batocera, RetroBat and the
/// other EmulationStation forks use the art paths as well.
pub fn gamelist_text(dir: &str, games: &[Game]) -> String {
    let prefix = format!("{dir}/");
    let mut out = format!("<?xml version=\"1.0\"?>\n<!-- {MARK}. Rewritten whenever the cartridge is. -->\n<gameList>\n");
    for game in games {
        let Some(file) = game.file.strip_prefix(&prefix) else {
            continue;
        };
        out.push_str("  <game>\n");
        out.push_str(&format!("    <path>./{}</path>\n", xml_escape(file)));
        out.push_str(&format!("    <name>{}</name>\n", xml_escape(&game.title)));
        for (tag, value) in [
            ("image", &game.cover),
            ("fanart", &game.background),
            ("marquee", &game.logo),
        ] {
            if let Some(value) = value {
                out.push_str(&format!("    <{tag}>../{}</{tag}>\n", xml_escape(value)));
            }
        }
        out.push_str("  </game>\n");
    }
    out.push_str("</gameList>\n");
    out
}

/// Write, replace or remove the front-end files to match `cartridge.conf`.
///
/// Called after the conf is written. A cartridge that is no longer emulated
/// loses the files it had, so a drive rewritten from SNES to a PC game does not
/// go on showing up in Pegasus. Problems are warnings: the cartridge itself is
/// already complete.
pub fn sync_files(root: &Path, warnings: &mut Vec<String>) {
    let conf = match std::fs::read_to_string(root.join("cartridge.conf")) {
        Ok(conf) => conf,
        Err(_) => return,
    };
    let games = games_in(&conf);
    let title = cartridge_title(&conf);

    // ---- metadata.pegasus.txt --------------------------------------------
    let pegasus = root.join(PEGASUS_FILE);
    if games.is_empty() {
        remove_ours(&pegasus, warnings);
    } else {
        write_ours(&pegasus, &pegasus_text(&title, &games), warnings);
    }

    // ---- <system>/gamelist.xml ---------------------------------------------
    let mut wanted: Vec<String> = Vec::new();
    for game in &games {
        let dir = system_dir(&game.platform);
        if game.file.starts_with(&format!("{dir}/")) && !wanted.contains(&dir) {
            wanted.push(dir);
        }
    }
    for dir in &wanted {
        let path = root.join(dir).join(GAMELIST_FILE);
        write_ours(&path, &gamelist_text(dir, &games), warnings);
    }
    // A system folder this cartridge no longer uses keeps its ROMs — nothing
    // here deletes a game — but loses a list that would describe them wrongly.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_dir() && !wanted.contains(&name) {
                let list = entry.path().join(GAMELIST_FILE);
                if is_ours(&list) {
                    remove_ours(&list, warnings);
                }
            }
        }
    }
}

/// Whether the cartridge at `root` carries an emulated game's ROM itself, so
/// an emulator playing it is reading from the drive.
pub fn carries_rom(root: &Path) -> bool {
    std::fs::read_to_string(root.join("cartridge.conf"))
        .map(|conf| {
            games_in(&conf)
                .iter()
                .any(|game| root.join(&game.file).is_file())
        })
        .unwrap_or(false)
}

fn cartridge_title(conf: &str) -> String {
    let ini = parse_ini(conf);
    ini_get(&ini, "collection", "title")
        .or_else(|| ini_get(&ini, "general", "title"))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "PC GamePak".to_string())
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Whether `path` is a file this module wrote: its mark in the first two lines.
fn is_ours(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|text| text.lines().take(2).any(|line| line.contains(MARK)))
        .unwrap_or(false)
}

fn write_ours(path: &Path, text: &str, warnings: &mut Vec<String>) {
    if path.exists() && !is_ours(path) {
        warnings.push(format!(
            "{} was already on the cartridge and is not PC GamePak's, so it was left as it is.",
            display(path)
        ));
        return;
    }
    if let Err(e) = std::fs::write(path, text) {
        warnings.push(format!("{} was not written: {e}", display(path)));
    }
}

fn remove_ours(path: &Path, warnings: &mut Vec<String>) {
    if !is_ours(path) {
        return;
    }
    if let Err(e) = std::fs::remove_file(path) {
        warnings.push(format!("{} was not removed: {e}", display(path)));
    }
}

fn display(path: &Path) -> String {
    let parent = path.parent().and_then(Path::file_name).map(PathBuf::from);
    match (parent, path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name).display().to_string(),
        _ => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    const SINGLE: &str = "title=Chrono Trigger\n\
                          executable=snes/Chrono Trigger.sfc\n\
                          cover=.gamepak/cover.png\n\
                          logo=.gamepak/logo.png\n\
                          platform=SNES\n";

    #[test]
    fn only_a_platform_other_than_pc_is_emulated() {
        assert!(is_emulated("SNES"));
        assert!(is_emulated(" gba "));
        assert!(!is_emulated("PC"));
        assert!(!is_emulated("pc"));
        assert!(!is_emulated(""));
    }

    #[test]
    fn systems_use_es_de_folder_names() {
        assert_eq!(system_dir("SNES"), "snes");
        assert_eq!(system_dir("PS1"), "psx");
        assert_eq!(system_dir("GameCube"), "gc");
        assert_eq!(system_dir("3DS"), "n3ds");
        assert_eq!(system_dir("TurboGrafx16"), "pcengine");
        assert_eq!(system_dir("Atari2600"), "atari2600");
        // Not in the table, still a usable folder.
        assert_eq!(system_dir("Pico-8"), "pico8");
        assert_eq!(system_dir("!!"), "roms");
    }

    #[test]
    fn an_extension_suggests_a_platform_only_when_it_is_unambiguous() {
        assert_eq!(platform_for_extension("sfc"), Some("SNES"));
        assert_eq!(platform_for_extension(".GBA"), Some("GBA"));
        assert_eq!(platform_for_extension("zip"), None);
        assert_eq!(platform_for_extension("iso"), None);
    }

    #[test]
    fn roms_and_disc_images_are_not_programs() {
        assert!(is_rom(Path::new("snes/Chrono Trigger.sfc")));
        assert!(is_rom(Path::new("psx/FF7/disc1.CHD")));
        assert!(!is_rom(Path::new("Games/FTL/FTLGame.exe")));
        assert!(!is_rom(Path::new("Games/Celeste/start.sh")));
        assert!(!is_rom(Path::new("Games/noextension")));
    }

    #[test]
    fn a_single_emulated_game_is_read_with_its_art() {
        let games = games_in(SINGLE);
        assert_eq!(
            games,
            [Game {
                title: "Chrono Trigger".into(),
                file: "snes/Chrono Trigger.sfc".into(),
                platform: "SNES".into(),
                cover: Some(".gamepak/cover.png".into()),
                background: None,
                logo: Some(".gamepak/logo.png".into()),
            }]
        );
    }

    #[test]
    fn pc_games_and_uris_are_not_listed() {
        assert!(games_in("title=FTL\nexecutable=steam://rungameid/212680\n").is_empty());
        assert!(games_in("title=X\nexecutable=Games/X/x.exe\n").is_empty());
        assert!(games_in("title=X\nexecutable=retroarch://x\nplatform=SNES\n").is_empty());
    }

    #[test]
    fn a_collection_lists_each_game_on_its_own_platform() {
        let conf = "[collection]\ntitle=Handhelds\nplatform=GBA\n\n\
                    [game]\ntitle=Minish Cap\nexecutable=gba\\minish.gba\n\n\
                    [game]\ntitle=Link's Awakening\nexecutable=gbc/la.gbc\nplatform=GBC\n\n\
                    [game]\ntitle=Celeste\nexecutable=Games/Celeste/Celeste.exe\nplatform=PC\n";
        let games = games_in(conf);
        let summary: Vec<(&str, &str)> = games
            .iter()
            .map(|g| (g.file.as_str(), g.platform.as_str()))
            .collect();
        assert_eq!(summary, [("gba/minish.gba", "GBA"), ("gbc/la.gbc", "GBC")]);

        let text = pegasus_text("Handhelds", &games);
        assert!(text
            .contains("collection: Handhelds (GBA)\nshortname: gba\nfiles:\n  gba/minish.gba\n"));
        assert!(text.contains("collection: Handhelds (GBC)\nshortname: gbc\n"));
        assert!(text.contains("game: Link's Awakening\nfile: gbc/la.gbc\n"));
    }

    #[test]
    fn pegasus_gets_one_collection_named_after_the_cartridge() {
        let text = pegasus_text("Chrono  Trigger", &games_in(SINGLE));
        assert!(text.starts_with("# Written by PC GamePak"));
        assert!(text.contains("collection: Chrono Trigger\nshortname: snes\n"));
        assert!(text.contains("assets.boxFront: .gamepak/cover.png\n"));
        assert!(text.contains("assets.logo: .gamepak/logo.png\n"));
        assert!(
            !text.contains("\nlaunch:"),
            "the emulator is the host's choice"
        );
    }

    #[test]
    fn the_gamelist_is_relative_to_its_folder_and_escaped() {
        let mut games = games_in(SINGLE);
        games[0].title = "Tom & Jerry <2>".into();
        let text = gamelist_text("snes", &games);
        assert!(text.contains("<path>./Chrono Trigger.sfc</path>"));
        assert!(text.contains("<name>Tom &amp; Jerry &lt;2&gt;</name>"));
        assert!(text.contains("<image>../.gamepak/cover.png</image>"));
        assert!(text.contains("<marquee>../.gamepak/logo.png</marquee>"));
        assert!(!text.contains("<fanart>"));
    }

    #[test]
    fn a_rom_is_copied_into_its_system_folder() {
        let scratch = Scratch::new("emulated-copy");
        let source = scratch.path().join("Chrono Trigger.sfc");
        std::fs::write(&source, b"rom").unwrap();
        let root = scratch.path().join("cart");
        std::fs::create_dir_all(&root).unwrap();

        let mut digests = crate::verify::Digests::new(&root);
        let (relative, bytes) = copy_rom(&source, &root, "SNES", Some(&mut digests)).unwrap();
        assert_eq!(relative, "snes/Chrono Trigger.sfc");
        assert_eq!(bytes, 3);
        assert_eq!(std::fs::read(root.join(&relative)).unwrap(), b"rom");
        assert_eq!(digests.into_manifest().files.len(), 1);

        assert!(copy_rom(&root.join(&relative), &root, "SNES", None).is_err());
    }

    #[test]
    fn sync_writes_replaces_and_removes_only_its_own_files() {
        let scratch = Scratch::new("emulated-sync");
        let root = scratch.path();
        std::fs::create_dir_all(root.join("snes")).unwrap();
        std::fs::write(root.join("cartridge.conf"), SINGLE).unwrap();

        let mut warnings = Vec::new();
        sync_files(root, &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(root.join(PEGASUS_FILE).is_file());
        assert!(root.join("snes").join(GAMELIST_FILE).is_file());
        assert!(!carries_rom(root), "the ROM itself is not there yet");
        std::fs::write(root.join("snes").join("Chrono Trigger.sfc"), b"rom").unwrap();
        assert!(carries_rom(root));

        // Rewritten as a GBA cartridge: the SNES list goes, the ROM stays.
        std::fs::create_dir_all(root.join("gba")).unwrap();
        std::fs::write(
            root.join("cartridge.conf"),
            "title=Minish Cap\nexecutable=gba/minish.gba\nplatform=GBA\n",
        )
        .unwrap();
        std::fs::write(root.join("snes").join("Chrono Trigger.sfc"), b"rom").unwrap();
        sync_files(root, &mut warnings);
        assert!(!root.join("snes").join(GAMELIST_FILE).exists());
        assert!(root.join("snes").join("Chrono Trigger.sfc").exists());
        assert!(root.join("gba").join(GAMELIST_FILE).is_file());

        // A list somebody scraped by hand is never replaced.
        std::fs::write(root.join("gba").join(GAMELIST_FILE), "<gameList/>").unwrap();
        sync_files(root, &mut warnings);
        assert_eq!(
            std::fs::read_to_string(root.join("gba").join(GAMELIST_FILE)).unwrap(),
            "<gameList/>"
        );
        assert_eq!(warnings.len(), 1);

        // And a PC cartridge carries none of it.
        std::fs::write(
            root.join("cartridge.conf"),
            "title=FTL\nexecutable=steam://rungameid/212680\n",
        )
        .unwrap();
        sync_files(root, &mut warnings);
        assert!(!root.join(PEGASUS_FILE).exists());
        assert!(
            root.join("gba").join(GAMELIST_FILE).exists(),
            "not ours, so kept"
        );
    }
}
