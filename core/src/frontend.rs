//! Which front-end a cartridge opens in.
//!
//! The launcher is one answer, not the only one. It ships with the project and
//! is on by default, because a cartridge that does nothing when you plug it in
//! is broken and because the launcher is the only front-end that needs no
//! second install. Everything else is a **plugin**: a separate thing, in a
//! separate process, usually in a separate repository, that puts the cartridge
//! where its own users already look — a row on the Steam home screen, a library
//! entry in Playnite.
//!
//! # What this module is, and is not
//!
//! It is a register of names and a record of which ones are switched on. It does
//! not start plugins, talk to them, or know how they work. That is the point:
//! a Decky plugin is Python inside Steam's own process tree and a Playnite
//! extension is C# inside Playnite's, and nothing sensible can drive either from
//! here.
//!
//! What they share is this file. A plugin reads the same `settings.json` the
//! launcher writes, finds whether it is the designated front-end, and behaves
//! accordingly. That is the whole contract, and it is deliberately the smallest
//! one that works: no socket, no daemon, no protocol to version.
//!
//! ```json
//! { "frontends": { "launcher": false, "decky": true } }
//! ```
//!
//! On a Steam Deck that says: do not open a window, the row on the home screen
//! is the front-end. The launcher reads it and quits on insert; the Decky plugin
//! reads it and draws its row.
//!
//! # Why more than one may be on
//!
//! Because the machine decides, not the format. A desktop that also runs
//! Playnite may reasonably want both the cartridge's own window and a library
//! entry; a Deck almost certainly wants only the row. Nothing here enforces a
//! single choice, and no front-end is entitled to assume it is alone.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The built-in launcher, which ships with the project.
pub const LAUNCHER: &str = "launcher";
/// The Decky plugin: cartridge games as a row on the Steam home screen.
pub const DECKY: &str = "decky";
/// The Playnite extension: a cartridge slot as the first tile of the library.
pub const PLAYNITE: &str = "playnite";
/// The GOG Galaxy 2.0 integration: cartridge games owned once seen, installed
/// while the cartridge is in.
pub const GOG_GALAXY: &str = "gog_galaxy";
/// Heroic Games Launcher, through `pc-gamepak-sync`: cartridge games as sideloads.
pub const HEROIC: &str = "heroic";
/// Pegasus Frontend, through `pc-gamepak-sync`: a "PC GamePak" collection.
pub const PEGASUS: &str = "pegasus";
/// ES-DE, through `pc-gamepak-sync`: a "PC GamePak" system.
pub const ESDE: &str = "esde";
/// LaunchBox and Big Box: a cartridge slot, as in Playnite.
pub const LAUNCHBOX: &str = "launchbox";

/// Whether a front-end comes with the project or has to be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Part of this project. Always present, so never "not found".
    BuiltIn,
    /// A separate install, which may or may not be there.
    Plugin,
}

/// One front-end this project knows the name of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontEnd {
    pub id: &'static str,
    /// What to call it in the settings dialog.
    pub name: &'static str,
    pub kind: Kind,
    /// One line saying what turning it on does.
    pub description: &'static str,
    /// Where the plugin lives when it is installed, resolved for this host.
    /// `None` for the built-in launcher and for a front-end that has no
    /// implementation yet.
    pub install_path: Option<String>,
    /// Whether that path is there. Always true for the built-in launcher.
    pub installed: bool,
    /// False for a front-end named here but not yet written, so the interface
    /// can show the shape without offering a switch that does nothing.
    pub implemented: bool,
}

