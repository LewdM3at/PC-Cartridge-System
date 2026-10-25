//! Changing a cartridge that already exists.
//!
//! Everything a cartridge says about itself lives in two small files —
//! `cartridge.conf` and `autorun.inf` — and a picture or two. Renaming a
//! collection, fixing a typo, swapping cover art or dropping a game from the
//! list should therefore cost nothing, and until now it meant writing the whole
//! cartridge again: hours, if the games were copied onto it.
//!
//! So this rewrites only the metadata. **Nothing here copies, deletes or moves a
//! game.** Taking a game out of the list leaves its files exactly where they
//! are; the launcher simply stops offering it, and putting it back is a matter
//! of typing its launch target again.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::cartridge::{self, CartridgeInfo};
use crate::{autorun, create};

/// A cartridge as the editor sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Editable {
    pub drive_path: String,
    pub title: String,
    /// The collection's art, as a data URI for the preview.
    pub cover: String,
    pub cover_path: String,
    /// The title logo, as a data URI. Without these the editor's slots had
    /// nothing to show and fell back to the cover, so a cartridge came back
    /// claiming artwork it did not have.
    pub logo: String,
    pub logo_path: String,
    /// The drive's icon, as a data URI.
    pub icon: String,
    pub icon_path: String,
    /// The hero, as a data URI. Shown in its own slot so a cartridge that has
    /// one says so, rather than the slot sitting empty over a picture that is
    /// already on the drive.
    pub background: String,
    pub background_path: String,
    pub is_bundle: bool,
    pub games: Vec<EditableGame>,
    /// True when the games live on the cartridge, which is what makes removing
    /// one from the list worth a warning.
    pub holds_game: bool,
    /// A combo drive: shown as a memory card too.
    pub memory_card: bool,
    /// The cartridge's `platform=`, `PC` when it has none.
    pub platform: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableGame {
    pub title: String,
    pub executable: String,
    pub cover: String,
    pub cover_path: String,
    /// The other three, so the editor's slots show what is on the drive rather
    /// than sitting empty over a picture the cartridge already has.
    pub background: String,
    pub background_path: String,
    pub logo: String,
    pub logo_path: String,
    pub icon: String,
    pub icon_path: String,
}

/// What the window is asking for.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    pub drive_path: String,
    pub title: String,
    /// A new picture for the collection. Absent leaves the existing art alone.
    #[serde(default)]
    pub cover_source: Option<String>,
    /// A new title logo, printed over the cover on the launcher. Absent leaves
    /// whatever the cartridge already had.
    #[serde(default)]
    pub logo_source: Option<String>,
    /// A new picture for the drive's icon in Explorer. Absent falls back to the
    /// cover, which is what a cartridge without its own icon has always used.
    #[serde(default)]
    pub icon_source: Option<String>,
    /// A new hero — the wide one. Absent leaves whatever the cartridge had.
    #[serde(default)]
    pub background_source: Option<String>,
    /// The games, in the order they should appear. A single-game cartridge has
    /// exactly one; removing the last one is refused.
    pub games: Vec<UpdateGame>,
    /// Take the cartridge's own logo off. For a collection that is the mark in
    /// the launcher's corner; `logo_source` absent only ever keeps it.
    #[serde(default)]
    pub remove_logo: bool,
    /// Make it a combo drive, or stop it being one. Absent keeps what it was.
    #[serde(default)]
    pub memory_card: Option<bool>,
    /// A new `platform=` for the cartridge. Absent keeps what it was.
    #[serde(default)]
    pub platform: Option<String>,
    /// Which game the drive icon is made from, as an index into `games`.
    /// Absent keeps the old rule: the icon slot, else the cover.
    #[serde(default)]
    pub primary_game: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGame {
    pub title: String,
    pub executable: String,
    #[serde(default)]
    pub cover_source: Option<String>,
    /// The other three, each optional and each left alone when absent.
    #[serde(default)]
    pub background_source: Option<String>,
    #[serde(default)]
    pub logo_source: Option<String>,
    #[serde(default)]
    pub icon_source: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub conf_path: String,
    pub cover_written: bool,
    pub autorun_written: bool,
    pub icon: Option<String>,
    pub warnings: Vec<String>,
}

