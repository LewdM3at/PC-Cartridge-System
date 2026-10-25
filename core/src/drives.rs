//! Finding drives the wizard is allowed to write a cartridge to.
//!
//! The filtering matters more than the listing: this tool writes files, so the
//! system disk and every ordinary system mount must never appear as a target.
//! On Unix that means only the automount directories a desktop uses for
//! removable media are considered.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A volume Windows can read but has not given a drive letter to.
///
/// Every path in this crate is a mount point, so a volume with no letter is one
/// nothing here can address: `GetLogicalDrives` does not report it, the wizard
/// does not list it, and a cartridge sitting on it is invisible until someone
/// opens Disk Management. Windows usually assigns a letter on its own, but not
/// always — a volume unmounted with `mountvol /D`, a machine with automount
/// turned off, or a drive whose remembered letter is now taken by something
/// else all come back with none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmountedVolume {
    pub disk: u32,
    pub partition: u32,
    /// Volume label, which is all there is to recognise it by.
    pub label: String,
    /// `exFAT`, `NTFS` — or empty, which is the common case here rather than
    /// the odd one: Windows reads the filesystem off a volume when it mounts
    /// it, so a volume it never mounted has none to report.
    pub filesystem: String,
    /// The partition's declared type — `IFS`, `FAT32`, `Basic` — which is the
    /// only clue to what is on it when `filesystem` came back empty.
    pub partition_type: String,
    pub total_bytes: u64,
}

/// A drive the wizard can offer as a cartridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetDrive {
    /// Mount point / drive root: `/run/media/you/CART` or `D:\`.
    pub path: String,
    /// Short name for the list.
    pub label: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// True when there is already a cartridge.conf here, so the wizard can warn
    /// before overwriting someone else's cartridge.
    pub has_cartridge: bool,
    /// The name in its `cartridge.conf`, when it has one.
    ///
    /// The volume label only changes when the drive is erased, so a drive
    /// written as "FTL" and rewritten as Cult of the Lamb is still labelled
    /// FTL. This is what it actually holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cartridge_title: Option<String>,
}

/// Directories a Linux desktop automounts removable media into.
#[cfg(unix)]
const AUTOMOUNT_ROOTS: [&str; 3] = ["/media", "/run/media", "/mnt"];

/// Whether a mount point may be offered as a cartridge target.
///
/// Deliberately a allowlist of automount locations rather than a denylist of
/// system paths: a new system mount showing up should fail closed.
pub fn is_writable_target(mount: &Path) -> bool {
    #[cfg(windows)]
    {
        // Drive letters are filtered by drive type instead; see list_drives.
        let _ = mount;
        true
    }
    #[cfg(unix)]
    {
        AUTOMOUNT_ROOTS.iter().any(|root| {
            let root = Path::new(root);
            // Must be *inside* an automount root, never the root itself.
            mount.starts_with(root) && mount != root
        })
    }
}

/// Whether this path is something that can be ejected at all.
///
/// A cartridge is not always a drive. A tag — see the watcher's `tags` module —
/// resolves to a directory in the user's own state folder, and the launcher
/// opens it exactly as it would a mount point, because to everything upstream
/// of this it *is* one.
///
/// Eject is where that stops being true. Handed a folder on the system disk,
/// `findmnt` cheerfully reports the device holding it, and the next step would
/// be asking udisks to unmount the user's home. udisks would almost certainly
/// refuse — but "the layer below will probably say no" is not a check.
pub fn is_ejectable(path: &Path) -> bool {
    #[cfg(windows)]
    {
        // A volume root, `D:\`, and nothing longer. A folder on C: is not a
        // volume and has no business being dismounted.
        let text = path.to_string_lossy();
        let trimmed = text.trim_end_matches(['\\', '/']);
        trimmed.len() == 2
            && trimmed.ends_with(':')
            && trimmed
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
    }
    #[cfg(unix)]
    {
        // The same allowlist the wizard writes through: somewhere a desktop
        // automounts removable media, and never the system.
        is_writable_target(path)
    }
}