/// Every front-end this build knows about, in the order to show them.
pub fn known() -> Vec<FrontEnd> {
    let decky = decky_path();
    let playnite = playnite_path();
    let galaxy = galaxy_path();
    let launchbox = launchbox_path();
    let sync = sync_path();
    // Heroic, Pegasus and ES-DE have no plugin runtime of their own: one small
    // program writes their files, so all three are installed when it is.
    let synced = |id, name, description| FrontEnd {
        id,
        name,
        kind: Kind::Plugin,
        description,
        installed: present(&sync),
        install_path: sync.as_ref().map(|path| path.display().to_string()),
        implemented: true,
    };
    vec![
        FrontEnd {
            id: LAUNCHER,
            name: "PC GamePak launcher",
            kind: Kind::BuiltIn,
            description: "The cartridge's own window: cover art, Play, Eject.",
            install_path: None,
            installed: true,
            implemented: true,
        },
        FrontEnd {
            id: DECKY,
            name: "Steam Deck row",
            kind: Kind::Plugin,
            description: "Cartridge games as a row on the Steam home screen, through Decky.",
            installed: decky.as_ref().is_some_and(|path| path.is_dir()),
            install_path: decky.map(|path| path.display().to_string()),
            implemented: true,
        },
        FrontEnd {
            id: PLAYNITE,
            name: "Playnite library",
            kind: Kind::Plugin,
            description: "A cartridge slot as the first tile in Playnite's library.",
            installed: playnite.as_ref().is_some_and(|path| path.is_dir()),
            install_path: playnite.map(|path| path.display().to_string()),
            implemented: true,
        },
        FrontEnd {
            id: GOG_GALAXY,
            name: "GOG Galaxy library",
            kind: Kind::Plugin,
            description: "Cartridge games in GOG Galaxy, installed while the cartridge is in.",
            installed: present(&galaxy),
            install_path: galaxy.map(|path| path.display().to_string()),
            implemented: true,
        },
        synced(
            HEROIC,
            "Heroic Games Launcher",
            "Cartridge games as sideloaded games in Heroic, through pc-gamepak-sync.",
        ),
        synced(
            PEGASUS,
            "Pegasus Frontend",
            "A PC GamePak collection in Pegasus, through pc-gamepak-sync.",
        ),
        synced(
            ESDE,
            "ES-DE",
            "A PC GamePak system in ES-DE, through pc-gamepak-sync.",
        ),
        FrontEnd {
            id: LAUNCHBOX,
            name: "LaunchBox",
            kind: Kind::Plugin,
            description: "A cartridge slot in LaunchBox and Big Box.",
            installed: present(&launchbox),
            install_path: launchbox.map(|path| path.display().to_string()),
            implemented: true,
        },
    ]
}

fn present(path: &Option<PathBuf>) -> bool {
    path.as_ref().is_some_and(|path| path.exists())
}

/// Where the GOG Galaxy integration lives: a folder of Galaxy's own plugins,
/// named after the release zip's top folder. `PC_GAMEPAK_GALAXY_DIR` overrides.
fn galaxy_path() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("PC_GAMEPAK_GALAXY_DIR") {
        return Some(PathBuf::from(from_env));
    }
    if !cfg!(target_os = "windows") {
        return None; // Galaxy 2.0 integrations are a Windows install here.
    }
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local)
            .join("GOG.com")
            .join("Galaxy")
            .join("plugins")
            .join("installed")
            .join("pc-gamepak-galaxy"),
    )
}

/// Where `pc-gamepak-sync` is installed, as its README installs it: beside the
/// launcher's settings on Windows, under `~/.local/share` elsewhere.
/// `PC_GAMEPAK_SYNC_PATH` overrides.
fn sync_path() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("PC_GAMEPAK_SYNC_PATH") {
        return Some(PathBuf::from(from_env));
    }
    if cfg!(target_os = "windows") {
        let local = std::env::var_os("LOCALAPPDATA")?;
        return Some(
            PathBuf::from(local)
                .join("PC-GamePak")
                .join("pc-gamepak-sync.pyz"),
        );
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("pc-gamepak")
            .join("pc-gamepak-sync.pyz"),
    )
}

/// Where Decky keeps its plugins.
///
/// `~/homebrew/plugins` is Decky Loader's own layout, on the Deck and anywhere
/// else it is installed. `PC_GAMEPAK_DECKY_DIR` overrides it, which is what the
/// tests use.
fn decky_path() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("PC_GAMEPAK_DECKY_DIR") {
        return Some(PathBuf::from(from_env));
    }
    if cfg!(target_os = "windows") {
        return None; // Decky is a SteamOS thing.
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("homebrew")
            .join("plugins")
            .join("pc-gamepak-decky"),
    )
}

/// Where the LaunchBox plugin lives: `Plugins\PCGamePak` inside LaunchBox's
/// own folder. LaunchBox installs wherever its user chose, `%USERPROFILE%\LaunchBox`
/// unless told otherwise; `PC_GAMEPAK_LAUNCHBOX_DIR` names another install.
fn launchbox_path() -> Option<PathBuf> {
    let root = match std::env::var_os("PC_GAMEPAK_LAUNCHBOX_DIR") {
        Some(dir) => PathBuf::from(dir),
        None if cfg!(target_os = "windows") => {
            PathBuf::from(std::env::var_os("USERPROFILE")?).join("LaunchBox")
        }
        None => return None,
    };
    Some(root.join("Plugins").join("PCGamePak"))
}

