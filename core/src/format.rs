//! Formatting a drive to exFAT or btrfs.
//!
//! This is the only code in the project that destroys data, so it is built to
//! refuse rather than to succeed. Four things must all hold before a single
//! command runs:
//!
//!   1. the target is in `drives::list_drives()` — an allowlist of removable,
//!      automounted volumes, re-derived here rather than taken from the caller;
//!   2. it is not the system drive;
//!   3. the caller echoed the drive's current label back exactly;
//!   4. formatting was explicitly asked for, per cartridge. It is never implied.
//!
//! **NTFS is the default**, and it took hardware to work out why. exFAT was the
//! obvious answer — Windows, Linux and macOS all read it with nothing to install
//! — and it is the one filesystem here that cannot hold what a cartridge needs
//! to carry:
//!
//! * **No symlinks.** Steam installs a compatibility tool into the library the
//!   game lives in, so launching a Windows game from an exFAT cartridge makes
//!   Steam try to unpack Proton onto it. Proton contains 1,892 symlinks and the
//!   first one ends the install — `AppError_11`, "Disk write error", which says
//!   nothing about why.
//! * **No executable bit.** Nothing on an exFAT cartridge is executable, which
//!   is why a carried Linux game has to be a shell script and why the launcher
//!   runs one through `bash`.
//! * **No permissions that persist**, so `chmod` does not survive a replug.
//!
//! NTFS has all three, Windows reads it natively, and the Linux kernel's `ntfs3`
//! driver mounts it read-write through udisks like any other removable volume —
//! measured on this project's own hardware, at `/run/media/$USER/…` with
//! `uid=1000` and no root.
//!
//! The cost, stated plainly: **macOS reads NTFS but does not write it.** A
//! cartridge handed to a Mac can be played from and copied off, and cannot take
//! a save or a playtime count back. exFAT remains the choice for a cartridge
//! that has to be writable on all three.
//!
//! btrfs is the third option and is unchanged: TRIM (`discard=async`) and
//! transparent zstd compression, on Linux only, since Windows cannot read it
//! without [WinBtrfs]. Both benefits are thinner than they look here — a USB
//! bridge only passes TRIM through when it speaks UASP and honours UNMAP, and
//! game data is already compressed.
//!
//! [WinBtrfs]: https://github.com/maharmstone/btrfs

use std::path::Path;

use crate::drives;

const BTRFS_MAX_LABEL: usize = 256;
const EXFAT_MAX_LABEL: usize = 11;
/// NTFS volume labels are 32 UTF-16 code units.
const NTFS_MAX_LABEL: usize = 32;
const EXT4_MAX_LABEL: usize = 16;
const XFS_MAX_LABEL: usize = 12;
const F2FS_MAX_LABEL: usize = 512;
const HFSPLUS_MAX_LABEL: usize = 255;
const APFS_MAX_LABEL: usize = 255;

/// Allocation unit for a cartridge's exFAT filesystem.
///
/// A cartridge holds a few enormous files, not many small ones, so the largest
/// practical cluster is the right trade: fewer allocation-table lookups per
/// gigabyte read, and a fragmentation pattern that stays sequential. The cost —
/// up to 128 KB wasted per file — is nothing against a 60 GB game. Left to
/// itself mkfs.exfat picks by volume size and lands lower on a 128 GB drive.
const EXFAT_CLUSTER_BYTES: &str = "128K";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filesystem {
    /// The default. Holds symlinks, the executable bit and permissions, which
    /// is what a cartridge carrying a game actually needs; written by Windows
    /// natively and by the kernel's `ntfs3` driver on Linux. Read-only on macOS.
    #[default]
    Ntfs,
    /// Readable and writable everywhere, and unable to hold a symlink. The right
    /// answer for a cartridge that only points at games, or one that has to be
    /// written by a Mac.
    Exfat,
    /// Linux only, and the most capable of the three where that is no obstacle.
    Btrfs,
    Ext4,
    Xfs,
    F2fs,
    #[serde(rename = "hfsplus")]
    HfsPlus,
    Apfs,
}

