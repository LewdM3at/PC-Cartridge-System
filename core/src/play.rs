//! Playing a cartridge game with no window.
//!
//! `pc-gamepak --drive D:\ --play 0` is for a front-end that has its own Play
//! button — Playnite's slot, a script, a desktop shortcut. It does what the
//! launcher window does around a launch, and nothing it shows:
//!
//! 1. settle a session that never closed, and bring the saves and shader caches
//!    off the cartridge;
//! 2. count the launch and start the game, the same call the window's Play makes;
//! 3. **stay running while the game does**, keeping the session's heartbeat;
//! 4. then close the session and take the saves and caches back to the drive.
//!
//! Staying up is the point. The front-end that started this is watching this
//! process, so its own play time is right, and the cartridge's hours are
//! measured rather than recorded as zero the way an auto-launch has to.
//!
//! # Knowing when the game has ended
//!
//! The launcher does not own the game's process — a Steam game is Steam's child,
//! not ours. What it can see is [`crate::busy::holders`]: a program running from
//! the cartridge. So the game is running while something on the drive is — or,
//! for a Steam game, while anything runs from its install folder in *any* Steam
//! library ([`steam_install_dir`]), because Steam may start another copy than
//! the cartridge's. [`Watch`] turns a poll of that into "still going" or "over",
//! with room for a slow start and for a game that restarts itself.
//!
//! A cartridge that only points at a game installed elsewhere has nothing on the
//! drive to watch. That is known before launching ([`CartridgeInfo::holds_game`])
//! and it gets the auto-launch treatment: started, counted, closed at once.
//!
//! # Why this is not a way round "nothing runs without a click"
//!
//! [`crate::insert`] will not auto-start a program off a cartridge, because
//! nobody asked. `--play` is somebody asking: it is what a Play button in another
//! program runs. It is never what the watcher runs on insert.

use std::path::Path;
use std::time::Duration;

use crate::cartridge::CartridgeInfo;
use crate::saves::{Direction, SyncOutcome};

/// How long a game may take to appear on the drive after it is started.
///
/// Generous on purpose: Steam checks for updates, syncs cloud saves and may show
/// a "preparing to launch" dialog before the game's own executable exists. Too
/// short and a slow start is recorded as a launch that never happened.
pub const APPEAR_WITHIN: Duration = Duration::from_secs(180);

/// How long nothing may run from the drive before the game counts as over.
///
/// A game that restarts itself — a launcher that execs the real binary, a
/// settings change that relaunches — leaves a gap. Ending the session in it
/// would push the saves back while the game is about to write them again.
pub const GONE_FOR: Duration = Duration::from_secs(15);

/// How often to look.
pub const POLL: Duration = Duration::from_secs(2);

/// The game `--play <index>` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub executable: String,
    pub title: String,
}

/// Which game `index` means on this cartridge.
///
/// A single-game cartridge has exactly one, at 0. A collection's games are
/// numbered in the order `cartridge.conf` lists them, which is the order every
/// front-end shows them in.
pub fn pick(info: &CartridgeInfo, index: usize) -> Result<Pick, String> {
    let (executable, title) = if info.is_bundle {
        let game = info.games.get(index).ok_or_else(|| {
            format!(
                "this cartridge has {} games; there is no game {index}",
                info.games.len()
            )
        })?;
        (game.executable.clone(), game.title.clone())
    } else if index == 0 {
        (info.executable.clone(), info.title.clone())
    } else {
        return Err(format!(
            "this cartridge has one game; there is no game {index}"
        ));
    };

    if executable.trim().is_empty() {
        return Err(format!("{title} has nothing to run"));
    }
    Ok(Pick { executable, title })
}

/// Whether the saves need a person before the game can start.
///
/// A conflict is both copies changed since the last sync. The window asks which
/// to keep; nothing without a window can, and starting the game anyway would
/// have it write a third version on top of the two nobody chose between.
pub fn needs_a_person(outcomes: &[SyncOutcome]) -> bool {
    outcomes
        .iter()
        .any(|outcome| outcome.direction == Direction::Conflict)
}

