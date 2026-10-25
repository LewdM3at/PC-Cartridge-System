//! The cartridges this machine's wizard has written.
//!
//! A cartridge spends most of its life unplugged, and once it is the wizard has
//! no way to know it exists. This keeps one line per write — what it is called,
//! what is on it, when — and a small copy of its cover, so the wizard can show
//! the shelf it has made. Only writes from now on are recorded; nothing is
//! reconstructed from drives seen in passing.
//!
//! Kept beside `settings.json`, never on a cartridge: it is this machine's
//! record of what it made, not something a cartridge carries.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cartridge;

/// Edge of the saved cover. Enough for a list row at twice its size.
const THUMB_EDGE: u32 = 240;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    /// Unique per write. Also the thumbnail's file stem.
    pub id: String,
    /// The name in the cartridge's conf when it was written.
    pub title: String,
    /// Every game on it, in play order. One for a single-game cartridge.
    pub games: Vec<String>,
    /// The volume label at the time, which is what the drive is called when
    /// it is plugged back in.
    pub drive_label: String,
    /// Seconds since the Unix epoch.
    pub written_at: u64,
    /// Bytes of game copied onto it; 0 when the games stayed on the PC.
    pub bytes_copied: u64,
    /// Thumbnail file name under the `created` folder, when a cover existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
}

/// One entry as the window wants it: the thumbnail inlined as a data URI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedView {
    #[serde(flatten)]
    pub entry: Created,
    pub cover: String,
}

fn dir() -> PathBuf {
    crate::settings::settings_dir().join("created")
}

fn list_path(dir: &Path) -> PathBuf {
    dir.join("created.json")
}

/// Every recorded cartridge, newest first. A missing or unreadable file is an
/// empty shelf, not an error.
pub fn list() -> Vec<CreatedView> {
    let dir = dir();
    load_from(&dir)
        .into_iter()
        .map(|entry| {
            let cover = entry
                .thumb
                .as_ref()
                .map(|thumb| cartridge::cover_as_data_uri(&dir.join(thumb).to_string_lossy()))
                .unwrap_or_default();
            CreatedView { entry, cover }
        })
        .collect()
}

/// Record a cartridge that has just been written at `drive_path`.
///
/// Reads the cartridge back rather than trusting the request, so the entry
/// says what actually landed — the title after sanitising, the games in the
/// order the conf has them, the art that was copied.
pub fn record(drive_path: &str, drive_label: &str, bytes_copied: u64) -> Result<(), String> {
    record_in(&dir(), drive_path, drive_label, bytes_copied, now())
}

/// Take one entry off the shelf, and its thumbnail with it.
pub fn forget(id: &str) -> Result<(), String> {
    forget_in(&dir(), id)
}

fn record_in(
    dir: &Path,
    drive_path: &str,
    drive_label: &str,
    bytes_copied: u64,
    written_at: u64,
) -> Result<(), String> {
    let info = cartridge::read_cartridge_info(drive_path)?;
    let games = if info.games.is_empty() {
        vec![info.title.clone()]
    } else {
        info.games.iter().map(|game| game.title.clone()).collect()
    };

    let mut entries = load_from(dir);
    // Two writes in the same second would otherwise share a thumbnail.
    let mut id = written_at.to_string();
    while entries.iter().any(|entry| entry.id == id) {
        id.push('b');
    }

    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    // A collection's own cover is optional; its first game's is what the
    // launcher shows first, so it stands in.
    let cover = Some(info.cover_path.as_str())
        .filter(|path| !path.is_empty())
        .or_else(|| info.games.first().map(|game| game.cover_path.as_str()))
        .filter(|path| !path.is_empty());
    let thumb = cover.and_then(|cover| save_thumb(&Path::new(drive_path).join(cover), dir, &id));

    entries.insert(
        0,
        Created {
            id,
            title: info.title,
            games,
            drive_label: drive_label.to_string(),
            written_at,
            bytes_copied,
            thumb,
        },
    );
    save_to(dir, &entries)
}