/// One filesystem, described well enough for the wizard to explain the choice.
///
/// The wizard reads this rather than carrying its own list, because the two
/// drifting apart is how a label limit ends up wrong in one place only.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesystemInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub label_limit: usize,
    /// What it is for, in the terms someone picking a cartridge format needs.
    pub summary: &'static str,
    /// Which desktops read it with nothing installed.
    pub native_on: &'static [&'static str],
    /// What has to be installed first, anywhere it is not native. Empty when
    /// there is nothing to say.
    pub needs: &'static str,
    /// Whether a Windows game can be run from it through Proton. This is the
    /// question that decides most cartridges, and the answer is "does it have
    /// symlinks": Steam installs a compatibility tool into the game's own
    /// library, and Proton is built out of them.
    pub runs_proton: bool,
    /// Whether the wizard can create it on the machine it is running on.
    pub can_create_here: bool,
}

impl Filesystem {
    /// Every filesystem, in the order the wizard shows them: the default first,
    /// then the two other answers a cartridge is normally given, then the rest.
    pub const ALL: [Self; 8] = [
        Self::Ntfs,
        Self::Exfat,
        Self::Btrfs,
        Self::Ext4,
        Self::Xfs,
        Self::F2fs,
        Self::HfsPlus,
        Self::Apfs,
    ];

    /// Longest volume label this filesystem will take.
    pub fn label_limit(self) -> usize {
        match self {
            Self::Btrfs => BTRFS_MAX_LABEL,
            Self::Exfat => EXFAT_MAX_LABEL,
            Self::Ntfs => NTFS_MAX_LABEL,
            Self::Ext4 => EXT4_MAX_LABEL,
            Self::Xfs => XFS_MAX_LABEL,
            Self::F2fs => F2FS_MAX_LABEL,
            Self::HfsPlus => HFSPLUS_MAX_LABEL,
            Self::Apfs => APFS_MAX_LABEL,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Exfat => "exfat",
            Self::Ntfs => "ntfs",
            Self::Btrfs => "btrfs",
            Self::Ext4 => "ext4",
            Self::Xfs => "xfs",
            Self::F2fs => "f2fs",
            Self::HfsPlus => "hfsplus",
            Self::Apfs => "apfs",
        }
    }

    /// What to call it in front of a person.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Btrfs => "btrfs",
            Self::Exfat => "exFAT",
            Self::Ntfs => "NTFS",
            Self::Ext4 => "ext4",
            Self::Xfs => "XFS",
            Self::F2fs => "F2FS",
            Self::HfsPlus => "HFS+",
            Self::Apfs => "APFS",
        }
    }

    /// The tool that creates it, so a missing one can be named before the
    /// drive is touched rather than after.
    pub fn mkfs_tool(self) -> &'static str {
        match self {
            Self::Exfat => "mkfs.exfat",
            Self::Ntfs => "mkfs.ntfs",
            Self::Btrfs => "mkfs.btrfs",
            Self::Ext4 => "mkfs.ext4",
            Self::Xfs => "mkfs.xfs",
            Self::F2fs => "mkfs.f2fs",
            Self::HfsPlus => "mkfs.hfsplus",
            Self::Apfs => "mkfs.apfs",
        }
    }

    /// Does a Windows game run from it through Proton?
    ///
    /// Symlinks are the whole question. Steam installs a compatibility tool
    /// into the library the game lives in, and Proton is roughly two thousand
    /// symlinks; a filesystem without them fails the install and Steam reports
    /// "Disk write error", naming nothing useful.
    pub fn runs_proton(self) -> bool {
        !matches!(self, Self::Exfat)
    }

    /// Does it store ownership on disk, rather than taking it from the mount?
    ///
    /// The ones that do come back owned by root, because mkfs ran under pkexec,
    /// and the next write anyone makes fails with EACCES until that is undone.
    pub fn stores_ownership(self) -> bool {
        matches!(
            self,
            Self::Btrfs | Self::Ext4 | Self::Xfs | Self::F2fs | Self::HfsPlus | Self::Apfs
        )
    }

    /// Can Windows' own Format-Volume make it? Only the two it has built in:
    /// btrfs needs WinBtrfs, and Format-Volume refuses it without.
    pub fn windows_can_create(self) -> bool {
        matches!(self, Self::Exfat | Self::Ntfs)
    }

    /// Can this machine make it: Format-Volume on Windows, mkfs elsewhere.
    /// Checking for `mkfs.*` on Windows found none, and left NTFS selectable
    /// only by accident.
    pub fn creatable_here(self) -> bool {
        if cfg!(windows) {
            self.windows_can_create()
        } else {
            crate::proc::tool_exists(self.mkfs_tool())
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::Exfat => "Reads and writes everywhere, macOS included, and holds none of what a game needs \u{2014} no symlinks, so Proton will not unpack onto it. Pick it only when a Mac has to write to the drive.",
            Self::Ntfs => "Reads natively on Windows and Linux, and Proton works. The best choice for a cartridge used on both.",
            Self::Btrfs => "Linux at its best: checksums, compression, snapshots. Windows needs WinBtrfs installed.",
            Self::Ext4 => "The plain Linux choice. Everything works and nothing is clever.",
            Self::Xfs => "Linux, tuned for very large files. Good for a cartridge of a few enormous games.",
            Self::F2fs => "Linux, designed for flash. Worth measuring against ext4 on a cheap USB drive.",
            Self::HfsPlus => "The older Mac format. Linux reads and writes it, but cannot create one without hfsprogs.",
            Self::Apfs => "The modern Mac format. Linux needs the linux-apfs-rw driver to read it at all.",
        }
    }

    fn native_on(self) -> &'static [&'static str] {
        match self {
            Self::Exfat => &["Windows", "Linux", "macOS"],
            Self::Ntfs => &["Windows", "Linux"],
            Self::Btrfs => &["Linux"],
            Self::Ext4 => &["Linux"],
            Self::Xfs => &["Linux"],
            Self::F2fs => &["Linux"],
            Self::HfsPlus => &["macOS", "Linux"],
            Self::Apfs => &["macOS"],
        }
    }

    fn needs(self) -> &'static str {
        match self {
            Self::Exfat => "",
            Self::Ntfs => "",
            Self::Btrfs => "WinBtrfs, to read it on Windows",
            Self::Ext4 => "Nothing on Linux. Not readable on Windows or macOS.",
            Self::Xfs => "Nothing on Linux. Not readable on Windows or macOS.",
            Self::F2fs => "Nothing on Linux. Not readable on Windows or macOS.",
            Self::HfsPlus => "hfsprogs, to create one on Linux",
            Self::Apfs => "linux-apfs-rw, to read it on Linux",
        }
    }

    /// Everything the wizard needs to offer this as a choice.
    pub fn info(self) -> FilesystemInfo {
        FilesystemInfo {
            id: self.id(),
            name: self.display_name(),
            label_limit: self.label_limit(),
            summary: self.summary(),
            native_on: self.native_on(),
            needs: self.needs(),
            runs_proton: self.runs_proton(),
            can_create_here: self.creatable_here(),
        }
    }
}