/// The Steam app id a `steam://rungameid/<id>` or `steam://run/<id>` names.
pub fn steam_app_id(executable: &str) -> Option<&str> {
    let lower = executable.trim().to_ascii_lowercase();
    let prefix = ["steam://rungameid/", "steam://run/"]
        .into_iter()
        .find(|prefix| lower.starts_with(prefix))?;
    let rest = &executable.trim()[prefix.len()..];
    let id = rest.split(['/', '?', '&']).next()?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

/// The folder a Steam game the cartridge carries is installed under, by name.
///
/// Read from the cartridge's own `appmanifest`, because the game's process may
/// not be running from the cartridge at all. Steam records a game in one
/// library but will happily run a second copy it finds in another — seen on
/// real hardware, FTL started from `F:\Games\Steam` with its cartridge plugged
/// in — and a watch that only looked at the drive then gave up while the game
/// was still being played.
pub fn steam_install_dir(root: &Path, executable: &str) -> Option<String> {
    let app_id = steam_app_id(executable)?;
    let manifest = format!("appmanifest_{app_id}.acf");
    [
        crate::steamlib::library_root(root).join("steamapps"),
        root.join("steamapps"),
    ]
    .iter()
    .find_map(|steamapps| {
        let text = std::fs::read_to_string(steamapps.join(&manifest)).ok()?;
        crate::steamlib::install_dir_in(&text)
    })
}

/// Whether an executable lives in `…/steamapps/common/<install_dir>/`, in any
/// Steam library on the machine. Case-insensitive, as Windows paths are.
pub fn is_in_steam_install(exe: &Path, install_dir: &str) -> bool {
    let parts: Vec<String> = exe
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let install_dir = install_dir.to_lowercase();
    // Four, not three: something has to be inside the folder for it to be a
    // program in it rather than the folder itself.
    parts
        .windows(4)
        .any(|four| four[0] == "steamapps" && four[1] == "common" && four[2] == install_dir)
}

/// What one look at the drive means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Keep looking.
    Wait,
    /// The game ran and has ended.
    Ended,
    /// The game never appeared on the drive within [`APPEAR_WITHIN`].
    NeverSeen,
}

/// Turns "is anything running from the drive" at a moment into whether the game
/// is over. Pure, so the timing rules are tested without starting anything.
#[derive(Debug, Clone)]
pub struct Watch {
    appear_within: Duration,
    gone_for: Duration,
    last_seen: Option<Duration>,
}

impl Default for Watch {
    fn default() -> Self {
        Watch::new(APPEAR_WITHIN, GONE_FOR)
    }
}

impl Watch {
    pub fn new(appear_within: Duration, gone_for: Duration) -> Self {
        Watch {
            appear_within,
            gone_for,
            last_seen: None,
        }
    }

    /// `running` as seen `elapsed` after the game was started.
    pub fn observe(&mut self, running: bool, elapsed: Duration) -> Step {
        if running {
            self.last_seen = Some(elapsed);
            return Step::Wait;
        }
        match self.last_seen {
            None if elapsed >= self.appear_within => Step::NeverSeen,
            None => Step::Wait,
            Some(seen) if elapsed.saturating_sub(seen) >= self.gone_for => Step::Ended,
            Some(_) => Step::Wait,
        }
    }