/// Filesystems that can never hold a cartridge.
pub fn is_pseudo_filesystem(fs: &str) -> bool {
    matches!(
        fs.to_ascii_lowercase().as_str(),
        "tmpfs"
            | "devtmpfs"
            | "proc"
            | "sysfs"
            | "cgroup"
            | "cgroup2"
            | "overlay"
            | "squashfs"
            | "autofs"
            | "devpts"
            | "mqueue"
            | "hugetlbfs"
            | "tracefs"
            | "debugfs"
            | "fusectl"
            | "configfs"
            | "securityfs"
            | "pstore"
            | "efivarfs"
            | "binfmt_misc"
            // read-only media cannot be written to
            | "iso9660"
            | "udf"
            // network shares are not cartridges
            | "nfs"
            | "nfs4"
            | "cifs"
            | "smbfs"
    )
}

/// One line of `/proc/mounts`, already unescaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub device: String,
    pub mount: PathBuf,
    pub fs_type: String,
    pub read_only: bool,
}

/// Parse `/proc/mounts`.
pub fn parse_proc_mounts(text: &str) -> Vec<MountEntry> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let device = fields.next()?;
            let mount = fields.next()?;
            let fs_type = fields.next()?;
            let options = fields.next().unwrap_or("");
            Some(MountEntry {
                device: unescape_mount(device),
                mount: PathBuf::from(unescape_mount(mount)),
                fs_type: fs_type.to_string(),
                read_only: options.split(',').any(|o| o == "ro"),
            })
        })
        .collect()
}

/// `/proc/mounts` escapes spaces and friends as octal.
fn unescape_mount(field: &str) -> String {
    if !field.contains('\\') {
        return field.to_string();
    }
    let bytes = field.as_bytes();
    let mut out = String::with_capacity(field.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let digits = &field[i + 1..i + 4];
            if let Ok(code) = u8::from_str_radix(digits, 8) {
                out.push(code as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Parse `df -P -k` output, returning (total_bytes, free_bytes).
///
/// `-P -k` rather than GNU's `-B1 --output=`: the POSIX form is understood by
/// coreutils, busybox and macOS alike.
pub fn parse_df(output: &str) -> Option<(u64, u64)> {
    for line in output.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            continue;
        }
        // Normally "Filesystem 1024-blocks Used Available …", but df wraps long
        // device names onto their own line, leaving the numbers first. Detect
        // which by whether the first field is a number.
        let numbers = if fields[0].parse::<u64>().is_ok() {
            &fields[..]
        } else {
            &fields[1..]
        };
        if numbers.len() < 3 {
            continue;
        }
        // blocks, used, available
        if let (Ok(total), Ok(avail)) = (numbers[0].parse::<u64>(), numbers[2].parse::<u64>()) {
            return Some((total * 1024, avail * 1024));
        }
    }
    None
}

/// Enumerate candidate cartridge drives.
pub fn list_drives() -> Vec<TargetDrive> {
    #[cfg(windows)]
    {
        windows_impl::list()
    }
    #[cfg(not(windows))]
    {
        unix_impl::list()
    }
}

/// True when this drive root already holds a cartridge.
fn has_cartridge(root: &Path) -> bool {
    root.join("cartridge.conf").is_file()
}

/// The cartridge's own name, read cheaply: the first `title=` in its conf.
///
/// Not `cartridge::read_cartridge_info`, which inlines every cover as a data
/// URI — far too much work for a line in a drive list. The first title is the
/// collection's on a collection, because `[collection]` is written first.
fn cartridge_title(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("cartridge.conf")).ok()?;
    title_in_conf(&text)
}

fn title_in_conf(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("title="))
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
}

#[cfg(not(windows))]
mod unix_impl {
    use super::*;

    pub fn list() -> Vec<TargetDrive> {
        // Linux only: /proc/mounts is the mount table. macOS is not a supported
        // platform for this project (no watcher, no installer), so there is no
        // fallback here rather than a half-working one.
        let Ok(text) = std::fs::read_to_string("/proc/mounts") else {
            return Vec::new();
        };

        parse_proc_mounts(&text)
            .into_iter()
            .filter(|entry| {
                !entry.read_only
                    && !is_pseudo_filesystem(&entry.fs_type)
                    && is_writable_target(&entry.mount)
            })
            .map(|entry| describe(&entry.mount))
            .filter(|drive| drive.total_bytes > 0)
            .collect()
    }

