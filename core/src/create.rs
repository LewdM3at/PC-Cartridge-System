//! The create-cartridge wizard: turn a drive into a cartridge.
//!
//! The full build, in order, each step optional except the last two:
//!
//!   1. format the drive to btrfs or exFAT     (opt in, destructive)
//!   2. copy the game onto it and register the
//!      cartridge as a Steam library           (opt in, slow)
//!   3. copy the cover art
//!   4. write cartridge.conf                   (always)
//!   5. write autorun.inf for the drive's name and icon in Explorer
//!
//! Game lists come from Playnite when it is installed — it aggregates Steam,
//! GOG, Epic, Xbox, emulators and anything added by hand — and from Steam's own
//! manifests otherwise, which is also the only option on Linux.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::drives::{self, TargetDrive};
use crate::{
    autorun, busy, folders, format, health, playnite, portable, settings, sgdb, steam, steamlib,
    trim, verify,
};

/// Past this, a DRAM-less drive behind a USB bridge has little room for its
/// garbage collector to work in. Matches health.rs, which says the same thing
/// about a cartridge that is merely plugged in.
const CROWDED_PERCENT: u8 = 85;

/// Largest cover we will copy onto a cartridge.
const MAX_COVER_BYTES: u64 = 8 * 1024 * 1024;

/// URI schemes Play is allowed to hand to the OS.
const ALLOWED_SCHEMES: [&str; 8] = [
    "steam://",
    "heroic://",
    "gog://",
    "epic://",
    "playnite://",
    "lutris://",
    "http://",
    "https://",
];

/// Where a game in the list came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Library {
    Steam,
    Playnite,
    /// Found by scanning a folder root. Its `id` is the game's own path, since
    /// there is no launcher holding an identifier for it.
    Folder,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    /// Steam app id, or Playnite GUID.
    pub id: String,
    pub name: String,
    pub library: Library,
    /// "Steam", "GOG", "Epic" … only Playnite reports this.
    pub source: String,
    pub size_on_disk: u64,
    pub has_cover: bool,
    /// What Play will start.
    pub executable: String,
    /// True when the game's files can be copied onto the cartridge.
    pub can_copy: bool,
}

/// The installed games, and why any library is missing from them.
///
/// The two are returned together because a partial list is still worth showing.
/// Steam almost always answers, so returning an error the moment Playnite fails
/// would throw away a usable list — but reporting only the games silently drops
/// the reason the other library is absent, which reads as "Playnite is not
/// supported" rather than "Playnite needs an exporter extension".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameList {
    pub games: Vec<GameInfo>,
    /// One line per library that could not be read. Empty on a clean run.
    pub problems: Vec<String>,
}

/// One game's metadata when creating a multi-game bundle cartridge.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleGameRequest {
    pub title: String,
    pub executable: String,
    /// This game's own hero, title logo and icon, each optional.
    ///
    /// The same four slots the cartridge itself has. A collection shown under a
    /// skin that wants heroes needs one per game, and a user who knows that is
    /// better served filling in heroes and leaving the rest empty than being
    /// made to supply four pictures for ten games.
    #[serde(default)]
    pub background_source: Option<String>,
    #[serde(default)]
    pub logo_source: Option<String>,
    #[serde(default)]
    pub icon_source: Option<String>,
    /// Steam app id, when the cover should come from Steam.
    #[serde(default)]
    pub app_id: Option<String>,
    /// Playnite GUID, when the cover should come from Playnite's cache.
    #[serde(default)]
    pub playnite_id: Option<String>,
    /// Absolute path to a user-chosen cover image for this game.
    #[serde(default)]
    pub cover_source: Option<String>,
    /// The game's folder, when the user pointed at one themselves.
    #[serde(default)]
    pub source_dir: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CartridgeRequest {
    /// The digest this cartridge is expected to come out with, if the person
    /// asking already knows it.
    ///
    /// Borrowed from Kazeta's cartridge creator, where a recipe declares the
    /// hash its finished cart must have: everybody who follows the recipe gets
    /// byte-identical carts, and finds out immediately when they do not. A
    /// `CartridgeRequest` is already a recipe in that sense; it just had no way
    /// to say what the answer should be.
    ///
    /// Checked after the copy is verified, never instead of it. A mismatch is a
    /// warning on the result rather than an error, because by then the
    /// cartridge exists and refusing to describe it would be unhelpful —
    /// `build-cart` turns that warning into a non-zero exit.
    #[serde(default)]
    pub expect_digest: Option<String>,
    /// Target drive root. Re-checked here against the allowed list.
    pub drive_path: String,
    pub title: String,
    pub executable: String,
    /// Steam app id, when the cover and files should come from Steam.
    #[serde(default)]
    pub app_id: Option<String>,
    /// Playnite GUID, when the cover should come from Playnite's cache.
    #[serde(default)]
    pub playnite_id: Option<String>,
    /// The game's folder, when the user pointed at one themselves.
    ///
    /// Playnite is not the only way to own a game. A folder chosen here is
    /// copied exactly as a Playnite install directory would be, which is what
    /// makes a cartridge of anything that is not in a launcher's library.
    #[serde(default)]
    pub source_dir: Option<String>,
    /// Absolute path to a user-chosen cover image instead.
    #[serde(default)]
    pub cover_source: Option<String>,
    /// Absolute path to a user-chosen Explorer icon for a single game.
    #[serde(default)]
    pub icon_source: Option<String>,
    /// Absolute path to a user-chosen launcher background for a single game.
    #[serde(default)]
    pub background_source: Option<String>,
    /// Absolute path to a user-chosen title logo for a single game.
    #[serde(default)]
    pub logo_source: Option<String>,
    /// Format the drive first. `format` re-checks the drive itself before it
    /// touches anything, so this asks rather than authorises.
    #[serde(default)]
    pub format_drive: bool,
    #[serde(default)]
    pub format_filesystem: Option<format::Filesystem>,
    #[serde(default)]
    pub format_label: Option<String>,
    /// Copy the game's files onto the cartridge.
    ///
    /// For a Steam game that also registers the drive as a Steam library. For
    /// anything else the folder is copied and `executable` is rewritten to point
    /// inside it.
    #[serde(default)]
    pub copy_game: bool,
    /// Move a Steam game instead of copying it: once the cartridge's copy has
    /// been read back and every file matched, delete the install this PC had and
    /// tell Steam the game now lives on the cartridge.
    ///
    /// Without it, a Steam game still installed on the PC is the copy Steam
    /// plays. Steam records an app as installed in exactly one library, and a
    /// second library holding the same app id is ignored — on 2026-09-12 a
    /// verified FTL cartridge launched `F:\Games\Steam\…\FTLGame.exe` every time.
    ///
    /// Needs `copy_game` and `verify_copy` both: nothing is deleted on the
    /// strength of a copy nobody checked. Steam games only — a folder picked by
    /// hand has no library to update and no way back if the pick was wrong.
    #[serde(default)]
    pub move_game: bool,
    /// Which file inside the copied folder Play should start, relative to the
    /// game's install directory. Only used for non-Steam games.
    #[serde(default)]
    pub copy_executable: Option<String>,
    /// When set, create a multi-game bundle cartridge. `title` becomes the
    /// collection title; the top-level `executable` field is ignored (each
    /// game in the vec carries its own).
    #[serde(default)]
    pub games: Option<Vec<BundleGameRequest>>,
    /// Absolute path to the collection's cover image for a bundle.
    #[serde(default)]
    pub collection_cover_source: Option<String>,
    /// Absolute path to the collection's Explorer icon.
    #[serde(default)]
    pub collection_icon_source: Option<String>,
    /// Absolute path to the collection's launcher background.
    #[serde(default)]
    pub collection_background_source: Option<String>,
    /// Absolute path to the collection's launcher title logo.
    #[serde(default)]
    pub collection_logo_source: Option<String>,
    /// Close Steam if it is in the way.
    ///
    /// Steam holds its library list in memory and writes it out when it exits,
    /// so rewriting a cartridge it knows about means closing it first. With this
    /// set, Steam is asked to close itself and waited for; without it, the build
    /// stops and says so before anything has been changed.
    #[serde(default)]
    pub close_steam: bool,
    /// Read the cartridge back after copying and check every file against the
    /// sum taken as it was written. One extra pass over the drive, and assumed
    /// rather than asked for: a request that says nothing about verifying gets
    /// it, because the failure it catches is silent and permanent.
    ///
    /// Note this is the *serde* default, so a hand-written JSON request that
    /// omits the field verifies. The derived `Default` still leaves it false;
    /// that one exists for tests, which set the field when they mean it.
    #[serde(default = "default_true")]
    pub verify_copy: bool,
    /// Ask the drive to release freed blocks once everything is written.
    ///
    /// Only worth it when the cartridge was not formatted first — mkfs already
    /// discards the whole volume — and it needs the same authentication prompt
    /// formatting does, so it is never implied.
    #[serde(default)]
    pub trim_after_write: bool,
    /// Write `autorun.ico` and `autorun.inf` at the root, so Explorer shows the
    /// cover instead of a grey disk.
    ///
    /// Defaults to true: a cartridge that does not name itself in Explorer is
    /// half a cartridge, and this is the behaviour every build had before the
    /// flag existed. It is a flag at all because the two files are visible
    /// clutter on a drive someone may also be using for something else.
    #[serde(default = "default_true")]
    pub write_icon: bool,
    /// Which game of a collection the drive icon is made from, as an index
    /// into `games`. A collection icon chosen outright still wins.
    #[serde(default)]
    pub primary_game: Option<usize>,
    /// A combo drive: the launcher also offers the memory card view of its
    /// saves. See [`crate::memcard`].
    #[serde(default)]
    pub memory_card: bool,
    /// What the cartridge is for, `platform=`: SNES, GBA... Empty or `PC` writes
    /// nothing, which reads as PC.
    #[serde(default)]
    pub platform: Option<String>,
    /// A ROM file to carry, for an emulated cartridge. Copied into the folder
    /// its platform's ROMs go in (`snes/`, `gba/`...) and made what Play opens.
    /// See [`crate::emulated`].
    #[serde(default)]
    pub rom_source: Option<String>,
}

/// The picture a drive icon is made from, relative to the cartridge root.
///
/// An icon chosen for the cartridge itself, else the primary game's icon, else
/// that game's cover, else `fallback` — which is what a collection used before
/// it had a primary game.
pub fn drive_icon_art<'a>(
    own: Option<&'a str>,
    games: &'a [GameArt],
    primary: Option<usize>,
    fallback: Option<&'a str>,
) -> Option<&'a str> {
    let game = primary.and_then(|index| games.get(index));
    own.or_else(|| game.and_then(|game| game.icon.as_deref().or(game.cover.as_deref())))
        .or(fallback)
}

/// serde needs a function; a bare `true` is not a valid default expression.
fn default_true() -> bool {
    true
}

