//! Memory cards: a drive that carries saves and hours, and no games.
//!
//! A cartridge carries a game and, if asked, its saves. A memory card is the
//! other half on its own, the way a PlayStation's was: plug it into the PC the
//! game is installed on and your save comes with you. It is a `memorycard.conf`
//! at the root of the drive, declaring saves in exactly the `save=` lines a
//! cartridge uses, so [`crate::saves`] syncs it without knowing the difference.
//!
//! ```text
//! title=Harry's Memory Card
//!
//! [game]
//! title=Stardew Valley
//! executable=steam://rungameid/413150
//! icon=.gamepak/card_0.png
//! save.windows=Stardew Valley|{appdata}/StardewValley/Saves
//! save.linux=Stardew Valley|{appdata}/StardewValley/Saves
//! ```
//!
//! A cartridge can be both — a *combo drive* — with `memory_card=yes` in its
//! own conf, or a `memorycard.conf` beside it carrying saves for other games.
//! The launcher then offers the same memory card view, behind a button, rather
//! than in place of the cartridge.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::saves::{self, SlotStatus};

/// The file that makes a drive a memory card.
pub const CONF: &str = "memorycard.conf";

/// A drive that is a memory card and nothing else. A drive with both files is
/// a cartridge; its conf decides whether it is a combo.
pub fn is_memory_card(root: &Path) -> bool {
    root.join(CONF).is_file() && !root.join("cartridge.conf").is_file()
}

/// A cartridge that is a memory card too: asked for in its conf, or carrying
/// a card's file beside it.
pub fn is_combo_drive(root: &Path, conf: &str) -> bool {
    is_combo(conf) || root.join(CONF).is_file()
}

/// Whether a cartridge has asked to be shown as a memory card too.
pub fn is_combo(conf: &str) -> bool {
    conf.lines()
        .filter_map(|line| line.split_once('='))
        .any(|(key, value)| {
            key.trim().eq_ignore_ascii_case("memory_card")
                && matches!(value.trim().to_lowercase().as_str(), "yes" | "true" | "1")
        })
}

// --------------------------------------------------------------------------
// Reading
// --------------------------------------------------------------------------

/// One save, as a block on the card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    #[serde(flatten)]
    pub status: SlotStatus,
    /// The game's name, or the slot's label when no game claims it.
    pub title: String,
    /// The game's icon, else its cover, as a `data:` URI; empty for none.
    pub icon: String,
    /// Hours and launches the drive has counted for this game.
    pub seconds: u64,
    pub launches: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardView {
    pub title: String,
    pub blocks: Vec<Block>,
    /// True for a drive that is a memory card and nothing else.
    pub card_only: bool,
}

/// Everything the launcher's memory card view shows.
pub fn view(root: &Path) -> CardView {
    let card_only = is_memory_card(root);
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap_or_default();
    let card = read(CONF);
    let cartridge = if card_only {
        String::new()
    } else {
        read("cartridge.conf")
    };
    // The card's own name when it has one, else the cartridge's.
    let named = if card.is_empty() { &cartridge } else { &card };
    let ini = crate::cartridge::parse_ini(named);
    let title = crate::cartridge::ini_get(&ini, "collection", "title")
        .or_else(|| crate::cartridge::ini_get(&ini, "general", "title"))
        .cloned()
        .unwrap_or_else(|| "Memory card".to_string());

    // The games that may own a save, keyed the way a slot names its game: the
    // cartridge's (a single game is its top-level keys) and the card's.
    let mut games = crate::cartridge::parse_game_sections(&cartridge);
    if games.is_empty() && !cartridge.is_empty() {
        let own = crate::cartridge::parse_ini(&cartridge);
        games.push(own.get("general").cloned().unwrap_or_default());
    }
    games.extend(crate::cartridge::parse_game_sections(&card));
    // First wins: a combo's own game, with its art, over the card's entry for it.
    let mut by_key: HashMap<String, &HashMap<String, String>> = HashMap::new();
    for game in &games {
        if let Some(exe) = game.get("executable") {
            by_key.entry(crate::stats::key_for(exe)).or_insert(game);
        }
    }
    let stats = crate::stats::read(root);

    let blocks = saves::status(root)
        .into_iter()
        .map(|status| {
            let game = status
                .slot
                .game
                .as_ref()
                .and_then(|key| by_key.get(key).copied())
                .or_else(|| {
                    games.iter().find(|game| {
                        game.get("title")
                            .is_some_and(|t| t.eq_ignore_ascii_case(&status.slot.label))
                    })
                });
            let art = |key: &str| {
                game.and_then(|game| game.get(key))
                    .and_then(|rel| crate::cartridge::join_within(root, rel))
                    .filter(|path| path.is_file())
                    .map(|path| crate::cartridge::cover_as_data_uri(&path.to_string_lossy()))
                    .unwrap_or_default()
            };
            let icon = Some(art("icon"))
                .filter(|uri| !uri.is_empty())
                .unwrap_or_else(|| art("cover"));
            let played = status
                .slot
                .game
                .as_ref()
                .and_then(|key| stats.games.get(key));
            Block {
                title: game
                    .and_then(|game| game.get("title").cloned())
                    .unwrap_or_else(|| status.slot.label.clone()),
                icon,
                seconds: played.map_or(0, |p| p.seconds),
                launches: played.map_or(0, |p| p.launches),
                status,
            }
        })
        .collect();

    CardView {
        title,
        blocks,
        card_only,
    }
}

