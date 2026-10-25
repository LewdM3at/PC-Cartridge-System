//! Counting the time a game is actually being played.
//!
//! Two questions, asked every [`TICK_SECONDS`] while a session is open:
//!
//! 1. **Is the game running?** A process whose executable, or whose working
//!    directory, is inside the game's folder. That is the method
//!    GameplayTimeTracker and GamingGaiden use, and it is the one that works
//!    for a `steam://` launch: the game is Steam's child, not the launcher's,
//!    but it still runs *from* somewhere — and for a cartridge, that somewhere
//!    is usually the cartridge. The working directory is checked as well as the
//!    executable because a game under Proton runs as a Wine binary from outside
//!    the game folder, with its working directory inside it.
//! 2. **Is anybody there?** See [`crate::idle`]. Once nothing has been touched
//!    for the idle threshold, the time since the last input is taken back out
//!    and nothing more is counted until someone comes back.
//!
//! When the game cannot be seen at all — a cartridge that points at a game
//! installed somewhere nobody can name, through a launcher that says nothing —
//! the tracker falls back to counting while the launcher's window is open,
//! which is what it always did, now with the idle pause as well.
//!
//! [`Tracker`] is a pure state machine: it is handed the time, whether the
//! game was seen, and the idle reading, and it keeps the arithmetic. Nothing in
//! it touches the system, which is what lets its rules be tested.

use std::path::{Path, PathBuf};

/// How often a running session looks at the process list and the idle clock.
pub const TICK_SECONDS: u64 = 15;

/// How long a launched game has to turn up before the tracker stops looking.
///
/// Steam checks for updates and syncs cloud saves before it starts anything,
/// which can take a minute or two on a slow connection. Five minutes covers
/// that without leaving a launch that failed outright being watched all night.
pub const APPEAR_GRACE_SECONDS: u64 = 5 * 60;

/// Consecutive ticks with no process before the game counts as closed.
///
/// Two, because a launcher stub that hands over to the real executable leaves
/// a moment with neither running.
const MISSING_TICKS_TO_END: u32 = 2;

/// A gap between ticks longer than this is a machine that was asleep, and
/// counts for nothing.
const MAX_TICK_GAP: u64 = TICK_SECONDS * 8;

/// The default for the idle pause, in minutes. GamingGaiden's default is ten.
pub const DEFAULT_IDLE_MINUTES: u64 = 10;

/// How a session is being measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Watching a folder for the game's process.
    Process,
    /// Counting while the launcher is open, because the game cannot be seen.
    Window,
}

/// What a tick decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// Still going.
    Running,
    /// The game has gone, or never came.
    Ended,
    /// The game never appeared: stop watching for it and count the window
    /// instead. The caller switches with [`Tracker::fall_back_to_window`].
    NeverSeen,
}

/// The arithmetic of one session.
#[derive(Debug, Clone)]
pub struct Tracker {
    mode: Mode,
    started: u64,
    last_tick: u64,
    /// Seconds from this threshold on count as away. Zero turns the pause off.
    idle_threshold: u64,
    seen: bool,
    missing: u32,
    /// Stretches that have been counted, oldest first, trimmed to the recent
    /// past. Kept so an idle stretch can be taken back exactly — the ticks
    /// before the threshold was crossed were counted, and some of them were
    /// the person already being away.
    counted: Vec<(u64, u64)>,
    active: u64,
}