impl CartridgeRequest {
    /// A single-game view of this request, so the copy helpers never have to
    /// know about bundles. The drive and the copy settings are shared; the
    /// game's own identity replaces the collection's.
    fn for_game(&self, game: &BundleGameRequest) -> CartridgeRequest {
        CartridgeRequest {
            title: game.title.clone(),
            executable: game.executable.clone(),
            app_id: game.app_id.clone(),
            playnite_id: game.playnite_id.clone(),
            cover_source: game.cover_source.clone(),
            // Per game, never inherited: the collection has no one folder, and
            // `..self.clone()` below would otherwise give every game in the
            // bundle the same source.
            source_dir: game.source_dir.clone(),
            games: None,
            // The format runs once, before any game is copied.
            format_drive: false,
            // Which file to start is picked per game by the ranking in
            // portable.rs: one dropdown per game would be more wizard than the
            // choice is worth.
            copy_executable: None,
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub step: &'static str,
    pub message: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CartridgeResult {
    pub conf_path: String,
    pub cover_written: bool,
    pub autorun_written: bool,
    pub icon: Option<String>,
    pub formatted: bool,
    pub formatted_filesystem: Option<format::Filesystem>,
    pub game_copied: bool,
    pub bytes_copied: u64,
    pub registered_with_steam: bool,
    /// Where the game was copied to, relative to the cartridge root.
    pub game_folder: Option<String>,
    /// How full the cartridge ended up, 0-100.
    pub used_percent: u8,
    /// What came of the TRIM, when one was asked for.
    pub trim: Option<String>,
    /// What came of the integrity check, when one was asked for.
    pub verified: Option<String>,
    /// Whether that check passed, when one was asked for.
    ///
    /// Separate from `verified` because that is prose: both a pass and a
    /// failure produce `Some(message)`, so the only way to tell them apart was
    /// to match on the wording. Anything acting on the outcome rather than
    /// showing it to a person needs a field it can branch on.
    pub verified_ok: Option<bool>,
    /// The cartridge's contents as one value, when there is a manifest to
    /// derive it from. See [`verify::cartridge_digest`].
    pub digest: Option<String>,
    /// Set when `expect_digest` was given and did not match.
    pub digest_matched: Option<bool>,
    /// True when Steam was closed to get at its library list.
    pub steam_closed: bool,
    /// True when a stale entry for this drive was taken out of that list.
    pub steam_entry_removed: bool,
    /// The PC installs deleted because the game was moved rather than copied,
    /// one line each: what, from where, and how much space came back.
    pub removed_from_pc: Vec<String>,
    pub warnings: Vec<String>,
}

/// Every game the wizard can offer, Playnite first.
///
/// `playnite_root_override` lets the wizard pass a user-supplied path when
/// auto-discovery did not find Playnite. Corresponds to the `PLAYNITE_ROOT`
/// environment variable, but can be set per-invocation without touching the
/// environment.
pub fn list_games(playnite_root_override: Option<&str>) -> Result<GameList, String> {
    let mut out = Vec::new();
    let mut problems = Vec::new();

    match playnite_games(playnite_root_override) {
        Ok(mut games) => out.append(&mut games),
        Err(e) => problems.push(e),
    }

    // Steam is still listed even with Playnite present: Playnite only knows
    // about games it has imported, and its export can be stale.
    match steam_games() {
        Ok(games) => {
            // Playnite's entry wins when both know a game, because it carries
            // the source label and launches through Playnite.
            let known: Vec<String> = out.iter().map(|g| g.name.to_lowercase()).collect();
            out.extend(
                games
                    .into_iter()
                    .filter(|g| !known.contains(&g.name.to_lowercase())),
            );
        }
        Err(e) => problems.push(e),
    }

    // Scanned folders go last, and lose every name collision. A game a launcher
    // knows about is better described by the launcher: it has an id, cover art
    // and a launch command, where a folder has only its own name.
    let mut known: Vec<String> = out.iter().map(|g| g.name.to_lowercase()).collect();
    for game in folder_games() {
        // Two roots can hold the same game — a copy kept on a second drive, or
        // one already written to a cartridge. Same name, different path, so the
        // path-level deduplication in `folders` cannot see it.
        let name = game.name.to_lowercase();
        if !known.contains(&name) {
            known.push(name);
            out.push(game);
        }
    }

    if out.is_empty() {
        return Err(if problems.is_empty() {
            "No installed games found.".to_string()
        } else {
            problems.join(" ")
        });
    }

    out.sort_by_key(|a| a.name.to_lowercase());
    Ok(GameList {
        games: out,
        problems,
    })
}

fn playnite_games(playnite_root_override: Option<&str>) -> Result<Vec<GameInfo>, String> {
    let root = playnite_root_override
        .map(PathBuf::from)
        .or_else(playnite::playnite_root)
        .ok_or_else(|| "Playnite not found.".to_string())?;
    let (_, games) = playnite::import_newest_in(&root).map_err(|e| match e {
        playnite::ImportError::NotFound => format!(
            "Playnite is installed at {} but has no JSON library export. \
             Install a JSON library exporter extension and run it.",
            root.display()
        ),
        other => other.to_string(),
    })?;

    Ok(games
        .into_iter()
        .map(|g| GameInfo {
            executable: g.launch_uri(),
            has_cover: g
                .cover
                .as_deref()
                .and_then(|c| playnite::resolve_cover(&root, c))
                .is_some(),
            // Anything Playnite knows the install directory for can be copied
            // wholesale; Play then points at a file on the cartridge.
            can_copy: g.install_dir.is_some(),
            // Walk the install directory to report a real size. This is done
            // eagerly because the list is already being built; individual games
            // are typically 1-100 GB so the cost is paid once per wizard open.
            // If the directory has gone missing (e.g., game was uninstalled but
            // the export is stale) the size stays 0 rather than failing the
            // whole list.
            size_on_disk: g
                .install_dir
                .as_deref()
                .map(portable::tree_size_of)
                .unwrap_or(0),
            id: g.id,
            name: g.name,
            library: Library::Playnite,
            source: g.source,
        })
        .collect())
}

/// The roots to scan: whatever the user chose, else what this machine looks
/// like it has.
pub fn folder_roots() -> Vec<PathBuf> {
    let configured = settings::load().game_folder_roots;
    if configured.is_empty() {
        return folders::default_roots();
    }
    configured.into_iter().map(PathBuf::from).collect()
}

/// What the artwork picker files a folder game's cover under.
///
/// The picker keys on the game's displayed name, and a folder game's name is
/// its folder name, so that is what has to be looked up here — keying this on
/// the path instead would never find anything the picker had saved.
fn folder_artwork_key(id: &str) -> String {
    // Deliberately not `Path::file_name`. Its idea of a separator is the host's,
    // so a Windows path handed to a Linux build comes back whole — `\` is an
    // ordinary filename character there — and the key becomes the entire path
    // instead of the folder. Both separators are split on here, because the id
    // travels between the two platforms and the path does not.
    id.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(id)
        .to_string()
}

/// A folder name, as something worth searching a store for.
///
/// Folders get named by whoever packaged the game: `DeadIsland2`,
/// `LEGOStarWarsTSS`, `God of War - Ragnarok`. Putting the word boundaries back
/// is the difference between a hit and nothing at all.
pub fn search_query_for(name: &str) -> String {
    search_query(name)
}

fn search_query(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 8);

    for (i, &c) in chars.iter().enumerate() {
        if c == '_' || c == '-' || c == '.' {
            out.push(' ');
            continue;
        }
        if i > 0 {
            let previous = chars[i - 1];
            let next = chars.get(i + 1).copied();
            // "DeadIsland" and "Island2": a new word starts wherever the kind
            // of character changes going up.
            let starts_word = (previous.is_lowercase() || previous.is_ascii_digit())
                && c.is_uppercase()
                || previous.is_alphabetic() && c.is_ascii_digit()
                // "LEGOStar": the end of an acronym is the capital *before* the
                // first lowercase one, not the first capital.
                || previous.is_uppercase()
                    && c.is_uppercase()
                    && next.is_some_and(|n| n.is_lowercase());
            if starts_word && !previous.is_whitespace() {
                out.push(' ');
            }
        }
        out.push(c);
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Ask SteamGridDB for a portrait cover for `title`.
///
/// The one way in from outside this module. Returns nothing when the lookup is
/// switched off, unkeyed, or the game is unknown to them.
pub fn fetch_cover_for(title: &str) -> Option<PathBuf> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    autofetch_cover(title)
}

/// Is this picture the shape of a cover?
///
/// A cover is 3:4 and the launcher's whole layout is built on it. Steam's cache
/// does not always hold one: `find_cover` falls back to `header.jpg`, which is
/// 460×215, and SteamGridDB's grid type serves that shape too. Anything wider
/// than it is tall is not a cover, whatever it is called.
///
/// Only the header is read — `image_dimensions` stops before the pixels — so
/// this costs a file open per game, not a decode.
fn is_portrait(path: &Path) -> bool {
    match image::image_dimensions(path) {
        Ok((width, height)) => height > width,
        // Unreadable or a format the decoder does not know: assume it is fine
        // rather than throw away art that may be perfectly good.
        Err(_) => true,
    }
}

/// The art to use for a game, preferring a real cover over a wide header.
///
/// `found` is whatever the launcher's own caches produced. When that turns out
/// to be landscape, SteamGridDB is asked for a portrait one under `title` —
/// which is also where a replacement is remembered, so it is fetched once.
/// Without a key, or without a match, the wide picture is kept: the launcher
/// letterboxes it rather than cropping, and some art beats none.
fn prefer_portrait(found: Option<PathBuf>, title: &str) -> Option<PathBuf> {
    let found = found?;
    if is_portrait(&found) || title.trim().is_empty() {
        return Some(found);
    }
    autofetch_cover(title.trim())
        .filter(|replacement| is_portrait(replacement))
        .or(Some(found))
}

/// Fetch a cover from SteamGridDB for a game no launcher has art for.
///
/// Reached only when the local lookup came back empty, and only for a game the
/// user has actually picked — the library list never calls this, so opening the
/// wizard with fifty scanned games does not fire fifty requests.
///
/// Does nothing at all unless the lookup is switched on and keyed: SteamGridDB
/// refuses unauthenticated requests, so on a default install this returns
/// before touching the network.
///
/// The result is filed under the same key and name the artwork picker uses, so
/// each game is fetched once and a cover chosen by hand later replaces it.
fn autofetch_cover(key: &str) -> Option<PathBuf> {
    settings::load().steamgriddb_key()?;

    // The folder name is the only title there is, and it is often not the
    // store's. `search_query` puts the spaces back; the raw name is tried after
    // it in case the folder was named properly and the split made it worse. No
    // match is not an error: a game SteamGridDB has never heard of simply stays
    // without a cover.
    let query = search_query(key);
    let game = sgdb::search_games(&query)
        .ok()
        .and_then(|found| found.into_iter().next())
        .or_else(|| {
            (query != key)
                .then(|| sgdb::search_games(key).ok())
                .flatten()
                .and_then(|found| found.into_iter().next())
        })?;
    let art = sgdb::get_artwork(game.id, sgdb::ArtworkType::Cover)
        .ok()?
        .into_iter()
        .next()?;
    // "grid" is what the picker calls a cover when it names a cache file, and
    // the two have to agree or each would re-download what the other has.
    let path = sgdb::download_artwork(&art.url, &format!("{key}-grid")).ok()?;
    sgdb::remember_last_used(key, &path).ok()?;
    Some(path)
}

/// Games found by scanning folders, as list entries.
///
/// Unlike Steam and Playnite this never fails: a root that has gone away, or
/// that cannot be read, contributes nothing and is not worth a problem line —
/// the defaults deliberately include roots that may not exist.
fn folder_games() -> Vec<GameInfo> {
    folders::scan(&folder_roots())
        .into_iter()
        .map(|game| {
            let id = game.path.to_string_lossy().to_string();
            GameInfo {
                // Nothing on disk says which file to run, and working it out
                // costs a walk per game. `portable` already picks at copy time,
                // so the answer is left until the game is actually chosen.
                executable: String::new(),
                // No launcher cache to draw on, so the only cover a folder game
                // can have is one already fetched from SteamGridDB for it.
                has_cover: sgdb::last_used_artwork(&folder_artwork_key(&id)).is_some(),
                can_copy: true,
                size_on_disk: portable::tree_size_of(&game.path),
                name: game.name,
                library: Library::Folder,
                source: game.source,
                id,
            }
        })
        .collect()
}

fn steam_games() -> Result<Vec<GameInfo>, String> {
    let root = steam::steam_root().ok_or_else(|| {
        "Could not find a Steam installation. Set STEAM_ROOT if it is somewhere unusual."
            .to_string()
    })?;

    let games = steam::installed_games(&root);
    if games.is_empty() {
        return Err(format!(
            "Found Steam at {} but no fully installed games.",
            root.display()
        ));
    }

    Ok(games
        .into_iter()
        .map(|g| GameInfo {
            executable: format!("steam://rungameid/{}", g.app_id),
            has_cover: g.cover_path.is_some(),
            can_copy: true,
            id: g.app_id,
            name: g.name,
            library: Library::Steam,
            source: "Steam".to_string(),
            size_on_disk: g.size_on_disk,
        })
        .collect())
}

/// Cover art for one game, as a data URI. Loaded one at a time: base64ing a
/// whole library at once would be tens of megabytes of IPC.
pub fn game_cover(library: Library, id: &str) -> String {
    let path = match library {
        Library::Steam => {
            if !is_numeric(id) {
                return String::new();
            }
            steam::steam_root()
                .and_then(|root| steam::find_cover(&root, id))
                .or_else(|| sgdb::last_used_artwork(&format!("steam:{id}")))
        }
        Library::Playnite => playnite::playnite_root()
            .and_then(|root| {
                let exports = playnite::find_exports(&root);
                exports
                    .iter()
                    .filter_map(|p| playnite::import_from(p).ok())
                    .flatten()
                    .find(|g| g.id == id)
                    .and_then(|g| g.cover)
                    .and_then(|c| playnite::resolve_cover(&root, &c))
            })
            .or_else(|| sgdb::last_used_artwork(&format!("playnite:{id}"))),
        // A folder game has no launcher cache behind it, so SteamGridDB is the
        // only place a cover can come from. What was picked or fetched before
        // wins; failing that one is fetched now, if the user has switched the
        // lookup on.
        Library::Folder => {
            let key = folder_artwork_key(id);
            sgdb::last_used_artwork(&key).or_else(|| autofetch_cover(&key))
        }
    };

    path.and_then(|p| read_as_data_uri(&p)).unwrap_or_default()
}

pub fn target_drives() -> Vec<TargetDrive> {
    drives::list_drives()
}

/// Whether this drive is currently listed as a Steam library folder.
pub fn steam_registration(drive_path: &str) -> bool {
    steam::steam_root()
        .map(|root| steamlib::is_registered(&root, Path::new(drive_path)))
        .unwrap_or(false)
}

/// Whether this cartridge carries Steam games at all.
///
/// A drive with no `steamapps/common` on it has nothing for Steam to find, so
/// offering to register it would be offering to add an empty library.
pub fn holds_steam_games(drive_path: &str) -> bool {
    let drive = Path::new(drive_path);
    steamlib::library_root(drive).join("steamapps/common").is_dir()
        // A cartridge written before the library moved into a folder of its
        // own still counts: it is exactly the one that needs fixing.
        || drive.join("steamapps/common").is_dir()
}

/// Put a cartridge back into Steam's library list.
///
/// Registration normally happens once, while the game is being copied. It needs
/// doing again more often than that: Steam drops a library whose drive was
/// missing when it started, which for a cartridge — a thing designed to spend
/// most of its life unplugged — is most of the time. Without this the only way
/// back was to copy the whole game again.
pub fn register_with_steam(drive_path: &str) -> Result<bool, String> {
    let root = steam::steam_root().ok_or_else(|| "Could not find Steam.".to_string())?;

    // Steam rewrites its library list from memory when it exits, so an entry
    // added now would be silently undone. Same reason the copy refuses.
    if steamlib::steam_is_running() {
        return Err(steamlib::LibraryError::SteamRunning.to_string());
    }

    let drive = Path::new(drive_path);

    // A cartridge written before the library had a folder of its own has its
    // steamapps at the root, which is the shape Steam will not take. Moving it
    // is a rename inside the volume, so the size of the game does not matter.
    steamlib::migrate_root_library(drive);

    // The drive itself may still be listed from that older layout, pointing at
    // a library Steam was never going to read.
    let _ = steamlib::unregister_library(&root, drive);

    let added = steamlib::register_library(&root, &steamlib::library_root(drive))
        .map(|written| written.is_some())
        .map_err(|e| e.to_string())?;

    // Registering is only half of it. Steam records an app as installed in one
    // library, so while the library it was copied from still claims this game,
    // the cartridge is a folder Steam never looks in — and deleting the local
    // files makes that worse rather than better, because the manifest stays and
    // Steam reads it as "installed, files missing" and offers to download.
    let retired = steamlib::retire_manifests(&steamlib::dead_manifests_elsewhere(&root, drive));

    Ok(added || !retired.is_empty())
}

/// What registering this cartridge would put right, in words.
///
/// Read before anything is changed: moving another library's manifest aside is
/// a change to somebody's game library, and it should be visible before it
/// happens rather than explained afterwards.
pub fn steam_registration_plan(drive_path: &str) -> Vec<String> {
    let Some(root) = steam::steam_root() else {
        return Vec::new();
    };
    let drive = Path::new(drive_path);

    let library = steamlib::library_root(drive);
    let mut out = Vec::new();

    if drive.join("steamapps").is_dir() && !library.exists() {
        out.push(format!(
            "Move {}\\steamapps into {} — Steam will not read a library at a drive root.",
            drive_path.trim_end_matches(['\\', '/']),
            library.display()
        ));
    }
    if steamlib::is_registered(&root, drive) {
        out.push(format!(
            "Drop {drive_path} from Steam's library list — a drive root is not a library."
        ));
    }
    if !steamlib::is_registered(&root, &library) {
        out.push(format!(
            "Add {} to Steam's library list.",
            library.display()
        ));
    }
    for manifest in steamlib::dead_manifests_elsewhere(&root, drive) {
        out.push(format!(
            "Move {} aside — it says the game is installed there, and the files are gone.",
            manifest.display()
        ));
    }
    out
}

/// Take a cartridge back out of Steam's library list.
///
/// For a cartridge that has been reformatted or repurposed. Entries are never
/// removed automatically — a cartridge is meant to spend most of its life
/// unplugged, so a missing folder is normal rather than stale.
pub fn unregister_from_steam(drive_path: &str) -> Result<bool, String> {
    let root = steam::steam_root().ok_or_else(|| "Could not find Steam.".to_string())?;
    let drive = Path::new(drive_path);

    // Both shapes: the library folder, and the drive root that older cartridges
    // were registered as before Steam's refusal of drive roots was understood.
    let library = steamlib::unregister_library(&root, &steamlib::library_root(drive))
        .map_err(|e| e.to_string())?;
    let bare = steamlib::unregister_library(&root, drive).map_err(|e| e.to_string())?;
    Ok(library || bare)
}

/// Describe what formatting a drive would destroy.
pub fn format_plan(path: &str) -> Result<format::FormatPlan, String> {
    format::plan(path).map_err(|e| e.to_string())
}

/// Build the cartridge.
pub fn create_cartridge(
    request: &CartridgeRequest,
    progress: &mut dyn FnMut(Progress),
) -> Result<CartridgeResult, String> {
    let mut warnings = Vec::new();
    let mut result = CartridgeResult {
        conf_path: String::new(),
        cover_written: false,
        autorun_written: false,
        icon: None,
        formatted: false,
        formatted_filesystem: None,
        game_copied: false,
        bytes_copied: 0,
        registered_with_steam: false,
        game_folder: None,
        used_percent: 0,
        trim: None,
        verified: None,
        verified_ok: None,
        digest: None,
        digest_matched: None,
        steam_closed: false,
        steam_entry_removed: false,
        removed_from_pc: Vec::new(),
        warnings: Vec::new(),
    };

    // Never trust the window's idea of where to write. The allowed set is
    // re-derived and an exact match required. Mutable: formatting can move
    // this (see the format step below).
    let mut root = resolve_target(&request.drive_path)?;

    // Every file copied across, with the sum taken as it was written. Only
    // filled in when the build was asked to check its own work.
    let mut written: Vec<verify::FileDigest> = Vec::new();

    let title = sanitize_conf_value(&request.title);
    if title.is_empty() {
        return Err("Give the cartridge a title.".into());
    }

    // Refused before anything is written, not discovered after the copy.
    if request.move_game && !(request.copy_game && request.verify_copy) {
        return Err(
            "Moving a game deletes this PC's copy of it, so it needs both copying and \
             verifying switched on: nothing is deleted on the strength of a copy nobody checked."
                .into(),
        );
    }

    // Steam installs to delete once the cartridge's copy has proved itself.
    let mut pc_installs: Vec<steamlib::InstalledGame> = Vec::new();

    // ---- 0. Steam's library list ----------------------------------------
    //
    // Rewriting a cartridge invalidates whatever Steam believed about it: the
    // game that entry describes is about to be formatted over, replaced, or
    // simply not be a Steam game any more. Left alone, Steam goes on listing a
    // game that is no longer there and offering to play it.
    //
    // This runs before anything is written, so a Steam that will not close
    // costs nothing — the alternative is discovering it after the drive has
    // already been formatted.
    let prepared = prepare_steam(request, &root, progress)?;
    result.steam_closed = prepared.closed;
    result.steam_entry_removed = prepared.entry_removed;
    warnings.extend(prepared.warnings);

    // ---- 1. Format ------------------------------------------------------
    if request.format_drive {
        let filesystem = request.format_filesystem.unwrap_or_default();
        let label = request
            .format_label
            .clone()
            .unwrap_or_else(|| default_label_for(filesystem, &title));

        progress(Progress {
            step: "format",
            message: format!(
                "Formatting {} to {}…",
                request.drive_path,
                filesystem.display_name()
            ),
            done_bytes: 0,
            total_bytes: 0,
        });

        // A fresh filesystem gets a fresh label, and on Linux the desktop
        // automounts by label — so the drive very often does not come back at
        // the path it was just at. Remember which device is actually behind
        // `root` before formatting erases that mapping, so it can be found
        // again afterward under whatever new name it gets, if format_drive's
        // own attempt below did not already resolve it.
        let device = current_device(&root);

        let remounted = format::format_drive(&request.drive_path, filesystem, &label)
            .map_err(|e| e.to_string())?;
        result.formatted = true;
        result.formatted_filesystem = Some(filesystem);

        // format_drive already tries to mount the fresh filesystem itself;
        // this only falls back to polling when that did not land in time.
        root = match remounted {
            Some(path) => PathBuf::from(path),
            None => wait_for_remount(root, device.as_deref())?,
        };
    }

    // ---- 2. Bundle mode: build every game, then bail early ---------------
    if let Some(bundle_games) = request.games.as_deref().filter(|g| !g.is_empty()) {
        // Nothing is written until every launch target that already exists
        // checks out. A copy creates its own target, so those are checked once
        // the files are there.
        if !request.copy_game {
            for game in bundle_games {
                validate_executable(&sanitize_conf_value(&game.executable), &root)?;
            }
        }

        // ---- Copy the games ----------------------------------------------
        //
        // Each game is copied as if it were the only one on the cartridge: the
        // copy helpers take a single-game view of the request and never learn
        // that this is a bundle.
        let mut entries: Vec<GameArt> = Vec::new();

        for game in bundle_games {
            let game_title = sanitize_conf_value(&game.title);
            if game_title.is_empty() {
                return Err("Every game in a collection needs a title.".into());
            }
            let mut executable = sanitize_conf_value(&game.executable);

            if request.copy_game {
                let job = request.for_game(game);
                match copy_game(&job, &root, progress) {
                    Ok(Some(copied)) => {
                        result.game_copied = true;
                        result.bytes_copied += copied.bytes;
                        result.registered_with_steam |= copied.registered_with_steam;
                        if result.game_folder.is_none() {
                            result.game_folder = copied.folder.clone();
                        }
                        written.extend(copied.digests);
                        pc_installs.extend(copied.pc_install);
                        // A generic copy moves the launch target onto the
                        // cartridge; a Steam copy keeps its steam:// URI.
                        if let Some(on_cartridge) = copied.executable {
                            executable = on_cartridge;
                        }
                    }
                    Ok(None) => {}
                    // The cartridge is still worth finishing without the files.
                    Err(e) => warnings.push(format!("{game_title} was not copied: {e}")),
                }
                validate_executable(&executable, &root)?;
            }

            entries.push(GameArt {
                title: game_title,
                executable,
                ..Default::default()
            });
        }

        // ---- Per-game cover art -------------------------------------------
        progress(Progress {
            step: "cover",
            message: "Copying cover art…".to_string(),
            done_bytes: 0,
            total_bytes: 0,
        });

        for (index, (game, entry)) in bundle_games.iter().zip(entries.iter_mut()).enumerate() {
            // The cover is the one that goes looking when nothing was chosen —
            // Steam and Playnite both keep one, and a collection with a blank
            // row is worse than a guessed poster. The other three are only ever
            // what somebody picked: there is nowhere to guess a hero from, and
            // an empty one is a cartridge saying it has no hero.
            match cover_source(
                game.cover_source.as_deref(),
                game.app_id.as_deref(),
                game.playnite_id.as_deref(),
                game.source_dir.as_deref(),
                &entry.title,
            ) {
                Ok(Some(source)) => match copy_cover(&source, &root, &format!("cover_{index}")) {
                    Ok(name) => entry.cover = Some(name),
                    Err(e) => {
                        warnings.push(format!("Cover art for {} was not copied: {e}", entry.title))
                    }
                },
                Ok(None) => {}
                Err(e) => warnings.push(format!("No cover for {}: {e}", entry.title)),
            }

            for (chosen, stem, slot) in [
                (game.background_source.as_deref(), "background", 0),
                (game.logo_source.as_deref(), "logo", 1),
                (game.icon_source.as_deref(), "icon", 2),
            ] {
                let Some(source) = chosen.map(Path::new).filter(|path| path.is_file()) else {
                    continue;
                };
                match copy_cover(source, &root, &format!("{stem}_{index}")) {
                    Ok(name) => match slot {
                        0 => entry.background = Some(name),
                        1 => entry.logo = Some(name),
                        _ => entry.icon = Some(name),
                    },
                    Err(e) => warnings.push(format!(
                        "The {stem} for {} was not copied: {e}",
                        entry.title
                    )),
                }
            }
        }

        // ---- The collection's own art -------------------------------------
        //
        // Whatever the wizard chose; failing that the first game's, so a
        // collection is never blank.
        let collection_art = match cover_source(
            request
                .collection_cover_source
                .as_deref()
                .or(request.cover_source.as_deref()),
            None,
            None,
            None,
            "",
        ) {
            Ok(found) => found,
            Err(e) => {
                warnings.push(format!("Collection cover art was not copied: {e}"));
                None
            }
        }
        .or_else(|| {
            let first = bundle_games.first()?;
            cover_source(
                first.cover_source.as_deref(),
                first.app_id.as_deref(),
                first.playnite_id.as_deref(),
                first.source_dir.as_deref(),
                &first.title,
            )
            .ok()
            .flatten()
        });

        // `copy_cover` returns the name the cartridge knows the art by, which
        // since the art moved into `.gamepak/` is a path and not a bare
        // filename. Keep that name: it is what cartridge.conf has to say, and
        // what the sweep below has to match. Joining it onto `root` here and
        // taking `file_name()` back off again at the conf - which is what this
        // did - wrote `cover=collection.png` for a file living at
        // `.gamepak/collection.png`, and the launcher then found no art at all.
        let collection_cover = match collection_art {
            Some(source) => match copy_cover(&source, &root, "collection") {
                Ok(name) => Some(name),
                Err(e) => {
                    warnings.push(format!("Collection cover art was not copied: {e}"));
                    None
                }
            },
            None => None,
        };
        let collection_icon = request
            .collection_icon_source
            .as_deref()
            .map(Path::new)
            .filter(|path| path.is_file())
            .and_then(|source| copy_cover(source, &root, "icon").ok());
        let collection_background = request
            .collection_background_source
            .as_deref()
            .map(Path::new)
            .filter(|path| path.is_file())
            .and_then(|source| copy_cover(source, &root, "background").ok());
        let collection_logo = request
            .collection_logo_source
            .as_deref()
            .map(Path::new)
            .filter(|path| path.is_file())
            .and_then(|source| copy_cover(source, &root, "logo").ok());
        result.cover_written = collection_cover.is_some();

        // ---- cartridge.conf -----------------------------------------------
        let tuples: Vec<ConfGame<'_>> = entries.iter().map(GameArt::as_conf).collect();
        let conf = render_bundle_conf(
            &title,
            collection_cover.as_deref(),
            collection_icon.as_deref(),
            collection_background.as_deref(),
            collection_logo.as_deref(),
            &tuples,
        );
        let looked_up: Vec<(&str, &str)> = entries
            .iter()
            .map(|game| (game.title.as_str(), game.executable.as_str()))
            .collect();
        let conf = with_estimates(conf, &looked_up, progress, &mut warnings);
        let card_games: Vec<(&str, &str, Option<&str>)> = entries
            .iter()
            .map(|game| {
                let icon = game.icon.as_deref().or(game.cover.as_deref());
                (game.title.as_str(), game.executable.as_str(), icon)
            })
            .collect();
        let conf = with_memory_card(
            conf,
            request.memory_card,
            &root,
            &card_games,
            progress,
            &mut warnings,
        );
        let conf = crate::cartridge::set_platform(&conf, request.platform.as_deref().unwrap_or(""));
        let conf_path = root.join("cartridge.conf");
        std::fs::write(&conf_path, conf)
            .map_err(|e| format!("Could not write {}: {e}", conf_path.display()))?;
        result.conf_path = conf_path.to_string_lossy().into_owned();
        mirror_history(&root, &mut warnings);
        crate::emulated::sync_files(&root, &mut warnings);

        // ---- autorun.inf ---------------------------------------------------
        if request.write_icon {
            progress(Progress {
                step: "autorun",
                message: "Naming the drive…".to_string(),
                done_bytes: 0,
                total_bytes: 0,
            });
            let autorun_source = drive_icon_art(
                collection_icon.as_deref(),
                &entries,
                request.primary_game,
                collection_cover.as_deref(),
            )
            .map(|art| root.join(art));
            match autorun::write_autorun(&root, &title, autorun_source.as_deref()) {
                Ok(icon) => {
                    result.autorun_written = true;
                    result.icon = icon;
                }
                Err(e) => warnings.push(format!("autorun.inf was not written: {e}")),
            }
        }

        sweep_stale_art(
            &root,
            entries
                .iter()
                .flat_map(|game| {
                    [&game.cover, &game.background, &game.logo, &game.icon].map(Option::as_deref)
                })
                .flatten()
                .chain(
                    [
                        &collection_cover,
                        &collection_icon,
                        &collection_background,
                        &collection_logo,
                    ]
                    .into_iter()
                    .filter_map(|name| name.as_deref()),
                )
                .chain(result.icon.as_deref())
                // With the drive icon turned off, autorun.inf is not rewritten
                // either, so whichever icon it already names is still the live
                // one. `cover.ico` is the name cartridges built before the
                // rename carry, and deleting it would strip their drive icon.
                .chain(
                    (!request.write_icon)
                        .then_some([ICON_NAME, "cover.ico"])
                        .into_iter()
                        .flatten(),
                ),
            &mut warnings,
        );

        if !result.cover_written {
            warnings.push(
                "No collection cover art on the cartridge. \
                 The launcher will show a placeholder."
                    .to_string(),
            );
        }

        finish(
            request,
            &root,
            written,
            &mut result,
            &mut warnings,
            progress,
        );
        remove_pc_installs(&pc_installs, &root, &mut result, &mut warnings, progress);
        result.warnings = warnings;
        return Ok(result);
    }

    // ---- 2b. Copy the game (single-game only) ----------------------------
    //
    // Before validating the executable, because a generic copy *creates* the
    // file that Play will point at: the target does not exist on the cartridge
    // until the folder has been copied across.
    let mut executable = sanitize_conf_value(&request.executable);

    // An emulated game's ROM. Not behind `copy_game`: the file is the game,
    // and there is nowhere else on the host it could be played from.
    if let Some(rom) = request
        .rom_source
        .as_deref()
        .map(str::trim)
        .filter(|rom| !rom.is_empty())
    {
        let platform = request.platform.as_deref().unwrap_or("");
        if !crate::emulated::is_emulated(platform) {
            return Err("A ROM needs a platform: say which system it is for.".into());
        }
        progress(Progress {
            step: "copy",
            message: format!("Copying {title}…"),
            done_bytes: 0,
            total_bytes: 0,
        });
        let mut digests = request.verify_copy.then(|| verify::Digests::new(&root));
        let (relative, bytes) =
            crate::emulated::copy_rom(Path::new(rom), &root, platform, digests.as_mut())?;
        written.extend(digests.map(|d| d.into_manifest().files).unwrap_or_default());
        result.game_copied = true;
        result.bytes_copied = bytes;
        result.game_folder = relative.rsplit_once('/').map(|(dir, _)| dir.to_string());
        executable = relative;
    }

    if request.copy_game
        && request
            .rom_source
            .as_deref()
            .is_none_or(|r| r.trim().is_empty())
    {
        match copy_game(request, &root, progress) {
            Ok(Some(copied)) => {
                result.game_copied = true;
                result.bytes_copied = copied.bytes;
                result.registered_with_steam = copied.registered_with_steam;
                result.game_folder = copied.folder.clone();
                written.extend(copied.digests);
                pc_installs.extend(copied.pc_install);
                // A generic copy replaces the launch target with a path on the
                // cartridge; a Steam copy keeps its steam:// URI.
                if let Some(on_cartridge) = copied.executable {
                    executable = on_cartridge;
                }
            }
            Ok(None) => {}
            Err(e) => {
                // The cartridge is still worth finishing without the files.
                warnings.push(format!("The game was not copied: {e}"));
            }
        }
    }

    // ---- 3. Check what Play will start -----------------------------------
    validate_executable(&executable, &root)?;

    // ---- 4. Cover art ---------------------------------------------------
    progress(Progress {
        step: "cover",
        message: "Copying cover art…".to_string(),
        done_bytes: 0,
        total_bytes: 0,
    });

    let cover_destination = match write_cover(&root, request) {
        Ok(name) => name,
        Err(e) => {
            warnings.push(format!("Cover art was not copied: {e}"));
            None
        }
    };
    result.cover_written = cover_destination.is_some();

    // The launcher's title logo, background and Explorer icon — each optional,
    // unrelated to the cover and to each other, and never guessed at: only
    // copied when the wizard (or SteamGridDB through it) actually chose one.
    let icon_destination = request
        .icon_source
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .and_then(|source| copy_cover(source, &root, "icon").ok());
    let background_destination = request
        .background_source
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .and_then(|source| copy_cover(source, &root, "background").ok());
    let logo_destination = request
        .logo_source
        .as_deref()
        .map(Path::new)
        .filter(|path| path.is_file())
        .and_then(|source| copy_cover(source, &root, "logo").ok());

    // ---- 5. cartridge.conf ----------------------------------------------
    let conf = render_cartridge_conf(
        &title,
        &executable,
        cover_destination.as_deref(),
        icon_destination.as_deref(),
        background_destination.as_deref(),
        logo_destination.as_deref(),
    );
    let conf = with_estimates(conf, &[(&title, &executable)], progress, &mut warnings);
    let conf = with_memory_card(
        conf,
        request.memory_card,
        &root,
        &[(
            &title,
            &executable,
            icon_destination.as_deref().or(cover_destination.as_deref()),
        )],
        progress,
        &mut warnings,
    );
    let conf = crate::cartridge::set_platform(&conf, request.platform.as_deref().unwrap_or(""));
    let conf_path = root.join("cartridge.conf");
    std::fs::write(&conf_path, conf)
        .map_err(|e| format!("Could not write {}: {e}", conf_path.display()))?;
    result.conf_path = conf_path.to_string_lossy().into_owned();
    mirror_history(&root, &mut warnings);
    crate::emulated::sync_files(&root, &mut warnings);

    // ---- 6. autorun.inf --------------------------------------------------
    if request.write_icon {
        progress(Progress {
            step: "autorun",
            message: "Naming the drive…".to_string(),
            done_bytes: 0,
            total_bytes: 0,
        });

        let autorun_source = icon_destination
            .as_deref()
            .or(cover_destination.as_deref())
            .map(|art| root.join(art));
        match autorun::write_autorun(&root, &title, autorun_source.as_deref()) {
            Ok(icon) => {
                result.autorun_written = true;
                result.icon = icon;
                if autorun_source.is_some() && result.icon.is_none() {
                    warnings.push(
                        "The chosen artwork could not be read as an image, so no drive icon \
                     was made and the cartridge keeps Explorer's default. A PNG, a JPEG or \
                     an .ico all convert."
                            .to_string(),
                    );
                }
            }
            Err(e) => warnings.push(format!("autorun.inf was not written: {e}")),
        }
    }

    sweep_stale_art(
        &root,
        [
            &cover_destination,
            &icon_destination,
            &background_destination,
            &logo_destination,
        ]
        .into_iter()
        .filter_map(|name| name.as_deref())
        .chain(result.icon.as_deref())
        // See the collection sweep: an icon this write did not replace is still
        // the one autorun.inf names, under whichever name it was written under.
        .chain(
            (!request.write_icon)
                .then_some([ICON_NAME, "cover.ico"])
                .into_iter()
                .flatten(),
        ),
        &mut warnings,
    );

    if !result.cover_written {
        warnings.push(
            "No cover art on the cartridge. The launcher will show a placeholder.".to_string(),
        );
    }

    finish(
        request,
        &root,
        written,
        &mut result,
        &mut warnings,
        progress,
    );
    remove_pc_installs(&pc_installs, &root, &mut result, &mut warnings, progress);
    result.warnings = warnings;
    Ok(result)
}

/// The last thing every build does, whatever shape the cartridge was.
///
/// Releases freed blocks if asked, then reports how full the drive ended up —
/// which matters more here than on an internal disk, because a DRAM-less drive
/// behind a USB bridge has no host memory to lean on and needs the room.
/// Delete art at the root that this write no longer refers to.
///
/// Every write lays art down under a fixed set of stems and rewrites
/// `cartridge.conf` from scratch, so anything left from an earlier write is
/// unreferenced the moment the new conf lands: `cover_3.jpg` from when the
/// collection had four games, or a `logo.png` shadowed by the `logo.jpg`
/// chosen since. Left alone it is dead weight, and worse, it makes the root
/// read as if the cartridge still carries art it has stopped using.
///
/// `keep` is every filename the new conf and autorun.inf point at.
fn sweep_stale_art<'a>(
    root: &Path,
    keep: impl Iterator<Item = &'a str>,
    warnings: &mut Vec<String>,
) {
    // `keep` names art the way the cartridge does - `.gamepak/cover_0.png` for
    // everything the wizard lays down, a bare `icon.ico` for the one file
    // Explorer needs at the root. Matching on the whole relative name rather
    // than the basename is what lets the same sweep clear the root copies a
    // pre-`.gamepak` cartridge still has: `cover_0.png` and
    // `.gamepak/cover_0.png` are different files, and only the second is kept.
    let keep: std::collections::HashSet<String> = keep
        .map(|name| name.replace('\\', "/").to_lowercase())
        .collect();

    for directory in [root.to_path_buf(), root.join(ASSET_DIR)] {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        let prefix = if directory == root {
            String::new()
        } else {
            format!("{ASSET_DIR}/")
        };

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if keep.contains(&format!("{prefix}{}", name.to_lowercase())) {
                continue;
            }
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !is_art_stem(&stem.to_lowercase()) {
                continue;
            }
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            if let Err(e) = std::fs::remove_file(&path) {
                warnings.push(format!("Stale artwork {name} was left behind: {e}"));
            }
        }
    }
}

/// Is this a name the writer itself lays art down under?
///
/// The sweep only ever touches these, so a file the owner put on the cartridge
/// by hand is never deleted by a rewrite.
fn is_art_stem(stem: &str) -> bool {
    matches!(
        stem,
        "collection" | "cover" | "icon" | "background" | "logo"
    ) || stem
        .strip_prefix("cover_")
        .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
}

/// The entries of the previous manifest that still describe this cartridge.
///
/// The manifest is a record of the whole cartridge, not of the last write.
/// Adding a game to a cartridge only copies that game, so saving `written`
/// alone would drop every game already there from the record; removing one
/// would leave its files listed forever. Both are fixed by carrying the old
/// entries forward, minus anything that has gone or has just been rewritten.
fn carried_manifest(root: &Path, written: &[verify::FileDigest]) -> Vec<verify::FileDigest> {
    let Some(previous) = verify::read_manifest(root) else {
        return Vec::new();
    };
    let rewritten: std::collections::HashSet<&str> =
        written.iter().map(|file| file.path.as_str()).collect();

    previous
        .files
        .into_iter()
        .filter(|file| !rewritten.contains(file.path.as_str()))
        .filter(|file| root.join(&file.path).is_file())
        .collect()
}

fn finish(
    request: &CartridgeRequest,
    root: &Path,
    written: Vec<verify::FileDigest>,
    result: &mut CartridgeResult,
    warnings: &mut Vec<String>,
    progress: &mut dyn FnMut(Progress),
) {
    // Whatever happens below, the record has to match the cartridge afterwards.
    let carried = carried_manifest(root, &written);

    if request.verify_copy && !written.is_empty() {
        // Only the files just written are read back: the rest were checked when
        // they were copied, and re-reading the whole cartridge on every write
        // would make adding one small game cost a full-drive verify.
        let manifest = verify::Manifest { files: written };
        let total = manifest.total_bytes();

        progress(Progress {
            step: "verify",
            message: "Reading the cartridge back…".to_string(),
            done_bytes: 0,
            total_bytes: total,
        });

        let problems = verify::verify(root, &manifest, &mut |done, total| {
            progress(Progress {
                step: "verify",
                message: "Reading the cartridge back…".to_string(),
                done_bytes: done,
                total_bytes: total,
            });
        });

        let files = manifest.files.len();
        if problems.is_empty() {
            result.verified = Some(format!(
                "Checked all {files} files against what was written; every one matches."
            ));
            result.verified_ok = Some(true);
            // Left on the cartridge so it can be checked again later, on a
            // machine that no longer has the original.
            let mut files = carried;
            files.extend(manifest.files);
            if let Err(e) = verify::write_manifest(root, &verify::Manifest { files }) {
                warnings.push(format!("The file list was not saved to the cartridge: {e}"));
            }
        } else {
            // Named individually up to a point: "3 files are wrong" is not
            // actionable, and the first few are usually the whole story.
            let named: Vec<String> = problems
                .iter()
                .take(5)
                .map(verify::Problem::describe)
                .collect();
            let more = problems.len().saturating_sub(named.len());
            let tail = if more > 0 {
                format!(" (and {more} more)")
            } else {
                String::new()
            };
            let message = format!(
                "The copy did not survive: {}{tail}. Copy it again, and if it keeps happening \
                 try another cable or port before blaming the drive.",
                named.join("; ")
            );
            result.verified = Some(message.clone());
            result.verified_ok = Some(false);
            warnings.push(message);
        }
    } else if let Some(previous) = verify::read_manifest(root) {
        // Nothing was copied, so there is nothing to check — but files may have
        // gone since, and a record naming games the cartridge no longer holds
        // is worse than a shorter one.
        if previous.files.len() != carried.len() {
            let manifest = verify::Manifest { files: carried };
            if let Err(e) = verify::write_manifest(root, &manifest) {
                warnings.push(format!("The file list was not brought up to date: {e}"));
            }
        }
    }

    // ---- the cartridge's name for itself ----------------------------------
    //
    // Read back from the drive rather than taken from the manifest that was
    // just assembled in memory, so this describes what a later `verify-cart`
    // on another machine will actually find. A cartridge written without the
    // verify option has no manifest and so has no digest; that is the honest
    // answer rather than a hash of nothing.
    if let Some(manifest) = verify::read_manifest(root) {
        let digest = verify::cartridge_digest(&manifest);
        let expected = request.expect_digest.as_deref().unwrap_or_default();
        result.digest_matched = verify::digest_matches(expected, &digest);
        if result.digest_matched == Some(false) {
            warnings.push(format!(
                "This cartridge came out as {} and the request expected {}. Same request, \
                 different bytes: check that the game files are the version the request was \
                 written against.",
                verify::short_digest(&digest),
                verify::short_digest(expected.trim()),
            ));
        }
        result.digest = Some(digest);
    }

    if request.trim_after_write {
        progress(Progress {
            step: "trim",
            message: "Releasing free space back to the drive…".to_string(),
            done_bytes: 0,
            total_bytes: 0,
        });
        let outcome = trim::trim(&root.to_string_lossy());
        result.trim = Some(outcome.message());
    }

    let health = health::inspect(&root.to_string_lossy());
    result.used_percent = health.used_percent;
    if health.used_percent >= CROWDED_PERCENT {
        warnings.push(format!(
            "The cartridge is {}% full. These drives have no DRAM of their own and cannot \
             borrow host memory over USB, so keeping 15% or so spare is what keeps random \
             reads quick.",
            health.used_percent
        ));
    }
}

/// Will this build put the drive into Steam's library list?
///
/// True only when files are actually being copied *and* at least one of them is
/// a Steam game — both shapes of request can carry one, a single game at the top
/// level and a collection per game.
fn will_register_with_steam(request: &CartridgeRequest) -> bool {
    if !request.copy_game {
        return false;
    }
    if request.app_id.as_deref().is_some_and(is_numeric) {
        return true;
    }
    request.games.as_deref().is_some_and(|games| {
        games
            .iter()
            .any(|game| game.app_id.as_deref().is_some_and(is_numeric))
    })
}

/// What had to happen to Steam before the cartridge could be written.
struct SteamPrepared {
    closed: bool,
    entry_removed: bool,
    warnings: Vec<String>,
}

/// Get Steam out of the way, and take this drive out of its library list.
///
/// Both halves are conditional: there is nothing to do unless Steam is
/// installed and either already knows about this drive or is about to.
fn prepare_steam(
    request: &CartridgeRequest,
    root: &Path,
    progress: &mut dyn FnMut(Progress),
) -> Result<SteamPrepared, String> {
    let mut prepared = SteamPrepared {
        closed: false,
        entry_removed: false,
        warnings: Vec::new(),
    };

    let Some(steam_root) = steam::steam_root() else {
        return Ok(prepared);
    };

    let registered = steamlib::is_registered(&steam_root, root);
    // A Steam copy will want to add this drive at the end, which is the same
    // file and so the same requirement.
    if !registered && !will_register_with_steam(request) {
        return Ok(prepared);
    }

    if steamlib::steam_is_running() {
        if !request.close_steam {
            return Err(
                "Steam is running, and it rewrites its library list from memory when it exits, \
                 so anything changed now would be undone. Close Steam, or tick \
                 “Close Steam if it is in the way”."
                    .to_string(),
            );
        }

        progress(Progress {
            step: "steam",
            message: "Asking Steam to close…".to_string(),
            done_bytes: 0,
            total_bytes: 0,
        });

        match steamlib::shutdown_steam(&steam_root) {
            steamlib::Shutdown::NotRunning => {}
            steamlib::Shutdown::Exited => prepared.closed = true,
            steamlib::Shutdown::StillRunning(why) => return Err(why),
        }
    }

    // Now that it is closed, drop the entry describing what used to be here.
    // A Steam copy adds a fresh one when it finishes; anything else leaves the
    // drive out of the list, which is correct, because it no longer holds the
    // game Steam thought it did.
    if registered {
        progress(Progress {
            step: "steam",
            message: "Taking the old cartridge out of Steam's library list…".to_string(),
            done_bytes: 0,
            total_bytes: 0,
        });

        match steamlib::unregister_library(&steam_root, root) {
            Ok(true) => prepared.entry_removed = true,
            Ok(false) => {}
            Err(e) => prepared
                .warnings
                .push(format!("Steam's library list was not updated: {e}")),
        }
    }

    Ok(prepared)
}

/// What a copy produced.
struct Copied {
    bytes: u64,
    /// Set when the launch target moved onto the cartridge.
    executable: Option<String>,
    /// Where the files landed, relative to the cartridge root.
    folder: Option<String>,
    registered_with_steam: bool,
    /// Every file that was written, and its sum, when the copy was asked to
    /// keep track. Empty otherwise.
    digests: Vec<crate::verify::FileDigest>,
    /// The Steam install this was copied from, when the request asked for a
    /// move. Deleted only after the whole cartridge has verified.
    pc_install: Option<steamlib::InstalledGame>,
}

/// Copy the game, by whichever route suits where it came from.
///
/// Steam games go through the library mechanism, because `steam://rungameid`
/// launches whatever copy Steam knows about and a loose folder would be ignored.
/// Everything else is a plain folder copy with Play pointed inside it, which is
/// the simpler and more honest arrangement — the cartridge really does carry the
/// game, with no launcher in the middle.
fn copy_game(
    request: &CartridgeRequest,
    root: &Path,
    progress: &mut dyn FnMut(Progress),
) -> Result<Option<Copied>, String> {
    if request.app_id.as_deref().is_some_and(is_numeric) {
        return copy_steam_game(request, root, progress);
    }
    copy_portable_game(request, root, progress)
}

/// Copy a self-contained game folder onto the cartridge.
fn copy_portable_game(
    request: &CartridgeRequest,
    root: &Path,
    progress: &mut dyn FnMut(Progress),
) -> Result<Option<Copied>, String> {
    let Some(source) = portable_source(request)? else {
        return Ok(None);
    };

    if !source.is_dir() {
        return Err(format!("{} is not there any more", source.display()));
    }

    // Copying the cartridge onto itself never terminates. Reachable as soon as
    // the source is a folder somebody picked, since the picker will happily
    // open on the drive being written to.
    if source == root || source.starts_with(root) {
        return Err(format!(
            "{} is on the cartridge already, so there is nothing to copy from",
            source.display()
        ));
    }

    let title = sanitize_conf_value(&request.title);
    let folder_name = portable::safe_folder_name(&title);
    // Everything the wizard copies lives under Games/, so the cartridge root
    // stays readable next to cartridge.conf and the cover.
    let relative_folder = format!("Games/{folder_name}");
    let destination = root.join("Games").join(&folder_name);

    let total = portable::tree_size_of(&source);
    let free = drives::list_drives()
        .into_iter()
        .find(|d| Path::new(&d.path) == root)
        .map(|d| d.free_bytes)
        .unwrap_or(0);
    if total > free {
        return Err(steamlib::LibraryError::NotEnoughSpace {
            needed: total,
            free,
        }
        .to_string());
    }

    // Decide what Play will run *before* copying, so a bad choice costs nothing.
    let chosen = choose_portable_executable(request, &source, &title)?;

    progress(Progress {
        step: "copy",
        message: format!("Copying {title}…"),
        done_bytes: 0,
        total_bytes: total,
    });

    let name = title.clone();
    let mut digests = request.verify_copy.then(|| verify::Digests::new(root));
    let bytes =
        steamlib::copy_tree_digesting(&source, &destination, digests.as_mut(), &mut |done| {
            progress(Progress {
                step: "copy",
                message: format!("Copying {name}…"),
                done_bytes: done,
                total_bytes: total,
            });
        })
        .map_err(|e| format!("{}: {e}", source.display()))?;

    Ok(Some(Copied {
        bytes,
        executable: Some(format!("{relative_folder}/{chosen}")),
        folder: Some(relative_folder),
        registered_with_steam: false,
        digests: digests.map(|d| d.into_manifest().files).unwrap_or_default(),
        // A folder picked by hand is never deleted; see `move_game`.
        pc_install: None,
    }))
}

/// Where a non-Steam game's files currently live.
/// Check a folder the user chose before anything is copied out of it.
///
/// The path arrives from the window, so it is re-checked here rather than
/// trusted: a command is reachable whatever the interface allowed. None of this
/// is about a hostile user — it is about the two mistakes that are easy to make
/// with a folder picker and expensive to make with a copy.
pub fn check_source_dir(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("No folder was chosen.".to_string());
    }
    let dir = PathBuf::from(trimmed);