fn forget_in(dir: &Path, id: &str) -> Result<(), String> {
    let mut entries = load_from(dir);
    let Some(at) = entries.iter().position(|entry| entry.id == id) else {
        return Ok(());
    };
    let gone = entries.remove(at);
    if let Some(thumb) = gone.thumb {
        let _ = std::fs::remove_file(dir.join(thumb));
    }
    save_to(dir, &entries)
}

/// A small PNG of the cover, so the shelf still has a face once the drive is
/// unplugged. Best effort: a cover that will not decode leaves the row plain.
fn save_thumb(cover: &Path, dir: &Path, id: &str) -> Option<String> {
    let image = image::open(cover).ok()?;
    let name = format!("{id}.png");
    image
        .thumbnail(THUMB_EDGE, THUMB_EDGE)
        .save(dir.join(&name))
        .ok()?;
    Some(name)
}

fn load_from(dir: &Path) -> Vec<Created> {
    std::fs::read_to_string(list_path(dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_to(dir: &Path, entries: &[Created]) -> Result<(), String> {
    let path = list_path(dir);
    let text = serde_json::to_string_pretty(entries)
        .map_err(|e| format!("could not encode the shelf: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cartridge_at(root: &Path, conf: &str) {
        std::fs::write(root.join("cartridge.conf"), conf).unwrap();
    }

    #[test]
    fn records_newest_first_and_forgets() {
        let scratch = crate::testutil::Scratch::new("created");
        let shelf = scratch.join("shelf");
        let drive = scratch.join("drive");
        std::fs::create_dir_all(&drive).unwrap();

        // Nothing written yet is an empty shelf, not an error.
        assert!(load_from(&shelf).is_empty());

        cartridge_at(
            &drive,
            "title=Hades\nexecutable=steam://rungameid/1145360\n",
        );
        record_in(&shelf, &drive.to_string_lossy(), "HADES", 0, 100).unwrap();

        let cover = image::RgbImage::from_pixel(600, 900, image::Rgb([200, 40, 40]));
        cover.save(drive.join("cover.png")).unwrap();
        cartridge_at(
            &drive,
            "[collection]\ntitle=Roguelikes\ncover=cover.png\n\n[game]\ntitle=Hades\nexecutable=a\n\n[game]\ntitle=FTL\nexecutable=b\n",
        );
        // Same second as the first: must not collide.
        record_in(&shelf, &drive.to_string_lossy(), "ROGUES", 42, 100).unwrap();

        let entries = load_from(&shelf);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Roguelikes");
        assert_eq!(entries[0].games, ["Hades", "FTL"]);
        assert_eq!(entries[0].drive_label, "ROGUES");
        assert_eq!(entries[0].bytes_copied, 42);
        assert_ne!(entries[0].id, entries[1].id);
        assert_eq!(entries[1].games, ["Hades"]);
        assert_eq!(entries[1].thumb, None, "no cover, no thumbnail");

        // The thumbnail is small and survives the drive going away.
        let thumb = shelf.join(entries[0].thumb.as_ref().expect("a thumbnail"));
        let (w, h) = image::image_dimensions(&thumb).unwrap();
        assert!(w <= THUMB_EDGE && h <= THUMB_EDGE, "{w}x{h}");
        std::fs::remove_dir_all(&drive).unwrap();
        assert!(thumb.is_file());

        forget_in(&shelf, &entries[0].id).unwrap();
        assert!(!thumb.exists());
        assert_eq!(load_from(&shelf).len(), 1);
        // Forgetting what is not there is not an error.
        forget_in(&shelf, "nope").unwrap();
    }

    #[test]
    fn a_drive_without_a_cartridge_is_not_recorded() {
        let scratch = crate::testutil::Scratch::new("created-empty");
        let shelf = scratch.join("shelf");
        assert!(record_in(&shelf, &scratch.path().to_string_lossy(), "X", 0, 1).is_err());
        assert!(load_from(&shelf).is_empty());
    }
}
