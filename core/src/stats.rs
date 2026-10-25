//! What a cartridge remembers about being played.
//!
//! `.gamepak/stats.json`, written on the drive rather than on the host. That
//! placement is the whole feature: a cartridge carried between a desktop, a
//! Steam Deck and somebody else's machine keeps one count of how often it has
//! been started and how long it has been played, the way a save file on a
//! memory card does. Per-host statistics already exist — Steam has them — and
//! they are exactly what a cartridge cannot use.
//!
//! Three rules, all of them consequences of living on removable media:
//!
//! 1. **Never fail a launch.** Every write here is best-effort. A cartridge
//!    mounted read-only, a full drive, a file some other tool has locked: the
//!    game still starts and the count is simply not kept. Callers log the
//!    error and carry on.
//! 2. **Write whole files.** A new file beside the old one, then a rename, so
//!    a drive pulled mid-write loses the update rather than the history.
//! 3. **Count the launch before the session.** A crash, a hard power-off, or a
//!    cartridge yanked out of the port takes the duration with it; the launch
//!    is already on the drive by then. Undercounting hours is honest.
//!    Inventing them is not.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The file this module owns, under [`crate::create::ASSET_DIR`].
pub const STATS_FILE: &str = "stats.json";

/// Bumped only if the shape changes in a way an older reader cannot survive.
/// Readers accept anything they can parse; this exists so a future format can
/// be told apart from this one rather than silently misread.
pub const STATS_VERSION: u32 = 1;

/// How often an open session re-stamps itself on the drive.
///
/// Taken from Kazeta, which does the same thing for the same reason: it writes
/// `playtime_end` every sixty seconds while a game runs, so a crash or a
/// yanked drive costs a minute rather than the whole session. This project
/// counted the launch before the game started and called losing the hours
/// "honest undercounting", which it was — and a minute of loss is more honest
/// still for one small write a minute.
///
/// Sixty seconds, not less: this lands on removable flash, and a cartridge
/// being written to every second for an eight-hour session is a cost nobody
/// asked for to sharpen a number nobody reads to the minute.
pub const HEARTBEAT_SECONDS: u64 = 60;

/// The longest a single session is allowed to contribute.
///
/// Wall clock is the only clock available — a game is a process the launcher
/// does not own, and on Steam it is not even a child — so a machine suspended
/// with the launcher open would otherwise wake up and add a week to the total.
/// Sixteen hours is longer than anyone's actual sitting and shorter than any
/// plausible suspend.
pub const MAX_SESSION_SECONDS: u64 = 16 * 60 * 60;

/// Everything one cartridge has recorded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Stats {
    pub version: u32,
    /// Keyed by [`key_for`] over the game's `executable`, so the same game is
    /// the same row on every machine the cartridge is plugged into.
    ///
    /// A `BTreeMap` rather than a `HashMap` because this gets serialised: an
    /// ordered file produces a stable diff, and a stats file that reshuffles
    /// itself on every launch is one that looks corrupt to anyone reading it.
    pub games: BTreeMap<String, GameStats>,
    /// The session that is running now, if one is.
    ///
    /// On the drive rather than in the launcher's memory, because the whole
    /// point is to survive the launcher not getting to finish. A record left
    /// here by a run that died is picked up by [`recover`] and turned into the
    /// hours it managed before it went.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_session: Option<OpenSession>,
}

/// A session in progress, as the drive records it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenSession {
    /// Which game, keyed as [`key_for`] keys it.
    pub key: String,
    /// Unix seconds when Play was pressed.
    pub started: u64,
    /// Unix seconds at the last heartbeat. This is the number that decides how
    /// much a crashed session is credited with.
    pub heartbeat: u64,
    /// Which machine had it open. Cosmetic, and useful in exactly the case
    /// that is hardest to reason about later: a cartridge pulled out of one PC
    /// and plugged into another, where the second one settles the first one's
    /// session.
    pub host: String,
    /// Seconds actually played so far, as the tracker counts them — the game
    /// seen running and somebody at the controls. Absent in a record written
    /// before the tracker existed, which settles the old way, by wall clock.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub played: Option<u64>,
}

/// One finished session, for the history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionRecord {
    /// Unix seconds when Play was pressed.
    pub started: u64,
    /// Seconds played.
    pub seconds: u64,
    /// Which machine.
    pub host: String,
}

/// How many sessions a game keeps in the stats file. Newest are kept.
pub const MAX_HISTORY: usize = 200;

