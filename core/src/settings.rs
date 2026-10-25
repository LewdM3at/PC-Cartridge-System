//! What the user has chosen, kept between runs.
//!
//! One file, a handful of keys, and a deliberate default: **nothing here is
//! switched on**. The wizard works with no network at all — game lists and
//! cover art come from caches Steam and Playnite have already written — and it
//! stays that way until someone opts in.
//!
//! The file lives beside the artwork cache: `%LOCALAPPDATA%\PC-GamePak`
//! on Windows, `~/.local/state/pc-gamepak` elsewhere.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Look artwork up on SteamGridDB. Off unless the user says otherwise: it
    /// is the only thing in the project that talks to the network.
    pub steamgriddb_enabled: bool,
    /// A personal SteamGridDB API key. Their v2 API refuses unauthenticated
    /// requests, so the integration does nothing useful without one.
    pub steamgriddb_api_key: String,
    /// Folders to scan for games that no launcher knows about.
    ///
    /// Empty means "whatever `folders::default_roots` finds", which is the
    /// state every install starts in. It is only written once the user edits
    /// the list, so a new launcher directory appearing on disk is picked up
    /// without anyone having to re-run anything.
    pub game_folder_roots: Vec<String>,

    // ---- What Create does without asking ---------------------------------
    //
    // The wizard has always had these controls; it just had nowhere to put the
    // answers. They were sent to `set_settings` as fields this struct did not
    // declare, and serde dropped them on the floor, so every one of them reset
    // on the next run. They live here now because Create stopped asking: it
    // reads them and gets on with it.
    /// `ntfs`, `exfat` or `btrfs`.
    ///
    /// NTFS by default: it is the only one of the three that holds a symlink,
    /// and Steam unpacking Proton onto a cartridge needs 1,892 of them. See
    /// [`crate::format`] for what each one costs.
    pub default_filesystem: String,
    /// Read the cartridge back and check it against what was written.
    ///
    /// On by default. The first two cartridges written on real hardware were
    /// checked, and one of them had two 2 GB archives that arrived with the
    /// right length and the wrong contents — a USB bridge dropping the link
    /// under a sustained write. Nothing else would have caught that until the
    /// game crashed on a level the user had not reached yet.
    pub default_verify: bool,
    /// Write `autorun.inf` and the drive icon so Explorer names the drive.
    pub default_icon: bool,
    /// Power the drive down when the write finishes.
    pub default_eject: bool,
    /// Register a Steam cartridge in `libraryfolders.vdf`.
    pub default_register_steam: bool,
    /// Copy the game onto the cartridge, rather than leaving it a key that
    /// points at an installed copy.
    pub default_copy: bool,
    /// Ask Steam to close before touching its library list.
    pub default_close_steam: bool,
    /// Apply the per-cartridge Windows tweaks. Windows-only, and off: it edits
    /// Defender and Search settings, which is not something to do unasked.
    pub default_tune: bool,
    /// Tell the drive which blocks it no longer has to keep.
    pub default_trim: bool,
    /// Cap the copy at this many MB/s. Zero writes as fast as the link allows.
    ///
    /// An NVMe stick in an aluminium enclosure has nowhere to put its heat, and
    /// a cartridge is the worst thermal case it ever sees: tens of gigabytes in
    /// one sustained pass. A bridge that overheats resets the bus, which is how
    /// this project's first 107 GB write produced three corrupt files.
    pub default_copy_rate_mb_s: u64,
    /// Format the drive before writing.
    ///
    /// Off unless asked for, and the backend still requires the drive's current
    /// label to be sent back before it will touch a filesystem — this decides
    /// whether Create *offers* to format, never whether the gate applies.
    pub default_format: bool,

    // ---- What the cartridge remembers ------------------------------------
    /// Reconcile the save directories a cartridge declares, on insert and on
    /// eject.
    ///
    /// **Off**, and the only one of the two that is. Everything else this
    /// project does on insert reads the cartridge; this writes to a directory
    /// in the user's home that they did not name, on the say-so of a file on a
    /// drive. That is a thing to opt into, however carefully `saves` goes
    /// about it.
    pub save_sync: bool,
    /// Which front-ends handle a cartridge. See [`crate::frontend`].
    ///
    /// The launcher is on and every plugin is off until somebody says otherwise.
    /// A plugin — the Decky row, a Playnite extension — reads this same file to
    /// find out whether it is the designated front-end, which is the whole of
    /// the contract between them.
    pub frontends: crate::frontend::Frontends,
    /// What happens when a cartridge is plugged in, *if* the launcher is one of
    /// the front-ends that is on.
    ///
    /// `focus_ui` — a window, which is what this has always done and what it
    /// still does unless told otherwise. `none`, `notify_only` and
    /// `auto_launch_game` are the other three; see [`crate::insert`], which
    /// also explains why auto-launch will not run a program that lives on the
    /// cartridge however emphatically it is switched on.
    pub on_cartridge_insert: crate::insert::InsertAction,
    /// Keep launches, hours and last-played in `.gamepak/stats.json` on the
    /// cartridge.
    ///
    /// **On.** It writes one small file, it writes it only to the drive the
    /// user just pressed Play on, and a cartridge that cannot be written to
    /// simply does not get one. The point of the feature is that the count
    /// survives being carried to another machine, which a default of off would
    /// quietly undo for everybody who never found the switch.
    pub track_playtime: bool,
    /// Stop counting playtime after this many minutes without any input —
    /// keyboard, mouse or controller. Zero counts through. See
    /// [`crate::playtrack`].
    pub idle_pause_minutes: u64,
    /// Look each game up on HowLongToBeat while a cartridge is written. **Off**:
    /// it is a network request to a site with no official API. See
    /// [`crate::hltb`].
    pub hltb_enabled: bool,
    /// Look up where each game keeps its saves, from Ludusavi's manifest.
    /// **Off**: it downloads a 17 MB file. See [`crate::ludusavi`].
    pub ludusavi_enabled: bool,

    // ---- GamePak triggers --------------------------------------------------
    /// Start a GamePak's action the moment its card or barcode arrives.
    ///
    /// **Off**: a trigger opens the launcher at READY — the host woken, the
    /// title and cover up — and Play starts it, as a cartridge does. On, a
    /// tap is the Play press, for a setup with no screen to press it on.
    pub launch_on_trigger: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            steamgriddb_enabled: false,
            steamgriddb_api_key: String::new(),
            game_folder_roots: Vec::new(),
            default_filesystem: "ntfs".to_string(),
            // Costs a read pass over the drive and is worth it: the alternative
            // is finding out from a crash months later.
            default_verify: true,
            // A cartridge that does not name itself in Explorer is half a
            // cartridge, and ejecting is what you were going to do anyway.
            default_icon: true,
            default_eject: true,
            default_register_steam: true,
            default_copy: true,
            default_close_steam: true,
            default_tune: false,
            default_trim: false,
            // Off: a drive with a heatsink, or a link too slow to trouble it,
            // should not be slowed for a problem it does not have.
            default_copy_rate_mb_s: 0,
            default_format: false,
            save_sync: false,
            track_playtime: true,
            launch_on_trigger: false,
            idle_pause_minutes: crate::playtrack::DEFAULT_IDLE_MINUTES,
            hltb_enabled: false,
            ludusavi_enabled: false,
            frontends: crate::frontend::Frontends::default(),
            on_cartridge_insert: crate::insert::InsertAction::default(),
        }
    }
}

