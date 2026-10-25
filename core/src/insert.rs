//! What should happen when a cartridge is plugged in.
//!
//! Until now the answer was fixed: a window opens. That is right on a desktop
//! and wrong in at least three other situations — a machine where the cartridge
//! is being written to rather than played, an HTPC that wants the game and not a
//! launcher, and somebody who finds a window appearing on its own rude.
//!
//! # Why the decision lives here and not in the watcher
//!
//! Three separate things open the launcher on insert: the resident watcher
//! (Windows, and the rootless Linux install), `gamepak-launcher-helper.sh`
//! (which udev runs for a system install), and the tray menu. Putting the choice
//! in any one of them would mean the other two ignored it, and a setting that is
//! obeyed depending on how you installed the program is worse than no setting.
//!
//! So the launcher decides, before it builds a window, and everything that
//! starts the launcher gets the behaviour for free.
//!
//! # Why `AutoLaunchGame` will not run a program off the cartridge
//!
//! This project's oldest promise is that nothing on a cartridge runs without a
//! click. `autorun.inf`'s `open=` key is deliberately ignored for exactly that
//! reason — it is the original removable-media malware vector, and Windows
//! itself stopped honouring it on non-optical media in Windows 7.
//!
//! Auto-launch keeps the promise by only ever starting a *URI*: `steam://`,
//! `heroic://` and the rest. Those are handled by a launcher already installed
//! on the machine, opening a game the user already owns; the operating system
//! made that association, not the cartridge. A cartridge that carries its own
//! executable still gets a window and still waits for a click — which is the
//! common case for a drive somebody handed you, and the one where auto-running
//! would be handing a stranger's binary the machine.

use serde::{Deserialize, Serialize};

use crate::cartridge::CartridgeInfo;

/// What the user asked for on insert.
///
/// `Deserialize` is hand-written rather than derived, and that is the whole
/// reason this type is not a plain `String`. A derived enum rejects a value it
/// does not recognise, and `settings::load_from` treats a file it cannot parse
/// as no file at all — so one unknown word here, from a newer build or a
/// hand-edited file, would silently reset every other setting the user has. It
/// now falls back to the default for this field alone and leaves the rest
/// standing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertAction {
    /// Do not react at all. The tray and the desktop entry still work.
    None,
    /// Open the launcher and bring it to the front. What this has always done,
    /// and the default.
    #[default]
    FocusUi,
    /// Start the game and show nothing — as far as it is safe to, see above.
    AutoLaunchGame,
    /// Say a cartridge is there and leave it at that.
    NotifyOnly,
}

impl InsertAction {
    /// Parse the value as it appears in the settings file, tolerantly.
    ///
    /// An unknown value means a settings file from a newer build, or a typo in
    /// a hand-edited one. Both should behave the way the program always has
    /// rather than doing nothing and looking broken.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().replace('-', "_").as_str() {
            "none" => InsertAction::None,
            "auto_launch_game" | "auto_launch" | "play" => InsertAction::AutoLaunchGame,
            "notify_only" | "notify" => InsertAction::NotifyOnly,
            _ => InsertAction::FocusUi,
        }
    }

    /// The value written to the settings file.
    pub fn as_str(&self) -> &'static str {
        match self {
            InsertAction::None => "none",
            InsertAction::FocusUi => "focus_ui",
            InsertAction::AutoLaunchGame => "auto_launch_game",
            InsertAction::NotifyOnly => "notify_only",
        }
    }
}

impl<'de> Deserialize<'de> for InsertAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Through a `Value` rather than a visitor over `&str`, so a settings
        // file that somehow has a number or an object here falls back too
        // instead of failing and taking the file with it.
        Ok(match serde_json::Value::deserialize(deserializer) {
            Ok(serde_json::Value::String(text)) => InsertAction::parse(&text),
            _ => InsertAction::default(),
        })
    }
}

/// What the launcher should actually do, once the cartridge has been read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reaction {
    /// Exit without building a window.
    Quit,
    /// Build the window, the way this has always worked.
    ShowWindow,
    /// Hand this URI to the system and exit. Never a path on the cartridge.
    Launch {
        executable: String,
        /// For the stats file, which wants a name beside the count.
        title: String,
    },
    /// Post a notification and exit.
    Notify { title: String, body: String },
}