    /// Whether the game has been seen at all.
    pub fn seen(&self) -> bool {
        self.last_seen.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::GameEntry;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn single(executable: &str) -> CartridgeInfo {
        CartridgeInfo {
            title: "FTL: Faster Than Light".into(),
            cover_path: String::new(),
            cover: String::new(),
            background: String::new(),
            background_path: String::new(),
            logo: String::new(),
            logo_path: String::new(),
            icon: String::new(),
            icon_path: String::new(),
            executable: executable.into(),
            drive_path: "D:\\".into(),
            holds_game: true,
            is_bundle: false,
            games: Vec::new(),
            skin_css: String::new(),
            how_long: Default::default(),
            memory_card: false,
            platform: "PC".into(),
        }
    }

    fn game(title: &str, executable: &str) -> GameEntry {
        GameEntry {
            title: title.into(),
            executable: executable.into(),
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

    #[test]
    fn a_single_game_cartridge_has_one_game_at_zero() {
        let info = single("steam://rungameid/212680");
        assert_eq!(
            pick(&info, 0),
            Ok(Pick {
                executable: "steam://rungameid/212680".into(),
                title: "FTL: Faster Than Light".into()
            })
        );
        assert!(pick(&info, 1).is_err());
    }

    #[test]
    fn a_collection_is_numbered_in_the_order_it_lists_its_games() {
        let mut info = single("");
        info.is_bundle = true;
        info.games = vec![
            game("God of War (2018)", "steam://rungameid/1593500"),
            game("God of War Ragnarok", "steam://rungameid/2322010"),
        ];
        assert_eq!(pick(&info, 1).unwrap().title, "God of War Ragnarok");
        assert!(pick(&info, 2).is_err(), "no third game");
    }

    #[test]
    fn a_game_with_nothing_to_run_is_refused_rather_than_started() {
        assert!(pick(&single("  "), 0).is_err());
    }

    fn outcome(direction: Direction) -> SyncOutcome {
        SyncOutcome {
            slot_id: "s".into(),
            label: "Saves".into(),
            direction,
            files: 0,
            bytes: 0,
            backup: String::new(),
            detail: String::new(),
        }
    }

    #[test]
    fn only_a_conflict_needs_somebody_to_choose() {
        assert!(!needs_a_person(&[]));
        assert!(!needs_a_person(&[
            outcome(Direction::Pull),
            outcome(Direction::Linked),
            outcome(Direction::Unusable)
        ]));
        assert!(needs_a_person(&[
            outcome(Direction::Pull),
            outcome(Direction::Conflict)
        ]));
    }

    #[test]
    fn steam_uris_name_their_app() {
        assert_eq!(steam_app_id("steam://rungameid/212680"), Some("212680"));
        assert_eq!(steam_app_id("STEAM://run/413150/"), Some("413150"));
        assert_eq!(steam_app_id("steam://rungameid/"), None);
        assert_eq!(steam_app_id("heroic://launch/x"), None);
        assert_eq!(steam_app_id("Games/ftl.exe"), None);
    }

    #[test]
    fn the_install_folder_comes_from_the_cartridges_own_manifest() {
        let scratch = crate::testutil::Scratch::new("play-steam");
        scratch.write(
            "SteamLibrary/steamapps/appmanifest_212680.acf",
            br#""AppState" { "appid" "212680" "installdir" "FTL Faster Than Light" }"#,
        );
        assert_eq!(
            steam_install_dir(scratch.path(), "steam://rungameid/212680").as_deref(),
            Some("FTL Faster Than Light")
        );
        assert_eq!(
            steam_install_dir(scratch.path(), "steam://rungameid/1"),
            None
        );
    }

    #[test]
    fn a_second_copy_in_another_library_is_the_same_game() {
        let dir = "FTL Faster Than Light";
        // The copy Steam actually ran, on another drive.
        let elsewhere =
            Path::new("F:/Games/Steam/steamapps/common/FTL Faster Than Light/FTLGame.exe");
        assert!(is_in_steam_install(elsewhere, dir));
        assert!(is_in_steam_install(
            Path::new("D:/SteamLibrary/steamapps/common/ftl faster than light/bin/FTLGame.exe"),
            dir
        ));
        // Not a different game whose name starts the same, nor the folder itself.
        assert!(!is_in_steam_install(
            Path::new("F:/steamapps/common/FTL Faster Than Light 2/x.exe"),
            dir
        ));
        assert!(!is_in_steam_install(
            Path::new("F:/steamapps/common/FTL Faster Than Light"),
            dir
        ));
        assert!(!is_in_steam_install(
            Path::new("C:/Windows/explorer.exe"),
            dir
        ));
    }

    #[test]
    fn a_slow_start_is_waited_for() {
        let mut watch = Watch::new(secs(180), secs(15));
        assert_eq!(watch.observe(false, secs(2)), Step::Wait);
        assert_eq!(watch.observe(false, secs(170)), Step::Wait);
        assert_eq!(watch.observe(true, secs(175)), Step::Wait);
        assert!(watch.seen());
    }

    #[test]
    fn a_game_that_never_appears_is_given_up_on() {
        let mut watch = Watch::new(secs(180), secs(15));
        assert_eq!(watch.observe(false, secs(180)), Step::NeverSeen);
        assert!(!watch.seen());
    }

    #[test]
    fn a_game_that_restarts_itself_is_still_the_same_session() {
        let mut watch = Watch::new(secs(180), secs(15));
        watch.observe(true, secs(10));
        // Gone for ten seconds while the launcher execs the real binary.
        assert_eq!(watch.observe(false, secs(12)), Step::Wait);
        assert_eq!(watch.observe(false, secs(20)), Step::Wait);
        assert_eq!(watch.observe(true, secs(22)), Step::Wait);
        // Then really gone.
        assert_eq!(watch.observe(false, secs(3600)), Step::Ended);
    }

    #[test]
    fn once_seen_the_start_timeout_no_longer_applies() {
        // Two hours in, a game that closed a moment ago is not "never seen".
        let mut watch = Watch::new(secs(180), secs(15));
        watch.observe(true, secs(7200));
        assert_eq!(watch.observe(false, secs(7202)), Step::Wait);
        assert_eq!(watch.observe(false, secs(7215)), Step::Ended);
    }
}