    fn describe(mount: &Path) -> TargetDrive {
        let (total, free) = df(mount).unwrap_or((0, 0));
        TargetDrive {
            label: mount
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| mount.to_string_lossy().into_owned()),
            path: mount.to_string_lossy().into_owned(),
            total_bytes: total,
            free_bytes: free,
            has_cartridge: super::has_cartridge(mount),
            cartridge_title: super::cartridge_title(mount),
        }
    }

    fn df(mount: &Path) -> Option<(u64, u64)> {
        let out = crate::proc::command("df")
            .arg("-P")
            .arg("-k")
            .arg(mount)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        parse_df(&String::from_utf8_lossy(&out.stdout))
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;

    pub fn list() -> Vec<TargetDrive> {
        let mask = unsafe { GetLogicalDrives() };
        let mut out = Vec::new();

        for bit in 0..26u32 {
            if mask & (1 << bit) == 0 {
                continue;
            }
            let letter = (b'A' + bit as u8) as char;
            let root = format!("{letter}:\\");
            let wide = wide(&root);

            // A USB-C NVMe enclosure usually reports FIXED, not REMOVABLE, so
            // both are offered. Network drives and optical media are not.
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            if kind != DRIVE_REMOVABLE && kind != DRIVE_FIXED {
                continue;
            }

            // FIXED is far too wide on its own: every internal data disk
            // reports it, so the list offered a 2 TB game library beside the
            // cartridges, with nothing but a typed label between it and mkfs.
            // What actually separates a cartridge from an internal disk is the
            // bus it hangs off, so ask the device rather than the volume.
            if kind == DRIVE_FIXED && !is_usb_attached(letter) {
                continue;
            }

            // Never offer the volume Windows itself booted from.
            if is_system_drive(letter) {
                continue;
            }

            let (total, free) = match disk_space(&wide) {
                Some(sizes) => sizes,
                None => continue, // no media in the slot
            };
            if total == 0 {
                continue;
            }

            let label = volume_label(&wide).unwrap_or_default();
            let path = Path::new(&root).to_path_buf();

            out.push(TargetDrive {
                label: if label.is_empty() {
                    format!("{letter}:")
                } else {
                    format!("{label} ({letter}:)")
                },
                path: root,
                total_bytes: total,
                free_bytes: free,
                has_cartridge: super::has_cartridge(&path),
                cartridge_title: super::cartridge_title(&path),
            });
        }

        out
    }

    /// The drive holding Windows, from %SystemDrive% (e.g. "C:").
    /// Is the disk behind this volume attached over USB?
    ///
    /// `GetDriveTypeW` describes the *volume* and calls a USB-C NVMe enclosure
    /// FIXED, exactly like the internal disk holding the game library. Only the
    /// device knows which bus it is on, so this opens the volume and asks it.
    ///
    /// Opening with no access rights is deliberate: it is enough for a query
    /// IOCTL and needs no elevation, where any real access right would prompt.
    /// Anything that cannot be opened or does not answer is treated as not USB,
    /// so a drive is left out of the list rather than offered up to a format.
    fn is_usb_attached(letter: char) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows_sys::Win32::System::Ioctl::{
            PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
            STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
        };
        use windows_sys::Win32::System::IO::DeviceIoControl;

        // 0x07, from the STORAGE_BUS_TYPE enum.
        const BUS_TYPE_USB: i32 = 7;

        // The volume path, without the trailing slash a device name cannot have.
        let path = wide(&format!("\\\\.\\{letter}:"));

        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                // A HANDLE here, not a pointer: 0 is "no template".
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return false;
        }

        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        // The descriptor is variable length — it carries its strings inline —
        // so give it room and read only the fixed head.
        let mut buffer = [0u8; 512];
        let mut returned: u32 = 0;

        let ok = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_QUERY_PROPERTY,
                &query as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
                buffer.as_mut_ptr() as *mut std::ffi::c_void,
                buffer.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        unsafe { CloseHandle(handle) };

        if ok == 0 || (returned as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return false;
        }
        let descriptor = unsafe { &*(buffer.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
        descriptor.BusType == BUS_TYPE_USB
    }

    fn is_system_drive(letter: char) -> bool {
        std::env::var("SystemDrive")
            .ok()
            .and_then(|s| s.chars().next())
            .map(|c| c.eq_ignore_ascii_case(&letter))
            .unwrap_or(letter == 'C')
    }

    fn disk_space(wide_root: &[u16]) -> Option<(u64, u64)> {
        let mut free_to_caller: u64 = 0;
        let mut total: u64 = 0;
        let mut total_free: u64 = 0;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide_root.as_ptr(),
                &mut free_to_caller,
                &mut total,
                &mut total_free,
            )
        };
        (ok != 0).then_some((total, free_to_caller))
    }

    fn volume_label(wide_root: &[u16]) -> Option<String> {
        let mut name = [0u16; 261];
        let ok = unsafe {
            GetVolumeInformationW(
                wide_root.as_ptr(),
                name.as_mut_ptr(),
                name.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ok == 0 {
            return None;
        }
        let len = name.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&name[..len]))
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// The filesystem name Windows reports for a volume — "exFAT", "NTFS".
    ///
    /// The same call that reads the label can fill this buffer, but `list()`
    /// has no use for it, so it asks separately rather than widening the
    /// struct every drive in the picker has to carry.
    pub fn filesystem(mount: &str) -> Option<String> {
        let root = wide(mount);
        let mut fs = [0u16; 32];
        let ok = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        if ok == 0 {
            return None;
        }
        let len = fs.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&fs[..len]))
    }
}

