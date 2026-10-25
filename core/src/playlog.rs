//! A cartridge's play history, written into its own `cartridge.conf`.
//!
//! `.gamepak/stats.json` is where the history is kept while it is being made —
//! the open session, the heartbeat, the crash insurance — and it stays the
//! source of truth. This copies the result into the conf, beside each game's
//! `title` and `executable`, so it can be read by anything that reads a
//! cartridge: a skin, a script, another front-end, or someone opening the file.
//!
//! ```ini
//! [game]
//! title=Hollow Knight
//! executable=steam://rungameid/367520
//! playtime=154800
//! launches=31
//! first_played=2026-03-02T19:04:11Z
//! last_played=2026-09-26T18:00:00Z
//! last_host=DESKTOP-7Q2
//! session=2026-09-26T18:00:00Z|5400|DESKTOP-7Q2
//! session=2026-09-24T20:11:02Z|3120|steamdeck
//! hltb_main=97200
//! ```
//!
//! A single-game cartridge carries the same keys at the top, with its own
//! `title` and `executable`. Times are seconds; dates are UTC in ISO 8601. The
//! `session` lines are newest first and capped at [`CONF_HISTORY`]; the stats
//! file keeps more.
//!
//! Only these keys are ever touched. Every other line — comments, `save=`
//! lines, anything typed in by hand — is left exactly where it was, and the
//! file is replaced whole, beside itself, so a drive pulled mid-write keeps
//! the old one.

use std::path::Path;

use crate::stats::{self, GameStats, Stats};

/// The keys this module owns and rewrites.
pub const PLAY_KEYS: [&str; 6] = [
    "playtime",
    "launches",
    "first_played",
    "last_played",
    "last_host",
    "session",
];

/// How many sessions go into the conf, newest first.
pub const CONF_HISTORY: usize = 30;

/// Copy what the stats file knows into `cartridge.conf`.
///
/// Best-effort in the same way the stats are: a cartridge with no conf, or one
/// that cannot be written, is left alone and the error handed back for a log.
pub fn mirror(root: &Path) -> Result<(), String> {
    let path = root.join("cartridge.conf");
    let Ok(conf) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let updated = apply(&conf, &stats::read(root));
    if updated == conf {
        return Ok(());
    }
    write_whole(&path, &updated)
}

/// The conf with each game's play keys replaced by what `stats` says.
pub fn apply(conf: &str, stats: &Stats) -> String {
    rewrite(conf, &is_play_key, &|owner| {
        stats
            .games
            .get(owner)
            .filter(|game| game.launches > 0 || game.seconds > 0)
            .map(render)
    })
}

/// Replace one family of keys in each game's section.
///
/// `owns` says which keys are being replaced. `lines_for` is handed each game's
/// key (as [`stats::key_for`] makes it) and returns its new lines, or `None` to
/// leave that section exactly as it is. The lines go after the section's own
/// keys and before the blank line that ends it, as `saves::preserve` puts
/// save lines, so the next section stays separated.
pub(crate) fn rewrite(
    conf: &str,
    owns: &dyn Fn(&str) -> bool,
    lines_for: &dyn Fn(&str) -> Option<Vec<String>>,
) -> String {
    let mut out = String::with_capacity(conf.len() + 512);
    for block in blocks(conf) {
        let Some(lines) = owner_of(&block).and_then(|owner| lines_for(&owner)) else {
            out.push_str(&block);
            continue;
        };
        let kept: String = block
            .split_inclusive('\n')
            .filter(|line| !key_of(line).is_some_and(|key| owns(&key)))
            .collect();
        let body = kept.trim_end_matches(['\n', '\r']);
        out.push_str(body);
        out.push('\n');
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        // One of the stripped newlines was put back above; the rest were the
        // blank lines between this section and the next.
        for _ in 1..kept[body.len()..].matches('\n').count() {
            out.push('\n');
        }
    }
    out
}

fn is_play_key(key: &str) -> bool {
    PLAY_KEYS.contains(&key)
}