/// One game's history, across every host that has played it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GameStats {
    /// Last title seen for this executable. Kept so a stats file read on its
    /// own says something; the cartridge's own `cartridge.conf` is the truth.
    pub title: String,
    /// How many times Play has been pressed.
    pub launches: u64,
    /// Total seconds played, summed over completed sessions only.
    pub seconds: u64,
    /// Unix seconds, first launch ever recorded. Zero if unknown.
    pub first_played: u64,
    /// Unix seconds, most recent launch. Zero if unknown.
    pub last_played: u64,
    /// The machine that recorded `last_played`, so "where did I leave off" has
    /// an answer. Best-effort and cosmetic: an empty string is fine.
    pub last_host: String,
    /// Finished sessions, oldest first, capped at [`MAX_HISTORY`].
    pub sessions: Vec<SessionRecord>,
}

impl GameStats {
    /// Add a finished session to the total and the history.
    fn add_session(&mut self, started: u64, seconds: u64, host: &str) {
        self.seconds = self.seconds.saturating_add(seconds);
        self.sessions.push(SessionRecord {
            started,
            seconds,
            host: host.to_string(),
        });
        let excess = self.sessions.len().saturating_sub(MAX_HISTORY);
        if excess > 0 {
            self.sessions.drain(..excess);
        }
    }
}

/// A launch that has been counted and whose duration is still running.
///
/// Returned by [`record_launch`] and handed back to [`record_session_end`].
/// Holding the root path inside means the caller cannot accidentally close a
/// session against a different cartridge.
#[derive(Debug, Clone)]
pub struct Session {
    root: PathBuf,
    key: String,
    started_unix: u64,
}

impl Session {
    /// Which game this session is for, as it is keyed in the file.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Unix seconds at which Play was pressed.
    pub fn started_unix(&self) -> u64 {
        self.started_unix
    }
}

/// Where the stats file lives for a given cartridge root.
pub fn stats_path(root: &Path) -> PathBuf {
    root.join(crate::create::ASSET_DIR).join(STATS_FILE)
}

/// The identity of a game, stable across the machines a cartridge visits.
///
/// Two things differ between hosts for the same `executable` line and neither
/// is a different game:
///
/// * **Separators.** A cartridge written on Windows says `Games\Foo\Foo.exe`;
///   read on Linux the same line is a path with backslashes in it. Normalising
///   to `/` keeps one row instead of two.
/// * **URI case.** `Steam://RunGameID/620` and `steam://rungameid/620` are the
///   same request to the same handler.
///
/// Case is only folded for URIs. A path is left alone apart from its
/// separators, because two files on a case-sensitive filesystem really can
/// differ by case and collapsing them would merge two games into one row.
pub fn key_for(executable: &str) -> String {
    let trimmed = executable.trim().replace('\\', "/");
    if is_uri(&trimmed) {
        trimmed.to_lowercase()
    } else {
        trimmed
    }
}