    if !dir.is_dir() {
        return Err(format!("{} is not a folder.", dir.display()));
    }

    // A drive root has no parent. Copying one would take the recycle bin, the
    // system volume information and every other game on the disk with it.
    if dir.parent().is_none() {
        return Err(format!(
            "{} is a whole drive. Choose the folder that holds the game.",
            dir.display()
        ));
    }

    // Windows itself is never a game, and the mistake is one click away in a
    // folder picker that opens on C:.
    for var in ["SystemRoot", "windir"] {
        if let Some(system) = std::env::var_os(var) {
            let system = PathBuf::from(system);
            if dir == system || dir.starts_with(&system) {
                return Err(format!(
                    "{} is inside Windows itself, which is not a game.",
                    dir.display()
                ));
            }
        }
    }

    Ok(dir)
}

fn portable_source(request: &CartridgeRequest) -> Result<Option<PathBuf>, String> {
    // A folder the user chose wins: they said exactly which one, and it is the
    // only route for a game no launcher knows about.
    if let Some(chosen) = request.source_dir.as_deref().map(str::trim) {
        if !chosen.is_empty() {
            return check_source_dir(chosen).map(Some);
        }
    }

    let Some(playnite_id) = request.playnite_id.as_deref() else {
        return Ok(None);
    };
    let root = playnite::playnite_root()
        .ok_or_else(|| "could not find Playnite, so there is no install directory".to_string())?;

    let game = playnite::find_exports(&root)
        .iter()
        .filter_map(|p| playnite::import_from(p).ok())
        .flatten()
        .find(|g| g.id == playnite_id)
        .ok_or_else(|| "that game is no longer in the Playnite export".to_string())?;

    match game.install_dir {
        Some(dir) => Ok(Some(dir)),
        None => Err("Playnite does not record an install directory for it".to_string()),
    }
}