/// Read a cartridge so it can be edited.
pub fn read(drive_path: &str) -> Result<Editable, String> {
    let info = cartridge::read_cartridge_info(drive_path)?;
    Ok(from_info(drive_path, info))
}

fn from_info(drive_path: &str, info: CartridgeInfo) -> Editable {
    // A single-game cartridge has no [game] sections, so the one game is
    // assembled from the top-level keys. The editor then treats both shapes the
    // same, which is why it can turn one into the other.
    let games = if info.games.is_empty() {
        vec![EditableGame {
            title: info.title.clone(),
            executable: info.executable.clone(),
            // A single game's art is the cartridge's art, which the editor
            // already shows in its own slots.
            cover: String::new(),
            cover_path: String::new(),
            background: String::new(),
            background_path: String::new(),
            logo: String::new(),
            logo_path: String::new(),
            icon: String::new(),
            icon_path: String::new(),
        }]
    } else {
        info.games
            .iter()
            .map(|game| EditableGame {
                title: game.title.clone(),
                executable: game.executable.clone(),
                cover: game.cover.clone(),
                cover_path: game.cover_path.clone(),
                background: game.background.clone(),
                background_path: game.background_path.clone(),
                logo: game.logo.clone(),
                logo_path: game.logo_path.clone(),
                icon: game.icon.clone(),
                icon_path: game.icon_path.clone(),
            })
            .collect()
    };

    Editable {
        drive_path: drive_path.to_string(),
        title: info.title,
        cover: info.cover,
        cover_path: info.cover_path,
        logo: info.logo,
        logo_path: info.logo_path,
        icon: info.icon,
        icon_path: info.icon_path,
        background: info.background,
        background_path: info.background_path,
        is_bundle: info.is_bundle,
        holds_game: info.holds_game,
        memory_card: info.memory_card,
        platform: info.platform.clone(),
        games,
    }
}

/// Give every game on a cartridge a fresh portrait cover.
///
/// The repair for a cartridge built when the only art to hand was Steam's
/// landscape header: a wide picture in a 2:3 slot reads as a bad crop however
/// it is displayed, and the fix is a different picture, not different CSS.
///
/// One SteamGridDB request per game, so it is an action someone asks for rather
/// than something that happens on open. A game nobody recognises keeps what it
/// has and says so; the rest are replaced.
pub fn refetch_artwork(drive_path: &str) -> Result<UpdateResult, String> {
    let root = create::resolve_target(drive_path)?;
    let current = read(drive_path)?;

    let mut missed = Vec::new();
    let games: Vec<UpdateGame> = current
        .games
        .iter()
        .map(|game| {
            let found = create::fetch_cover_for(&game.title);
            if found.is_none() {
                missed.push(game.title.clone());
            }
            UpdateGame {
                title: game.title.clone(),
                executable: game.executable.clone(),
                cover_source: found.map(|path| path.to_string_lossy().into_owned()),
                // Refetching replaces posters. The other three are the user's
                // choices and are not something to overwrite behind them.
                ..Default::default()
            }
        })
        .collect();

    // The collection's own art is the first game's, which has just been
    // replaced; leaving it out keeps whatever the cartridge already had.
    let mut result = update_at(
        &root,
        &UpdateRequest {
            drive_path: drive_path.to_string(),
            title: current.title,
            // Refetching posters replaces each game's art and nothing else, so
            // the cartridge's own three slots are left exactly as they are.
            cover_source: None,
            logo_source: None,
            icon_source: None,
            background_source: None,
            games,
            primary_game: None,
            remove_logo: false,
            memory_card: None,
            platform: None,
        },
    )?;

    if !missed.is_empty() {
        result.warnings.push(format!(
            "No artwork was found for {}. {} kept what {} had.",
            missed.join(", "),
            if missed.len() == 1 { "It" } else { "They" },
            if missed.len() == 1 { "it" } else { "they" },
        ));
    }
    Ok(result)
}

/// Write the changes back.
pub fn update(request: &UpdateRequest) -> Result<UpdateResult, String> {
    let root = create::resolve_target(&request.drive_path)?;
    update_at(&root, request)
}