/// Every filesystem the wizard can offer, in the order it should show them.
pub fn all_filesystems() -> Vec<FilesystemInfo> {
    Filesystem::ALL.iter().map(|f| f.info()).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum FormatError {
    NotRemovable(String),
    SystemDrive(String),
    BadLabel(String),
    NoDevice(String),
    ToolMissing(String),
    Failed(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::NotRemovable(p) => write!(
                f,
                "{p} is not a removable drive this tool will touch, let alone format."
            ),
            FormatError::SystemDrive(p) => {
                write!(f, "{p} is the system drive. Refusing to format it.")
            }
            FormatError::BadLabel(l) => write!(
                f,
                "{l:?} is not a usable volume label. Use only letters, digits, \
                 spaces, - or _, and keep it within the filesystem's label limit."
            ),
            FormatError::NoDevice(p) => {
                write!(f, "Could not work out which device backs {p}.")
            }
            FormatError::ToolMissing(t) => write!(
                f,
                "{t} is not installed, so the drive cannot be formatted here. \
                 Format it yourself and run the wizard again."
            ),
            FormatError::Failed(m) => write!(f, "Formatting failed: {m}"),
        }
    }
}

/// What a format would do, for the warning shown before it runs.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatPlan {
    pub path: String,
    /// What the drive is called now, so the warning can name what is about to
    /// be erased rather than describing it as "the selected drive".
    pub current_label: String,
    pub device: Option<String>,
    pub total_bytes: u64,
    /// Human-readable summary of what is about to be destroyed.
    pub warning: String,
}

/// Validate a proposed volume label for the chosen filesystem.
pub fn check_label_for(filesystem: Filesystem, label: &str) -> Result<String, FormatError> {
    let trimmed = label.trim();
    if trimmed.is_empty() || trimmed.len() > filesystem.label_limit() {
        return Err(FormatError::BadLabel(label.to_string()));
    }
    // Keep to characters every tool and both OSes accept in a volume label.
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_')
    {
        return Err(FormatError::BadLabel(label.to_string()));
    }
    Ok(trimmed.to_string())
}