/// The path inside the game folder that Play should start.
///
/// The caller's choice is honoured if it is a real file that stays inside the
/// folder; otherwise the best-ranked candidate is used.
fn choose_portable_executable(
    request: &CartridgeRequest,
    source: &Path,
    title: &str,
) -> Result<String, String> {
    if let Some(chosen) = request.copy_executable.as_deref().map(str::trim) {
        if !chosen.is_empty() {
            // The window supplied this, so it is checked the same way a
            // cartridge-supplied path would be.
            let relative = chosen.replace('\\', "/");
            if relative
                .split('/')
                .any(|part| part == ".." || part.is_empty())
                || Path::new(&relative).is_absolute()
                || relative.contains(':')
            {
                return Err(format!("{chosen} is not a path inside the game folder"));
            }
            if !source.join(&relative).is_file() {
                return Err(format!("{chosen} is not in the game folder"));
            }
            return Ok(relative);
        }
    }

    let play_action = playnite_play_action(request);
    portable::find_executables(source, title, play_action.as_deref())
        .into_iter()
        .next()
        .map(|c| c.relative)
        .ok_or_else(|| {
            "no program found in the game folder, so there would be nothing for Play to start"
                .to_string()
        })
}

fn playnite_play_action(request: &CartridgeRequest) -> Option<String> {
    let playnite_id = request.playnite_id.as_deref()?;
    let root = playnite::playnite_root()?;
    playnite::find_exports(&root)
        .iter()
        .filter_map(|p| playnite::import_from(p).ok())
        .flatten()
        .find(|g| g.id == playnite_id)
        .and_then(|g| g.play_action)
}