// --------------------------------------------------------------------------
// Moving one save
// --------------------------------------------------------------------------

/// Copy one save onto the PC (`to` is `"pc"`) or onto the card (`"card"`).
///
/// The same backed-up copy the automatic sync does, pointed by hand: whatever
/// is replaced is moved aside and kept. Refused when the source is empty, so
/// a slot nobody has played cannot wipe the other side.
pub fn copy(root: &Path, slot_id: &str, to: &str) -> Result<saves::SyncOutcome, String> {
    let mut status = find(root, slot_id)?;
    let (direction, source_bytes) = match to {
        "pc" => (saves::Direction::Pull, status.cartridge_bytes),
        "card" => (saves::Direction::Push, status.host_bytes),
        other => return Err(format!("copy to pc or card, not {other}")),
    };
    if status.host_path.is_empty() {
        return Err(format!(
            "{} has no save folder on this PC.",
            status.slot.label
        ));
    }
    if source_bytes == 0 {
        return Err(format!(
            "There is no {} save to copy.",
            if to == "pc" { "card" } else { "PC" }
        ));
    }
    status.direction = direction;
    saves::sync_slot(root, &status)
}

/// Take one save off the card. It is moved aside, not deleted, so a mistake
/// can be undone by hand; the copy on the PC is not touched.
pub fn remove(root: &Path, slot_id: &str) -> Result<(), String> {
    let status = find(root, slot_id)?;
    saves::back_up(&saves::slot_path(root, &status.slot))?;
    Ok(())
}

/// The folder one save lives in, for showing in a file manager: this PC's
/// copy when there is one, since that is the one the game reads, else the
/// card's.
pub fn folder(root: &Path, slot_id: &str) -> Result<std::path::PathBuf, String> {
    let status = find(root, slot_id)?;
    let host = Path::new(&status.host_path);
    if !status.host_path.is_empty() && host.is_dir() {
        return Ok(host.to_path_buf());
    }
    let card = saves::slot_path(root, &status.slot);
    if card.is_dir() {
        return Ok(card);
    }
    Err(format!(
        "{} has no save on this PC or the card yet.",
        status.slot.label
    ))
}

fn find(root: &Path, slot_id: &str) -> Result<SlotStatus, String> {
    saves::status(root)
        .into_iter()
        .find(|status| status.slot.id == slot_id)
        .ok_or_else(|| format!("No save called {slot_id} on this drive."))
}

// --------------------------------------------------------------------------
// Writing a card
// --------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CardRequest {
    pub drive_path: String,
    pub title: String,
    pub games: Vec<CardGame>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardGame {
    pub title: String,
    /// Optional; with it the card can show the hours a cartridge counted.
    #[serde(default)]
    pub executable: String,
    /// Where the library keeps this game, so its art can be the block's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playnite_id: Option<String>,
    /// A picture for the block. A path already on the card is kept as it is.
    #[serde(default)]
    pub icon_source: Option<String>,
    /// A folder picked by hand, as a portable template. Works everywhere.
    #[serde(default)]
    pub save: Option<String>,
    /// Per-platform templates, from a lookup. Each overrides `save` there.
    #[serde(default)]
    pub save_windows: Option<String>,
    #[serde(default)]
    pub save_linux: Option<String>,
}