/// What filesystem is mounted at `mount`, as a name worth showing someone.
///
/// Empty when it cannot be read: the launcher prints the drive's name alone
/// rather than inventing a filesystem for it.
pub fn filesystem_at(mount: &Path) -> String {
    #[cfg(windows)]
    {
        windows_impl::filesystem(&mount.to_string_lossy()).unwrap_or_default()
    }

    #[cfg(not(windows))]
    {
        let Ok(text) = std::fs::read_to_string("/proc/mounts") else {
            return String::new();
        };
        parse_proc_mounts(&text)
            .into_iter()
            .find(|entry| entry.mount == mount)
            .map(|entry| display_filesystem(&entry.fs_type))
            .unwrap_or_default()
    }
}

/// `/proc/mounts` spells these in lower case; the names people know have
/// capitals in them. Anything unrecognised is passed through as it was read
/// rather than guessed at.
#[cfg(not(windows))]
fn display_filesystem(fs_type: &str) -> String {
    match fs_type {
        "exfat" => "exFAT".to_string(),
        "vfat" | "msdos" => "FAT32".to_string(),
        "ntfs" | "ntfs3" | "fuseblk" => "NTFS".to_string(),
        other => other.to_string(),
    }
}

/// Volumes Windows can read but has not lettered.
///
/// USB-attached only, and only filesystems Windows mounts: a btrfs cartridge
/// written on Linux also has no letter, but giving it one would not make it
/// readable, and offering it as a target would only invite someone to reformat
/// the thing by accident. Read-only, so no elevation and no prompt — an empty
/// list is the right answer when the query cannot be run at all.
#[cfg(windows)]
pub fn unmounted_volumes() -> Vec<UnmountedVolume> {
    // One compact JSON object per line rather than one array: `ConvertTo-Json`
    // in Windows PowerShell 5.1 unwraps a single-element array into a bare
    // object and has no `-AsArray` to stop it, so a one-drive machine — the
    // common case here — would return a shape the parser did not expect.
    const QUERY: &str = "\
        $ErrorActionPreference='Stop'; \
        Get-Disk | Where-Object BusType -eq 'USB' | Get-Partition | \
        Where-Object { -not $_.DriveLetter -and $_.Size -gt 100MB } | \
        ForEach-Object { \
          $v = $_ | Get-Volume -ErrorAction SilentlyContinue; \
          [pscustomobject]@{ \
            disk = $_.DiskNumber; partition = $_.PartitionNumber; \
            label = [string]$v.FileSystemLabel; \
            filesystem = [string]$v.FileSystem; \
            partitionType = [string]$_.Type; \
            totalBytes = [uint64]$_.Size \
          } | ConvertTo-Json -Compress \
        }";

    let Ok(out) = crate::proc::command("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            QUERY,
        ])
        .output()
    else {
        return Vec::new();
    };

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<UnmountedVolume>(line.trim()).ok())
        .filter(is_mountable)
        .collect()
}