/// Candidates for what Play should start, best guess first.
pub fn executable_choices_in(dir: &str, title: &str) -> Result<Vec<portable::Candidate>, String> {
    let dir = check_source_dir(dir)?;
    // No play action to go on: nothing recorded which file this folder starts,
    // which is the whole reason the ranking in portable.rs exists.
    Ok(portable::find_executables(&dir, title, None))
}

pub fn executable_choices(playnite_id: &str) -> Result<Vec<portable::Candidate>, String> {
    let root = playnite::playnite_root().ok_or_else(|| "Could not find Playnite.".to_string())?;
    let game = playnite::find_exports(&root)
        .iter()
        .filter_map(|p| playnite::import_from(p).ok())
        .flatten()
        .find(|g| g.id == playnite_id)
        .ok_or_else(|| "That game is no longer in the Playnite export.".to_string())?;

    let dir = game
        .install_dir
        .ok_or_else(|| "Playnite does not record an install directory for it.".to_string())?;

    Ok(portable::find_executables(
        &dir,
        &game.name,
        game.play_action.as_deref(),
    ))
}

/// Copy a Steam game onto the cartridge and register it as a Steam library.
///
/// Returns the bytes copied and whether Steam was told about the drive, or
/// `None` when this cartridge is not a copyable Steam game.
fn copy_steam_game(
    request: &CartridgeRequest,
    root: &Path,
    progress: &mut dyn FnMut(Progress),
) -> Result<Option<Copied>, String> {
    let Some(app_id) = request.app_id.as_deref().filter(|id| is_numeric(id)) else {
        return Ok(None);
    };

    let steam_root =
        steam::steam_root().ok_or_else(|| steamlib::LibraryError::SteamNotFound.to_string())?;

    // Steam rewrites libraryfolders.vdf from memory when it exits, so a
    // registration made now would be silently undone.
    if steamlib::steam_is_running() {
        return Err(steamlib::LibraryError::SteamRunning.to_string());
    }

    // Never the cartridge's own copy: see `locate_off`.
    let game = steamlib::locate_off(&steam_root, app_id, root).ok_or_else(|| {
        if steamlib::locate(&steam_root, app_id).is_some() {
            format!(
                "the only copy of {} Steam knows about is the one on this cartridge, so there \
                 is nothing to copy it from",
                request.title
            )
        } else {
            steamlib::LibraryError::GameNotFound(request.title.clone()).to_string()
        }
    })?;

    // Measured, not taken from the manifest. `SizeOnDisk` is what Steam last
    // recorded, and it drifts from what is really there: FTL's says 286,586,003
    // bytes and the folder holds 287,269,333, so progress ran to 100.2%. The walk
    // is metadata only, and nothing next to a copy of the same tree.
    let measured = steamlib::tree_size(&game.install_path);
    let total = if measured > 0 {
        measured
    } else {
        game.size_on_disk
    };

    // Check space before starting a copy that could run for many minutes.
    let free = drives::list_drives()
        .into_iter()
        .find(|d| Path::new(&d.path) == root)
        .map(|d| d.free_bytes)
        .unwrap_or(0);
    if total > free {
        return Err(steamlib::LibraryError::NotEnoughSpace {
            needed: total,
            free,
        }
        .to_string());
    }

    let install_dir_name = game
        .install_path
        .file_name()
        .ok_or_else(|| "the game's install directory has no name".to_string())?;
    let destination = steamlib::library_root(root)
        .join("steamapps/common")
        .join(install_dir_name);

    progress(Progress {
        step: "copy",
        message: format!("Copying {}…", game.name),
        done_bytes: 0,
        total_bytes: total,
    });

    let name = game.name.clone();
    let mut digests = request.verify_copy.then(|| verify::Digests::new(root));
    let copied = steamlib::copy_tree_digesting(
        &game.install_path,
        &destination,
        digests.as_mut(),
        &mut |done| {
            progress(Progress {
                step: "copy",
                message: format!("Copying {name}…"),
                done_bytes: done,
                total_bytes: total,
            });
        },
    )
    .map_err(|e| format!("{}: {e}", game.install_path.display()))?;

    // The manifest is how Steam recognises the game in this library.
    let steamapps = steamlib::library_root(root).join("steamapps");
    let manifest_destination = steamapps.join(
        game.manifest_path
            .file_name()
            .ok_or_else(|| "the manifest has no filename".to_string())?,
    );
    std::fs::create_dir_all(&steamapps)
        .and_then(|_| copy_small_file(&game.manifest_path, &manifest_destination))
        .map_err(|e| format!("could not copy the app manifest: {e}"))?;

    progress(Progress {
        step: "register",
        message: "Registering the cartridge with Steam…".to_string(),
        done_bytes: copied,
        total_bytes: total,
    });

    // The library folder, not the drive: Steam refuses a drive root outright.
    let registered = steamlib::register_library(&steam_root, &steamlib::library_root(root))
        .map_err(|e| e.to_string())?
        .is_some();

    Ok(Some(Copied {
        bytes: copied,
        // Steam launches by app id wherever the library lives, so the launch
        // target is unchanged.
        executable: None,
        folder: Some(format!("{}/steamapps/common", steamlib::LIBRARY_DIR)),
        registered_with_steam: registered,
        digests: digests.map(|d| d.into_manifest().files).unwrap_or_default(),
        pc_install: request.move_game.then_some(game),
    }))
}

/// What a move would delete from this PC, in words, before anything is written.
///
/// One line per Steam game in the request that Steam has installed somewhere:
/// the folder, and its size. A game Steam cannot find is said to be so rather
/// than left out, because a plan that silently shrinks reads as a plan that
/// covers everything.
pub fn move_plan(request: &CartridgeRequest) -> Vec<String> {
    let Some(steam_root) = steam::steam_root() else {
        return vec!["Steam is not installed here, so nothing would be moved.".into()];
    };
    let games: Vec<(String, String)> = match request.games.as_deref().filter(|g| !g.is_empty()) {
        Some(games) => games
            .iter()
            .filter_map(|game| Some((game.app_id.clone()?, game.title.clone())))
            .collect(),
        None => request
            .app_id
            .clone()
            .map(|id| vec![(id, request.title.clone())])
            .unwrap_or_default(),
    };
    games
        .into_iter()
        .filter(|(id, _)| is_numeric(id))
        .map(|(id, title)| {
            match steamlib::locate_off(&steam_root, &id, Path::new(&request.drive_path)) {
                Some(game) => format!(
                    "delete {} ({}) once the cartridge verifies",
                    game.install_path.display(),
                    format::human_bytes(steamlib::tree_size(&game.install_path))
                ),
                None => format!("{title}: Steam has no install of app {id} to delete"),
            }
        })
        .collect()
}