/// Validate a proposed volume label against the default filesystem.
///
/// That is NTFS here, so a label this accepts fits the format a new cartridge
/// will get unless the caller asks for another one explicitly.
pub fn check_label(label: &str) -> Result<String, FormatError> {
    check_label_for(Filesystem::default(), label)
}

/// Describe what formatting `path` would destroy, refusing anything ineligible.
pub fn plan(path: &str) -> Result<FormatPlan, FormatError> {
    let drive = drives::list_drives()
        .into_iter()
        .find(|d| Path::new(&d.path) == Path::new(path))
        .ok_or_else(|| FormatError::NotRemovable(path.to_string()))?;

    if is_system_drive(Path::new(path)) {
        return Err(FormatError::SystemDrive(path.to_string()));
    }

    let device = backing_device(Path::new(path));
    let label = current_label(&drive);

    Ok(FormatPlan {
        warning: format!(
            "Everything on {} ({}) will be erased.",
            label,
            crate::format::human_bytes(drive.total_bytes)
        ),
        path: drive.path.clone(),
        current_label: label,
        device,
        total_bytes: drive.total_bytes,
    })
}

/// Format the drive, having checked everything.
///
/// There used to be a fourth gate here: the drive's current label had to be
/// typed back before this would run. It went because the wizard had already
/// stopped asking — formatting is a setting now, and the request was filled in
/// from the plan the same code had just produced, so the check was comparing a
/// value against itself. A gate that cannot fail is not a gate.
///
/// Three real ones remain, all of them checked here rather than trusted from
/// the caller: the drive must be on the removable allowlist, it must not be the
/// system drive, and formatting must have been explicitly asked for.
///
/// Returns the path the drive can be found at afterward, when known. A fresh
/// filesystem gets a fresh label, and on Linux the desktop automounts by
/// label, so that is not necessarily `path` any more — `run_format` mounts it
/// back itself rather than leave that to whatever else might be watching for
/// it. `None` means formatting succeeded but the new mount point could not be
/// determined; the caller should keep looking rather than trust `path`.
pub fn format_drive(
    path: &str,
    filesystem: Filesystem,
    new_label: &str,
) -> Result<Option<String>, FormatError> {
    // plan() is what re-derives eligibility from the system rather than taking
    // the caller's word for it, so it stays the first thing that happens.
    let plan = plan(path)?;
    let label = check_label_for(filesystem, new_label)?;

    run_format(&plan, filesystem, &label)
}

/// The name to show for a drive about to be erased. An unlabelled drive would
/// leave the warning with nothing to name, so its short name stands in.
fn current_label(drive: &drives::TargetDrive) -> String {
    let label = drive.label.trim();
    if label.is_empty() {
        Path::new(&drive.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| drive.path.clone())
    } else {
        label.to_string()
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value >= 100.0 || unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(windows)]
fn is_system_drive(path: &Path) -> bool {
    let letter = path.to_string_lossy().chars().next().unwrap_or('C');
    std::env::var("SystemDrive")
        .ok()
        .and_then(|s| s.chars().next())
        .map(|c| c.eq_ignore_ascii_case(&letter))
        .unwrap_or(letter.eq_ignore_ascii_case(&'C'))
}

#[cfg(not(windows))]
fn is_system_drive(path: &Path) -> bool {
    // A removable mount is never / or /home, but check anyway: this is the last
    // line before mkfs.
    matches!(
        path.to_string_lossy().trim_end_matches('/'),
        "" | "/boot" | "/home" | "/usr" | "/var" | "/etc"
    )
}

#[cfg(windows)]
fn backing_device(path: &Path) -> Option<String> {
    // On Windows the drive letter *is* the handle used to format.
    Some(path.to_string_lossy().trim_end_matches('\\').to_string())
}

#[cfg(not(windows))]
fn backing_device(path: &Path) -> Option<String> {
    let out = crate::proc::command("findmnt")
        .args(["-n", "-o", "SOURCE", "--target"])
        .arg(path)
        .output()
        .ok()?;
    let device = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!device.is_empty() && device.starts_with("/dev/")).then_some(device)
}