#[cfg(not(windows))]
pub fn unmounted_volumes() -> Vec<UnmountedVolume> {
    // Linux automounts under /media or /run/media and there are no drive
    // letters to hand out; `list_drives` already sees whatever is mounted.
    Vec::new()
}

/// Whether Windows would be able to read this volume once it had a letter.
///
/// Asking `Get-Volume` for the filesystem is the reliable answer but not an
/// available one: Windows identifies a filesystem when it mounts a volume, and
/// a volume with no drive letter is usually one it has never mounted, so the
/// name comes back empty for exactly the drives this function exists to find.
///
/// So fall back to the partition type, which the partition table states
/// outright. On MBR that is decisive — `IFS` is the type byte NTFS and exFAT
/// both claim, and a Linux filesystem declares 0x83, which surfaces here as
/// `Unknown`. On GPT it is not: Windows and Linux share the basic-data GUID,
/// so a `Basic` partition with no readable filesystem could be either, and
/// lettering a btrfs cartridge would not make it readable — only easier to
/// reformat by accident. Those are left alone unless the filesystem is known.
#[cfg(windows)]
fn is_mountable(volume: &UnmountedVolume) -> bool {
    match volume.filesystem.to_ascii_uppercase().as_str() {
        "NTFS" | "EXFAT" | "FAT32" | "FAT" | "FAT16" | "FAT12" | "REFS" => true,
        "" => matches!(
            volume.partition_type.to_ascii_uppercase().as_str(),
            "IFS" | "FAT32" | "FAT16" | "FAT12" | "HUGE"
        ),
        _ => false,
    }
}