impl Tracker {
    pub fn new(mode: Mode, now: u64, idle_minutes: u64) -> Self {
        Self {
            mode,
            started: now,
            last_tick: now,
            idle_threshold: idle_minutes.saturating_mul(60),
            seen: false,
            missing: 0,
            counted: Vec::new(),
            active: 0,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Seconds of play counted so far.
    pub fn active_seconds(&self) -> u64 {
        self.active
    }

    /// Stop looking for a process and count while the window is open.
    pub fn fall_back_to_window(&mut self, now: u64) {
        self.mode = Mode::Window;
        self.last_tick = now;
    }

    /// Advance to `now`.
    ///
    /// `running` is whether the game's process was seen (ignored in window
    /// mode), and `idle` is seconds since the last input, if that is known.
    pub fn tick(&mut self, now: u64, running: bool, idle: Option<u64>) -> Tick {
        let from = self.last_tick;
        self.last_tick = now.max(from);
        let gap = now.saturating_sub(from);

        if self.mode == Mode::Process {
            if !running {
                if self.seen {
                    self.missing += 1;
                    if self.missing >= MISSING_TICKS_TO_END {
                        return Tick::Ended;
                    }
                    // A tick with nothing running is not play, even before the
                    // end is certain.
                    return Tick::Running;
                }
                if now.saturating_sub(self.started) >= APPEAR_GRACE_SECONDS {
                    return Tick::NeverSeen;
                }
                return Tick::Running;
            }
            self.seen = true;
            self.missing = 0;
        }

        // A machine that slept between ticks was not being played.
        if gap == 0 || gap > MAX_TICK_GAP {
            return Tick::Running;
        }

        match idle {
            Some(away) if self.idle_threshold > 0 && away >= self.idle_threshold => {
                // Away. Take back whatever was counted since the last input,
                // and count nothing for this tick.
                self.refund_since(now.saturating_sub(away));
            }
            _ => self.count(from, now),
        }
        self.forget_before(now.saturating_sub(self.idle_threshold.max(3600) * 2));
        Tick::Running
    }

    fn count(&mut self, from: u64, to: u64) {
        self.active += to - from;
        self.counted.push((from, to));
    }

    /// Remove every counted second from `since` onwards.
    fn refund_since(&mut self, since: u64) {
        let mut kept = Vec::with_capacity(self.counted.len());
        for &(from, to) in &self.counted {
            if to <= since {
                kept.push((from, to));
            } else if from < since {
                self.active -= to - since;
                kept.push((from, since));
            } else {
                self.active -= to - from;
            }
        }
        self.counted = kept;
    }

    fn forget_before(&mut self, cutoff: u64) {
        self.counted.retain(|&(_, to)| to > cutoff);
    }
}

/// The folders whose processes are the game, for a launch target.
///
/// * A program on the cartridge: the top folder it sits in, so a launcher stub
///   that starts the real executable next to it is still the same game. A
///   program at the cartridge's root has only the root to watch.
/// * A Steam game: its install folder — on the cartridge when it was copied
///   there, or wherever Steam keeps it on this machine.
/// * Anything else: the cartridge itself. If the game lives there, it is seen;
///   if it does not, nothing is, and the tracker falls back to the window.
pub fn watch_dirs(root: &Path, executable: &str, steam_root: Option<&Path>) -> Vec<PathBuf> {
    let executable = executable.trim();
    if let Some(app_id) = steam_app_id(executable) {
        if let Some(dir) = steam_install_dir(root, &app_id, steam_root) {
            return vec![dir];
        }
        return vec![root.to_path_buf()];
    }
    if executable.contains("://") {
        return vec![root.to_path_buf()];
    }
    // The game's own folder: `Games/Tunic` for `Games/Tunic/Tunic.exe`, so two
    // carried games under one `Games` folder are not mistaken for each other,
    // and `Tunic` for `Tunic/Tunic.exe`.
    let folders: Vec<_> = Path::new(executable)
        .parent()
        .map(|parent| parent.components().take(2).collect())
        .unwrap_or_default();
    if folders.is_empty() {
        return vec![root.to_path_buf()];
    }
    vec![folders
        .iter()
        .fold(root.to_path_buf(), |path, part| path.join(part.as_os_str()))]
}

/// The app id in a `steam://rungameid/<id>` or `steam://run/<id>` target.
pub fn steam_app_id(executable: &str) -> Option<String> {
    let lower = executable.trim().to_lowercase();
    let rest = lower
        .strip_prefix("steam://rungameid/")
        .or_else(|| lower.strip_prefix("steam://run/"))?;
    let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!id.is_empty()).then_some(id)
}

/// Where a Steam game's files are: the cartridge's own library first.
fn steam_install_dir(root: &Path, app_id: &str, steam_root: Option<&Path>) -> Option<PathBuf> {
    let on_cartridge = [
        crate::steamlib::library_root(root).join("steamapps"),
        // Cartridges written before the library had a folder of its own.
        root.join("steamapps"),
    ];
    for steamapps in on_cartridge {
        if let Some(dir) = install_dir_in(&steamapps, app_id) {
            return Some(dir);
        }
    }
    let steam_root = steam_root?;
    crate::steamlib::locate(steam_root, app_id).map(|game| game.install_path)
}

fn install_dir_in(steamapps: &Path, app_id: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(steamapps.join(format!("appmanifest_{app_id}.acf"))).ok()?;
    let kv = crate::steam::parse_keyvalues(&text);
    let dir = kv.get("AppState")?.get("installdir")?.as_str()?.to_string();
    let path = steamapps.join("common").join(dir);
    path.is_dir().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = TICK_SECONDS;

    /// Run a tracker through `ticks` ticks from `start`, each seeing `running`
    /// and `idle`.
    fn run(
        tracker: &mut Tracker,
        start: u64,
        ticks: u64,
        running: bool,
        idle: impl Fn(u64) -> Option<u64>,
    ) -> u64 {
        let mut now = start;
        for _ in 0..ticks {
            now += T;
            tracker.tick(now, running, idle(now));
        }
        now
    }

    #[test]
    fn a_running_game_counts_every_tick() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        run(&mut t, 0, 40, true, |_| Some(0));
        assert_eq!(t.active_seconds(), 40 * T);
    }