/// The work, against a root that has already been checked.
///
/// Split out so it can be tested without a removable drive: `update` is the
/// half that decides *where*, and refuses anywhere it should not write.
pub fn update_at(root: &Path, request: &UpdateRequest) -> Result<UpdateResult, String> {
    let title = create::sanitize_conf_value(&request.title);
    if title.is_empty() {
        return Err("Give the cartridge a title.".into());
    }
    if request.games.is_empty() {
        return Err("A cartridge needs at least one game.".into());
    }

    let mut result = UpdateResult::default();
    let mut warnings = Vec::new();

    // What is on the cartridge now, so superseded art can be tidied up and
    // untouched art kept.
    let existing = cartridge::read_cartridge_info(&root.to_string_lossy()).ok();

    // ---- The games -------------------------------------------------------
    let mut titles = Vec::with_capacity(request.games.len());
    let mut executables = Vec::with_capacity(request.games.len());
    for (index, game) in request.games.iter().enumerate() {
        let game_title = create::sanitize_conf_value(&game.title);
        if game_title.is_empty() {
            return Err(format!("Game {} needs a title.", index + 1));
        }
        let executable = create::sanitize_conf_value(&game.executable);
        create::validate_executable(&executable, root)?;
        titles.push(game_title);
        executables.push(executable);
    }

    // Every game carries the same four pictures the cartridge does, and each
    // is resolved the same way, so they are described once and walked rather
    // than written out four times over.
    type Previous = fn(&cartridge::GameEntry) -> &str;
    type Chosen = fn(&UpdateGame) -> Option<&str>;
    const KINDS: [(&str, Previous, Chosen); 4] = [
        ("cover", |g| &g.cover_path, |u| u.cover_source.as_deref()),
        (
            "background",
            |g| &g.background_path,
            |u| u.background_source.as_deref(),
        ),
        ("logo", |g| &g.logo_path, |u| u.logo_source.as_deref()),
        ("icon", |g| &g.icon_path, |u| u.icon_source.as_deref()),
    ];

    let mut per_game: Vec<Vec<Option<String>>> = Vec::with_capacity(KINDS.len());

    for (stem, previous, chosen) in KINDS {
        // A game keeps the art it already had unless a new picture was chosen
        // for it — and the art belongs to the *game*, not to the slot it sits
        // in. The list arrives in its new order, so looking art up by position
        // handed each game whatever used to be in its seat: one press of the
        // reorder button and every poster on the cartridge moved along by one.
        let mut art: Vec<Option<String>> = executables
            .iter()
            .map(|executable| {
                let found =
                    existing.as_ref()?.games.iter().find(|game| {
                        create::sanitize_conf_value(&game.executable) == *executable
                    })?;
                let path = previous(found);
                (!path.is_empty())
                    .then(|| file_name_relative(root, path))
                    .flatten()
            })
            .collect();

        for (index, game) in request.games.iter().enumerate() {
            let Some(source) = chosen(game)
                .map(str::trim)
                .filter(|source| !source.is_empty())
            else {
                continue;
            };
            // `<kind>_<index>` is the natural name and nearly always free, but
            // after a reorder that file can still belong to a game that kept
            // its art — and writing there would quietly replace that game's
            // picture with this one's.
            let name = free_art_stem(stem, index, &art);
            match create::copy_cover(Path::new(source), root, &name) {
                Ok(written) => art[index] = Some(written),
                Err(e) => warnings.push(format!(
                    "The {stem} for {} was not changed: {e}",
                    titles[index]
                )),
            }
        }

        per_game.push(art);
    }

    let entries: Vec<create::GameArt> = titles
        .into_iter()
        .zip(executables)
        .enumerate()
        .map(|(index, (title, executable))| create::GameArt {
            title,
            executable,
            cover: per_game[0][index].clone(),
            background: per_game[1][index].clone(),
            logo: per_game[2][index].clone(),
            icon: per_game[3][index].clone(),
        })
        .collect();

    // ---- The collection's own art ---------------------------------------
    let previous_cover = existing
        .as_ref()
        .map(|info| info.cover_path.clone())
        .filter(|path| !path.is_empty());

    let stem = if entries.len() > 1 {
        "collection"
    } else {
        "cover"
    };
    let cover_name = match request.cover_source.as_deref().map(str::trim) {
        Some(source) if !source.is_empty() => {
            match create::copy_cover(Path::new(source), root, stem) {
                Ok(name) => {
                    result.cover_written = true;
                    Some(name)
                }
                Err(e) => {
                    warnings.push(format!("Cover art was not changed: {e}"));
                    previous_cover
                        .as_deref()
                        .and_then(|p| file_name_relative(root, p))
                }
            }
        }
        _ => previous_cover
            .as_deref()
            .and_then(|path| file_name_relative(root, path)),
    };

    // ---- The logo ---------------------------------------------------------
    //
    // The hero used to be carried forward and never edited, because the
    // launcher had stopped reading heroes: it wants a window three times as
    // wide as a cover, and there was only one window. Skins can size their own
    // now, so a wide one has somewhere to go and is worth being able to change.
    let existing_background = existing
        .as_ref()
        .map(|info| info.background_path.clone())
        .filter(|path| !path.is_empty())
        .and_then(|path| file_name_relative(root, &path));
    let background_name = match request.background_source.as_deref().map(str::trim) {
        Some(source) if !source.is_empty() => {
            match create::copy_cover(Path::new(source), root, "background") {
                Ok(name) => Some(name),
                Err(e) => {
                    warnings.push(format!("The hero was not changed: {e}"));
                    existing_background.clone()
                }
            }
        }
        _ => existing_background.clone(),
    };
    let existing_logo = existing
        .as_ref()
        .map(|info| info.logo_path.clone())
        .filter(|path| !path.is_empty())
        .and_then(|path| file_name_relative(root, &path));

    let logo_name = match request.logo_source.as_deref().map(str::trim) {
        Some(source) if !source.is_empty() => {
            match create::copy_cover(Path::new(source), root, "logo") {
                Ok(name) => Some(name),
                Err(e) => {
                    warnings.push(format!("The logo was not changed: {e}"));
                    existing_logo.clone()
                }
            }
        }
        _ if request.remove_logo => {
            if let Some(old) = existing_logo
                .as_deref()
                .filter(|name| is_ours_to_delete(name))
            {
                let _ = std::fs::remove_file(root.join(old));
            }
            None
        }
        _ => existing_logo.clone(),
    };

    // ---- cartridge.conf --------------------------------------------------

    let conf = if entries.len() > 1 {
        let games: Vec<create::ConfGame<'_>> =
            entries.iter().map(create::GameArt::as_conf).collect();
        create::render_bundle_conf(
            &title,
            cover_name.as_deref(),
            None,
            None,
            logo_name.as_deref(),
            &games,
        )
    } else {
        create::render_cartridge_conf(
            &title,
            &entries[0].executable,
            cover_name.as_deref(),
            None,
            background_name.as_deref(),
            logo_name.as_deref(),
        )
    };

    // The conf is rendered from scratch, which is how anything left from an
    // earlier write goes away. Save declarations are the exception: nothing in
    // this editor can write one, so a rewrite would drop the lines somebody
    // typed in by hand and orphan the saves already on the drive.
    let conf_path = root.join("cartridge.conf");
    let previous = std::fs::read_to_string(&conf_path).ok();
    let conf = match &previous {
        Some(previous) => crate::saves::preserve(previous, &conf),
        None => conf,
    };
    // Combo drive: as asked, else as it was. The render above starts from
    // nothing, so without this a rename quietly turned a combo back into a
    // plain cartridge.
    let combo = request
        .memory_card
        .unwrap_or_else(|| previous.as_deref().is_some_and(crate::memcard::is_combo));
    let turned_on = combo && !previous.as_deref().is_some_and(crate::memcard::is_combo);
    let conf = if turned_on {
        // Newly a combo: its games go onto its card, the way Create does it.
        let games: Vec<(&str, &str, Option<&str>)> = entries
            .iter()
            .map(|game| (game.title.as_str(), game.executable.as_str(), None))
            .collect();
        crate::create::with_memory_card(conf, true, root, &games, &mut |_| {}, &mut warnings)
    } else if combo {
        format!("memory_card=yes\n{conf}")
    } else {
        conf
    };
    // The platform as asked; absent, the line `preserve` carried over stays.
    let conf = match &request.platform {
        Some(platform) => crate::cartridge::set_platform(&conf, platform),
        None => conf,
    };
    std::fs::write(&conf_path, conf)
        .map_err(|e| format!("Could not write {}: {e}", conf_path.display()))?;
    result.conf_path = conf_path.to_string_lossy().into_owned();
    // And the play history, which the stats file holds and the render above
    // knows nothing about.
    if let Err(e) = crate::playlog::mirror(root) {
        warnings.push(format!(
            "Play history was not copied into cartridge.conf: {e}"
        ));
    }
    // A platform changed here changes which emulator front-ends list it.
    crate::emulated::sync_files(root, &mut warnings);

    // ---- autorun.inf ------------------------------------------------------
    //
    // The icon slot is what Explorer shows. Without one the cover stands in,
    // which is what every cartridge did before the slot existed.
    let icon_source = match request.icon_source.as_deref().map(str::trim) {
        Some(source) if !source.is_empty() => {
            match create::copy_cover(Path::new(source), root, "icon") {
                Ok(name) => Some(root.join(name)),
                Err(e) => {
                    warnings.push(format!("The icon was not changed: {e}"));
                    None
                }
            }
        }
        _ => None,
    };
    let cover_full = icon_source.or_else(|| {
        create::drive_icon_art(None, &entries, request.primary_game, cover_name.as_deref())
            .map(|art| root.join(art))
    });
    match autorun::write_autorun(root, &title, cover_full.as_deref()) {
        Ok(icon) => {
            result.autorun_written = true;
            result.icon = icon;
        }
        Err(e) => warnings.push(format!("autorun.inf was not rewritten: {e}")),
    }

    // ---- Tidy up the picture that was replaced ---------------------------
    if let (Some(previous), Some(now)) = (previous_cover.as_deref(), cover_name.as_deref()) {
        if let Some(old_name) = file_name_relative(root, previous) {
            if old_name != now && is_ours_to_delete(&old_name) {
                let _ = std::fs::remove_file(root.join(&old_name));
            }
        }
    }

    result.warnings = warnings;
    Ok(result)
}