/// The one call the launcher makes: everything the settings say, applied.
///
/// Split from [`decide`] so the two questions stay separate and both stay
/// tested. This one asks whether the launcher is a front-end at all — a Deck with
/// the Decky row switched on and the launcher off must not have a window appear
/// over the top of it — and only then what the launcher should do about the
/// cartridge.
pub fn decide_for(settings: &crate::settings::Settings, cartridge: &CartridgeInfo) -> Reaction {
    if !settings.frontends.is_on(crate::frontend::LAUNCHER) {
        // Something else is the front-end. Note that this is not the same as
        // `InsertAction::None`: that is the launcher being the front-end and
        // choosing to stay out of the way, which keeps its tray and its desktop
        // entry as the way in. Either way, no window.
        return Reaction::Quit;
    }
    decide(settings.on_cartridge_insert, cartridge)
}

/// Turn a setting and a cartridge into one thing to do.
///
/// Falls back to `ShowWindow` rather than doing nothing whenever the chosen
/// action cannot be carried out — an empty cartridge, a collection with no one
/// game to start, or a launch target that is a program on the drive. A setting
/// that silently does nothing looks like a bug; a window that appears when the
/// shortcut could not be taken is at worst a mild surprise, and it puts the
/// choice back where it started.
pub fn decide(action: InsertAction, cartridge: &CartridgeInfo) -> Reaction {
    match action {
        InsertAction::None => Reaction::Quit,
        InsertAction::FocusUi => Reaction::ShowWindow,
        InsertAction::NotifyOnly => Reaction::Notify {
            title: if cartridge.title.trim().is_empty() {
                "Cartridge inserted".to_string()
            } else {
                cartridge.title.clone()
            },
            body: notify_body(cartridge),
        },
        InsertAction::AutoLaunchGame => match auto_launch_target(cartridge) {
            Some((executable, title)) => Reaction::Launch { executable, title },
            None => Reaction::ShowWindow,
        },
    }
}

fn notify_body(cartridge: &CartridgeInfo) -> String {
    let count = cartridge.games.len();
    if count > 1 {
        format!("{count} games — open PC GamePak to play")
    } else {
        "Open PC GamePak to play".to_string()
    }
}

/// The one game a cartridge can be started without asking, if there is one.
///
/// `None` for all three of the cases where there is no safe unambiguous answer:
/// nothing to launch, more than one game so no way to know which, and a program
/// on the cartridge rather than a URI the system already knows how to open.
fn auto_launch_target(cartridge: &CartridgeInfo) -> Option<(String, String)> {
    if cartridge.games.len() > 1 {
        // A collection has no single game to mean. Picking the first would be
        // inventing an intention.
        return None;
    }

    // A single-game cartridge keeps its target at the top level; a collection of
    // exactly one has it in the list, with a better title.
    let (executable, title) = match cartridge.games.first() {
        Some(game) => (game.executable.as_str(), game.title.as_str()),
        None => (cartridge.executable.as_str(), cartridge.title.as_str()),
    };

    let executable = executable.trim();
    if !is_uri(executable) {
        return None;
    }
    Some((executable.to_string(), title.trim().to_string()))
}