/// Where the Playnite extension lives.
///
/// Playnite is Windows software, and its extensions sit under the roaming
/// profile in a folder named after the extension's `Id`, which is `PCGamePak`.
/// Returned on other platforms too when the override is set, because Playnite
/// runs under Proton and the tests have to reach this.
fn playnite_path() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("PC_GAMEPAK_PLAYNITE_DIR") {
        return Some(PathBuf::from(from_env));
    }
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("Playnite")
            .join("Extensions")
            .join("PCGamePak"),
    )
}

/// Which front-ends are switched on.
///
/// Stored as a map rather than a set of named fields so a plugin this build has
/// never heard of survives a round trip through the settings file. A launcher
/// that dropped unknown keys would silently switch off a front-end installed by
/// a newer version of itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Frontends {
    enabled: BTreeMap<String, bool>,
}

impl<'de> Deserialize<'de> for Frontends {
    /// Hand-written for the same reason [`crate::insert::InsertAction`]'s is:
    /// `settings::load_from` treats a file it cannot parse as no file at all, so
    /// one malformed value here would silently reset every other setting the
    /// user has. Anything unusable costs this field alone.
    ///
    /// A `true`/`false` per name is what it reads. A value that is neither — a
    /// string, a number — is dropped rather than guessed at, because a plugin
    /// being on is not something to infer from `"yes"` when the file was meant
    /// to hold booleans.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Ok(serde_json::Value::Object(map)) = serde_json::Value::deserialize(deserializer)
        else {
            return Ok(Frontends::default());
        };
        Ok(Frontends {
            enabled: map
                .into_iter()
                .filter_map(|(id, value)| value.as_bool().map(|on| (id, on)))
                .collect(),
        })
    }
}

impl Frontends {
    /// Whether `id` should handle a cartridge.
    ///
    /// The default when nothing has been said is the thing that makes an install
    /// work out of the box: the launcher is on, every plugin is off. A plugin
    /// that has just been installed still has to be switched on, which is the
    /// same rule the rest of this project's settings follow.
    pub fn is_on(&self, id: &str) -> bool {
        match self.enabled.get(id) {
            Some(on) => *on,
            None => id == LAUNCHER,
        }
    }

    pub fn set(&mut self, id: &str, on: bool) {
        self.enabled.insert(id.to_string(), on);
    }

    /// The ids that are on, including any this build does not recognise.
    pub fn all_on(&self) -> Vec<String> {
        let mut on: Vec<String> = self
            .enabled
            .iter()
            .filter(|(_, value)| **value)
            .map(|(id, _)| id.clone())
            .collect();
        if !self.enabled.contains_key(LAUNCHER) {
            on.insert(0, LAUNCHER.to_string());
        }
        on
    }

