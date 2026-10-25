//! Reading the mount table, and deciding what changed.
//!
//! Kept here rather than in `gamepak-core` on purpose. Core links serde and
//! ureq, which is fine for a launcher that runs for ten seconds and exits, and
//! not fine for a process that is resident for the whole session. This is a few
//! dozen lines of parsing; the dependency would be megabytes.
//!
//! The rules for what counts as a cartridge match `core/src/drives.rs`, which is
//! the other half of the same idea: automount locations only, never a denylist.

use std::path::{Path, PathBuf};

/// Where desktops automount removable media. A mount anywhere else is somebody's
/// filesystem, not a cartridge.
const AUTOMOUNT_ROOTS: [&str; 3] = ["/media", "/run/media", "/mnt"];

/// One line of `/proc/self/mounts` — only the two fields this needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub point: PathBuf,
    pub fs_type: String,
}

/// Parse `/proc/self/mounts`.
pub fn parse(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let _device = fields.next()?;
            let point = fields.next()?;
            let fs_type = fields.next()?;
            Some(Mount {
                point: PathBuf::from(unescape(point)),
                fs_type: fs_type.to_string(),
            })
        })
        .collect()
}

/// `/proc/self/mounts` escapes spaces and friends as octal.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let digits: String = chars.clone().take(3).collect();
        match u32::from_str_radix(&digits, 8)
            .ok()
            .filter(|_| digits.len() == 3)
            .and_then(char::from_u32)
        {
            Some(decoded) => {
                out.push(decoded);
                for _ in 0..3 {
                    chars.next();
                }
            }
            None => out.push(c),
        }
    }
    out
}

/// Could a cartridge be here?
///
/// The filesystem type is checked as well as the location, because a bind mount
/// or an overlay under /mnt is not a drive somebody plugged in.
pub fn is_candidate(mount: &Mount) -> bool {
    if is_pseudo(&mount.fs_type) {
        return false;
    }
    AUTOMOUNT_ROOTS.iter().any(|root| {
        let root = Path::new(root);
        mount.point.starts_with(root) && mount.point != root
    })
}

fn is_pseudo(fs: &str) -> bool {
    matches!(
        fs,
        "proc"
            | "sysfs"
            | "devtmpfs"
            | "devpts"
            | "tmpfs"
            | "ramfs"
            | "cgroup"
            | "cgroup2"
            | "pstore"
            | "bpf"
            | "securityfs"
            | "debugfs"
            | "tracefs"
            | "configfs"
            | "fusectl"
            | "hugetlbfs"
            | "mqueue"
            | "autofs"
            | "binfmt_misc"
            | "efivarfs"
            | "squashfs"
            | "overlay"
            | "nsfs"
    )
}

/// Mount points that appeared, filtered to places a cartridge could be.
pub fn arrivals(before: &[Mount], after: &[Mount]) -> Vec<PathBuf> {
    changes(after, before)
}

/// Mount points that went away. Not filtered by filesystem type: by the time a
/// drive is yanked, all we have is the path it used to be at.
pub fn departures(before: &[Mount], after: &[Mount]) -> Vec<PathBuf> {
    changes(before, after)
}

fn changes(present: &[Mount], absent: &[Mount]) -> Vec<PathBuf> {
    present
        .iter()
        .filter(|mount| is_candidate(mount))
        .filter(|mount| !absent.iter().any(|other| other.point == mount.point))
        .map(|mount| mount.point.clone())
        .collect()
}

/// Is there a cartridge at this mount point?
///
/// Without this every USB stick would pop a window. `autorun.inf` counts because
/// the launcher reads one for a label and an icon, which is enough to show
/// something useful.
pub fn is_cartridge(root: &Path) -> bool {
    root.join("cartridge.conf").is_file()
        || root.join("memorycard.conf").is_file()
        || root.join("autorun.inf").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
/dev/sda2 / ext4 rw,relatime 0 0
tmpfs /run tmpfs rw,nosuid,nodev 0 0
/dev/sdb1 /run/media/harry/CINDER exfat rw,nosuid,nodev,relatime 0 0
/dev/sdc1 /run/media/harry/Backup\\040Drive ext4 rw,relatime 0 0
";

    fn sample() -> Vec<Mount> {
        parse(SAMPLE)
    }

    #[test]
    fn reads_the_mount_table() {
        let mounts = sample();
        assert_eq!(mounts.len(), 5);
        assert_eq!(mounts[1].point, PathBuf::from("/"));
        assert_eq!(mounts[3].fs_type, "exfat");
    }

    #[test]
    fn a_mount_point_with_a_space_survives() {
        let mounts = sample();
        assert_eq!(
            mounts[4].point,
            PathBuf::from("/run/media/harry/Backup Drive")
        );
    }

    #[test]
    fn only_automounted_real_filesystems_are_candidates() {
        let mounts = sample();
        let candidates: Vec<_> = mounts
            .iter()
            .filter(|m| is_candidate(m))
            .map(|m| m.point.clone())
            .collect();
        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/run/media/harry/CINDER"),
                PathBuf::from("/run/media/harry/Backup Drive"),
            ]
        );
    }

    #[test]
    fn the_automount_root_itself_is_not_a_cartridge() {
        // /run/media is often a tmpfs the desktop creates; a cartridge is always
        // a directory inside it.
        let root = Mount {
            point: PathBuf::from("/run/media"),
            fs_type: "ext4".to_string(),
        };
        assert!(!is_candidate(&root));
    }

    #[test]
    fn a_drive_appearing_is_the_only_thing_reported() {
        let before = sample();
        let mut after = before.clone();
        after.push(Mount {
            point: PathBuf::from("/run/media/harry/HOLLOW"),
            fs_type: "exfat".to_string(),
        });
        // Something unrelated moves at the same time; it is not a candidate.
        after.push(Mount {
            point: PathBuf::from("/tmp/scratch"),
            fs_type: "ext4".to_string(),
        });

        assert_eq!(
            arrivals(&before, &after),
            vec![PathBuf::from("/run/media/harry/HOLLOW")]
        );
        assert!(departures(&before, &after).is_empty());
    }

    #[test]
    fn a_drive_leaving_is_reported_once() {
        let before = sample();
        let after: Vec<Mount> = before
            .iter()
            .filter(|m| m.point != Path::new("/run/media/harry/CINDER"))
            .cloned()
            .collect();

        assert_eq!(
            departures(&before, &after),
            vec![PathBuf::from("/run/media/harry/CINDER")]
        );
        assert!(arrivals(&before, &after).is_empty());
    }

    #[test]
    fn a_table_that_did_not_change_reports_nothing() {
        // The kernel wakes a poller for any mount activity anywhere, so this is
        // the common case, not an edge case.
        let before = sample();
        let after = sample();
        assert!(arrivals(&before, &after).is_empty());
        assert!(departures(&before, &after).is_empty());
    }

    #[test]
    fn a_volume_without_a_cartridge_file_is_not_a_cartridge() {
        let dir = std::env::temp_dir().join(format!("gamepak-mounts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_cartridge(&dir));

        // A memory card opens the launcher too, which shows it as one.
        std::fs::write(dir.join("memorycard.conf"), b"title=Card\n").unwrap();
        assert!(is_cartridge(&dir));

        std::fs::write(dir.join("cartridge.conf"), b"title=X\n").unwrap();
        assert!(is_cartridge(&dir));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