impl Settings {
    /// The key to send, or nothing when the integration is off or unkeyed.
    pub fn steamgriddb_key(&self) -> Option<&str> {
        if !self.steamgriddb_enabled {
            return None;
        }
        let key = self.steamgriddb_api_key.trim();
        (!key.is_empty()).then_some(key)
    }
}

/// Where the settings file lives. `PC_GAMEPAK_CONFIG_DIR` overrides it, which
/// is what the tests use.
pub fn settings_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PC_GAMEPAK_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            if !local.trim().is_empty() {
                return PathBuf::from(local).join("PC-GamePak");
            }
        }
        PathBuf::from(".")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("pc-gamepak")
    }
}

fn settings_path() -> PathBuf {
    settings_dir().join("settings.json")
}

/// Read the settings. A missing or unreadable file is not an error — it means
/// the defaults, which are what a fresh install runs on.
pub fn load() -> Settings {
    load_from(&settings_path())
}

pub fn load_from(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    save_to(&settings_path(), settings)
}

/// Save what the settings form sent, keeping the front-end switches that are
/// already on disk.
///
/// The form does not carry `frontends` — the switches are saved one at a time
/// by `set_frontend` as they are clicked — so the object it sends deserialises
/// with an empty map. Saved as it came, that switched every plugin off whenever
/// any other setting changed: Playnite, GOG Galaxy and the rest went quiet,
/// silently, the next time somebody typed an API key. The switches on disk are
/// the current ones, so those are kept.
pub fn save_form(incoming: Settings) -> Result<Settings, String> {
    save_form_to(&settings_path(), incoming)
}