/// A `cover_<n>` stem that no other game's kept art is already using.
///
/// The slot's own number is the right answer almost every time; it is not when
/// a reorder has left a different game still holding that file.
fn free_art_stem(kind: &str, index: usize, art: &[Option<String>]) -> String {
    let taken: Vec<&str> = art
        .iter()
        .enumerate()
        .filter(|(other, _)| *other != index)
        .filter_map(|(_, name)| name.as_deref())
        .map(stem_of)
        .collect();

    let preferred = format!("{kind}_{index}");
    if !taken.contains(&preferred.as_str()) {
        return preferred;
    }
    // Finite `taken`, so some number is always free.
    (0..)
        .map(|n| format!("{kind}_{n}"))
        .find(|candidate| !taken.contains(&candidate.as_str()))
        .expect("a free art stem")
}

/// The part of a file name before its extension.
fn stem_of(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// An absolute path under the cartridge, back to the name the conf would use.
fn file_name_relative(root: &Path, absolute: &str) -> Option<String> {
    crate::verify::relative_path(root, Path::new(absolute))
}

/// Only art this tool wrote is ever removed.
///
/// A cartridge is somebody's drive; a file named anything else is theirs, even
/// if `cover=` happened to point at it.
fn is_ours_to_delete(name: &str) -> bool {
    let stem = name.rsplit('/').next().unwrap_or(name);
    stem.starts_with("cover") || stem.starts_with("collection") || stem.starts_with("logo")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    fn single_game(scratch: &Scratch) {
        scratch.write(
            "cartridge.conf",
            b"title=Hollow Knight\nexecutable=steam://rungameid/367520\ncover=cover.jpg\n",
        );
        scratch.write("cover.jpg", b"pretend art");
    }

    fn request(title: &str, games: &[(&str, &str)]) -> UpdateRequest {
        UpdateRequest {
            drive_path: String::new(),
            title: title.to_string(),
            cover_source: None,
            logo_source: None,
            icon_source: None,
            background_source: None,
            games: games
                .iter()
                .map(|(t, e)| UpdateGame {
                    title: t.to_string(),
                    executable: e.to_string(),
                    ..Default::default()
                })
                .collect(),
            primary_game: None,
            remove_logo: false,
            memory_card: None,
            platform: None,
        }
    }

    #[test]
    fn a_single_game_cartridge_reads_as_one_game() {
        let scratch = Scratch::new("edit-read");
        single_game(&scratch);

        let editable = read(scratch.path().to_str().unwrap()).unwrap();
        assert_eq!(editable.title, "Hollow Knight");
        assert!(!editable.is_bundle);
        // Even though the file has no [game] section, the editor sees one game,
        // so both shapes are edited the same way.
        assert_eq!(editable.games.len(), 1);
        assert_eq!(editable.games[0].executable, "steam://rungameid/367520");
    }

    #[test]
    fn renaming_rewrites_the_conf_and_keeps_the_art() {
        let scratch = Scratch::new("edit-rename");
        single_game(&scratch);

        let result = update_at(
            scratch.path(),
            &request(
                "Hollow Knight: Silksong",
                &[("Hollow Knight: Silksong", "steam://rungameid/367520")],
            ),
        )
        .unwrap();

        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(conf.contains("title=Hollow Knight: Silksong"), "{conf}");
        // The picture was not touched, and the conf still points at it.
        assert!(conf.contains("cover=cover.jpg"), "{conf}");
        assert!(scratch.join("cover.jpg").is_file());
    }

    #[test]
    fn a_combo_drive_stays_one_until_asked() {
        let scratch = Scratch::new("edit-combo");
        scratch.write(
            "cartridge.conf",
            b"memory_card=yes\ntitle=Stardew\nexecutable=steam://rungameid/413150\n",
        );
        let games = [("Stardew", "steam://rungameid/413150")];

        update_at(scratch.path(), &request("Stardew Valley", &games)).unwrap();
        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(
            crate::memcard::is_combo(&conf),
            "a rename dropped it: {conf}"
        );
        assert!(read(&scratch.path().to_string_lossy()).unwrap().memory_card);

        let mut req = request("Stardew Valley", &games);
        req.memory_card = Some(false);
        update_at(scratch.path(), &req).unwrap();
        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(!crate::memcard::is_combo(&conf), "{conf}");
    }

    #[test]
    fn a_collection_logo_can_be_taken_off() {
        let scratch = Scratch::new("edit-remove-logo");
        scratch.write(".gamepak/logo.png", b"bluey");
        scratch.write(
            "cartridge.conf",
            b"[collection]\ntitle=Mixed\nlogo=.gamepak/logo.png\n\n\
              [game]\ntitle=Bluey\nexecutable=steam://rungameid/1\n\n[game]\ntitle=Blanc\nexecutable=steam://rungameid/2\n",
        );
        let games = [
            ("Bluey", "steam://rungameid/1"),
            ("Blanc", "steam://rungameid/2"),
        ];

        // Absent keeps it, as it always did.
        update_at(scratch.path(), &request("Mixed", &games)).unwrap();
        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(conf.contains("logo=.gamepak/logo.png"), "{conf}");

        let mut req = request("Mixed", &games);
        req.remove_logo = true;
        update_at(scratch.path(), &req).unwrap();
        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(!conf.contains("logo="), "{conf}");
        assert!(!scratch.join(".gamepak/logo.png").exists());
    }

    #[test]
    fn new_art_replaces_the_old_file() {
        let scratch = Scratch::new("edit-art");
        single_game(&scratch);
        scratch.write("chosen.png", b"a different picture");

        let mut req = request(
            "Hollow Knight",
            &[("Hollow Knight", "steam://rungameid/367520")],
        );
        req.cover_source = Some(scratch.join("chosen.png").to_string_lossy().into_owned());
        let result = update_at(scratch.path(), &req).unwrap();

        assert!(result.cover_written);
        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(conf.contains("cover=.gamepak/cover.png"), "{conf}");
        assert!(scratch.join(".gamepak").join("cover.png").is_file());
        // The superseded picture is cleared away rather than left behind, even
        // though it sat at the root and its replacement does not.
        assert!(!scratch.join("cover.jpg").exists());
    }

    #[test]
    fn a_picture_this_tool_did_not_write_is_left_alone() {
        let scratch = Scratch::new("edit-art-theirs");
        scratch.write(
            "cartridge.conf",
            b"title=Tunic\nexecutable=steam://rungameid/553420\ncover=my-own-photo.jpg\n",
        );
        scratch.write("my-own-photo.jpg", b"someone's own file");
        scratch.write("chosen.png", b"a different picture");

        let mut req = request("Tunic", &[("Tunic", "steam://rungameid/553420")]);
        req.cover_source = Some(scratch.join("chosen.png").to_string_lossy().into_owned());
        update_at(scratch.path(), &req).unwrap();

        assert!(scratch.join(".gamepak").join("cover.png").is_file());
        assert!(
            scratch.join("my-own-photo.jpg").is_file(),
            "a file we did not write must survive being replaced in the conf"
        );
    }

    #[test]
    fn a_single_game_can_become_a_collection() {
        let scratch = Scratch::new("edit-to-bundle");
        single_game(&scratch);

        update_at(
            scratch.path(),
            &request(
                "Hollow Knight Collection",
                &[
                    ("Hollow Knight", "steam://rungameid/367520"),
                    ("Silksong", "steam://rungameid/1030300"),
                ],
            ),
        )
        .unwrap();

        let info = cartridge::read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert!(info.is_bundle);
        assert_eq!(info.games.len(), 2);
        assert_eq!(info.title, "Hollow Knight Collection");
        // And Enter still starts something.
        assert_eq!(info.executable, "steam://rungameid/367520");
    }

    #[test]
    fn dropping_a_game_leaves_its_files_where_they_are() {
        let scratch = Scratch::new("edit-drop");
        scratch.write(
            "cartridge.conf",
            b"[collection]\ntitle=Two Games\n\n[game]\ntitle=One\nexecutable=steam://rungameid/1\n\n[game]\ntitle=Two\nexecutable=steam://rungameid/2\n",
        );
        scratch.write("Games/Two/Two.exe", b"a game nobody asked to delete");

        update_at(
            scratch.path(),
            &request("One", &[("One", "steam://rungameid/1")]),
        )
        .unwrap();

        let info = cartridge::read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert!(
            !info.is_bundle,
            "one game left means it is no longer a bundle"
        );
        assert!(
            scratch.join("Games/Two/Two.exe").is_file(),
            "removing a game from the list must never delete it"
        );
    }

    #[test]
    fn the_order_of_the_games_is_the_order_given() {
        let scratch = Scratch::new("edit-order");
        scratch.write(
            "cartridge.conf",
            b"[collection]\ntitle=Series\n\n[game]\ntitle=First\nexecutable=steam://rungameid/1\n\n[game]\ntitle=Second\nexecutable=steam://rungameid/2\n",
        );

        update_at(
            scratch.path(),
            &request(
                "Series",
                &[
                    ("Second", "steam://rungameid/2"),
                    ("First", "steam://rungameid/1"),
                ],
            ),
        )
        .unwrap();

        let info = cartridge::read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert_eq!(info.games[0].title, "Second");
        assert_eq!(info.games[1].title, "First");
    }

    #[test]
    fn reordering_moves_each_game_with_its_own_poster() {
        let scratch = Scratch::new("edit-order-art");
        scratch.write(
            "cartridge.conf",
            b"[collection]\ntitle=Series\n\n[game]\ntitle=First\nexecutable=steam://rungameid/1\ncover=cover_0.jpg\n\n[game]\ntitle=Second\nexecutable=steam://rungameid/2\ncover=cover_1.jpg\n",
        );
        scratch.write("cover_0.jpg", b"first");
        scratch.write("cover_1.jpg", b"second");

        update_at(
            scratch.path(),
            &request(
                "Series",
                &[
                    ("Second", "steam://rungameid/2"),
                    ("First", "steam://rungameid/1"),
                ],
            ),
        )
        .unwrap();

        // Looked up by position, Second would come back holding cover_0.jpg —
        // the poster that belongs to First.
        let info = cartridge::read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert!(
            info.games[0].cover_path.ends_with("cover_1.jpg"),
            "Second kept its own art, got {}",
            info.games[0].cover_path
        );
        assert!(
            info.games[1].cover_path.ends_with("cover_0.jpg"),
            "First kept its own art, got {}",
            info.games[1].cover_path
        );
    }

    #[test]
    fn new_art_never_lands_on_a_file_another_game_still_uses() {
        let scratch = Scratch::new("edit-order-collide");
        scratch.write(
            "cartridge.conf",
            b"[collection]\ntitle=Series\n\n[game]\ntitle=First\nexecutable=steam://rungameid/1\ncover=cover_0.jpg\n\n[game]\ntitle=Second\nexecutable=steam://rungameid/2\ncover=cover_1.jpg\n",
        );
        scratch.write("cover_0.jpg", b"first");
        scratch.write("cover_1.jpg", b"second");
        let fresh = scratch.write("new.jpg", b"brand new art");

        // Second moves to slot 0 and gets new art. The obvious name for slot 0
        // is cover_0.jpg, which First is still using.
        let mut req = request(
            "Series",
            &[
                ("Second", "steam://rungameid/2"),
                ("First", "steam://rungameid/1"),
            ],
        );
        req.games[0].cover_source = Some(fresh.to_string_lossy().into_owned());

        update_at(scratch.path(), &req).unwrap();

        assert_eq!(
            std::fs::read(scratch.join("cover_0.jpg")).unwrap(),
            b"first",
            "First's poster was overwritten by Second's new art"
        );
        let info = cartridge::read_cartridge_info(scratch.path().to_str().unwrap()).unwrap();
        assert!(info.games[1].cover_path.ends_with("cover_0.jpg"));
    }

    #[test]
    fn an_empty_title_or_an_empty_cartridge_is_refused() {
        let scratch = Scratch::new("edit-refuse");
        single_game(&scratch);

        assert!(update_at(
            scratch.path(),
            &request("   ", &[("X", "steam://rungameid/1")])
        )
        .unwrap_err()
        .contains("title"));
        assert!(update_at(scratch.path(), &request("Fine", &[]))
            .unwrap_err()
            .contains("at least one game"));
        assert!(update_at(
            scratch.path(),
            &request("Fine", &[("  ", "steam://rungameid/1")])
        )
        .unwrap_err()
        .contains("needs a title"));

        // None of that touched the cartridge.
        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(conf.contains("title=Hollow Knight"), "{conf}");
    }

    #[test]
    fn a_rename_keeps_the_estimate_and_the_play_history() {
        let scratch = Scratch::new("edit-keeps-play");
        scratch.write(
            "cartridge.conf",
            b"title=Hollow Knight\nexecutable=steam://rungameid/367520\nhltb_id=9\nhltb_main=97200\n",
        );
        let session =
            crate::stats::record_launch(scratch.path(), "steam://rungameid/367520", "HK").unwrap();
        crate::stats::record_session_played(&session, 1800).unwrap();

        update_at(
            scratch.path(),
            &request("HK", &[("HK", "steam://rungameid/367520")]),
        )
        .unwrap();

        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(conf.contains("title=HK"), "{conf}");
        assert!(conf.contains("hltb_main=97200"), "{conf}");
        assert!(conf.contains("playtime=1800"), "{conf}");
        assert!(conf.contains("launches=1"), "{conf}");
    }

    #[test]
    fn a_launch_target_that_is_not_on_the_cartridge_is_refused() {
        let scratch = Scratch::new("edit-bad-exe");
        single_game(&scratch);

        // The same confinement the wizard applies: a path must stay on the
        // cartridge, and must exist.
        let err = update_at(
            scratch.path(),
            &request("Hollow Knight", &[("Hollow Knight", "../../etc/passwd")]),
        )
        .unwrap_err();
        assert!(!err.is_empty());

        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).unwrap();
        assert!(conf.contains("steam://rungameid/367520"), "{conf}");
    }

    #[test]
    fn renaming_a_cartridge_does_not_orphan_its_saves() {
        // The editor renders cartridge.conf from scratch, and nothing in it
        // can write a save line — so without care, renaming a cartridge would
        // drop the declaration and leave the saves on the drive attached to
        // nothing.
        let scratch = Scratch::new("edit-keeps-saves");
        scratch.write(
            "cartridge.conf",
            b"title=Old name\nexecutable=steam://rungameid/1\nsave=Saves|{appdata}/Foo/Saves\n",
        );

        update_at(
            scratch.path(),
            &request("New name", &[("New name", "steam://rungameid/1")]),
        )
        .expect("update");

        let conf = std::fs::read_to_string(scratch.join("cartridge.conf")).expect("read back");
        assert!(conf.contains("title=New name"), "{conf}");
        assert!(conf.contains("save=Saves|{appdata}/Foo/Saves"), "{conf}");
        // And the declaration still parses against the renamed cartridge.
        assert_eq!(crate::saves::declared(scratch.path()).len(), 1);
    }
}