    #[test]
    fn nothing_counts_before_the_game_appears() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        let now = run(&mut t, 0, 8, false, |_| Some(0));
        assert_eq!(t.active_seconds(), 0);
        run(&mut t, now, 4, true, |_| Some(0));
        // The first tick that sees it counts the stretch since the last look.
        assert_eq!(t.active_seconds(), 4 * T);
    }

    #[test]
    fn a_game_that_never_appears_is_reported() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        assert_eq!(t.tick(T, false, None), Tick::Running);
        assert_eq!(t.tick(APPEAR_GRACE_SECONDS, false, None), Tick::NeverSeen);
    }

    #[test]
    fn a_closed_game_ends_the_session_after_two_empty_looks() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        let now = run(&mut t, 0, 4, true, |_| Some(0));
        assert_eq!(t.tick(now + T, false, Some(0)), Tick::Running);
        assert_eq!(t.tick(now + 2 * T, false, Some(0)), Tick::Ended);
        // The empty looks were not play.
        assert_eq!(t.active_seconds(), 4 * T);
    }

    #[test]
    fn a_stub_handing_over_to_the_game_does_not_end_it() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        let now = run(&mut t, 0, 2, true, |_| Some(0));
        assert_eq!(t.tick(now + T, false, Some(0)), Tick::Running);
        assert_eq!(t.tick(now + 2 * T, true, Some(0)), Tick::Running);
        assert_eq!(t.tick(now + 3 * T, false, Some(0)), Tick::Running);
    }

    #[test]
    fn walking_away_takes_the_idle_stretch_back() {
        // Played for ten minutes, then left the game paused for an hour.
        let mut t = Tracker::new(Mode::Process, 0, 10);
        let played = run(&mut t, 0, 40, true, |_| Some(0));
        let left_at = played;
        run(&mut t, played, 240, true, |now| Some(now - left_at));
        assert_eq!(t.active_seconds(), 40 * T);
    }

    #[test]
    fn coming_back_starts_counting_again() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        let left_at = run(&mut t, 0, 20, true, |_| Some(0));
        let back = run(&mut t, left_at, 80, true, |now| Some(now - left_at));
        run(&mut t, back, 20, true, |_| Some(0));
        assert_eq!(t.active_seconds(), 40 * T);
    }

    #[test]
    fn a_short_pause_under_the_threshold_still_counts() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        let left_at = run(&mut t, 0, 20, true, |_| Some(0));
        // Five minutes without touching anything: a cutscene, not an absence.
        let back = run(&mut t, left_at, 20, true, |now| Some(now - left_at));
        run(&mut t, back, 20, true, |_| Some(0));
        assert_eq!(t.active_seconds(), 60 * T);
    }

    #[test]
    fn an_unknown_idle_clock_never_pauses() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        run(&mut t, 0, 200, true, |_| None);
        assert_eq!(t.active_seconds(), 200 * T);
    }

    #[test]
    fn a_threshold_of_zero_turns_the_pause_off() {
        let mut t = Tracker::new(Mode::Process, 0, 0);
        run(&mut t, 0, 100, true, Some);
        assert_eq!(t.active_seconds(), 100 * T);
    }

    #[test]
    fn a_machine_that_slept_between_ticks_adds_nothing() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        let now = run(&mut t, 0, 4, true, |_| Some(0));
        t.tick(now + 8 * 3600, true, Some(0));
        assert_eq!(t.active_seconds(), 4 * T);
    }

    #[test]
    fn window_mode_counts_without_a_process() {
        let mut t = Tracker::new(Mode::Window, 0, 10);
        run(&mut t, 0, 10, false, |_| Some(0));
        assert_eq!(t.active_seconds(), 10 * T);
    }

    #[test]
    fn falling_back_counts_from_the_switch() {
        let mut t = Tracker::new(Mode::Process, 0, 10);
        assert_eq!(t.tick(APPEAR_GRACE_SECONDS, false, None), Tick::NeverSeen);
        t.fall_back_to_window(APPEAR_GRACE_SECONDS);
        run(&mut t, APPEAR_GRACE_SECONDS, 4, false, |_| Some(0));
        assert_eq!(t.active_seconds(), 4 * T);
    }

    #[test]
    fn steam_targets_give_their_app_id() {
        assert_eq!(
            steam_app_id("steam://rungameid/413150").as_deref(),
            Some("413150")
        );
        assert_eq!(steam_app_id("Steam://Run/620").as_deref(), Some("620"));
        assert_eq!(steam_app_id("steam://rungameid/"), None);
        assert_eq!(steam_app_id("heroic://launch/x"), None);
    }

    #[test]
    fn a_program_on_the_cartridge_is_watched_by_its_own_folder() {
        let root = Path::new("/run/media/you/CART");
        assert_eq!(
            watch_dirs(root, "Games/Tunic/Tunic.exe", None),
            vec![root.join("Games").join("Tunic")]
        );
        assert_eq!(
            watch_dirs(root, "Games/Tunic/bin/x64/Tunic.exe", None),
            vec![root.join("Games").join("Tunic")]
        );
        assert_eq!(
            watch_dirs(root, "Tunic/Tunic.exe", None),
            vec![root.join("Tunic")]
        );
        assert_eq!(watch_dirs(root, "start.sh", None), vec![root.to_path_buf()]);
        assert_eq!(
            watch_dirs(root, "playnite://playnite/start/x", None),
            vec![root.to_path_buf()]
        );
    }

    #[test]
    fn a_steam_game_on_the_cartridge_is_watched_in_its_install_folder() {
        let scratch = crate::testutil::Scratch::new("playtrack-steam");
        let root = scratch.path();
        let steamapps = crate::steamlib::library_root(root).join("steamapps");
        std::fs::create_dir_all(steamapps.join("common").join("Stardew Valley")).unwrap();
        std::fs::write(
            steamapps.join("appmanifest_413150.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"413150\"\n\t\"installdir\"\t\t\"Stardew Valley\"\n}\n",
        )
        .unwrap();
        assert_eq!(
            watch_dirs(root, "steam://rungameid/413150", None),
            vec![steamapps.join("common").join("Stardew Valley")]
        );
        // Not on the cartridge and no Steam to ask: the cartridge itself.
        assert_eq!(
            watch_dirs(root, "steam://rungameid/1", None),
            vec![root.to_path_buf()]
        );
    }
}