/// Give `volume` the first free drive letter, and return its new root.
///
/// Needs administrator, so it elevates on its own the way the formatter does
/// rather than making the whole wizard run as admin. The UAC prompt is the
/// point at which the user agrees to it; nothing here assigns a letter behind
/// their back, which is also why listing is a separate call.
#[cfg(windows)]
pub fn mount_volume(volume: &UnmountedVolume) -> Result<String, String> {
    let letter = free_drive_letter().ok_or("Every drive letter from D to Z is taken.")?;

    let script = format!(
        "$ErrorActionPreference='Stop'; \
         Set-Partition -DiskNumber {} -PartitionNumber {} -NewDriveLetter {letter}",
        volume.disk, volume.partition
    );
    let quoted = format!("'{}'", script.replace('\'', "''"));

    let status = crate::proc::command("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "Start-Process powershell.exe -Verb RunAs -Wait -WindowStyle Hidden \
                 -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-Command',{quoted})"
            ),
        ])
        .status()
        .map_err(|e| format!("powershell.exe could not be run: {e}"))?;

    if !status.success() {
        return Err(format!(
            "Windows would not assign a drive letter to {} (exit {:?}).",
            if volume.label.is_empty() {
                "the volume".to_string()
            } else {
                volume.label.clone()
            },
            status.code()
        ));
    }

    // Set-Partition returns before the volume is necessarily addressable, and
    // the caller's next move is to read the root, so wait for it to appear
    // rather than handing back a path that is not there yet.
    let root = format!("{letter}:\\");
    for _ in 0..20 {
        if Path::new(&root).is_dir() {
            return Ok(root);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(root)
}

#[cfg(not(windows))]
pub fn mount_volume(_volume: &UnmountedVolume) -> Result<String, String> {
    Err("Drive letters are a Windows idea; nothing to assign here.".to_string())
}

/// The first letter from D that no volume is using.
///
/// From D rather than A: the floppy letters are conventionally left alone, and
/// C is the system disk. B is skipped even when free — Windows will hand it out
/// but plenty of software still assumes it is not a real drive.
#[cfg(windows)]
fn free_drive_letter() -> Option<char> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;

    let mask = unsafe { GetLogicalDrives() };
    (3..26u32)
        .find(|bit| mask & (1 << bit) == 0)
        .map(|bit| (b'A' + bit as u8) as char)
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_title_comes_from_the_conf_not_the_volume_label() {
        let single =
            "# PC GamePak\n\ntitle=Cult of the Lamb\nexecutable=steam://rungameid/1313140\n";
        assert_eq!(title_in_conf(single).as_deref(), Some("Cult of the Lamb"));

        // A collection's own name is written before any of its games.
        let bundle = "[collection]\ntitle=Roguelikes\n\n[game]\ntitle=FTL\nexecutable=x\n";
        assert_eq!(title_in_conf(bundle).as_deref(), Some("Roguelikes"));

        assert_eq!(title_in_conf("title=  \nexecutable=x\n"), None);
        assert_eq!(title_in_conf("executable=x\n"), None);
    }

    use super::*;

    #[cfg(unix)]
    #[test]
    fn only_a_removable_mount_can_be_ejected() {
        assert!(is_ejectable(Path::new("/run/media/harry/CINDER")));
        assert!(is_ejectable(Path::new("/media/HOLLOW")));

        // A tag's virtual cartridge lives here, and there is nothing to eject.
        assert!(!is_ejectable(Path::new(
            "/home/harry/.local/state/pc-gamepak/tags/04A224B2"
        )));
        assert!(!is_ejectable(Path::new("/home/harry")));
        assert!(!is_ejectable(Path::new("/")));
        // The automount root itself is not a cartridge either.
        assert!(!is_ejectable(Path::new("/run/media")));
    }

    #[cfg(windows)]
    #[test]
    fn a_volume_windows_never_mounted_is_still_offered_a_letter() {
        let volume = |filesystem: &str, partition_type: &str| UnmountedVolume {
            disk: 4,
            partition: 1,
            label: String::new(),
            filesystem: filesystem.to_string(),
            partition_type: partition_type.to_string(),
            total_bytes: 256_059_448_832,
        };

        // The case this whole feature is for. Windows has not mounted it, so
        // it cannot say what the filesystem is; the MBR type byte can, and
        // requiring the filesystem name hid the one drive that needed a letter.
        assert!(is_mountable(&volume("", "IFS")));
        assert!(is_mountable(&volume("", "FAT32")));

        // Named outright, whatever the partition table says.
        assert!(is_mountable(&volume("exFAT", "IFS")));
        assert!(is_mountable(&volume("NTFS", "Basic")));

        // Nothing Windows can read. A letter would not change that.
        assert!(!is_mountable(&volume("btrfs", "Basic")));
        assert!(!is_mountable(&volume("ext4", "Unknown")));
        assert!(!is_mountable(&volume("", "Unknown")));

        // GPT states no more than "basic data", which Linux writes too, so an
        // unreadable one stays put rather than being offered as a target.
        assert!(!is_mountable(&volume("", "Basic")));

        // Never the machine's own boot furniture.
        assert!(!is_mountable(&volume("", "System")));
        assert!(!is_mountable(&volume("", "Reserved")));
        assert!(!is_mountable(&volume("", "Recovery")));
    }

    #[cfg(windows)]
    #[test]
    fn only_a_volume_root_can_be_ejected() {
        assert!(is_ejectable(Path::new("D:\\")));
        assert!(is_ejectable(Path::new("D:")));
        assert!(is_ejectable(Path::new("d:/")));

        // Where a tag's virtual cartridge lives.
        assert!(!is_ejectable(Path::new(
            "C:\\Users\\harry\\AppData\\Local\\PC-GamePak\\tags\\04A224B2"
        )));
        assert!(!is_ejectable(Path::new("C:\\Users")));
        assert!(!is_ejectable(Path::new("\\\\server\\share")));
    }

    #[test]
    fn parses_proc_mounts() {
        let text = "\
/dev/sda2 / ext4 rw,relatime 0 0
proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0
/dev/sdb1 /run/media/harry/CINDER exfat rw,nosuid,nodev,relatime,uid=1000 0 0
/dev/sr0 /media/harry/DVD iso9660 ro,nosuid,nodev,relatime 0 0
";
        let mounts = parse_proc_mounts(text);
        assert_eq!(mounts.len(), 4);
        assert_eq!(mounts[0].mount, Path::new("/"));
        assert_eq!(mounts[2].fs_type, "exfat");
        assert_eq!(mounts[2].mount, Path::new("/run/media/harry/CINDER"));
        assert!(!mounts[2].read_only);
        assert!(mounts[3].read_only);
    }

    #[test]
    fn unescapes_octal_in_mount_points() {
        let text = "/dev/sdb1 /run/media/harry/MY\\040CART exfat rw 0 0\n";
        let mounts = parse_proc_mounts(text);
        assert_eq!(mounts[0].mount, Path::new("/run/media/harry/MY CART"));
    }

    #[test]
    #[cfg(unix)]
    fn only_automounted_media_is_a_writable_target() {
        // The whole safety property of the wizard.
        for good in [
            "/media/harry/CART",
            "/run/media/harry/CART",
            "/mnt/cartridge",
        ] {
            assert!(
                is_writable_target(Path::new(good)),
                "{good} should be allowed"
            );
        }
        for bad in [
            "/",
            "/home",
            "/home/harry",
            "/usr",
            "/etc",
            "/boot/efi",
            "/var/lib/docker",
            // the automount roots themselves are not drives
            "/media",
            "/run/media",
            "/mnt",
            // near-misses that must not slip through prefix matching
            "/mnt-backup",
            "/media-server/data",
        ] {
            assert!(!is_writable_target(Path::new(bad)), "{bad} must be refused");
        }
    }

    #[test]
    fn rejects_pseudo_and_readonly_filesystems() {
        for fs in [
            "tmpfs", "proc", "sysfs", "overlay", "iso9660", "nfs4", "cifs",
        ] {
            assert!(is_pseudo_filesystem(fs), "{fs}");
        }
        for fs in [
            "ext4", "exfat", "vfat", "ntfs", "ntfs3", "btrfs", "xfs", "f2fs",
        ] {
            assert!(!is_pseudo_filesystem(fs), "{fs} should be usable");
        }
        // Case from /proc/mounts is not guaranteed.
        assert!(is_pseudo_filesystem("TmpFS"));
    }

    #[test]
    fn parses_df_output() {
        let out = "\
Filesystem     1024-blocks     Used Available Capacity Mounted on
/dev/sdb1        124993536 30000000  94993536      25% /run/media/harry/CINDER
";
        let (total, free) = parse_df(out).unwrap();
        assert_eq!(total, 124_993_536 * 1024);
        assert_eq!(free, 94_993_536 * 1024);
    }

    #[test]
    fn parses_df_with_a_wrapped_device_name() {
        // Long device names push the numbers onto the next line.
        let out = "\
Filesystem                1024-blocks     Used Available Capacity Mounted on
/dev/mapper/a-very-long-device-name
                            124993536 30000000  94993536      25% /mnt/cart
";
        let (total, free) = parse_df(out).unwrap();
        assert_eq!(total, 124_993_536 * 1024);
        assert_eq!(free, 94_993_536 * 1024);
    }

    #[test]
    fn df_garbage_is_none_not_a_panic() {
        assert_eq!(parse_df(""), None);
        assert_eq!(parse_df("Filesystem 1024-blocks\n"), None);
        assert_eq!(parse_df("header\nnot numbers at all here\n"), None);
    }

    #[test]
    fn listing_drives_does_not_panic() {
        // Smoke test against the live mount table.
        let _ = list_drives();
    }
}