/// The lowercased key of a `key=value` line.
fn key_of(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with(['#', ';', '[']) {
        return None;
    }
    let (key, _) = trimmed.split_once('=')?;
    Some(key.trim().to_lowercase())
}

fn render(game: &GameStats) -> Vec<String> {
    let mut lines = vec![
        format!("playtime={}", game.seconds),
        format!("launches={}", game.launches),
    ];
    if game.first_played > 0 {
        lines.push(format!("first_played={}", iso8601(game.first_played)));
    }
    if game.last_played > 0 {
        lines.push(format!("last_played={}", iso8601(game.last_played)));
    }
    let host = clean(&game.last_host);
    if !host.is_empty() {
        lines.push(format!("last_host={host}"));
    }
    for session in game.sessions.iter().rev().take(CONF_HISTORY) {
        lines.push(format!(
            "session={}|{}|{}",
            iso8601(session.started),
            session.seconds,
            clean(&session.host)
        ));
    }
    lines
}

/// A host name as a conf value: one line, and no `|` to split on.
fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\n' | '\r' | '|'))
        .collect::<String>()
        .trim()
        .to_string()
}

/// Split a conf into sections, each keeping its own trailing blank lines.
fn blocks(conf: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in conf.split_inclusive('\n') {
        if line.trim_start().starts_with('[') && !current.trim().is_empty() {
            out.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The game a section describes, keyed as the stats file keys it: the one
/// with an `executable`, which is a single game's top section or a `[game]`.
fn owner_of(block: &str) -> Option<String> {
    let first = block.lines().map(str::trim).find(|l| !l.is_empty())?;
    if first.starts_with('[') {
        let name = first
            .trim_start_matches('[')
            .split(']')
            .next()?
            .trim()
            .to_lowercase();
        if name != "game" {
            return None;
        }
    }
    block.lines().find_map(|line| {
        let line = line.trim();
        let (key, value) = line.split_once('=')?;
        (key.trim().eq_ignore_ascii_case("executable") && !value.trim().is_empty())
            .then(|| stats::key_for(value.trim()))
    })
}

/// Replace a file with new contents, beside itself first.
pub(crate) fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("conf.tmp");
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(windows)]
    let _ = std::fs::remove_file(path);
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// Unix seconds as `2026-09-26T18:00:00Z`.
///
/// Howard Hinnant's days-to-civil, which is exact for every date this will
/// ever see and saves a dependency on a date library for one format.
pub fn iso8601(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::SessionRecord;

    fn game(seconds: u64, launches: u64, sessions: &[(u64, u64)]) -> GameStats {
        GameStats {
            title: String::new(),
            launches,
            seconds,
            first_played: 1_700_000_000,
            last_played: 1_758_909_600,
            last_host: "DESKTOP".into(),
            sessions: sessions
                .iter()
                .map(|&(started, seconds)| SessionRecord {
                    started,
                    seconds,
                    host: "DESKTOP".into(),
                })
                .collect(),
        }
    }

    fn stats_with(key: &str, g: GameStats) -> Stats {
        let mut stats = Stats::default();
        stats.games.insert(key.to_string(), g);
        stats
    }

    #[test]
    fn dates_are_utc_iso_8601() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_758_909_600), "2025-09-26T18:00:00Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn a_single_game_gets_its_keys_at_the_top() {
        let conf = "# PC GamePak\n\ntitle=Hollow Knight\nexecutable=steam://rungameid/367520\ncover=.gamepak/cover.png\n";
        let out = apply(
            conf,
            &stats_with(
                "steam://rungameid/367520",
                game(5400, 2, &[(1_758_900_000, 3600), (1_758_909_600, 1800)]),
            ),
        );
        assert!(out.starts_with(conf.trim_end()), "{out}");
        assert!(out.contains("\nplaytime=5400\nlaunches=2\n"), "{out}");
        assert!(out.contains("last_played=2025-09-26T18:00:00Z\n"), "{out}");
        // Newest first.
        let first = out
            .find("session=2025-09-26T18:00:00Z|1800|DESKTOP")
            .unwrap();
        let second = out.find("|3600|DESKTOP").unwrap();
        assert!(first < second, "{out}");
    }

    #[test]
    fn rewriting_replaces_rather_than_repeats() {
        let conf = "title=X\nexecutable=Games/X/x.exe\n";
        let once = apply(conf, &stats_with("Games/X/x.exe", game(60, 1, &[(1, 60)])));
        let twice = apply(
            &once,
            &stats_with("Games/X/x.exe", game(120, 2, &[(1, 60), (2, 60)])),
        );
        assert_eq!(twice.matches("playtime=").count(), 1, "{twice}");
        assert!(twice.contains("playtime=120\n"));
        assert_eq!(twice.matches("session=").count(), 2);
    }

    #[test]
    fn each_game_in_a_collection_gets_its_own() {
        let conf = "[collection]\ntitle=Roguelikes\n\n[game]\ntitle=FTL\nexecutable=steam://rungameid/212680\n\n[game]\ntitle=Hades\nexecutable=steam://rungameid/1145360\nsave=Hades|{documents}/Saved Games/Hades\n";
        let mut stats = stats_with("steam://rungameid/212680", game(600, 1, &[(5, 600)]));
        stats
            .games
            .insert("steam://rungameid/1145360".into(), game(7200, 3, &[]));
        let out = apply(conf, &stats);

        let ftl = out.find("title=FTL").unwrap();
        let hades = out.find("title=Hades").unwrap();
        let ftl_time = out.find("playtime=600").unwrap();
        let hades_time = out.find("playtime=7200").unwrap();
        assert!(ftl < ftl_time && ftl_time < hades, "{out}");
        assert!(hades < hades_time, "{out}");
        // Nothing lands in the collection's own section, and the blank line
        // between sections and the save line survive.
        assert!(
            out.starts_with("[collection]\ntitle=Roguelikes\n\n[game]"),
            "{out}"
        );
        assert!(
            out.contains("session=1970-01-01T00:00:05Z|600|DESKTOP\n\n[game]"),
            "{out}"
        );
        assert!(
            out.contains("save=Hades|{documents}/Saved Games/Hades\nplaytime=7200"),
            "{out}"
        );
    }

    #[test]
    fn a_game_never_played_gets_nothing() {
        let conf = "title=X\nexecutable=x://1\n";
        assert_eq!(apply(conf, &Stats::default()), conf);
    }

    #[test]
    fn hand_written_lines_and_hltb_are_left_alone() {
        let conf = "; my notes\ntitle=X\nexecutable=x://1\nhltb_main=3600\n";
        let out = apply(conf, &stats_with("x://1", game(10, 1, &[])));
        assert!(out.contains("; my notes\n"));
        assert!(out.contains("hltb_main=3600\n"));
    }

    #[test]
    fn the_history_in_the_conf_is_capped() {
        let sessions: Vec<(u64, u64)> = (0..100).map(|i| (i * 1000, 60)).collect();
        let out = apply(
            "executable=x://1\n",
            &stats_with("x://1", game(6000, 100, &sessions)),
        );
        assert_eq!(out.matches("session=").count(), CONF_HISTORY);
    }

    #[test]
    fn a_host_cannot_break_the_line_it_is_on() {
        assert_eq!(clean("evil|host\nname"), "evilhostname");
    }

    #[test]
    fn mirror_writes_the_file_on_the_drive() {
        let scratch = crate::testutil::Scratch::new("playlog-mirror");
        let root = scratch.path();
        std::fs::write(root.join("cartridge.conf"), "title=X\nexecutable=x://1\n").unwrap();
        let session = stats::record_launch(root, "x://1", "X").unwrap();
        stats::record_session_played(&session, 900).unwrap();
        mirror(root).unwrap();
        let conf = std::fs::read_to_string(root.join("cartridge.conf")).unwrap();
        assert!(conf.contains("playtime=900\nlaunches=1\n"), "{conf}");
        assert!(!root.join("cartridge.conf.tmp").exists());
    }
}