#[cfg(windows)]
fn run_format(
    plan: &FormatPlan,
    filesystem: Filesystem,
    label: &str,
) -> Result<Option<String>, FormatError> {
    let letter = plan
        .device
        .clone()
        .ok_or_else(|| FormatError::NoDevice(plan.path.clone()))?;

    // Format-Volume needs administrator, so it is elevated on its own rather
    // than requiring the whole wizard to run as admin.
    //
    // NTFS and exFAT are both built into Windows, and `Format-Volume` takes
    // either name as-is. btrfs is not: it needs WinBtrfs
    // (https://github.com/maharmstone/btrfs) installed first, which is why it
    // is Linux-only in practice.
    // exFAT gets the same 128 KB allocation unit the Linux path asks for, so a
    // cartridge is laid out identically whichever machine made it. btrfs has no
    // equivalent knob here and takes its own default.
    let allocation = match filesystem {
        Filesystem::Exfat => " -AllocationUnitSize 131072",
        // NTFS's default cluster is right for a mixed drive and Format-Volume
        // rejects the large ones exFAT is happy with, so everything else is
        // left alone.
        _ => "",
    };
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         Format-Volume -DriveLetter {} -FileSystem {} -NewFileSystemLabel '{}'{} \
         -Confirm:$false -Force",
        letter.trim_end_matches(':'),
        filesystem.display_name(),
        label.replace('\'', "''"),
        allocation
    );

    let status = crate::proc::command("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden \
                 -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-Command',{})",
                powershell_quote(&script)
            ),
        ])
        .status()
        .map_err(|e| FormatError::ToolMissing(format!("powershell.exe ({e})")))?;

    if status.success() {
        // The drive letter is the handle used throughout, and Format-Volume
        // does not change it.
        Ok(Some(plan.path.clone()))
    } else {
        Err(FormatError::Failed(format!(
            "Format-Volume exited with {:?}",
            status.code()
        )))
    }
}

#[cfg(windows)]
fn powershell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(not(windows))]
fn run_format(
    plan: &FormatPlan,
    filesystem: Filesystem,
    label: &str,
) -> Result<Option<String>, FormatError> {
    let device = plan
        .device
        .clone()
        .ok_or_else(|| FormatError::NoDevice(plan.path.clone()))?;

    // Unmount first; mkfs on a mounted filesystem would corrupt it.
    let _ = crate::proc::command("udisksctl")
        .args(["unmount", "-b", &device, "--no-user-interaction"])
        .status();

    // mkfs needs root. pkexec raises the desktop's own authentication dialog
    // rather than the wizard handling a password itself.
    let (program, args) = mkfs_command(&device, filesystem, label);
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = crate::proc::command(program)
        .args(&argv)
        .output()
        .map_err(|e| FormatError::ToolMissing(format!("{program} ({e})")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let message = [stderr.trim(), stdout.trim()]
            .iter()
            .find(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("exited with {:?}", output.status.code()));
        return Err(FormatError::Failed(friendly_pkexec_error(&message)));
    }

    // btrfs stores ownership as real, on-disk metadata — unlike exFAT, which
    // has no owner of its own and takes whatever the mount's uid=/gid=
    // options say. mkfs.btrfs, above, ran as root (via pkexec), so the root
    // of the new filesystem is owned by root right now; without reclaiming
    // it, the very next write anyone makes here fails with EACCES.
    if filesystem.stores_ownership() {
        reclaim_ownership(&device);
    }

    // A fresh filesystem carries a fresh label, and udisks automounts by
    // label — so relying on it to notice on its own can race, or (headless,
    // no automount daemon watching this device) never resolve at all. Ask
    // for the mount directly instead of hoping one arrives; if this doesn't
    // land in time, the caller polls as a fallback.
    let _ = crate::proc::command("udisksctl")
        .args(["mount", "-b", &device, "--no-user-interaction"])
        .status();

    Ok(mounted_path_for(&device))
}

/// Hand a freshly made btrfs filesystem's root back to whoever is running the
/// wizard, rather than leave it owned by root.
///
/// This mounts and unmounts the device privately (a scratch mountpoint under
/// `/run`, gone by the time this returns) rather than reusing the wizard's
/// own `udisksctl mount` for it, so the volume is never exposed — even
/// briefly — to its normal, unprivileged owner before ownership is fixed.
/// Reuses the elevation `mkfs.btrfs` itself already needed, so this costs no
/// extra prompt beyond the one formatting was always going to ask for.
#[cfg(not(windows))]
fn reclaim_ownership(device: &str) {
    let Ok(uid_out) = crate::proc::command("id").arg("-u").output() else {
        return;
    };
    let Ok(gid_out) = crate::proc::command("id").arg("-g").output() else {
        return;
    };
    let uid = String::from_utf8_lossy(&uid_out.stdout).trim().to_string();
    let gid = String::from_utf8_lossy(&gid_out.stdout).trim().to_string();
    if uid.is_empty() || gid.is_empty() {
        return;
    }

    // $1 is the device rather than an interpolated string, so nothing about
    // the device path is ever parsed by the shell.
    let script = "set -e; mnt=$(mktemp -d /run/gamepak-format.XXXXXX); \
                  mount \"$1\" \"$mnt\"; chown \"$2:$3\" \"$mnt\"; \
                  umount \"$mnt\"; rmdir \"$mnt\"";
    let _ = crate::proc::command("pkexec")
        .args(["sh", "-c", script, "reclaim-ownership", device, &uid, &gid])
        .status();
}

/// Turn one specific, common pkexec failure into something a user can
/// actually act on.
///
/// When polkit cannot reach a graphical authentication agent for the calling
/// session, it falls back to a text agent — which then fails outright,
/// because a GUI app has no controlling terminal for it to prompt on. The
/// result is `pkexec`'s own internal error text, which says nothing about
/// what to do: "Error creating textual authentication agent: Error opening
/// current controlling terminal for the process (`/dev/tty'): No such
/// device or address". Anything else is passed through unchanged; this is
/// not a general pkexec-error translator, just a fix for the one message
/// that is actively misleading.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn friendly_pkexec_error(raw: &str) -> String {
    if raw.contains("textual authentication agent") || raw.contains("/dev/tty") {
        format!(
            "Could not ask for a password: no authentication dialog was available \
             ({raw}). This usually clears up on its own — try again. If it keeps \
             happening, your desktop session may be missing a polkit authentication \
             agent (polkit-kde-authentication-agent-1, polkit-gnome-authentication-agent-1, \
             or similar), or it isn't running."
        )
    } else {
        raw.to_string()
    }
}