/// Whether a launch target names a scheme rather than a file on the drive.
///
/// Deliberately structural — `scheme://` — rather than a list of the schemes
/// this project happens to know. The launcher keeps that list because it
/// decides what to *run*; keying a row only needs to know whether case is
/// significant, and no scheme anywhere is case-sensitive.
fn is_uri(value: &str) -> bool {
    let Some(sep) = value.find("://") else {
        return false;
    };
    // A Windows path can contain `//` after a drive letter but never `x://`
    // with a legal scheme in front of it: schemes start with a letter and
    // continue with letters, digits, `+`, `-` or `.`, and are at least two
    // characters, which "C" is not.
    let scheme = &value[..sep];
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Read what the cartridge has recorded, or an empty set.
///
/// A missing file and an unreadable one are the same answer on purpose: this
/// is called to draw a badge, and a cartridge with no history and a cartridge
/// whose history cannot be read both have nothing to show. Corruption is not
/// repaired here — the next write replaces the file wholesale.
pub fn read(root: &Path) -> Stats {
    let Ok(text) = std::fs::read_to_string(stats_path(root)) else {
        return Stats::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// One game's history, or zeroes.
pub fn for_game(root: &Path, executable: &str) -> GameStats {
    read(root)
        .games
        .remove(&key_for(executable))
        .unwrap_or_default()
}

/// Count a launch, and open a session for the duration.
///
/// The count lands on the drive before the game starts, which is the ordering
/// that survives a crash. The returned [`Session`] is only needed to add the
/// duration later; dropping it loses the hours and keeps the launch.
pub fn record_launch(root: &Path, executable: &str, title: &str) -> Result<Session, String> {
    let key = key_for(executable);
    let now = now_unix();
    let host = host_name();

    let mut stats = read(root);
    // Settle whatever the last run left behind before opening a new session,
    // or opening this one would overwrite it and lose those hours for good.
    let _ = settle(&mut stats);
    let entry = stats.games.entry(key.clone()).or_default();
    if !title.trim().is_empty() {
        entry.title = title.trim().to_string();
    }
    entry.launches = entry.launches.saturating_add(1);
    if entry.first_played == 0 {
        entry.first_played = now;
    }
    entry.last_played = now;
    entry.last_host = host.clone();

    stats.open_session = Some(OpenSession {
        key: key.clone(),
        started: now,
        heartbeat: now,
        host,
        played: None,
    });
    write(root, &stats)?;

    Ok(Session {
        root: root.to_path_buf(),
        key,
        started_unix: now,
    })
}

/// Re-stamp the open session, so a crash after this point keeps the hours up to
/// it.
///
/// Called every [`HEARTBEAT_SECONDS`] while a game is running. Cheap, and
/// deliberately not fussy: a cartridge that has gone, or whose record no longer
/// names this session, is not an error worth surfacing — the session is simply
/// no longer being counted.
pub fn touch_session(session: &Session) -> Result<(), String> {
    touch(session, None)
}

/// A heartbeat that also records how much of the session was play.
///
/// What the tracker calls: a crash after this point is settled with the
/// seconds counted here, not the wall-clock time since Play — which would count
/// the hour the game sat paused as an hour played.
pub fn touch_session_played(session: &Session, played: u64) -> Result<(), String> {
    touch(session, Some(played))
}

fn touch(session: &Session, played: Option<u64>) -> Result<(), String> {
    let mut stats = read(&session.root);
    let Some(open) = stats.open_session.as_mut() else {
        return Ok(());
    };
    if open.key != session.key || open.started != session.started_unix {
        // Something else opened a session over this one. Leave it alone.
        return Ok(());
    }
    open.heartbeat = now_unix();
    if played.is_some() {
        open.played = played;
    }
    write(&session.root, &stats)
}

/// Turn a session left behind by a run that died into the hours it managed.
///
/// Call this when a cartridge turns up — the launcher does, on insert. A session
/// that ended properly left nothing here; one that did not left a start and a
/// last heartbeat, and the difference is what the machine got through before it
/// went. Returns the seconds credited, and `None` when there was nothing to
/// settle, which is the ordinary case.
///
/// Up to [`HEARTBEAT_SECONDS`] of real play is still lost, because the last
/// heartbeat is the last thing known to be true. That is the trade the whole
/// mechanism makes.
pub fn recover(root: &Path) -> Option<u64> {
    let mut stats = read(root);
    let credited = settle(&mut stats)?;
    // Written even when `credited` is zero: the stale record has to go, or
    // every later read keeps finding it.
    write(root, &stats).ok()?;
    Some(credited)
}

/// Fold an abandoned session into the totals and clear it.
///
/// Shared by [`recover`] and [`record_launch`] so a launch cannot quietly
/// discard the previous run's session by overwriting the record.
fn settle(stats: &mut Stats) -> Option<u64> {
    let open = stats.open_session.take()?;
    let played = open
        .played
        .unwrap_or_else(|| open.heartbeat.saturating_sub(open.started))
        .min(MAX_SESSION_SECONDS);
    if played > 0 {
        let entry = stats.games.entry(open.key).or_default();
        entry.add_session(open.started, played, &open.host);
    }
    Some(played)
}

/// Add a finished session's duration to the total.
///
/// Clamped at both ends. A clock that went backwards between launch and exit —
/// an NTP correction, a dual-boot machine whose other operating system keeps
/// local time in the RTC — yields nothing rather than a negative number wrapped
/// around a `u64`, and no single session may contribute more than
/// [`MAX_SESSION_SECONDS`].
pub fn record_session_end(session: &Session) -> Result<u64, String> {
    let played = now_unix().saturating_sub(session.started_unix);
    record_session_played(session, played)
}

/// Close a session with the seconds the tracker counted, rather than the wall
/// clock since Play.
pub fn record_session_played(session: &Session, played: u64) -> Result<u64, String> {
    let played = played.min(MAX_SESSION_SECONDS);
    if played == 0 {
        return Ok(0);
    }

    let mut stats = read(&session.root);
    // This session is being closed properly, so its own open record is spent
    // rather than something to recover. Clearing it before adding the seconds
    // is what stops the duration being counted twice.
    //
    // Matched on the game alone, not on the start time as well. Two sessions
    // for one game cannot overlap — starting a game settles the previous
    // session first — and a stricter match would leave the record behind
    // whenever the two disagreed, which then gets recovered as a second helping
    // of the same hours. If this does clear a record that was not strictly
    // ours, the cost is a session that would have to be recovered losing its
    // crash insurance; the cost the other way is counting time twice, and
    // inventing hours is the one thing this file must not do.
    if stats
        .open_session
        .as_ref()
        .is_some_and(|open| open.key == session.key)
    {
        stats.open_session = None;
    }
    let entry = stats.games.entry(session.key.clone()).or_default();
    entry.add_session(session.started_unix, played, &host_name());
    write(&session.root, &stats)?;
    Ok(played)
}

/// Replace the stats file, whole.
///
/// Written beside itself and renamed into place, so the failure mode of a
/// cartridge pulled mid-write is a lost update rather than a truncated file
/// that reads as an empty history. The temporary file shares the directory
/// because a rename across filesystems is not atomic — and on Windows, not
/// a rename at all.
pub fn write(root: &Path, stats: &Stats) -> Result<(), String> {
    let path = stats_path(root);
    let dir = path
        .parent()
        .ok_or_else(|| "stats path has no parent".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let mut out = stats.clone();
    out.version = STATS_VERSION;
    let text = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;

    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    // Windows will not rename onto an existing file; every other platform
    // will. Removing first opens a window where neither file is in place,
    // which is why the temporary one is written first and kept until it lands.
    #[cfg(windows)]
    let _ = std::fs::remove_file(&path);
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("{}: {e}", path.display()))
        }
    }
}

/// Seconds since the epoch, or zero on a clock that predates it.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What to call this machine in `last_host`.
///
/// Environment first, because it costs nothing and is set on both Windows and
/// most shells; `/etc/hostname` after that for the Linux case where it is not.
/// No subprocess: this runs on every launch, and spawning `hostname` to
/// decorate a tooltip is not a trade worth making.
pub fn host_name() -> String {
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(value) = std::env::var(var) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    #[test]
    fn a_windows_path_and_a_linux_path_are_one_game() {
        assert_eq!(key_for("Games\\Foo\\Foo.exe"), key_for("Games/Foo/Foo.exe"));
    }

    #[test]
    fn uri_case_does_not_split_a_row() {
        assert_eq!(
            key_for("Steam://RunGameID/620"),
            key_for("steam://rungameid/620")
        );
    }

    #[test]
    fn path_case_does_split_a_row() {
        // Two real files on ext4. Folding these together would merge two games.
        assert_ne!(key_for("Games/foo.sh"), key_for("Games/FOO.sh"));
    }

    #[test]
    fn a_drive_letter_is_not_a_scheme() {
        assert!(!is_uri("C://Games/Foo.exe"));
        assert!(is_uri("steam://rungameid/620"));
        assert!(is_uri("ms-settings://x"));
    }

    #[test]
    fn a_cartridge_with_no_history_reads_as_empty() {
        let scratch = Scratch::new("stats-empty");
        let stats = read(scratch.path());
        assert_eq!(stats.games.len(), 0);
        assert_eq!(for_game(scratch.path(), "steam://rungameid/1").launches, 0);
    }

    #[test]
    fn a_corrupt_file_reads_as_empty_rather_than_failing() {
        let scratch = Scratch::new("stats-corrupt");
        scratch.write(".gamepak/stats.json", b"{ this is not json");
        assert_eq!(read(scratch.path()).games.len(), 0);
    }

    #[test]
    fn a_crash_is_settled_with_what_was_played_not_what_elapsed() {
        // Two hours on the clock, of which forty minutes were play: the game
        // sat paused for the rest, and the tracker said so at its heartbeat.
        let scratch = Scratch::new("stats-played");
        let session = record_launch(scratch.path(), "x://1", "X").expect("record");
        touch_session_played(&session, 2400).expect("heartbeat");
        ran_for(scratch.path(), 7200);
        drop(session);

        assert_eq!(recover(scratch.path()), Some(2400));
        assert_eq!(for_game(scratch.path(), "x://1").seconds, 2400);
    }

    #[test]
    fn every_finished_session_goes_into_the_history() {
        let scratch = Scratch::new("stats-history");
        let first = record_launch(scratch.path(), "x://1", "X").expect("record");
        record_session_played(&first, 600).expect("close");
        let second = record_launch(scratch.path(), "x://1", "X").expect("record");
        record_session_played(&second, 1200).expect("close");

        let game = for_game(scratch.path(), "x://1");
        assert_eq!(game.seconds, 1800);
        let lengths: Vec<u64> = game.sessions.iter().map(|s| s.seconds).collect();
        assert_eq!(lengths, vec![600, 1200]);
        assert_eq!(game.sessions[0].started, first.started_unix());
    }

    #[test]
    fn the_history_keeps_the_newest() {
        let mut game = GameStats::default();
        for i in 0..(MAX_HISTORY as u64 + 5) {
            game.add_session(i, 1, "h");
        }
        assert_eq!(game.sessions.len(), MAX_HISTORY);
        assert_eq!(game.sessions[0].started, 5);
        assert_eq!(
            game.seconds,
            MAX_HISTORY as u64 + 5,
            "the total keeps everything"
        );
    }

    #[test]
    fn the_launch_is_counted_before_the_game_starts() {
        let scratch = Scratch::new("stats-launch");
        let session =
            record_launch(scratch.path(), "steam://rungameid/620", "Portal 2").expect("record");

        // On the drive already, with no session end anywhere in sight.
        let entry = for_game(scratch.path(), "steam://rungameid/620");
        assert_eq!(entry.launches, 1);
        assert_eq!(entry.title, "Portal 2");
        assert_eq!(entry.seconds, 0);
        assert!(entry.first_played > 0);
        assert_eq!(entry.first_played, entry.last_played);
        assert_eq!(session.key(), "steam://rungameid/620");
    }

    #[test]
    fn launches_accumulate_and_first_played_does_not_move() {
        let scratch = Scratch::new("stats-repeat");
        let first = record_launch(scratch.path(), "steam://rungameid/620", "Portal 2")
            .expect("record")
            .started_unix();
        record_launch(scratch.path(), "steam://rungameid/620", "Portal 2").expect("record");
        record_launch(scratch.path(), "steam://rungameid/620", "Portal 2").expect("record");

        let entry = for_game(scratch.path(), "steam://rungameid/620");
        assert_eq!(entry.launches, 3);
        assert_eq!(entry.first_played, first);
    }

    #[test]
    fn a_session_that_dies_before_its_first_heartbeat_keeps_its_launch() {
        let scratch = Scratch::new("stats-crash");
        let session = record_launch(scratch.path(), "Games/Foo/Foo.exe", "Foo").expect("record");
        drop(session); // the machine lost power here, inside the first minute

        let entry = for_game(scratch.path(), "Games/Foo/Foo.exe");
        assert_eq!(entry.launches, 1);
        assert_eq!(entry.seconds, 0, "nothing was known to be true yet");
        // And the record it left behind settles to nothing rather than lingering.
        assert_eq!(recover(scratch.path()), Some(0));
        assert!(read(scratch.path()).open_session.is_none());
    }

    /// Move an open session's start back from its heartbeat by that duration.
    ///
    /// The alternative is a test that sleeps for an hour.
    fn ran_for(root: &Path, seconds: u64) {
        let mut stats = read(root);
        let open = stats.open_session.as_mut().expect("a session is open");
        open.started = open.heartbeat.saturating_sub(seconds);
        write(root, &stats).expect("write");
    }

    #[test]
    fn a_heartbeat_lets_a_crashed_session_keep_its_hours() {
        // The whole point of the mechanism: the launcher never gets to close
        // this session, and the hours survive anyway.
        let scratch = Scratch::new("stats-heartbeat");
        let session =
            record_launch(scratch.path(), "steam://rungameid/620", "Portal 2").expect("record");
        touch_session(&session).expect("heartbeat");
        ran_for(scratch.path(), 7200);
        drop(session); // power cut, two hours in

        assert_eq!(recover(scratch.path()), Some(7200));
        assert_eq!(
            for_game(scratch.path(), "steam://rungameid/620").seconds,
            7200
        );
    }

    #[test]
    fn a_recovered_session_is_only_credited_once() {
        let scratch = Scratch::new("stats-recover-once");
        let session = record_launch(scratch.path(), "steam://rungameid/1", "X").expect("record");
        touch_session(&session).expect("heartbeat");
        ran_for(scratch.path(), 600);
        drop(session);

        assert_eq!(recover(scratch.path()), Some(600));
        assert_eq!(recover(scratch.path()), None, "nothing left to settle");
        assert_eq!(for_game(scratch.path(), "steam://rungameid/1").seconds, 600);
    }

    /// Close a session whose start was moved `pretend` seconds into the past.
    ///
    /// `record_session_end` reads the wall clock in whole seconds, so a test
    /// that crosses a second boundary between launch and close (a slow CI
    /// runner does) sees a second or two more than it pretended. The tests
    /// care that the time is about right and is then counted exactly once,
    /// so they check the stored hours against what this returns.
    fn close_after(session: &Session, pretend: u64) -> u64 {
        let added = record_session_end(session).expect("close");
        assert!(
            (pretend..=pretend + 2).contains(&added),
            "closed with {added}s, not about {pretend}s"
        );
        added
    }

    #[test]
    fn a_session_closed_properly_leaves_nothing_to_recover() {
        let scratch = Scratch::new("stats-clean-close");
        let mut session =
            record_launch(scratch.path(), "steam://rungameid/1", "X").expect("record");
        touch_session(&session).expect("heartbeat");
        session.started_unix -= 1800;

        let added = close_after(&session, 1800);
        assert_eq!(recover(scratch.path()), None, "the record was spent");
        // And the duration was counted once, not once here and once on recovery.
        assert_eq!(
            for_game(scratch.path(), "steam://rungameid/1").seconds,
            added
        );
    }

    #[test]
    fn starting_a_second_game_settles_the_first_rather_than_discarding_it() {
        // A new launch overwrites the open record. Without settling first, the
        // previous run's hours would go with it.
        let scratch = Scratch::new("stats-settle-on-launch");
        let first = record_launch(scratch.path(), "steam://rungameid/1", "One").expect("record");
        touch_session(&first).expect("heartbeat");
        ran_for(scratch.path(), 900);

        record_launch(scratch.path(), "steam://rungameid/2", "Two").expect("record");
        assert_eq!(for_game(scratch.path(), "steam://rungameid/1").seconds, 900);
        assert_eq!(
            read(scratch.path()).open_session.unwrap().key,
            "steam://rungameid/2"
        );
    }

    #[test]
    fn a_heartbeat_for_a_session_that_has_been_replaced_changes_nothing() {
        let scratch = Scratch::new("stats-stale-heartbeat");
        let stale = record_launch(scratch.path(), "steam://rungameid/1", "One").expect("record");
        let current = record_launch(scratch.path(), "steam://rungameid/2", "Two").expect("record");

        touch_session(&stale).expect("stale heartbeat");
        let open = read(scratch.path()).open_session.expect("still open");
        assert_eq!(
            open.key,
            current.key(),
            "the stale one must not steal the record"
        );
    }

    #[test]
    fn a_recovered_session_is_capped_like_any_other() {
        let scratch = Scratch::new("stats-recover-cap");
        let session = record_launch(scratch.path(), "steam://rungameid/1", "X").expect("record");
        touch_session(&session).expect("heartbeat");
        ran_for(scratch.path(), 7 * 24 * 60 * 60);
        drop(session);

        assert_eq!(recover(scratch.path()), Some(MAX_SESSION_SECONDS));
    }

    #[test]
    fn a_cartridge_with_nothing_running_says_so_rather_than_writing() {
        let scratch = Scratch::new("stats-no-session");
        record_launch(scratch.path(), "steam://rungameid/1", "X").expect("record");
        recover(scratch.path()); // clears the one just opened
                                 // A stats file with no session in it does not carry an empty record.
        let text = std::fs::read_to_string(stats_path(scratch.path())).expect("read");
        assert!(!text.contains("openSession"), "{text}");
    }

    #[test]
    fn a_session_left_by_another_machine_is_settled_here() {
        // The cartridge was pulled out of one PC mid-game and plugged into
        // another. The hours the first one managed are the cartridge's.
        let scratch = Scratch::new("stats-other-host");
        let mut stats = Stats::default();
        let now = now_unix();
        stats.open_session = Some(OpenSession {
            key: "steam://rungameid/620".to_string(),
            started: now - 3600,
            heartbeat: now - 60,
            host: "workshop".to_string(),
            played: None,
        });
        write(scratch.path(), &stats).expect("seed");

        assert_eq!(recover(scratch.path()), Some(3540));
        assert_eq!(
            for_game(scratch.path(), "steam://rungameid/620").seconds,
            3540
        );
    }

    #[test]
    fn a_closed_session_adds_its_duration() {
        let scratch = Scratch::new("stats-duration");
        let mut session =
            record_launch(scratch.path(), "steam://rungameid/620", "Portal 2").expect("record");
        // Pretend Play was pressed an hour ago rather than sleeping for one.
        session.started_unix -= 3600;

        let added = close_after(&session, 3600);
        assert_eq!(
            for_game(scratch.path(), "steam://rungameid/620").seconds,
            added
        );
    }

    #[test]
    fn a_suspended_machine_cannot_add_a_week() {
        let scratch = Scratch::new("stats-suspend");
        let mut session =
            record_launch(scratch.path(), "steam://rungameid/1", "X").expect("record");
        session.started_unix -= 7 * 24 * 60 * 60;

        assert_eq!(
            record_session_end(&session).expect("close"),
            MAX_SESSION_SECONDS
        );
    }

    #[test]
    fn a_clock_that_went_backwards_adds_nothing() {
        let scratch = Scratch::new("stats-backwards");
        let mut session =
            record_launch(scratch.path(), "steam://rungameid/1", "X").expect("record");
        session.started_unix += 10_000; // launched "in the future"

        assert_eq!(record_session_end(&session).expect("close"), 0);
        assert_eq!(for_game(scratch.path(), "steam://rungameid/1").seconds, 0);
    }

    #[test]
    fn two_games_on_one_cartridge_are_counted_apart() {
        let scratch = Scratch::new("stats-two");
        record_launch(scratch.path(), "steam://rungameid/620", "Portal 2").expect("record");
        record_launch(scratch.path(), "steam://rungameid/400", "Portal").expect("record");
        record_launch(scratch.path(), "steam://rungameid/400", "Portal").expect("record");

        let stats = read(scratch.path());
        assert_eq!(stats.games.len(), 2);
        assert_eq!(stats.games["steam://rungameid/620"].launches, 1);
        assert_eq!(stats.games["steam://rungameid/400"].launches, 2);
    }

    #[test]
    fn history_written_on_another_machine_is_added_to_not_replaced() {
        let scratch = Scratch::new("stats-carried");
        // What the cartridge arrives holding, written by some other host.
        let mut carried = Stats::default();
        carried.games.insert(
            "steam://rungameid/620".to_string(),
            GameStats {
                title: "Portal 2".into(),
                launches: 9,
                seconds: 36_000,
                first_played: 1_700_000_000,
                last_played: 1_700_003_600,
                last_host: "deck".into(),
                sessions: Vec::new(),
            },
        );
        write(scratch.path(), &carried).expect("seed");

        let mut session =
            record_launch(scratch.path(), "steam://rungameid/620", "Portal 2").expect("record");
        session.started_unix -= 60;
        let added = close_after(&session, 60);

        let entry = for_game(scratch.path(), "steam://rungameid/620");
        assert_eq!(
            entry.launches, 10,
            "the other machine's nine are still there"
        );
        assert_eq!(entry.seconds, 36_000 + added);
        assert_eq!(entry.first_played, 1_700_000_000, "first play is not today");
        assert!(entry.last_played > 1_700_003_600);
    }

    #[test]
    fn writing_leaves_no_temporary_file_behind() {
        let scratch = Scratch::new("stats-tmp");
        record_launch(scratch.path(), "steam://rungameid/1", "X").expect("record");

        let dir = scratch.join(".gamepak");
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn the_file_names_its_own_version() {
        let scratch = Scratch::new("stats-version");
        record_launch(scratch.path(), "steam://rungameid/1", "X").expect("record");
        let text = std::fs::read_to_string(stats_path(scratch.path())).expect("read");
        assert!(text.contains("\"version\": 1"), "{text}");
    }

    #[test]
    fn a_read_only_cartridge_reports_rather_than_panicking() {
        // A path that cannot be created: the parent is a file, not a directory.
        let scratch = Scratch::new("stats-readonly");
        scratch.write("root", b"not a directory");
        let err = record_launch(&scratch.join("root"), "steam://rungameid/1", "X")
            .expect_err("must not succeed");
        assert!(!err.is_empty());
    }
}