pub fn save_form_to(path: &Path, mut incoming: Settings) -> Result<Settings, String> {
    incoming.frontends = load_from(path).frontends;
    save_to(path, &incoming)?;
    Ok(incoming)
}

pub fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("could not encode the settings: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_is_offline() {
        let settings = Settings::default();
        assert!(!settings.steamgriddb_enabled);
        assert!(settings.steamgriddb_key().is_none());
    }

    #[test]
    fn settings_survive_a_round_trip() {
        let scratch = crate::testutil::Scratch::new("settings");
        let path = scratch.join("settings.json");

        // Nothing written yet: the defaults stand rather than an error.
        assert_eq!(load_from(&path), Settings::default());

        let chosen = Settings {
            steamgriddb_enabled: true,
            steamgriddb_api_key: "  abc123  ".to_string(),
            ..Settings::default()
        };
        save_to(&path, &chosen).unwrap();

        let read_back = load_from(&path);
        assert_eq!(read_back, chosen);
        // Whitespace around a pasted key is the user's, not theirs to debug.
        assert_eq!(read_back.steamgriddb_key(), Some("abc123"));
    }

    #[test]
    fn every_default_the_wizard_sends_survives_a_save() {
        // These were declared nowhere, so `set_settings` accepted them and
        // serde dropped them: the wizard's own defaults reset on every run.
        let scratch = crate::testutil::Scratch::new("settings-defaults");
        let path = scratch.join("settings.json");

        let chosen = Settings {
            default_filesystem: "btrfs".to_string(),
            // Every value here is the opposite of the default, so a field that
            // is silently dropped shows up as a mismatch rather than passing by
            // coincidence.
            default_verify: false,
            default_icon: false,
            default_eject: false,
            default_register_steam: false,
            default_format: true,
            save_sync: true,
            track_playtime: false,
            idle_pause_minutes: 3,
            hltb_enabled: true,
            ludusavi_enabled: true,
            on_cartridge_insert: crate::insert::InsertAction::AutoLaunchGame,
            ..Settings::default()
        };
        save_to(&path, &chosen).unwrap();

        assert_eq!(load_from(&path), chosen);
    }

    #[test]
    fn saving_the_settings_form_does_not_switch_the_plugins_off() {
        // The form sends every setting but the front-end switches. Saving it
        // must not turn off what `set_frontend` turned on.
        let scratch = crate::testutil::Scratch::new("settings-form");
        let path = scratch.join("settings.json");
        std::fs::write(
            &path,
            br#"{"frontends":{"launcher":false,"playnite":true,"heroic":true}}"#,
        )
        .unwrap();

        // What the form's JSON becomes: no `frontends` key at all.
        let from_form: Settings =
            serde_json::from_str(r#"{"steamgriddbApiKey":"typed just now"}"#).unwrap();
        assert!(!from_form.frontends.is_on(crate::frontend::PLAYNITE));

        let saved = save_form_to(&path, from_form).unwrap();
        let on_disk = load_from(&path);
        for settings in [&saved, &on_disk] {
            assert!(settings.frontends.is_on(crate::frontend::PLAYNITE));
            assert!(settings.frontends.is_on(crate::frontend::HEROIC));
            assert!(!settings.frontends.is_on(crate::frontend::LAUNCHER));
            assert_eq!(settings.steamgriddb_api_key, "typed just now");
        }
    }

    #[test]
    fn a_malformed_frontends_value_does_not_reset_every_other_setting() {
        // `load_from` treats an unparseable file as no file, so one bad value
        // would silently undo everything the user has configured.
        let scratch = crate::testutil::Scratch::new("settings-frontends");
        let path = scratch.join("settings.json");
        std::fs::write(
            &path,
            br#"{"frontends":["launcher"],"steamgriddbApiKey":"kept"}"#,
        )
        .unwrap();

        let loaded = load_from(&path);
        assert!(loaded.frontends.is_on(crate::frontend::LAUNCHER));
        assert_eq!(loaded.steamgriddb_api_key, "kept");
    }

    #[test]
    fn a_fresh_install_opens_a_window_on_insert() {
        // The behaviour every existing install already has. Anything else here
        // would change what happens on a machine nobody reconfigured.
        assert_eq!(
            Settings::default().on_cartridge_insert,
            crate::insert::InsertAction::FocusUi
        );
    }

    #[test]
    fn a_settings_file_with_an_unreadable_insert_action_still_loads() {
        // serde would reject an unknown enum value and take the whole file with
        // it, resetting every other setting. The field is deserialised through
        // the tolerant parser for that reason.
        let scratch = crate::testutil::Scratch::new("settings-insert");
        let path = scratch.join("settings.json");
        std::fs::write(
            &path,
            br#"{"onCartridgeInsert":"teleport","steamgriddbApiKey":"kept"}"#,
        )
        .unwrap();

        let loaded = load_from(&path);
        assert_eq!(
            loaded.on_cartridge_insert,
            crate::insert::InsertAction::FocusUi
        );
        assert_eq!(loaded.steamgriddb_api_key, "kept");
    }

    #[test]
    fn a_fresh_install_counts_hours_and_leaves_the_home_directory_alone() {
        let fresh = Settings::default();
        // The cartridge's own file, written only because Play was pressed.
        assert!(fresh.track_playtime);
        // A directory in the user's home, named by a file on somebody else's
        // drive. That one is asked for.
        assert!(!fresh.save_sync);
    }

    #[test]
    fn a_fresh_install_writes_a_named_cartridge_and_ejects_it() {
        let fresh = Settings::default();
        assert!(fresh.default_icon, "a cartridge should name itself");
        assert!(fresh.default_eject);
        assert!(fresh.default_register_steam);
        // Formatting erases a drive, so it never happens unless it is asked
        // for. Verifying only costs time, and the time buys the one guarantee
        // nothing else on a cartridge provides.
        assert!(!fresh.default_format);
        assert!(fresh.default_verify, "a copy should check its own work");
    }

    #[test]
    fn a_key_is_only_offered_when_the_lookup_is_on() {
        let off = Settings {
            steamgriddb_enabled: false,
            steamgriddb_api_key: "abc123".to_string(),
            ..Settings::default()
        };
        assert!(off.steamgriddb_key().is_none());

        let on_but_unkeyed = Settings {
            steamgriddb_enabled: true,
            steamgriddb_api_key: "   ".to_string(),
            ..Settings::default()
        };
        assert!(on_but_unkeyed.steamgriddb_key().is_none());
    }

    #[test]
    fn nonsense_in_the_file_falls_back_to_the_defaults() {
        let scratch = crate::testutil::Scratch::new("settings-junk");
        let path = scratch.join("settings.json");
        std::fs::write(&path, b"{ not json at all").unwrap();
        assert_eq!(load_from(&path), Settings::default());
    }
}