    /// Whether anything at all is going to react to a cartridge.
    ///
    /// Worth asking before saying nothing: a user who has switched the launcher
    /// off and not switched a plugin on has made the cartridge inert, and almost
    /// certainly did not mean to.
    pub fn any_on(&self) -> bool {
        !self.all_on().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_opens_the_launcher_and_no_plugin() {
        let fresh = Frontends::default();
        assert!(fresh.is_on(LAUNCHER));
        assert!(!fresh.is_on(DECKY));
        assert!(!fresh.is_on(PLAYNITE));
        assert_eq!(fresh.all_on(), vec![LAUNCHER.to_string()]);
        assert!(fresh.any_on());
    }

    #[test]
    fn the_deck_arrangement_is_one_line_of_settings() {
        // The case this exists for: no window, the row on the home screen.
        let mut frontends = Frontends::default();
        frontends.set(LAUNCHER, false);
        frontends.set(DECKY, true);

        assert!(!frontends.is_on(LAUNCHER));
        assert!(frontends.is_on(DECKY));
        assert_eq!(frontends.all_on(), vec![DECKY.to_string()]);
    }

    #[test]
    fn more_than_one_may_be_on() {
        // A desktop that runs Playnite may want both. Nothing here objects.
        let mut frontends = Frontends::default();
        frontends.set(PLAYNITE, true);
        assert_eq!(
            frontends.all_on(),
            vec![LAUNCHER.to_string(), PLAYNITE.to_string()]
        );
    }

    #[test]
    fn switching_everything_off_is_allowed_and_detectable() {
        // Allowed, because somebody may want a cartridge that does nothing until
        // they open it themselves. Detectable, because they may not have meant to.
        let mut frontends = Frontends::default();
        frontends.set(LAUNCHER, false);
        assert!(!frontends.any_on());
    }

    #[test]
    fn a_plugin_this_build_has_never_heard_of_survives_the_file() {
        // A launcher that dropped unknown keys would switch off a front-end
        // installed by a newer version of itself.
        let json = r#"{"launcher":false,"some-future-shell":true}"#;
        let frontends: Frontends = serde_json::from_str(json).expect("parse");
        assert!(frontends.is_on("some-future-shell"));
        assert_eq!(frontends.all_on(), vec!["some-future-shell".to_string()]);

        let written = serde_json::to_string(&frontends).expect("write");
        assert!(written.contains("some-future-shell"), "{written}");
    }

    #[test]
    fn the_file_holds_a_plain_map_of_names() {
        // The contract a plugin in another language has to read. Kept flat on
        // purpose: a Python or C# plugin should need no schema to follow it.
        let mut frontends = Frontends::default();
        frontends.set(LAUNCHER, false);
        frontends.set(DECKY, true);
        assert_eq!(
            serde_json::to_string(&frontends).expect("write"),
            r#"{"decky":true,"launcher":false}"#
        );
    }

    #[test]
    fn nonsense_in_the_file_does_not_take_the_settings_with_it() {
        // `settings::load_from` treats a file it cannot parse as no file at all,
        // so a malformed value here must cost this field and nothing else.
        for junk in ["[]", "\"launcher\"", "42", "null", "true"] {
            let parsed: Frontends =
                serde_json::from_str(junk).unwrap_or_else(|e| panic!("{junk} failed: {e}"));
            assert_eq!(parsed, Frontends::default(), "{junk}");
        }

        // An object whose values are the wrong type keeps the ones that are not.
        let mixed: Frontends =
            serde_json::from_str(r#"{"launcher":false,"decky":"yes","playnite":true}"#)
                .expect("parse");
        assert!(!mixed.is_on(LAUNCHER), "a real boolean is honoured");
        assert!(
            !mixed.is_on(DECKY),
            "\"yes\" is not a boolean and must not be read as one"
        );
        assert!(mixed.is_on(PLAYNITE));
    }

    #[test]
    fn the_register_names_the_launcher_first_and_lists_every_plugin() {
        let all = known();
        assert_eq!(all[0].id, LAUNCHER);
        assert_eq!(all[0].kind, Kind::BuiltIn);
        assert!(all[0].installed, "the built-in one is always there");

        for id in [
            DECKY, PLAYNITE, GOG_GALAXY, HEROIC, PEGASUS, ESDE, LAUNCHBOX,
        ] {
            let plugin = all.iter().find(|f| f.id == id).expect("listed");
            assert!(plugin.implemented, "{id}");
            assert_eq!(plugin.kind, Kind::Plugin, "{id}");
        }

        let mut ids: Vec<_> = all.iter().map(|f| f.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "no id is listed twice");
    }

    #[test]
    fn heroic_pegasus_and_esde_are_installed_together_with_the_sync_tool() {
        // One program writes all three front-ends' files, so one file decides
        // whether any of their switches can do anything.
        let scratch = crate::testutil::Scratch::new("frontend-sync");
        let tool = scratch.join("pc-gamepak-sync.pyz");
        std::env::set_var("PC_GAMEPAK_SYNC_PATH", &tool);

        let installed = |id| {
            known()
                .into_iter()
                .find(|f| f.id == id)
                .expect("listed")
                .installed
        };
        for id in [HEROIC, PEGASUS, ESDE] {
            assert!(!installed(id), "{id} without the sync tool");
        }

        std::fs::write(&tool, b"zip").expect("write");
        for id in [HEROIC, PEGASUS, ESDE] {
            assert!(installed(id), "{id} with the sync tool");
        }

        std::env::remove_var("PC_GAMEPAK_SYNC_PATH");
    }

    #[test]
    fn a_plugin_is_reported_installed_only_when_it_is_there() {
        let scratch = crate::testutil::Scratch::new("frontend-detect");
        let missing = scratch.join("not-installed");
        std::env::set_var("PC_GAMEPAK_DECKY_DIR", &missing);
        let decky = known().into_iter().find(|f| f.id == DECKY).expect("listed");
        assert!(!decky.installed, "{decky:?}");
        assert_eq!(decky.install_path, Some(missing.display().to_string()));

        let there = scratch.join("installed");
        std::fs::create_dir_all(&there).expect("mkdir");
        std::env::set_var("PC_GAMEPAK_DECKY_DIR", &there);
        let decky = known().into_iter().find(|f| f.id == DECKY).expect("listed");
        assert!(decky.installed, "{decky:?}");

        std::env::remove_var("PC_GAMEPAK_DECKY_DIR");
    }
}
