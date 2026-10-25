//! Writing slower than the drive can manage, on purpose.
//!
//! An M.2 NVMe in an aluminium USB enclosure has nowhere to put its heat. A
//! cartridge is written in one sustained pass — tens of gigabytes at whatever
//! the link will carry — which is the worst thermal case a stick like that ever
//! sees, and hotter than anything it was specified for. What follows is the
//! bridge chip resetting the bus, which is the failure this project has already
//! recorded twice on real hardware: nineteen `UASPStor` resets across one
//! 107 GB write, and three files that came off the drive with the wrong
//! contents.
//!
//! That was read at the time as a cable and a port, and the cable and the port
//! were genuinely bad. Heat is the other half, and it is the half a program can
//! actually do something about: the drive cannot be made to dissipate more, but
//! it can be given less to dissipate.
//!
//! So the copy can be capped. The cap is a whole-transfer average rather than a
//! per-chunk delay, so a burst that finishes early is allowed to — what matters
//! is the sustained figure the drive has to shed, not the shape of any one
//! second.
//!
//! Off by default. A drive with a heatsink, or one being written over a link too
//! slow to trouble it, should not be slowed down for a problem it does not have.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Megabytes per second the copy is allowed to average. Zero means no cap.
///
/// Global because it describes the machine and the drive rather than any one
/// file: every copy in a single write shares the same enclosure, and threading
/// the number through each of them would say otherwise.
static LIMIT_MB_S: AtomicU64 = AtomicU64::new(0);

/// Cap the copy rate. Zero lifts the cap.
pub fn set_limit_mb_s(limit: u64) {
    LIMIT_MB_S.store(limit, Ordering::Relaxed);
}

/// The cap in force, if any.
pub fn limit_mb_s() -> u64 {
    LIMIT_MB_S.load(Ordering::Relaxed)
}

/// Sleep long enough that `bytes` written since `started` averages the cap.
///
/// Called after each chunk. Returns immediately when there is no cap, when the
/// transfer is already slower than the cap, or when the wait would be too short
/// to be worth a syscall.
pub fn pace(bytes: u64, started: Instant) {
    let limit = limit_mb_s();
    if limit == 0 || bytes == 0 {
        return;
    }

    let allowed = Duration::from_secs_f64(bytes as f64 / (limit as f64 * 1_000_000.0));
    let spent = started.elapsed();
    if let Some(wait) = allowed.checked_sub(spent) {
        // Under a millisecond the sleep costs more than it saves, and the next
        // chunk will carry the debt anyway.
        if wait >= Duration::from_millis(1) {
            std::thread::sleep(wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cap_means_no_wait() {
        set_limit_mb_s(0);
        let started = Instant::now();
        pace(500_000_000, started);
        assert!(started.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn a_transfer_already_under_the_cap_is_not_slowed() {
        set_limit_mb_s(100);
        // 1 MB against a 100 MB/s cap is allowed 10ms; claiming it took a full
        // second means the copy is already far slower than the cap.
        let started = Instant::now() - Duration::from_secs(1);
        let before = Instant::now();
        pace(1_000_000, started);
        assert!(before.elapsed() < Duration::from_millis(50));
        set_limit_mb_s(0);
    }

    #[test]
    fn going_too_fast_waits() {
        set_limit_mb_s(50);
        // 10 MB at 50 MB/s is allowed 200ms, and none of it has been spent.
        let started = Instant::now();
        let before = Instant::now();
        pace(10_000_000, started);
        let waited = before.elapsed();
        set_limit_mb_s(0);
        assert!(
            waited >= Duration::from_millis(150),
            "expected to wait about 200ms, waited {waited:?}"
        );
    }

    #[test]
    fn the_limit_round_trips() {
        set_limit_mb_s(120);
        assert_eq!(limit_mb_s(), 120);
        set_limit_mb_s(0);
        assert_eq!(limit_mb_s(), 0);
    }
}