/// Delete this PC's copy of every Steam game that was moved rather than copied.
///
/// Last, after `finish` has read the cartridge back, and only when every file
/// matched. A copy that did not verify leaves the original exactly where it was
/// — the one outcome worse than a slow cartridge is a game that exists nowhere.
fn remove_pc_installs(
    games: &[steamlib::InstalledGame],
    root: &Path,
    result: &mut CartridgeResult,
    warnings: &mut Vec<String>,
    progress: &mut dyn FnMut(Progress),
) {
    if games.is_empty() {
        return;
    }
    let names = games
        .iter()
        .map(|game| game.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if result.verified_ok != Some(true) {
        warnings.push(format!(
            "{names} stayed on this PC: the copy on the cartridge did not check out, so the \
             original was not deleted. Steam will go on playing the PC's copy."
        ));
        return;
    }
    let Some(steam_root) = steam::steam_root() else {
        warnings.push(format!(
            "{names} stayed on this PC: Steam could not be found to update."
        ));
        return;
    };

    for game in games {
        progress(Progress {
            step: "move",
            message: format!("Removing {} from this PC…", game.name),
            done_bytes: 0,
            total_bytes: 0,
        });
        match remove_pc_install(&steam_root, root, game, warnings) {
            Ok((library, freed)) => result.removed_from_pc.push(format!(
                "{} from {} ({} freed)",
                game.name,
                library.display(),
                format::human_bytes(freed)
            )),
            Err(e) => warnings.push(format!("{} stayed on this PC: {e}", game.name)),
        }
    }
}

/// Delete one Steam install, having checked it is the thing it claims to be.
///
/// The order is chosen so that stopping anywhere leaves Steam believing the
/// truth. The manifest goes first: from then on Steam does not claim the game
/// here, and the only manifest left for it is the cartridge's. Then the library
/// list, then the files — a folder that fails to delete is wasted space, not a
/// game Steam thinks is installed.
///
/// Workshop content and the shader cache under the old library are left alone:
/// neither is the install, and Steam rebuilds both.
fn remove_pc_install(
    steam_root: &Path,
    root: &Path,
    game: &steamlib::InstalledGame,
    warnings: &mut Vec<String>,
) -> Result<(PathBuf, u64), String> {
    let library = steamlib::library_of_install(game).ok_or_else(|| {
        format!(
            "{} is not laid out like a Steam install",
            game.install_path.display()
        )
    })?;

    // Never the cartridge itself, whatever path reaches it.
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if canonical(&game.install_path).starts_with(canonical(root)) {
        return Err("that copy is the one on the cartridge".into());
    }

    // The copy replacing it has to be where Steam will look for it.
    let cartridge_library = steamlib::library_root(root);
    let install_dir = game
        .install_path
        .file_name()
        .ok_or_else(|| "the install folder has no name".to_string())?;
    let manifest_name = game
        .manifest_path
        .file_name()
        .ok_or_else(|| "the manifest has no name".to_string())?;
    if !cartridge_library
        .join("steamapps/common")
        .join(install_dir)
        .is_dir()
        || !cartridge_library
            .join("steamapps")
            .join(manifest_name)
            .is_file()
    {
        return Err("the cartridge does not hold a copy Steam can find".into());
    }

    if steamlib::steam_is_running() {
        return Err(steamlib::LibraryError::SteamRunning.to_string());
    }
    let busy = busy::holders(&game.install_path);
    if !busy.holders.is_empty() {
        return Err(format!(
            "it is in use by {}",
            busy.holders
                .iter()
                .map(|holder| holder.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let freed = steamlib::tree_size(&game.install_path);

    std::fs::remove_file(&game.manifest_path)
        .map_err(|e| format!("could not delete {}: {e}", game.manifest_path.display()))?;

    if let Err(e) = steamlib::record_moved_app(
        steam_root,
        &game.app_id,
        &library,
        &cartridge_library,
        freed,
    ) {
        // Not fatal: with the manifest gone, Steam finds the truth when it scans.
        warnings.push(format!(
            "Steam's library list still names {} under {} until Steam next starts. ({e})",
            game.name,
            library.display()
        ));
    }

    std::fs::remove_dir_all(&game.install_path).map_err(|e| {
        format!(
            "Steam no longer lists it here, but {} could not be deleted: {e}",
            game.install_path.display()
        )
    })?;

    Ok((library, freed))
}

/// After a format the mount point can briefly disappear.
/// The device node currently mounted at `path`, if any.
///
/// Only meaningful on Linux, where a device can be looked up in
/// `/proc/mounts`; on Windows the drive letter is the handle used throughout
/// a format, so there is nothing to remember ahead of time.
#[cfg(not(windows))]
fn current_device(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string("/proc/mounts").ok()?;
    drives::parse_proc_mounts(&text)
        .into_iter()
        .find(|entry| entry.mount == path)
        .map(|entry| entry.device)
}

#[cfg(windows)]
fn current_device(_path: &Path) -> Option<String> {
    None
}

/// Wait for the drive to come back after formatting.
///
/// On Windows the drive letter survives a format, so this just polls `path`
/// itself. On Linux the desktop automounts by volume label, so a freshly
/// labelled filesystem can land at an entirely new path; when the device that
/// used to be at `path` is known, this looks for wherever *it* ended up
/// instead of waiting on a path that may never reappear.
fn wait_for_remount(path: PathBuf, device: Option<&str>) -> Result<PathBuf, String> {
    for _ in 0..40 {
        if path.is_dir() {
            return Ok(path);
        }
        if let Some(device) = device {
            if let Ok(text) = std::fs::read_to_string("/proc/mounts") {
                if let Some(entry) = drives::parse_proc_mounts(&text)
                    .into_iter()
                    .find(|entry| entry.device == device)
                {
                    return Ok(entry.mount);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err(format!(
        "{} did not come back after formatting. Reconnect the drive and try again.",
        path.display()
    ))
}

/// A default volume name derived from the title, for the default filesystem.
pub fn default_label(title: &str) -> String {
    default_label_for(format::Filesystem::default(), title)
}

/// A default volume name derived from the title.
///
/// Kept within the chosen filesystem's own limit, which is what makes the
/// difference visible: exFAT allows 11 characters, so *Hollow Knight* becomes
/// *Hollow Knig*, while btrfs has room for the whole thing.
pub fn default_label_for(filesystem: format::Filesystem, title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // 64 is well inside btrfs's 256 and long enough for any game's name; a
    // volume label the width of a sentence helps nobody.
    let limit = filesystem.label_limit().min(64);
    let truncated: String = cleaned.chars().take(limit).collect();
    let trimmed = truncated.trim().to_string();
    if trimmed.is_empty() {
        "Cartridge".to_string()
    } else {
        trimmed
    }
}

/// Check the requested drive is one we are actually willing to write to.
pub(crate) fn resolve_target(requested: &str) -> Result<PathBuf, String> {
    if requested.trim().is_empty() {
        return Err("Choose a drive first.".into());
    }
    let requested_path = Path::new(requested);

    let matched = drives::list_drives()
        .iter()
        .any(|drive| Path::new(&drive.path) == requested_path);

    if !matched {
        return Err(format!(
            "{requested} is not a removable drive this tool will write to. \
             Re-scan and pick a drive from the list."
        ));
    }
    if !requested_path.is_dir() {
        return Err(format!("{requested} is not there any more."));
    }
    Ok(requested_path.to_path_buf())
}

/// Ask HowLongToBeat about each game and put its figures under that game.
///
/// Only when the user has switched the lookup on, and never fatal: a game it
/// cannot find, or a site that has changed under it, is a warning and a
/// cartridge without an estimate. The first network failure stops the rest,
/// because the next nine requests would fail the same way, slowly.
fn with_estimates(
    conf: String,
    games: &[(&str, &str)],
    progress: &mut dyn FnMut(Progress),
    warnings: &mut Vec<String>,
) -> String {
    if !crate::settings::load().hltb_enabled || games.is_empty() {
        return conf;
    }
    progress(Progress {
        step: "hltb",
        message: "Looking up how long it takes to beat…".to_string(),
        done_bytes: 0,
        total_bytes: 0,
    });
    let mut found: std::collections::HashMap<String, Vec<String>> = Default::default();
    for (title, executable) in games {
        match crate::hltb::lookup(title) {
            Ok(Some(estimate)) => {
                found.insert(crate::stats::key_for(executable), estimate.conf_lines());
            }
            Ok(None) => warnings.push(format!("HowLongToBeat has no match for {title}.")),
            Err(e) => {
                warnings.push(format!("No HowLongToBeat figures: {e}"));
                break;
            }
        }
    }
    crate::playlog::rewrite(&conf, &|key| key.starts_with("hltb_"), &|owner| {
        found.get(owner).cloned()
    })
}

/// Make a cartridge a combo drive, and put its own games on its memory card.
///
/// `memory_card=yes` goes at the top, where every reader looks. Each game's
/// save folder comes from Ludusavi when that lookup is on and goes into
/// `memorycard.conf` beside the cartridge, the one place the memory card view
/// and the wizard both edit, so writing the card again later replaces these
/// rather than doubling them. `games` is `(title, executable, icon)`, the icon
/// relative to the drive.
pub(crate) fn with_memory_card(
    conf: String,
    on: bool,
    root: &Path,
    games: &[(&str, &str, Option<&str>)],
    progress: &mut dyn FnMut(Progress),
    warnings: &mut Vec<String>,
) -> String {
    if !on {
        return conf;
    }
    let conf = format!("memory_card=yes\n{conf}");
    if !crate::settings::load().ludusavi_enabled {
        warnings.push(
            "Combo drive: no save folders were looked up. Switch on Ludusavi in \
             Settings, or add the games on the Memory card page."
                .to_string(),
        );
        return conf;
    }
    progress(Progress {
        step: "saves",
        message: "Finding where each game keeps its saves…".to_string(),
        done_bytes: 0,
        total_bytes: 0,
    });
    let mut card = Vec::new();
    for (title, executable, icon) in games {
        let steam_id = executable.strip_prefix("steam://rungameid/");
        match crate::ludusavi::lookup(title, steam_id) {
            Ok(Some(places)) => card.push(crate::memcard::CardGame {
                title: title.to_string(),
                executable: executable.to_string(),
                icon_source: icon.map(|rel| root.join(rel).to_string_lossy().into_owned()),
                save_windows: places.windows,
                save_linux: places.linux,
                ..Default::default()
            }),
            Ok(None) => warnings.push(format!(
                "Ludusavi does not know where {title} keeps its saves; add it on the \
                 Memory card page."
            )),
            Err(e) => {
                warnings.push(e);
                break;
            }
        }
    }
    if !card.is_empty() {
        match crate::memcard::add_games(root, card) {
            Ok(more) => warnings.extend(more),
            Err(e) => warnings.push(format!("The memory card was not written: {e}")),
        }
    }
    conf
}

/// Put back whatever play history the stats file has for the games now on the
/// cartridge. The conf was just written from scratch, and a game played before
/// under the same executable keeps its hours.
fn mirror_history(root: &Path, warnings: &mut Vec<String>) {
    if let Err(e) = crate::playlog::mirror(root) {
        warnings.push(format!(
            "Play history was not copied into cartridge.conf: {e}"
        ));
    }
}

/// Copy the chosen art to the cartridge. Returns where it landed.
fn write_cover(root: &Path, request: &CartridgeRequest) -> Result<Option<String>, String> {
    let Some(source) = cover_source(
        request.cover_source.as_deref(),
        request.app_id.as_deref(),
        request.playnite_id.as_deref(),
        request.source_dir.as_deref(),
        &request.title,
    )?
    else {
        return Ok(None);
    };
    copy_cover(&source, root, "cover").map(Some)
}

/// Where the art for one game currently lives.
///
/// A path chosen in the wizard wins; otherwise it comes from Steam's cache,
/// Playnite's, or the last artwork downloaded for this game, whichever the game
/// came from. `Ok(None)` means there is simply no art to copy, which is not an
/// error.
pub(crate) fn cover_source(
    chosen: Option<&str>,
    app_id: Option<&str>,
    playnite_id: Option<&str>,
    source_dir: Option<&str>,
    title: &str,
) -> Result<Option<PathBuf>, String> {
    if let Some(path) = chosen.map(str::trim).filter(|p| !p.is_empty()) {
        // Chosen by hand, so it is right by definition — whatever shape it is.
        return Ok(Some(PathBuf::from(path)));
    }
    if let Some(app_id) = app_id.filter(|id| is_numeric(id)) {
        let local = steam::steam_root().and_then(|root| steam::find_cover(&root, app_id));
        let found = local.or_else(|| sgdb::last_used_artwork(&format!("steam:{app_id}")));
        return Ok(prefer_portrait(found, title));
    }
    if let Some(playnite_id) = playnite_id {
        let root_dir = playnite::playnite_root()
            .ok_or_else(|| "no Playnite installation to take the cover from".to_string())?;
        let found = playnite::find_exports(&root_dir)
            .iter()
            .filter_map(|p| playnite::import_from(p).ok())
            .flatten()
            .find(|g| g.id == playnite_id)
            .and_then(|g| g.cover)
            .and_then(|c| playnite::resolve_cover(&root_dir, &c));
        let found = found.or_else(|| sgdb::last_used_artwork(&format!("playnite:{playnite_id}")));
        return Ok(prefer_portrait(found, title));
    }
    // A scanned folder game. No launcher cache to fall back on, so this is the
    // same lookup-then-fetch the wizard does when the game is picked — without
    // it a folder game's cover would show in the wizard and be missing from the
    // cartridge, because the wizard only passes a path for art chosen by hand.
    if let Some(dir) = source_dir.map(str::trim).filter(|d| !d.is_empty()) {
        let key = folder_artwork_key(dir);
        return Ok(sgdb::last_used_artwork(&key).or_else(|| autofetch_cover(&key)));
    }
    Ok(None)
}

/// Where a cartridge keeps everything that is not addressed to Windows.
///
/// The root of a cartridge is a place someone looks at: it is a drive, and they
/// opened it to see what is on it. A ten-game collection put fifteen files
/// there, thirteen of them artwork nobody needs to see. Those live here now.
///
/// Three things stay at the root because something outside this program reads
/// them there: `autorun.inf`, which Windows only reads from the root; the icon
/// it names; and `cartridge.conf`, which is the file a person edits by hand.
pub const ASSET_DIR: &str = ".gamepak";

/// The drive icon Explorer reads, which stays at the root because `autorun.inf`
/// does. Named for what it is: it is built from the cover when nothing better
/// was chosen, but it is the icon, and a cartridge that set its own icon art
/// was writing a file called `cover.ico` that had never been a cover.
pub const ICON_NAME: &str = "icon.ico";

/// Copy art onto the cartridge as `.gamepak/<stem>.<its extension>`.
///
/// Returns the name relative to the cartridge root, which is what goes into
/// cartridge.conf. Forward slash regardless of platform, like the manifest, so
/// a cartridge written on Windows reads on Linux.
pub(crate) fn copy_cover(source: &Path, root: &Path, stem: &str) -> Result<String, String> {
    let meta = std::fs::metadata(source).map_err(|e| format!("{}: {e}", source.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", source.display()));
    }
    if meta.len() > MAX_COVER_BYTES {
        return Err(format!(
            "{} is {:.1} MB; the limit is {} MB",
            source.display(),
            meta.len() as f64 / 1_048_576.0,
            MAX_COVER_BYTES / 1_048_576
        ));
    }

    // Keep the source's extension so the launcher picks the right MIME type.
    let extension = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_else(|| "jpg".to_string());
    let relative = format!("{ASSET_DIR}/{stem}.{extension}");

    let destination = root.join(ASSET_DIR).join(format!("{stem}.{extension}"));
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        crate::autorun::hide(parent);
    }
    copy_small_file(source, &destination)
        .map_err(|e| format!("could not write {}: {e}", destination.display()))?;
    Ok(relative)
}

/// Copy one small file without the OS fast path.
///
/// `std::fs::copy` (and `std::io::copy` between two `File`s) can try
/// `copy_file_range`/`sendfile` on Linux, which some FUSE filesystems don't
/// implement — an exFAT driver has been observed failing both calls above
/// with ENOSYS even though the destination mount is otherwise writable. A
/// manual buffer loop sidesteps that fast path entirely; both files here are
/// small enough (a manifest, a cover under `MAX_COVER_BYTES`) that losing it
/// costs nothing worth trading reliability for.
fn copy_small_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::io::{Read, Write};

    let mut source = std::fs::File::open(from)?;
    let mut destination = std::fs::File::create(to)?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        destination.write_all(&buffer[..read])?;
    }
    destination.sync_all()?;
    Ok(())
}

/// Strip anything that would corrupt the `key=value` file.
///
/// Newlines are the one that matters: a title containing one could otherwise
/// append an `executable=` line of its own choosing.
pub fn sanitize_conf_value(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A cartridge may name a known URI scheme, or a file on the cartridge itself.
pub fn validate_executable(executable: &str, root: &Path) -> Result<(), String> {
    if executable.is_empty() {
        return Err("Set what Play should start.".into());
    }

    let lower = executable.to_lowercase();
    if ALLOWED_SCHEMES.iter().any(|s| lower.starts_with(s)) {
        return Ok(());
    }

    // Anything with a scheme we do not know is refused rather than written out
    // and handed to the shell later.
    if let Some(colon) = executable.find(':') {
        let looks_like_scheme = executable[..colon]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.');
        let is_drive_letter = colon == 1;
        if looks_like_scheme && !is_drive_letter && executable[colon..].starts_with("://") {
            return Err(format!(
                "{executable} uses a scheme this launcher will not open. Supported: {}",
                ALLOWED_SCHEMES.join(", ")
            ));
        }
    }

    let candidate = Path::new(executable);
    if candidate.is_absolute() || executable.contains(':') {
        return Err(
            "A program has to live on the cartridge, so use a path relative to its root."
                .to_string(),
        );
    }
    use std::path::Component;
    for component in candidate.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err("The program path must not leave the cartridge.".to_string());
        }
    }

    if !root.join(candidate).exists() {
        return Err(format!("{executable} is not on the cartridge yet."));
    }
    Ok(())
}

/// Render the conf file, with a header explaining where it came from.
pub fn render_cartridge_conf(
    title: &str,
    executable: &str,
    cover: Option<&str>,
    icon: Option<&str>,
    background: Option<&str>,
    logo: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str("# PC GamePak\n");
    out.push_str("# Written by the create-cartridge wizard. Safe to edit by hand.\n");
    out.push('\n');
    out.push_str(&format!("title={title}\n"));
    out.push_str(&format!("executable={executable}\n"));
    if let Some(cover) = cover {
        out.push_str(&format!("cover={cover}\n"));
    }
    if let Some(icon) = icon {
        out.push_str(&format!("icon={icon}\n"));
    }
    if let Some(background) = background {
        out.push_str(&format!("background={background}\n"));
    }
    if let Some(logo) = logo {
        out.push_str(&format!("logo={logo}\n"));
    }
    out
}

/// Render the bundle (multi-game) conf, with a `[collection]` header and one
/// `[game]` block per game.
pub fn render_bundle_conf(
    collection_title: &str,
    collection_cover: Option<&str>,
    collection_icon: Option<&str>,
    collection_background: Option<&str>,
    collection_logo: Option<&str>,
    games: &[ConfGame<'_>],
) -> String {
    let mut out = String::new();
    out.push_str("# PC GamePak\n");
    out.push_str("# Written by the create-cartridge wizard. Safe to edit by hand.\n");
    out.push('\n');
    out.push_str("[collection]\n");
    out.push_str(&format!("title={collection_title}\n"));
    if let Some(cover) = collection_cover {
        out.push_str(&format!("cover={cover}\n"));
    }
    if let Some(icon) = collection_icon {
        out.push_str(&format!("icon={icon}\n"));
    }
    if let Some(background) = collection_background {
        out.push_str(&format!("background={background}\n"));
    }
    if let Some(logo) = collection_logo {
        out.push_str(&format!("logo={logo}\n"));
    }
    out.push('\n');
    for game in games {
        out.push_str("[game]\n");
        out.push_str(&format!("title={}\n", game.title));
        out.push_str(&format!("executable={}\n", game.executable));
        for (key, value) in [
            ("cover", game.cover),
            ("background", game.background),
            ("logo", game.logo),
            ("icon", game.icon),
        ] {
            if let Some(value) = value {
                out.push_str(&format!("{key}={value}\n"));
            }
        }
        out.push('\n');
    }
    out
}

/// One game's art while it is being written, before the conf is rendered.
///
/// Owns its strings, where `ConfGame` borrows them: this is filled in over the
/// course of a copy, and that one is a view of the result.
#[derive(Debug, Clone, Default)]
pub struct GameArt {
    pub title: String,
    pub executable: String,
    pub cover: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
    pub icon: Option<String>,
}

impl GameArt {
    pub fn as_conf(&self) -> ConfGame<'_> {
        ConfGame {
            title: &self.title,
            executable: &self.executable,
            cover: self.cover.as_deref(),
            background: self.background.as_deref(),
            logo: self.logo.as_deref(),
            icon: self.icon.as_deref(),
        }
    }
}

/// One game as `cartridge.conf` records it.
///
/// A struct rather than the tuple this used to be: four optional pictures and
/// two strings is past the point where position tells anyone which is which.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConfGame<'a> {
    pub title: &'a str,
    pub executable: &'a str,
    pub cover: Option<&'a str>,
    pub background: Option<&'a str>,
    pub logo: Option<&'a str>,
    pub icon: Option<&'a str>,
}

/// A name for a cartridge carrying several games, from what they are called.
///
/// Sequels usually share their opening words — *God of War* and *God of War
/// Ragnarök* — so the shared run becomes the collection's name. When the titles
/// share nothing worth using, the count says what it is instead. The wizard
/// offers this; the user can always type their own.
pub fn suggest_collection_name(titles: &[String]) -> String {
    let cleaned: Vec<Vec<String>> = titles
        .iter()
        .map(|t| {
            sanitize_conf_value(t)
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .filter(|words: &Vec<String>| !words.is_empty())
        .collect();

    let Some(first) = cleaned.first() else {
        return "Collection".to_string();
    };
    if cleaned.len() == 1 {
        return first.join(" ");
    }

    // The longest run of opening words every title agrees on, compared without
    // case but kept in the first title's casing.
    let mut shared: Vec<&str> = Vec::new();
    for (index, word) in first.iter().enumerate() {
        let agreed = cleaned[1..].iter().all(|words| {
            words
                .get(index)
                .is_some_and(|other| other.to_lowercase() == word.to_lowercase())
        });
        if !agreed {
            break;
        }
        shared.push(word);
    }

    // One short shared word ("The", "Halo") names nothing on its own.
    let worth_using = shared.len() >= 2
        || shared
            .first()
            .is_some_and(|w| w.chars().count() >= 4 && !is_stop_word(w));

    if worth_using {
        format!("{} Collection", shared.join(" "))
    } else {
        format!("{} and {} more", first.join(" "), cleaned.len() - 1)
    }
}

fn is_stop_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "the" | "a" | "an" | "of" | "and" | "for" | "in" | "on"
    )
}

fn is_numeric(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn read_as_data_uri(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_COVER_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/jpeg",
    };
    Some(format!(
        "data:{mime};base64,{}",
        crate::cartridge::base64_encode(&bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_folder_name_becomes_something_searchable() {
        // Every one of these is a real folder name that found nothing on
        // SteamGridDB until the word boundaries were put back.
        assert_eq!(search_query("DeadIsland2"), "Dead Island 2");
        assert_eq!(search_query("HogwartsLegacy"), "Hogwarts Legacy");
        assert_eq!(search_query("DyingLight"), "Dying Light");
        // The end of an acronym is the capital before the first lowercase one.
        assert_eq!(search_query("LEGOStarWarsTSS"), "LEGO Star Wars TSS");
        // Separators are word boundaries too.
        assert_eq!(search_query("God of War - Ragnarok"), "God of War Ragnarok");
        assert_eq!(search_query("Black_Myth-Wukong"), "Black Myth Wukong");
    }

    #[test]
    fn a_name_that_is_already_a_title_is_left_alone() {
        assert_eq!(search_query("Ghost of Tsushima DC"), "Ghost of Tsushima DC");
        assert_eq!(
            search_query("XCOM 2 War of the Chosen"),
            "XCOM 2 War of the Chosen"
        );
        // An all-caps title is not an acronym run to be broken up.
        assert_eq!(search_query("DEATHLOOP"), "DEATHLOOP");
    }

    #[test]
    fn a_folder_game_is_filed_under_its_folder_name() {
        // The artwork picker keys on the displayed name, so the lookup has to
        // reduce the path back to exactly that or it would never find what the
        // picker saved.
        assert_eq!(
            folder_artwork_key(r"F:\Games\Epic\HogwartsLegacy"),
            "HogwartsLegacy"
        );
        // Both separators, on both platforms. This used to go through
        // Path::file_name, which meant the Windows case above returned the
        // whole path on Linux and the test only passed on one of the two
        // machines CI runs.
        assert_eq!(
            folder_artwork_key("/home/harry/Games/HogwartsLegacy"),
            "HogwartsLegacy"
        );
        // A trailing separator is the folder, not an empty name.
        assert_eq!(folder_artwork_key(r"F:\Games\Epic\Control\"), "Control");
        assert_eq!(folder_artwork_key("/games/Control/"), "Control");
        // Nothing to split is still an answer.
        assert_eq!(folder_artwork_key("Control"), "Control");
    }

    #[test]
    fn sanitises_values_that_would_corrupt_the_file() {
        assert_eq!(
            sanitize_conf_value("Doom\nexecutable=evil.exe"),
            "Doom executable=evil.exe"
        );
        assert_eq!(sanitize_conf_value("  Hollow   Knight  "), "Hollow Knight");
        assert_eq!(sanitize_conf_value("\r\n\t "), "");
    }

    #[test]
    fn the_drive_icon_follows_the_primary_game() {
        let game = |cover: &str, icon: Option<&str>| GameArt {
            cover: Some(cover.to_string()),
            icon: icon.map(str::to_string),
            ..Default::default()
        };
        let games = [
            game(".gamepak/cover_0.png", None),
            game(".gamepak/cover_1.png", Some(".gamepak/icon_1.png")),
        ];
        let fallback = Some(".gamepak/collection.png");

        // Its own icon first, then its cover.
        assert_eq!(
            drive_icon_art(None, &games, Some(1), fallback),
            Some(".gamepak/icon_1.png")
        );
        assert_eq!(
            drive_icon_art(None, &games, Some(0), fallback),
            Some(".gamepak/cover_0.png")
        );
        // A cartridge icon chosen outright wins over any game.
        assert_eq!(
            drive_icon_art(Some("own.png"), &games, Some(1), fallback),
            Some("own.png")
        );
        // No primary, or one past the end, is the rule from before.
        assert_eq!(drive_icon_art(None, &games, None, fallback), fallback);
        assert_eq!(drive_icon_art(None, &games, Some(9), fallback), fallback);
    }

    #[test]
    fn renders_a_conf_that_round_trips() {
        let conf = render_cartridge_conf(
            "Hollow Knight",
            "steam://rungameid/367520",
            Some("cover.jpg"),
            Some("icon.ico"),
            Some("background.jpg"),
            Some("logo.png"),
        );
        assert!(conf.contains("title=Hollow Knight\n"));
        assert!(conf.contains("executable=steam://rungameid/367520\n"));
        assert!(conf.contains("cover=cover.jpg\n"));
        assert!(conf.contains("icon=icon.ico\n"));
        assert!(conf.contains("background=background.jpg\n"));
        assert!(conf.contains("logo=logo.png\n"));

        let bare = render_cartridge_conf("X", "steam://rungameid/1", None, None, None, None);
        assert!(!bare.contains("cover="));
        assert!(!bare.contains("icon="));
        assert!(!bare.contains("background="));
        assert!(!bare.contains("logo="));
    }

    #[test]
    fn accepts_known_schemes_including_playnite() {
        let root = Path::new("/media/x");
        for good in [
            "steam://rungameid/367520",
            "playnite://playnite/start/2b3c4d5e-1111-2222-3333-444455556666",
            "heroic://launch/gog/1207658921",
            "https://example.com/play",
        ] {
            assert!(validate_executable(good, root).is_ok(), "{good}");
        }
    }

    #[test]
    fn refuses_unknown_schemes_and_off_cartridge_programs() {
        let root = Path::new("/media/x");
        for bad in [
            "file:///etc/passwd",
            "javascript://alert(1)",
            "/usr/bin/bash",
            "../../../usr/bin/bash",
            "C:\\Windows\\System32\\cmd.exe",
            "",
        ] {
            assert!(validate_executable(bad, root).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_chosen_game_folder_is_checked_before_anything_is_copied() {
        let scratch = crate::testutil::Scratch::new("source");
        let game = scratch.join("Split Fiction");
        std::fs::create_dir_all(&game).unwrap();

        assert_eq!(check_source_dir(game.to_str().unwrap()).unwrap(), game);
        // Surrounding whitespace is a paste, not a different folder.
        let padded = format!("  {}  ", game.display());
        assert_eq!(check_source_dir(&padded).unwrap(), game);

        // Nothing chosen, and a path that is not a folder.
        assert!(check_source_dir("").is_err());
        assert!(check_source_dir("   ").is_err());
        std::fs::write(scratch.join("notes.txt"), b"x").unwrap();
        assert!(check_source_dir(scratch.join("notes.txt").to_str().unwrap()).is_err());
        assert!(check_source_dir(scratch.join("nope").to_str().unwrap()).is_err());

        // A whole drive: copying one takes every other game on the disk, the
        // recycle bin and System Volume Information with it.
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        let err = check_source_dir(root).unwrap_err();
        assert!(err.contains("whole drive"), "{err}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_itself_is_never_a_game_folder() {
        // One click away in a picker that opens on C:.
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let err = check_source_dir(&system).unwrap_err();
        assert!(err.contains("inside Windows itself"), "{err}");

        let err = check_source_dir(&format!("{system}\\System32")).unwrap_err();
        assert!(err.contains("inside Windows itself"), "{err}");
    }

    #[test]
    fn a_request_that_says_nothing_about_verifying_still_verifies() {
        // The failure verifying catches is silent: right length, wrong bytes,
        // and a copy that reported success. Anything driving this by JSON —
        // build-cart, a script — gets the check without having to know to ask.
        let required =
            r#""drivePath":"/mnt/cart","title":"Tunic","executable":"Games/Tunic/TUNIC.exe""#;

        let quiet: CartridgeRequest = serde_json::from_str(&format!("{{{required}}}")).unwrap();
        assert!(quiet.verify_copy);

        // Still a real switch, not a constant.
        let off: CartridgeRequest =
            serde_json::from_str(&format!(r#"{{{required},"verifyCopy":false}}"#)).unwrap();
        assert!(!off.verify_copy);
    }

    #[test]
    fn a_bundle_does_not_hand_every_game_the_same_folder() {
        // for_game() fills the rest of the request with `..self.clone()`, so a
        // source folder set on the collection would otherwise be copied once
        // per game under each game's name.
        let request = CartridgeRequest {
            source_dir: Some("/games/wukong".to_string()),
            ..Default::default()
        };
        let job = request.for_game(&BundleGameRequest {
            title: "Split Fiction".to_string(),
            source_dir: Some("/games/split".to_string()),
            ..Default::default()
        });
        assert_eq!(job.source_dir.as_deref(), Some("/games/split"));

        let job = request.for_game(&BundleGameRequest {
            title: "Hades".to_string(),
            ..Default::default()
        });
        assert_eq!(job.source_dir, None);
    }

    #[test]
    fn refuses_targets_that_are_not_removable_drives() {
        for bad in ["/", "/home", "/etc", "/usr/local", ""] {
            assert!(resolve_target(bad).is_err(), "{bad} must never be a target");
        }
    }

    #[test]
    fn creating_a_cartridge_on_a_bad_target_writes_nothing() {
        let request = CartridgeRequest {
            drive_path: "/".to_string(),
            title: "Evil".to_string(),
            executable: "steam://rungameid/1".to_string(),
            format_drive: true,
            ..Default::default()
        };
        let mut seen = Vec::new();
        let err = create_cartridge(&request, &mut |p| seen.push(p)).unwrap_err();
        assert!(err.contains("not a removable drive"), "{err}");
        // Crucially, it bailed before the format step emitted anything.
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn steam_is_only_disturbed_when_its_library_list_is_going_to_change() {
        // Not copying anything: Steam has no idea this is happening.
        let mut request = CartridgeRequest {
            app_id: Some("367520".into()),
            copy_game: false,
            ..Default::default()
        };
        assert!(!will_register_with_steam(&request));

        // Copying a Steam game does add the drive to the list.
        request.copy_game = true;
        assert!(will_register_with_steam(&request));

        // A GOG or itch game is copied without Steam ever hearing about it.
        request.app_id = Some("b7f3-tunic".into());
        assert!(!will_register_with_steam(&request));
        request.app_id = None;
        assert!(!will_register_with_steam(&request));

        // A collection counts if any one of its games is a Steam game.
        request.games = Some(vec![
            BundleGameRequest {
                playnite_id: Some("b7f3-tunic".into()),
                ..Default::default()
            },
            BundleGameRequest {
                app_id: Some("1593500".into()),
                ..Default::default()
            },
        ]);
        assert!(will_register_with_steam(&request));

        // And a collection of non-Steam games does not.
        request.games = Some(vec![BundleGameRequest {
            playnite_id: Some("b7f3-tunic".into()),
            ..Default::default()
        }]);
        assert!(!will_register_with_steam(&request));
    }

    #[test]
    fn a_per_game_request_keeps_the_drive_but_never_the_format() {
        let request = CartridgeRequest {
            drive_path: "/media/cart".into(),
            title: "God of War Collection".into(),
            collection_cover_source: Some("/pictures/collection.png".into()),
            format_drive: true,
            copy_game: true,
            copy_executable: Some("wrong-game.exe".into()),
            ..Default::default()
        };
        let game = BundleGameRequest {
            title: "God of War".into(),
            executable: "steam://rungameid/1593500".into(),
            app_id: Some("1593500".into()),
            ..Default::default()
        };

        let job = request.for_game(&game);
        assert_eq!(job.drive_path, "/media/cart");
        assert!(job.copy_game);
        assert_eq!(job.title, "God of War");
        assert_eq!(job.app_id.as_deref(), Some("1593500"));
        // Formatting once per game would wipe whatever was copied before it.
        assert!(!job.format_drive);
        // And the collection's own fields must not leak into a game: the
        // single-game executable pick belongs to a different game entirely.
        assert!(job.copy_executable.is_none());
        assert!(job.games.is_none());
    }

    #[test]
    fn art_is_copied_under_the_stem_it_was_given() {
        let scratch = crate::testutil::Scratch::new("cover-copy");
        scratch.write("source.PNG", b"not really a png");
        let root = scratch.path().join("cartridge");
        std::fs::create_dir_all(&root).unwrap();

        let name = copy_cover(&scratch.path().join("source.PNG"), &root, "cover_1").unwrap();

        // Lowercased extension, because that name goes straight into
        // cartridge.conf for the launcher to resolve — and under .gamepak, so a
        // ten-game cartridge does not put thirteen pictures in the drive root.
        assert_eq!(name, ".gamepak/cover_1.png");
        assert!(root.join(".gamepak").join("cover_1.png").is_file());
    }

    #[test]
    fn the_conf_names_art_where_the_art_actually_is() {
        let scratch = crate::testutil::Scratch::new("cover-conf");
        scratch.write("art.png", b"not really a png");
        let root = scratch.path().join("cartridge");
        std::fs::create_dir_all(&root).unwrap();

        let request = CartridgeRequest {
            title: "Hollow Knight".to_string(),
            executable: "steam://rungameid/367520".to_string(),
            cover_source: Some(scratch.path().join("art.png").display().to_string()),
            ..Default::default()
        };
        let name = write_cover(&root, &request).unwrap().unwrap();

        // The whole point of the return value: it is the name the cartridge
        // refers to the art by, so it has to survive into cartridge.conf with
        // the folder still attached. Taking `file_name()` off it here wrote a
        // conf pointing at a root that holds nothing.
        assert_eq!(name, ".gamepak/cover.png");
        let conf = render_cartridge_conf("Hollow Knight", "x", Some(&name), None, None, None);
        assert!(conf.contains("cover=.gamepak/cover.png\n"), "{conf}");
    }

    #[test]
    fn a_rewrite_sweeps_the_asset_folder_and_the_root_it_replaced() {
        let scratch = crate::testutil::Scratch::new("sweep-assets");
        let root = scratch.path();
        std::fs::create_dir(root.join(ASSET_DIR)).unwrap();

        for name in [
            // Where art lives now.
            ".gamepak/cover_0.png",
            ".gamepak/logo.png",
            // A fourth game that has since been removed.
            ".gamepak/cover_3.png",
            // Where art lived before, on a cartridge written by an older
            // build. The rewrite put its replacement under .gamepak, so these
            // are duplicates the drive root should not keep showing.
            "cover_0.png",
            "logo.png",
            // Explorer's, and not under .gamepak because autorun.inf cannot
            // point there.
            "icon.ico",
            // Never the writer's to delete, wherever it sits.
            "screenshot.png",
        ] {
            std::fs::write(root.join(name), b"x").unwrap();
        }

        let mut warnings = Vec::new();
        sweep_stale_art(
            root,
            [".gamepak/cover_0.png", ".gamepak/logo.png", "icon.ico"].into_iter(),
            &mut warnings,
        );
        assert!(warnings.is_empty(), "{warnings:?}");

        for kept in [
            ".gamepak/cover_0.png",
            ".gamepak/logo.png",
            "icon.ico",
            "screenshot.png",
        ] {
            assert!(root.join(kept).is_file(), "{kept} was swept");
        }
        for gone in [".gamepak/cover_3.png", "cover_0.png", "logo.png"] {
            assert!(!root.join(gone).exists(), "{gone} survived");
        }
    }

    #[test]
    fn art_that_is_missing_or_oversized_is_refused() {
        let scratch = crate::testutil::Scratch::new("cover-refuse");
        let root = scratch.path().join("cartridge");
        std::fs::create_dir_all(&root).unwrap();

        assert!(copy_cover(&scratch.path().join("nothing.jpg"), &root, "cover").is_err());

        scratch.write("huge.jpg", &vec![0u8; (MAX_COVER_BYTES + 1) as usize]);
        let err = copy_cover(&scratch.path().join("huge.jpg"), &root, "cover").unwrap_err();
        assert!(err.contains("the limit is"), "{err}");
        assert!(!root.join("cover.jpg").exists());
    }

    #[test]
    fn a_chosen_cover_beats_the_libraries() {
        // Nothing is looked up when the wizard supplied a path, so this holds
        // on a machine with neither Steam nor Playnite installed.
        let chosen =
            cover_source(Some("/pictures/gow.png"), Some("1593500"), None, None, "").unwrap();
        assert_eq!(chosen, Some(PathBuf::from("/pictures/gow.png")));

        // A folder game's own path loses to one chosen by hand too.
        let chosen = cover_source(
            Some("/pictures/gow.png"),
            None,
            None,
            Some(r"B:\Games\God of War"),
            "God of War",
        )
        .unwrap();
        assert_eq!(chosen, Some(PathBuf::from("/pictures/gow.png")));

        // Blank counts as unset rather than as a path.
        assert!(cover_source(Some("   "), None, None, None, "")
            .unwrap()
            .is_none());
        assert!(cover_source(None, None, None, None, "").unwrap().is_none());
    }

    #[test]
    fn a_collection_is_named_after_what_the_games_share() {
        assert_eq!(
            suggest_collection_name(&["God of War".into(), "God of War Ragnarök".into(),]),
            "God of War Collection"
        );
        assert_eq!(
            suggest_collection_name(&["Mass Effect 2".into(), "Mass Effect 3".into()]),
            "Mass Effect Collection"
        );
        // Compared without case, but written in the first title's casing.
        assert_eq!(
            suggest_collection_name(&["Halo 3".into(), "HALO 3 ODST".into()]),
            "Halo 3 Collection"
        );
    }

    #[test]
    fn titles_with_nothing_in_common_are_counted_instead() {
        let name =
            suggest_collection_name(&["Hollow Knight".into(), "Hades".into(), "Tunic".into()]);
        assert_eq!(name, "Hollow Knight and 2 more");

        // A single shared stop word names nothing on its own.
        let name = suggest_collection_name(&["The Witness".into(), "The Talos Principle".into()]);
        assert_eq!(name, "The Witness and 1 more");
    }

    #[test]
    fn naming_a_collection_copes_with_nothing_to_go_on() {
        assert_eq!(suggest_collection_name(&["Solo".into()]), "Solo");
        assert_eq!(suggest_collection_name(&[]), "Collection");
        assert_eq!(suggest_collection_name(&["   ".into()]), "Collection");
    }

    #[test]
    fn what_the_wizard_writes_for_a_bundle_is_what_the_launcher_reads() {
        // The round trip is the property that matters: the writer and the
        // reader are in different modules and could drift apart.
        let scratch = crate::testutil::Scratch::new("bundle-round-trip");
        let root = scratch.path();
        scratch.write("gow.png", b"pretend png");

        let art = copy_cover(&root.join("gow.png"), root, "cover_0").unwrap();
        scratch.write("gow-hero.png", b"pretend hero");
        let hero = copy_cover(&root.join("gow-hero.png"), root, "background_0").unwrap();
        let conf = render_bundle_conf(
            "God of War Collection",
            Some("collection.jpg"),
            None,
            None,
            None,
            &[
                ConfGame {
                    title: "God of War",
                    executable: "steam://rungameid/1593500",
                    cover: Some(art.as_str()),
                    // The hero this game carries itself, which a skin asking
                    // for heroes reads instead of the collection's.
                    background: Some(hero.as_str()),
                    ..Default::default()
                },
                ConfGame {
                    title: "God of War Ragnarök",
                    executable: "steam://rungameid/2322010",
                    ..Default::default()
                },
            ],
        );
        std::fs::write(root.join("cartridge.conf"), conf).unwrap();

        let info = crate::cartridge::read_cartridge_info(root.to_str().unwrap()).unwrap();

        assert!(info.is_bundle);
        assert_eq!(info.title, "God of War Collection");
        assert_eq!(info.games.len(), 2);
        assert_eq!(info.games[0].title, "God of War");
        assert_eq!(info.games[1].executable, "steam://rungameid/2322010");
        // Each game carries its own four now, so a collection under a skin that
        // wants heroes has one per game rather than one for the shelf.
        assert!(
            info.games[0].background_path.ends_with("background_0.png"),
            "{:?}",
            info.games[0].background_path
        );
        // And an absent one stays absent rather than falling back to a picture
        // that happens to be on the drive.
        assert!(info.games[1].background_path.is_empty());
        assert!(info.games[0].logo_path.is_empty());
        assert!(
            info.games[0].cover_path.ends_with("cover_0.png"),
            "{:?}",
            info.games[0].cover_path
        );
        // Enter still plays something: the first game is the primary target.
        assert_eq!(info.executable, "steam://rungameid/1593500");
    }

    #[test]
    fn a_supplied_executable_must_stay_inside_the_game_folder() {
        let scratch = crate::testutil::Scratch::new("chosen");
        scratch.write("Game.exe", b"x");
        scratch.write("bin/run.exe", b"x");

        let pick = |chosen: &str| {
            let request = CartridgeRequest {
                title: "Game".into(),
                copy_executable: Some(chosen.to_string()),
                ..Default::default()
            };
            choose_portable_executable(&request, scratch.path(), "Game")
        };

        assert_eq!(pick("Game.exe").unwrap(), "Game.exe");
        // Backslashes are normalised, since the window may send either.
        assert_eq!(pick("bin\\run.exe").unwrap(), "bin/run.exe");

        // The window is not trusted with a path any more than a cartridge is.
        for bad in [
            "../../../etc/passwd",
            "bin/../../escape.exe",
            "/usr/bin/bash",
            "C:\\Windows\\System32\\cmd.exe",
            "missing.exe",
        ] {
            assert!(pick(bad).is_err(), "{bad} should have been refused");
        }
    }

    #[test]
    fn without_a_choice_the_best_candidate_is_used() {
        let scratch = crate::testutil::Scratch::new("auto");
        scratch.write("unins000.exe", b"x");
        scratch.write("Hollow Knight.exe", b"x");

        let request = CartridgeRequest {
            title: "Hollow Knight".into(),
            ..Default::default()
        };
        assert_eq!(
            choose_portable_executable(&request, scratch.path(), "Hollow Knight").unwrap(),
            "Hollow Knight.exe"
        );
    }

    #[test]
    fn a_folder_with_nothing_runnable_is_an_error_not_a_guess() {
        let scratch = crate::testutil::Scratch::new("norun");
        scratch.write("data.pak", b"x");
        let request = CartridgeRequest {
            title: "Empty".into(),
            ..Default::default()
        };
        let err = choose_portable_executable(&request, scratch.path(), "Empty").unwrap_err();
        assert!(err.contains("nothing for Play to start"), "{err}");
    }

    #[test]
    fn derives_a_label_the_chosen_filesystem_will_accept() {
        use format::Filesystem;

        // NTFS is the default now and has room for 32 characters, so the
        // cartridge gets the game's actual name rather than a truncation.
        assert_eq!(default_label("Hollow Knight"), "Hollow Knight");
        assert_eq!(default_label("God of War Ragnarok"), "God of War Ragnarok");
        assert_eq!(default_label("!!!"), "Cartridge");
        assert_eq!(default_label(""), "Cartridge");

        // exFAT's eleven characters are the tight case, and the reason the
        // limit is per filesystem rather than one number.
        assert_eq!(
            default_label_for(Filesystem::Exfat, "Hollow Knight"),
            "Hollow Knig"
        );
        assert_eq!(
            default_label_for(Filesystem::Exfat, "Cinder & Salt"),
            "Cinder Salt"
        );

        // btrfs has room for the whole name, however long.
        assert_eq!(
            default_label_for(Filesystem::Btrfs, "Hollow Knight"),
            "Hollow Knight"
        );

        // Whatever it produces must pass the formatter's own check, for the
        // filesystem it was derived for.
        for filesystem in [Filesystem::Ntfs, Filesystem::Exfat, Filesystem::Btrfs] {
            for title in [
                "Hollow Knight",
                "Cinder & Salt",
                "!!!",
                "",
                "A",
                &"A".repeat(300),
            ] {
                let label = default_label_for(filesystem, title);
                assert!(
                    format::check_label_for(filesystem, &label).is_ok(),
                    "{title:?} on {filesystem:?} gave unusable label {label:?}"
                );
            }
        }
    }
    #[test]
    fn the_sweep_only_claims_names_the_writer_lays_down() {
        for ours in [
            "collection",
            "cover",
            "icon",
            "background",
            "logo",
            "cover_0",
            "cover_12",
        ] {
            assert!(is_art_stem(ours), "{ours}");
        }
        // Anything else on the cartridge belongs to whoever put it there.
        for theirs in [
            "cover_",
            "cover_a",
            "covers",
            "screenshot",
            "readme",
            "manual_1",
            "",
        ] {
            assert!(!is_art_stem(theirs), "{theirs}");
        }
    }

    #[test]
    fn a_rewrite_clears_art_it_no_longer_refers_to() {
        let scratch = crate::testutil::Scratch::new("sweep");
        let root = scratch.path();

        for name in [
            // Still referred to by the new conf.
            "collection.png",
            "cover_0.jpg",
            "cover.ico",
            // Left over: a fourth game that is gone, and a logo whose
            // replacement was chosen in a different format.
            "cover_3.jpg",
            "logo.png",
            // Never the writer's to delete.
            "cartridge.conf",
            "screenshot.png",
            "cover_notes.txt",
        ] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        std::fs::create_dir(root.join("icon.d")).unwrap();

        let mut warnings = Vec::new();
        sweep_stale_art(
            root,
            ["collection.png", "cover_0.jpg", "cover.ico", "logo.jpg"].into_iter(),
            &mut warnings,
        );
        assert!(warnings.is_empty(), "{warnings:?}");

        for kept in [
            "collection.png",
            "cover_0.jpg",
            "cover.ico",
            "cartridge.conf",
            "screenshot.png",
            "cover_notes.txt",
        ] {
            assert!(root.join(kept).is_file(), "{kept} was swept");
        }
        assert!(!root.join("cover_3.jpg").exists(), "a stale cover survived");
        assert!(!root.join("logo.png").exists(), "a shadowed logo survived");
        // A directory that happens to share a stem is not a file to remove.
        assert!(root.join("icon.d").is_dir());
    }

    #[test]
    fn the_file_list_survives_adding_a_game_and_forgets_a_removed_one() {
        let scratch = crate::testutil::Scratch::new("manifest-carry");
        let root = scratch.path();

        let digest = |path: &str, crc: u32| verify::FileDigest {
            path: path.to_string(),
            bytes: 1,
            crc,
        };

        // What an earlier write left: two games, and one cover that is about to
        // be replaced.
        for path in ["hollow/game.exe", "tunic/game.exe"] {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, b"x").unwrap();
        }
        verify::write_manifest(
            root,
            &verify::Manifest {
                files: vec![
                    digest("hollow/game.exe", 1),
                    digest("tunic/game.exe", 2),
                    digest("gone/game.exe", 3),
                ],
            },
        )
        .unwrap();

        // This write only copied Tunic again, so only Tunic is in `written`.
        let carried = carried_manifest(root, &[digest("tunic/game.exe", 9)]);
        let paths: Vec<&str> = carried.iter().map(|f| f.path.as_str()).collect();

        // Hollow Knight is still on the cartridge and stays in the record even
        // though this write never touched it.
        assert_eq!(paths, vec!["hollow/game.exe"]);
        // Tunic is left to the fresh digest, and the deleted game is dropped.
        assert!(!paths.contains(&"tunic/game.exe"));
        assert!(!paths.contains(&"gone/game.exe"));
    }

    #[test]
    fn a_cartridge_with_no_file_list_carries_nothing() {
        let scratch = crate::testutil::Scratch::new("manifest-none");
        assert!(carried_manifest(scratch.path(), &[]).is_empty());
    }
}