/// One of a card's games, as the wizard shows it for editing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardGameView {
    #[serde(flatten)]
    pub game: CardGame,
    /// The block's picture as a `data:` URI, for the preview.
    pub icon: String,
}

/// A card's games, the way [`write`] was given them, so adding one more does
/// not mean entering the others again. Empty for a drive that is not a card.
pub fn games(root: &Path) -> Vec<CardGameView> {
    let Ok(text) = std::fs::read_to_string(root.join(CONF)) else {
        return Vec::new();
    };
    crate::cartridge::parse_game_sections(&text)
        .into_iter()
        .map(|section| {
            let get = |key: &str| section.get(key).cloned().filter(|v| !v.is_empty());
            let template = |key: &str| get(key).map(|v| split_template(&v));
            let icon_path = get("icon")
                .and_then(|rel| crate::cartridge::join_within(root, &rel))
                .filter(|path| path.is_file());
            CardGameView {
                icon: icon_path
                    .as_ref()
                    .map(|path| crate::cartridge::cover_as_data_uri(&path.to_string_lossy()))
                    .unwrap_or_default(),
                game: CardGame {
                    title: get("title").unwrap_or_default(),
                    executable: get("executable").unwrap_or_default(),
                    icon_source: icon_path.map(|path| path.to_string_lossy().into_owned()),
                    save: template("save"),
                    save_windows: template("save.windows"),
                    save_linux: template("save.linux"),
                    app_id: None,
                    playnite_id: None,
                },
            }
        })
        .collect()
}

/// `Label|template` to the template.
fn split_template(value: &str) -> String {
    value
        .split_once('|')
        .map_or(value, |(_, template)| template)
        .trim()
        .to_string()
}

/// Write `memorycard.conf`, and the block pictures, onto a drive.
///
/// Only ever onto a drive the wizard lists, re-checked here rather than
/// trusted from the window.
pub fn write(request: &CardRequest) -> Result<Vec<String>, String> {
    let root = crate::create::resolve_target(&request.drive_path)?;
    write_at(&root, request)
}

/// Add games to the card on a drive, making one if there is none; a game it
/// already has is replaced. How a combo cartridge puts its own games on its
/// card.
pub fn add_games(root: &Path, new: Vec<CardGame>) -> Result<Vec<String>, String> {
    let title = std::fs::read_to_string(root.join(CONF))
        .ok()
        .and_then(|text| {
            let ini = crate::cartridge::parse_ini(&text);
            crate::cartridge::ini_get(&ini, "general", "title").cloned()
        })
        .unwrap_or_else(|| "Memory card".to_string());
    let mut games: Vec<CardGame> = games(root).into_iter().map(|view| view.game).collect();
    games.extend(new);
    write_at(
        root,
        &CardRequest {
            drive_path: String::new(),
            title,
            games,
        },
    )
}

/// Beside a cartridge's `cartridge.conf` this makes a combo drive: the
/// cartridge itself is not touched, and the launcher shows it as both.
fn write_at(root: &Path, request: &CardRequest) -> Result<Vec<String>, String> {
    let title = crate::create::sanitize_conf_value(&request.title);
    if title.is_empty() {
        return Err("A memory card needs a name.".into());
    }

    // A game named twice is one game: the later entry is the newer answer, and
    // it takes the earlier one's place rather than adding a second block.
    let mut unique: Vec<&CardGame> = Vec::new();
    for game in &request.games {
        let key = game.title.trim().to_lowercase();
        match unique
            .iter()
            .position(|g| g.title.trim().to_lowercase() == key)
        {
            Some(at) => unique[at] = game,
            None => unique.push(game),
        }
    }

    let mut warnings = Vec::new();
    let mut out =
        format!("; A PC GamePak memory card: saves and hours, no games.\ntitle={title}\n");
    for (index, game) in unique.into_iter().enumerate() {
        let name = crate::create::sanitize_conf_value(&game.title);
        if name.is_empty() {
            continue;
        }
        let templates = [
            ("save", &game.save),
            ("save.windows", &game.save_windows),
            ("save.linux", &game.save_linux),
        ];
        if templates
            .iter()
            .all(|(_, t)| t.as_deref().is_none_or(str::is_empty))
        {
            warnings.push(format!("{name} has no save folder, so it was left off."));
            continue;
        }
        out.push_str(&format!("\n[game]\ntitle={name}\n"));
        let exe = crate::create::sanitize_conf_value(&game.executable);
        if !exe.is_empty() {
            out.push_str(&format!("executable={exe}\n"));
        }
        if let Some(icon) = icon_for(root, game, index, &mut warnings) {
            out.push_str(&format!("icon={icon}\n"));
        }
        let label = name.replace('|', " ");
        for (key, template) in templates {
            if let Some(template) = template.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                let template = crate::create::sanitize_conf_value(template);
                out.push_str(&format!("{key}={label}|{template}\n"));
            }
        }
    }

    let path = root.join(CONF);
    std::fs::write(&path, out).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(warnings)
}