/// Whether this names a scheme the operating system resolves, rather than a file
/// on the cartridge.
///
/// An allowlist, not a shape test. `crate::stats::key_for` asks the structural
/// question — "is case significant here" — and can afford to accept any scheme.
/// This one decides whether to run something unprompted, so it names the
/// handlers it is prepared to trust and refuses everything else, including
/// schemes that would be perfectly fine to open from a click: `file://` reaches
/// the whole filesystem, and `ms-settings://` and its kin are not games.
pub fn is_uri(executable: &str) -> bool {
    const TRUSTED: &[&str] = &[
        "steam://",
        "heroic://",
        "gog://",
        "epic://",
        "playnite://",
        "lutris://",
        "itch://",
        "minigalaxy://",
    ];
    let lower = executable.trim().to_lowercase();
    TRUSTED.iter().any(|scheme| lower.starts_with(scheme))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::GameEntry;

    fn empty_game(title: &str, executable: &str) -> GameEntry {
        GameEntry {
            title: title.to_string(),
            executable: executable.to_string(),
            cover: String::new(),
            cover_path: String::new(),
            background: String::new(),
            background_path: String::new(),
            logo: String::new(),
            logo_path: String::new(),
            icon: String::new(),
            icon_path: String::new(),
            how_long: Default::default(),
            platform: "PC".into(),
        }
    }

    fn cartridge(title: &str, executable: &str) -> CartridgeInfo {
        CartridgeInfo {
            title: title.to_string(),
            executable: executable.to_string(),
            cover_path: String::new(),
            cover: String::new(),
            background: String::new(),
            background_path: String::new(),
            logo: String::new(),
            logo_path: String::new(),
            icon: String::new(),
            icon_path: String::new(),
            drive_path: "/run/media/you/CART".to_string(),
            holds_game: false,
            is_bundle: false,
            games: Vec::new(),
            skin_css: String::new(),
            how_long: Default::default(),
            memory_card: false,
            platform: "PC".into(),
        }
    }

    #[test]
    fn the_default_is_what_this_has_always_done() {
        assert_eq!(InsertAction::default(), InsertAction::FocusUi);
        assert_eq!(
            decide(
                InsertAction::default(),
                &cartridge("X", "steam://rungameid/1")
            ),
            Reaction::ShowWindow
        );
    }

    #[test]
    fn every_spelling_in_the_settings_file_round_trips() {
        for action in [
            InsertAction::None,
            InsertAction::FocusUi,
            InsertAction::AutoLaunchGame,
            InsertAction::NotifyOnly,
        ] {
            assert_eq!(InsertAction::parse(action.as_str()), action);
        }
    }

    #[test]
    fn any_shape_of_junk_in_the_file_falls_back_rather_than_failing() {
        // The point of the hand-written Deserialize: a value this build cannot
        // read must cost this one field, not the whole settings file.
        for json in [
            r#""teleport""#,
            "42",
            "null",
            "true",
            "[1,2]",
            r#"{"a":1}"#,
            r#""auto_launch_game""#,
        ] {
            let parsed: InsertAction =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("{json} failed: {e}"));
            let expected = if json.contains("auto_launch_game") {
                InsertAction::AutoLaunchGame
            } else {
                InsertAction::FocusUi
            };
            assert_eq!(parsed, expected, "{json}");
        }
    }

    #[test]
    fn what_is_written_is_what_is_read_back() {
        for action in [
            InsertAction::None,
            InsertAction::FocusUi,
            InsertAction::AutoLaunchGame,
            InsertAction::NotifyOnly,
        ] {
            let json = serde_json::to_string(&action).expect("serialise");
            assert_eq!(json, format!("\"{}\"", action.as_str()));
            assert_eq!(
                serde_json::from_str::<InsertAction>(&json).expect("round trip"),
                action
            );
        }
    }

    #[test]
    fn a_value_this_build_does_not_know_behaves_as_it_always_has() {
        // A settings file from a newer build, or a typo in a hand-edited one.
        for junk in [
            "",
            "   ",
            "nonsense",
            "AUTO-LAUNCH-GAME",
            " Notify ",
            "None",
        ] {
            let parsed = InsertAction::parse(junk);
            let expected = match junk.trim().to_lowercase().as_str() {
                "none" => InsertAction::None,
                "auto-launch-game" => InsertAction::AutoLaunchGame,
                "notify" => InsertAction::NotifyOnly,
                _ => InsertAction::FocusUi,
            };
            assert_eq!(parsed, expected, "{junk:?}");
        }
    }

    #[test]
    fn none_means_no_window_at_all() {
        assert_eq!(
            decide(InsertAction::None, &cartridge("X", "steam://rungameid/1")),
            Reaction::Quit
        );
    }

    #[test]
    fn a_steam_cartridge_starts_itself() {
        let reaction = decide(
            InsertAction::AutoLaunchGame,
            &cartridge("Cyberpunk 2077", "steam://rungameid/1091500"),
        );
        assert_eq!(
            reaction,
            Reaction::Launch {
                executable: "steam://rungameid/1091500".to_string(),
                title: "Cyberpunk 2077".to_string(),
            }
        );
    }

    #[test]
    fn a_program_on_the_cartridge_still_waits_for_a_click() {
        // The promise this project is built on. A drive somebody handed you must
        // not get to run a binary because a setting was left on.
        for carried in [
            "Games/Tunic/TUNIC.exe",
            "Games\\Tunic\\TUNIC.exe",
            "./start.sh",
            "/usr/bin/evil",
        ] {
            assert_eq!(
                decide(InsertAction::AutoLaunchGame, &cartridge("Tunic", carried)),
                Reaction::ShowWindow,
                "{carried} must not auto-run"
            );
        }
    }

    #[test]
    fn a_scheme_that_is_not_a_game_launcher_is_refused() {
        // `file://` reaches the whole filesystem and the ms- schemes are not
        // games. All of them are fine to open from a click and none of them is
        // fine to open because a drive was plugged in.
        for uri in [
            "file:///etc/passwd",
            "ms-settings://x",
            "http://example.com/x",
            "https://example.com/x",
            "javascript:alert(1)",
            "vbscript:x",
            "smb://host/share/game.exe",
        ] {
            assert!(!is_uri(uri), "{uri} must not be auto-launched");
            assert_eq!(
                decide(InsertAction::AutoLaunchGame, &cartridge("X", uri)),
                Reaction::ShowWindow,
                "{uri}"
            );
        }
    }

    #[test]
    fn the_launchers_own_schemes_are_accepted_whatever_their_case() {
        for uri in [
            "steam://rungameid/620",
            "STEAM://rungameid/620",
            "heroic://launch/gog/1207658921",
            "lutris://x",
            "itch://x",
        ] {
            assert!(is_uri(uri), "{uri} should be launchable");
        }
    }

    #[test]
    fn a_collection_has_no_single_game_to_mean() {
        let mut cart = cartridge("Two games", "steam://rungameid/1");
        cart.is_bundle = true;
        cart.games = vec![
            empty_game("One", "steam://rungameid/1"),
            empty_game("Two", "steam://rungameid/2"),
        ];
        assert_eq!(
            decide(InsertAction::AutoLaunchGame, &cart),
            Reaction::ShowWindow
        );
    }

    #[test]
    fn a_collection_of_exactly_one_uses_that_games_own_title() {
        let mut cart = cartridge("The Shelf", "steam://rungameid/1");
        cart.is_bundle = true;
        cart.games = vec![empty_game("Hollow Knight", "steam://rungameid/367520")];
        assert_eq!(
            decide(InsertAction::AutoLaunchGame, &cart),
            Reaction::Launch {
                executable: "steam://rungameid/367520".to_string(),
                title: "Hollow Knight".to_string(),
            }
        );
    }

    #[test]
    fn a_cartridge_with_nothing_to_launch_shows_the_window() {
        assert_eq!(
            decide(InsertAction::AutoLaunchGame, &cartridge("Broken", "")),
            Reaction::ShowWindow
        );
    }

    #[test]
    fn the_launcher_stays_out_of_the_way_when_a_plugin_is_the_front_end() {
        // The Deck arrangement: the row on the Steam home screen is the
        // front-end, and a window appearing over the top of it would be the bug.
        let mut settings = crate::settings::Settings::default();
        settings.frontends.set(crate::frontend::LAUNCHER, false);
        settings.frontends.set(crate::frontend::DECKY, true);

        let cart = cartridge("Hollow Knight", "steam://rungameid/367520");
        assert_eq!(decide_for(&settings, &cart), Reaction::Quit);

        // And it overrides the insert action rather than being overridden by it:
        // a launcher that is not a front-end does not auto-launch either.
        settings.on_cartridge_insert = InsertAction::AutoLaunchGame;
        assert_eq!(decide_for(&settings, &cart), Reaction::Quit);
    }

    #[test]
    fn with_the_launcher_on_the_insert_action_decides() {
        let mut settings = crate::settings::Settings::default();
        let cart = cartridge("Hollow Knight", "steam://rungameid/367520");
        // The default install: a window.
        assert_eq!(decide_for(&settings, &cart), Reaction::ShowWindow);

        settings.on_cartridge_insert = InsertAction::AutoLaunchGame;
        assert_eq!(
            decide_for(&settings, &cart),
            Reaction::Launch {
                executable: "steam://rungameid/367520".to_string(),
                title: "Hollow Knight".to_string(),
            }
        );
    }

    #[test]
    fn a_notification_names_the_cartridge() {
        let reaction = decide(InsertAction::NotifyOnly, &cartridge("Tomb Raider", "x"));
        assert_eq!(
            reaction,
            Reaction::Notify {
                title: "Tomb Raider".to_string(),
                body: "Open PC GamePak to play".to_string(),
            }
        );
    }

    #[test]
    fn a_notification_for_a_collection_counts_the_games() {
        let mut cart = cartridge("The Shelf", "x");
        cart.games = vec![
            empty_game("One", "steam://rungameid/1"),
            empty_game("Two", "steam://rungameid/2"),
            empty_game("Three", "steam://rungameid/3"),
        ];
        let Reaction::Notify { body, .. } = decide(InsertAction::NotifyOnly, &cart) else {
            panic!("expected a notification");
        };
        assert!(body.starts_with("3 games"), "{body}");
    }

    #[test]
    fn a_nameless_cartridge_still_gets_a_notification_title() {
        let reaction = decide(InsertAction::NotifyOnly, &cartridge("   ", "x"));
        let Reaction::Notify { title, .. } = reaction else {
            panic!("expected a notification");
        };
        assert_eq!(title, "Cartridge inserted");
    }
}