/// Where `device` is mounted right now, if anywhere.
#[cfg(not(windows))]
fn mounted_path_for(device: &str) -> Option<String> {
    let text = std::fs::read_to_string("/proc/mounts").ok()?;
    drives::parse_proc_mounts(&text)
        .into_iter()
        .find(|entry| entry.device == device)
        .map(|entry| entry.mount.to_string_lossy().into_owned())
}

/// Build the mkfs invocation. Split out so the argument order can be tested
/// without running anything.
#[cfg_attr(windows, allow(dead_code))]
pub fn mkfs_command(
    device: &str,
    filesystem: Filesystem,
    label: &str,
) -> (&'static str, Vec<String>) {
    (
        "pkexec",
        match filesystem {
            // -f: without it, mkfs.btrfs refuses to touch a device that
            // already has a filesystem signature on it — which is exactly
            // the case every time this runs, since the whole point of this
            // call is overwriting whatever is there now. The label-typed
            // confirmation above this in the call chain is the real safety
            // gate; by the time mkfs runs, the user has already agreed to
            // the wipe.
            Filesystem::Btrfs => vec![
                "mkfs.btrfs".to_string(),
                "-f".to_string(),
                "-L".to_string(),
                label.to_string(),
                device.to_string(),
            ],
            // --fast skips zeroing the volume, which on a 1 TB cartridge is the
            // difference between a minute and an afternoon. Deliberately no
            // --force ("do it even if mounted"): the call chain unmounts first,
            // and if that did not take, failing loudly is the better outcome
            // than writing a new filesystem over a mounted one.
            //
            // No cluster size given: NTFS keeps its own default, unlike the
            // exFAT arm below. The 128 KB there buys fewer allocation-table
            // lookups per gigabyte, which is a FAT-family problem — NTFS
            // allocates in extents and does not have it, and a non-default
            // cluster size on NTFS costs compatibility for nothing.
            Filesystem::Ntfs => vec![
                "mkfs.ntfs".to_string(),
                "--fast".to_string(),
                "--label".to_string(),
                label.to_string(),
                device.to_string(),
            ],
            // -F is exfatprogs' equivalent of the above (-f there means
            // "full format" instead).
            Filesystem::Exfat => vec![
                "mkfs.exfat".to_string(),
                "-F".to_string(),
                "-c".to_string(),
                EXFAT_CLUSTER_BYTES.to_string(),
                "-n".to_string(),
                label.to_string(),
                device.to_string(),
            ],
            // -m 0 leaves nothing reserved for root. The 5% default exists so a
            // full root filesystem is still repairable; on a cartridge it is
            // six gigabytes of a 128 GB drive given up for nothing.
            Filesystem::Ext4 => vec![
                "mkfs.ext4".to_string(),
                "-F".to_string(),
                "-m".to_string(),
                "0".to_string(),
                "-L".to_string(),
                label.to_string(),
                device.to_string(),
            ],
            Filesystem::Xfs => vec![
                "mkfs.xfs".to_string(),
                "-f".to_string(),
                "-L".to_string(),
                label.to_string(),
                device.to_string(),
            ],
            Filesystem::F2fs => vec![
                "mkfs.f2fs".to_string(),
                "-f".to_string(),
                "-l".to_string(),
                label.to_string(),
                device.to_string(),
            ],
            Filesystem::HfsPlus => vec![
                "mkfs.hfsplus".to_string(),
                "-v".to_string(),
                label.to_string(),
                device.to_string(),
            ],
            Filesystem::Apfs => vec![
                "mkfs.apfs".to_string(),
                "-L".to_string(),
                label.to_string(),
                device.to_string(),
            ],
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_polkit_agent_gets_an_actual_explanation() {
        let raw = "Error creating textual authentication agent: Error opening current \
                    controlling terminal for the process (`/dev/tty'): No such device or address";
        let friendly = friendly_pkexec_error(raw);
        assert_ne!(friendly, raw);
        assert!(friendly.contains("try again"), "{friendly}");
        assert!(
            friendly.contains(raw),
            "the original detail should still be in there: {friendly}"
        );
    }

    #[test]
    fn any_other_pkexec_failure_passes_through_unchanged() {
        let raw = "mkfs.btrfs: /dev/sdb1 appears to contain an existing filesystem";
        assert_eq!(friendly_pkexec_error(raw), raw);
    }

    #[test]
    fn accepts_sensible_labels_and_preserves_case() {
        assert_eq!(
            check_label_for(Filesystem::Btrfs, "cinder").unwrap(),
            "cinder"
        );
        assert_eq!(
            check_label_for(Filesystem::Btrfs, "  Hollow ").unwrap(),
            "Hollow"
        );
        assert_eq!(
            check_label_for(Filesystem::Exfat, "CART_01").unwrap(),
            "CART_01"
        );
        assert_eq!(
            check_label_for(Filesystem::Exfat, "MY CART").unwrap(),
            "MY CART"
        );
    }

    #[test]
    fn refuses_bad_labels_for_any_supported_filesystem() {
        for bad in [
            "",
            "   ",
            "bad/slash",
            "quote\"mark",
            "semi;colon",
            "new\nline",
        ] {
            assert!(
                matches!(check_label(bad), Err(FormatError::BadLabel(_))),
                "{bad:?}"
            );
        }
        assert!(check_label_for(Filesystem::Exfat, "ELEVENCHARS").is_ok());
        assert!(check_label_for(Filesystem::Exfat, "TWELVECHARSX").is_err());
        assert!(check_label_for(Filesystem::Btrfs, &"A".repeat(256)).is_ok());
        assert!(check_label_for(Filesystem::Btrfs, &"A".repeat(257)).is_err());
    }

    #[test]
    fn refuses_to_format_anything_not_on_the_removable_allowlist() {
        // The guard that matters. None of these are removable mounts, so plan()
        // must refuse before any device is even looked up.
        for path in ["/", "/home", "/etc", "/usr/local", "/media", ""] {
            let err = plan(path).unwrap_err();
            assert!(
                matches!(
                    err,
                    FormatError::NotRemovable(_) | FormatError::SystemDrive(_)
                ),
                "{path} gave {err:?}"
            );
        }
    }

    #[test]
    fn format_refuses_a_drive_it_does_not_recognise() {
        // Eligibility is re-derived inside format_drive, so an ineligible path
        // is refused there and not merely filtered out of the drive list.
        let err = format_drive("/", Filesystem::Btrfs, "CART").unwrap_err();
        assert!(matches!(
            err,
            FormatError::NotRemovable(_) | FormatError::SystemDrive(_)
        ));
    }

    #[test]
    #[cfg(not(windows))]
    fn system_paths_are_recognised() {
        assert!(is_system_drive(Path::new("/home")));
        assert!(is_system_drive(Path::new("/etc")));
        assert!(!is_system_drive(Path::new("/run/media/harry/CINDER")));
    }

    #[test]
    fn mkfs_arguments_are_in_the_right_order() {
        let (program, args) = mkfs_command("/dev/sdb1", Filesystem::Btrfs, "Cinder");
        assert_eq!(program, "pkexec");
        assert_eq!(args, vec!["mkfs.btrfs", "-f", "-L", "Cinder", "/dev/sdb1"]);
    }

    #[test]
    fn ntfs_mkfs_arguments_are_in_the_right_order() {
        let (program, args) = mkfs_command("/dev/sdb1", Filesystem::Ntfs, "Cinder");
        assert_eq!(program, "pkexec");
        // --force is absent on purpose: a device still mounted when this runs
        // means the unmount above it failed, and the right outcome then is an
        // error, not a new filesystem written over a mounted one. Two arms for
        // NTFS were merged together once, each with its own answer to that, and
        // only the second one being unreachable gave it away.
        assert_eq!(
            args,
            vec!["mkfs.ntfs", "--fast", "--label", "Cinder", "/dev/sdb1"]
        );
    }

    #[test]
    fn every_filesystem_asks_for_the_label_it_was_given() {
        // Not a tautology: each arm spells the label flag differently (-L, -n,
        // -l, -v, --label), and an arm that forgot it entirely would produce a
        // cartridge named after nothing, which the drive watcher cannot match.
        for filesystem in Filesystem::ALL {
            let (_, args) = mkfs_command("/dev/sdb1", filesystem, "Cinder");
            assert!(
                args.contains(&"Cinder".to_string()),
                "{filesystem:?} dropped the label: {args:?}"
            );
            assert_eq!(
                args.last().map(String::as_str),
                Some("/dev/sdb1"),
                "{filesystem:?} must name the device last"
            );
        }
    }

    #[test]
    fn exfat_mkfs_arguments_are_in_the_right_order() {
        let (program, args) = mkfs_command("/dev/sdb1", Filesystem::Exfat, "Cinder");
        assert_eq!(program, "pkexec");
        // The cluster size is set rather than left to mkfs, which picks by
        // volume size and lands lower than a cartridge wants.
        assert_eq!(
            args,
            vec![
                "mkfs.exfat",
                "-F",
                "-c",
                "128K",
                "-n",
                "Cinder",
                "/dev/sdb1"
            ]
        );
    }

    #[test]
    fn formats_byte_counts_for_the_warning() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(128_035_676_160), "128 GB");
        assert_eq!(human_bytes(1_500_000_000), "1.5 GB");
    }

    #[test]
    fn the_default_filesystem_is_the_one_that_can_hold_a_game() {
        // exFAT reads everywhere and cannot hold a symlink, which is what Steam
        // needs 1,892 of to put Proton on a cartridge. NTFS can, Windows writes
        // it natively, and Linux mounts it with ntfs3 — so that is the default
        // now, and the cost is that macOS reads it without writing it.
        assert_eq!(Filesystem::default(), Filesystem::Ntfs);

        // Each one's own limit, which is the thing callers have to respect.
        assert!(check_label_for(Filesystem::Exfat, &"A".repeat(11)).is_ok());
        assert!(check_label_for(Filesystem::Exfat, &"A".repeat(12)).is_err());
        assert!(check_label_for(Filesystem::Ntfs, &"A".repeat(32)).is_ok());
        assert!(check_label_for(Filesystem::Ntfs, &"A".repeat(33)).is_err());
        assert!(check_label_for(Filesystem::Btrfs, &"A".repeat(200)).is_ok());

        // The wrapper follows the default, so it is no longer the strictest of
        // the three. Anything derived for one filesystem and used on another
        // has to be checked against that one — which `default_label_for` and
        // `check_label_for` are for, and which every real caller uses.
        assert!(check_label(&"A".repeat(32)).is_ok());
        assert!(check_label(&"A".repeat(33)).is_err());
    }

    #[test]
    fn a_label_that_fits_exfat_fits_all_three() {
        // The property worth having: eleven characters is the floor, so a
        // cartridge named for the tightest filesystem can be reformatted to any
        // of the others without renaming it.
        let tight = "CINDER SALT";
        assert_eq!(tight.len(), 11);
        for filesystem in [Filesystem::Ntfs, Filesystem::Exfat, Filesystem::Btrfs] {
            assert!(
                check_label_for(filesystem, tight).is_ok(),
                "{filesystem:?} rejected {tight:?}"
            );
        }
    }
}