fn icon_for(
    root: &Path,
    game: &CardGame,
    index: usize,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let chosen = game
        .icon_source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // Already on the card: keep the name it has rather than copying it over
    // itself.
    if let Some(rel) = chosen.and_then(|path| Path::new(path).strip_prefix(root).ok()) {
        return Some(rel.to_string_lossy().replace('\\', "/"));
    }
    // Nothing chosen: the library's own cover, found the way Create finds it.
    let source = crate::create::cover_source(
        chosen,
        game.app_id.as_deref(),
        game.playnite_id.as_deref(),
        None,
        &game.title,
    )
    .ok()
    .flatten()?;
    match crate::create::copy_cover(&source, root, &format!("card_{index}")) {
        Ok(name) => Some(name),
        Err(e) => {
            warnings.push(format!("{}'s picture was not copied: {e}", game.title));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    #[test]
    fn a_card_is_a_card_only_without_a_cartridge() {
        let scratch = Scratch::new("memcard-kind");
        assert!(!is_memory_card(scratch.path()));
        scratch.write(CONF, b"title=Card\n");
        assert!(is_memory_card(scratch.path()));
        scratch.write("cartridge.conf", b"title=Game\n");
        assert!(!is_memory_card(scratch.path()));
    }

    #[test]
    fn a_cartridge_asks_to_be_a_combo() {
        assert!(is_combo("title=X\nmemory_card=yes\n"));
        assert!(is_combo("[collection]\nMemory_Card = TRUE\n"));
        assert!(!is_combo("title=X\nmemory_card=no\n"));
        assert!(!is_combo("title=X\n"));
    }

    #[test]
    fn the_view_names_each_block_after_its_game() {
        let scratch = Scratch::new("memcard-view");
        scratch.write(".gamepak/card_0.png", b"not really a png");
        scratch.write(
            CONF,
            b"title=Harry's Card\n\n[game]\ntitle=Stardew Valley\n\
              executable=steam://rungameid/413150\nicon=.gamepak/card_0.png\n\
              save=Stardew Valley|{appdata}/StardewValley/Saves\n\n\
              [game]\ntitle=Bluey\nsave=Bluey|{localappdata}/Bluey\n",
        );
        let view = view(scratch.path());
        assert!(view.card_only);
        assert_eq!(view.title, "Harry's Card");
        let titles: Vec<&str> = view.blocks.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, ["Stardew Valley", "Bluey"]);
        // An icon that is on the card is carried; one that is not stays empty.
        assert!(
            view.blocks[0].icon.starts_with("data:"),
            "{}",
            view.blocks[0].icon
        );
        assert!(view.blocks[1].icon.is_empty());
    }

    #[test]
    fn the_folder_shown_is_the_card_copy_when_the_pc_has_none() {
        let scratch = Scratch::new("memcard-folder");
        // A template no machine resolves, so there is no PC copy to prefer.
        scratch.write(
            CONF,
            b"title=Card\n\n[game]\ntitle=Odd\nsave=Odd|{nowhere}/Odd\n",
        );
        let id = saves::declared(scratch.path())[0].id.clone();
        assert!(folder(scratch.path(), &id).is_err(), "nothing anywhere yet");

        let slot = saves::slot_path(scratch.path(), &saves::declared(scratch.path())[0]);
        std::fs::create_dir_all(&slot).unwrap();
        assert_eq!(folder(scratch.path(), &id).unwrap(), slot);
        assert!(folder(scratch.path(), "no-such-save").is_err());
    }

    #[test]
    fn writes_a_card_the_save_module_reads_back() {
        let scratch = Scratch::new("memcard-write");
        let card = scratch.join("card");
        std::fs::create_dir_all(&card).unwrap();
        scratch.write("pic.png", b"picture");
        let request = CardRequest {
            drive_path: String::new(),
            title: "Harry's Card".into(),
            games: vec![
                CardGame {
                    title: "Stardew Valley".into(),
                    executable: "steam://rungameid/413150".into(),
                    icon_source: Some(scratch.join("pic.png").to_string_lossy().into_owned()),
                    save_windows: Some("{appdata}/StardewValley/Saves".into()),
                    save_linux: Some("{appdata}/StardewValley/Saves".into()),
                    ..Default::default()
                },
                CardGame {
                    title: "Bluey".into(),
                    save: Some("{localappdata}/Bluey".into()),
                    ..Default::default()
                },
                CardGame {
                    title: "Nowhere".into(),
                    ..Default::default()
                },
            ],
        };
        let warnings = write_at(&card, &request).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("Nowhere"));

        let text = std::fs::read_to_string(card.join(CONF)).unwrap();
        assert!(text.contains("icon=.gamepak/card_0.png"), "{text}");
        assert!(card.join(".gamepak/card_0.png").is_file());
        let labels: Vec<String> = saves::declared(&card)
            .into_iter()
            .map(|slot| slot.label)
            .collect();
        assert_eq!(labels, ["Stardew Valley", "Bluey"]);

        // Read back for editing, it is what was written: nothing to re-enter,
        // and the picture on the card stays where it is.
        let back = games(&card);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].game.title, "Stardew Valley");
        assert_eq!(back[0].game.executable, "steam://rungameid/413150");
        assert_eq!(
            back[0].game.save_windows.as_deref(),
            Some("{appdata}/StardewValley/Saves")
        );
        assert_eq!(back[1].game.save.as_deref(), Some("{localappdata}/Bluey"));
        let again = CardRequest {
            games: back.into_iter().map(|view| view.game).collect(),
            ..request.clone()
        };
        write_at(&card, &again).unwrap();
        let text = std::fs::read_to_string(card.join(CONF)).unwrap();
        assert!(text.contains("icon=.gamepak/card_0.png"), "{text}");

        // Beside a cartridge it makes a combo drive: the cartridge's own file
        // is left alone, its saves and the card's show together, and the
        // launcher reads it as a cartridge that is a memory card too.
        let cartridge = "title=Hades\nexecutable=steam://rungameid/1145360\n\
                         save=Hades|{documents}/Saved Games/Hades\n";
        std::fs::write(card.join("cartridge.conf"), cartridge).unwrap();
        write_at(&card, &request).unwrap();
        assert_eq!(
            std::fs::read_to_string(card.join("cartridge.conf")).unwrap(),
            cartridge
        );
        assert!(!is_memory_card(&card));
        assert!(is_combo_drive(&card, cartridge));
        let combo = view(&card);
        let titles: Vec<&str> = combo.blocks.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, ["Hades", "Stardew Valley", "Bluey"]);
        assert_eq!(combo.title, "Harry's Card");
        // Adding a game the card has, or the cartridge declares, doubles nothing.
        add_games(
            &card,
            vec![
                CardGame {
                    title: "bluey".into(),
                    save: Some("{localappdata}/Bluey2".into()),
                    ..Default::default()
                },
                CardGame {
                    title: "Hades".into(),
                    save: Some("{documents}/Saved Games/Hades".into()),
                    ..Default::default()
                },
            ],
        )
        .unwrap();
        let titles: Vec<String> = view(&card).blocks.into_iter().map(|b| b.title).collect();
        assert_eq!(titles, ["Hades", "Stardew Valley", "bluey"]);
        assert_eq!(games(&card).len(), 3);
        // And a drive that is not a listed removable one never gets that far.
        assert!(write(&request).is_err());
    }
}
