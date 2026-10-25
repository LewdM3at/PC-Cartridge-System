//! How well is this cartridge actually connected?
//!
//! Three things decide whether a cartridge performs like the drive inside it,
//! and all three are invisible until you go looking:
//!
//!   * **The negotiated link.** A front-panel port, a hub or a charge-only
//!     cable quietly gives you 5 Gbps, or USB 2.0's 480 Mbps, and nothing says
//!     so. People diagnose this as a slow drive for months.
//!   * **UASP or BOT.** Bulk-Only Transport allows one command in flight; UASP
//!     queues them. On the small random reads a game streams, that is the
//!     difference between smooth and stuttering, and which one you get depends
//!     on the enclosure's firmware and the port it is in.
//!   * **How full it is.** Almost every M.2 2230 drive is DRAM-less and leans
//!     on the Host Memory Buffer — host RAM, borrowed over PCIe, holding the
//!     flash translation table. A USB bridge does not provide HMB, so the
//!     translation table is paged from the flash itself, and the fuller the
//!     drive, the more that costs.
//!
//! On Linux all of this is read straight out of sysfs — no processes, no
//! libraries. On Windows the device tree is walked once, lazily, through
//! PowerShell, for the transport and the hub the drive is plugged into, and
//! that hub is asked for the speed it negotiated. Whatever cannot be read is
//! left empty rather than guessed at.

use std::path::Path;

use serde::Serialize;

/// Above this, a DRAM-less drive's garbage collection has little room to work.
const CROWDED_PERCENT: u8 = 85;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    /// "10 Gbps", "5 Gbps", "480 Mbps", or empty when it cannot be read.
    pub link: String,
    /// Raw negotiated speed, for the caller that wants to compare rather than
    /// print.
    pub link_mbps: Option<u32>,
    /// "UASP" or "BOT", or empty when unknown.
    pub transport: String,
    /// The volume's own name — "CINDER" — or empty when it has none. This is
    /// what is printed on the cartridge in Explorer, so the launcher can say
    /// which drive it is looking at without the user reading a mount path.
    pub label: String,
    /// "exFAT", "btrfs", "ntfs", or empty when it cannot be read.
    pub filesystem: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// 0-100. Saturates rather than dividing by zero on an unreadable volume.
    pub used_percent: u8,
    /// Things worth telling the user, in plain sentences.
    pub warnings: Vec<String>,
}

/// Look at the cartridge mounted at `mount`.
pub fn inspect(mount: &str) -> Health {
    let (total_bytes, free_bytes) = capacity(Path::new(mount));
    let used_percent = if total_bytes == 0 {
        0
    } else {
        let used = total_bytes.saturating_sub(free_bytes);
        ((used as f64 / total_bytes as f64) * 100.0)
            .round()
            .min(100.0) as u8
    };

    let link = probe(mount);
    let (label, filesystem) = volume(mount);
    let mut health = Health {
        link: link
            .as_ref()
            .and_then(|l| l.mbps)
            .map(speed_label)
            .unwrap_or_default(),
        link_mbps: link.as_ref().and_then(|l| l.mbps),
        transport: link.map(|l| l.transport).unwrap_or_default(),
        label,
        filesystem,
        total_bytes,
        free_bytes,
        used_percent,
        warnings: Vec::new(),
    };
    health.warnings = advise(&health);
    health
}

/// What is worth saying about a connection, given what was measured.
///
/// Split out from the reading so the wording is testable without a drive.
fn advise(health: &Health) -> Vec<String> {
    let mut out = Vec::new();

    match health.link_mbps {
        Some(mbps) if mbps < 1000 => out.push(format!(
            "Connected at {} — this is a USB 2.0 port or cable. Games will stream badly. \
             Try a different port, and the cable the enclosure came with.",
            speed_label(mbps)
        )),
        Some(mbps) if mbps < 10_000 => out.push(format!(
            "Connected at {}, about half what the enclosure can do. That is usually a \
             front-panel port, a hub, or a cable that is not rated for 10 Gbps.",
            speed_label(mbps)
        )),
        _ => {}
    }

    if health.transport == "BOT" {
        out.push(
            "Running in BOT mode, which sends one command at a time. UASP queues them and \
             is worth roughly two to three times as much on the small random reads a game \
             streams. Usually a different port or enclosure firmware fixes it."
                .to_string(),
        );
    }

    if health.used_percent >= CROWDED_PERCENT {
        out.push(format!(
            "{}% full. These drives have no DRAM of their own and cannot borrow host memory \
             over USB, so the last 15% costs more than it looks like it should. Leaving some \
             room back keeps random reads quick.",
            health.used_percent
        ));
    }

    out
}

fn speed_label(mbps: u32) -> String {
    if mbps >= 1000 && mbps.is_multiple_of(1000) {
        format!("{} Gbps", mbps / 1000)
    } else if mbps >= 1000 {
        format!("{:.1} Gbps", mbps as f64 / 1000.0)
    } else {
        format!("{mbps} Mbps")
    }
}

/// What the bus says about this drive.
struct Link {
    mbps: Option<u32>,
    transport: String,
}

/// The volume's own name and filesystem.
///
/// The name comes from the same enumeration the capacity does, so a cartridge
/// is described by exactly what the drive picker would have called it. Both are
/// empty rather than approximated when the volume cannot be read.
fn volume(mount: &str) -> (String, String) {
    let path = Path::new(mount);
    let label = crate::drives::list_drives()
        .into_iter()
        .find(|d| Path::new(&d.path) == path)
        .map(|d| d.label)
        .unwrap_or_default();
    (label, crate::drives::filesystem_at(path))
}

// --------------------------------------------------------------------------
// Linux: sysfs
// --------------------------------------------------------------------------

#[cfg(not(windows))]
fn capacity(mount: &Path) -> (u64, u64) {
    crate::drives::list_drives()
        .into_iter()
        .find(|d| Path::new(&d.path) == mount)
        .map(|d| (d.total_bytes, d.free_bytes))
        .unwrap_or((0, 0))
}

#[cfg(not(windows))]
fn probe(mount: &str) -> Option<Link> {
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    let device = device_for_mount(&mounts, Path::new(mount))?;
    let block = block_name(&device)?;
    usb_link(Path::new("/sys"), &block)
}

/// The device backing a mount point, from `/proc/mounts`.
#[cfg(not(windows))]
fn device_for_mount(mounts: &str, mount: &Path) -> Option<String> {
    crate::drives::parse_proc_mounts(mounts)
        .into_iter()
        .find(|entry| entry.mount == mount)
        .map(|entry| entry.device)
        .filter(|device| device.starts_with("/dev/"))
}

/// `/dev/sdb1` is a partition; sysfs holds the link details on the disk, `sdb`.
///
/// The two naming schemes have to be told apart: `nvme0n1` and `mmcblk0` end in
/// digits that are part of the disk's name, and only gain a `p1` when they are
/// partitioned, while `sdb` gains a bare `1`.
#[cfg(not(windows))]
fn block_name(device: &str) -> Option<String> {
    let name = device.strip_prefix("/dev/")?;
    if name.is_empty() {
        return None;
    }

    // nvme0n1p3 -> nvme0n1, mmcblk0p1 -> mmcblk0. The disk part ends in a digit,
    // which is what separates this from a name that merely contains a "p".
    if let Some((disk, partition)) = name.rsplit_once('p') {
        if !partition.is_empty()
            && partition.bytes().all(|b| b.is_ascii_digit())
            && disk.ends_with(|c: char| c.is_ascii_digit())
        {
            return Some(disk.to_string());
        }
    }

    // sdb1 -> sdb. Only for the schemes where trailing digits really are the
    // partition number.
    if ["sd", "vd", "hd"].iter().any(|p| name.starts_with(p)) {
        let disk = name.trim_end_matches(|c: char| c.is_ascii_digit());
        if !disk.is_empty() {
            return Some(disk.to_string());
        }
    }

    Some(name.to_string())
}

/// Walk from a block device up to the USB device that carries it.
///
/// `sys_root` is a parameter so this can be pointed at a fake tree in the
/// tests; in use it is `/sys`.
#[cfg(not(windows))]
fn usb_link(sys_root: &Path, block: &str) -> Option<Link> {
    // /sys/class/block/sdb -> ../../devices/…/usb2/2-4/2-4:1.0/host6/…/block/sdb
    let resolved = std::fs::read_link(sys_root.join("class/block").join(block)).ok()?;
    let full = sys_root.join("class/block").join(resolved);

    // The USB interface directory is the one named like "2-4:1.0"; its driver
    // is uas or usb-storage, and its parent is the device that knows the speed.
    let mut interface: Option<std::path::PathBuf> = None;
    let mut walked = std::path::PathBuf::new();
    for part in full.components() {
        walked.push(part);
        let name = match part.as_os_str().to_str() {
            Some(name) => name,
            None => continue,
        };
        if is_usb_interface(name) {
            interface = Some(walked.clone());
        }
    }
    let interface = interface?;

    let transport = std::fs::read_link(interface.join("driver"))
        .ok()
        .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(transport_name))
        .unwrap_or_default();

    let mbps = interface
        .parent()
        .and_then(|device| std::fs::read_to_string(device.join("speed")).ok())
        .and_then(|text| text.trim().parse::<f64>().ok())
        .map(|speed| speed.round() as u32);

    Some(Link { mbps, transport })
}

/// USB interface directories are named `<bus>-<port>:<config>.<interface>`.
#[cfg(not(windows))]
fn is_usb_interface(name: &str) -> bool {
    let Some((device, config)) = name.split_once(':') else {
        return false;
    };
    device.contains('-') && device.starts_with(|c: char| c.is_ascii_digit()) && config.contains('.')
}

#[cfg(not(windows))]
fn transport_name(driver: &str) -> String {
    match driver {
        "uas" => "UASP".to_string(),
        "usb-storage" => "BOT".to_string(),
        other => other.to_string(),
    }
}

// --------------------------------------------------------------------------
// Windows
// --------------------------------------------------------------------------

#[cfg(windows)]
fn capacity(mount: &Path) -> (u64, u64) {
    crate::drives::list_drives()
        .into_iter()
        .find(|d| Path::new(&d.path) == mount)
        .map(|d| (d.total_bytes, d.free_bytes))
        .unwrap_or((0, 0))
}

/// Walks from a drive letter up the device tree, printing what it passes.
///
/// `Get-PhysicalDisk`'s `BusType` is `USB` for every enclosure, which says
/// nothing about either question this module exists to answer, so the tree is
/// walked instead: the USB node's service is the transport, its parent and port
/// are where to ask the hub for the negotiated speed, and the PCI node the walk
/// ends on is the host controller.
///
/// `@LETTER@` is replaced rather than formatted in, so the braces stay PowerShell's.
#[cfg(windows)]
const TREE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$n = (Get-Partition -DriveLetter @LETTER@).DiskNumber
"bus=" + (Get-Disk -Number $n).BusType
$id = (Get-CimInstance Win32_DiskDrive -Filter "Index=$n").PNPDeviceID
$seen = $false
for ($i = 0; $i -lt 16 -and $id; $i++) {
  $p = @{}
  Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_Service,DEVPKEY_Device_Parent,DEVPKEY_Device_Address,DEVPKEY_Device_FriendlyName,DEVPKEY_Device_DeviceDesc -ErrorAction SilentlyContinue |
    ForEach-Object { $p[$_.KeyName] = $_.Data }
  if ($id -like 'PCI\*') {
    $name = $p['DEVPKEY_Device_FriendlyName']
    if (-not $name) { $name = $p['DEVPKEY_Device_DeviceDesc'] }
    "controller=" + $name
    break
  }
  if (-not $seen -and $p['DEVPKEY_Device_Service'] -in 'UASPStor','USBSTOR') {
    $seen = $true
    "service=" + $p['DEVPKEY_Device_Service']
    "hub=" + $p['DEVPKEY_Device_Parent']
    "port=" + $p['DEVPKEY_Device_Address']
  }
  $id = $p['DEVPKEY_Device_Parent']
}
"#;

/// The transport, and the fastest the link can be running at, for a volume.
///
/// The speed comes from the hub the device is plugged into, which is the only
/// place Windows keeps the negotiated one. If the hub will not answer, a
/// USB 2.0 host controller still settles it: nothing on one runs faster than
/// 480 Mbps. A USB 3 controller settles nothing — the port can still have
/// fallen back — so that is left unknown rather than guessed at.
#[cfg(windows)]
fn probe(mount: &str) -> Option<Link> {
    let letter = mount.trim_end_matches('\\').trim_end_matches(':');
    if letter.len() != 1 || !letter.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }

    let script = TREE_SCRIPT.replace("@LETTER@", letter);
    let out = crate::proc::command("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    let tree = UsbTree::parse(&String::from_utf8_lossy(&out.stdout));

    let transport = tree.transport();
    if transport.is_empty() {
        // Not USB at all — an internal disk, or a bus this does not know. Its
        // bus type is the most honest thing to say about it.
        return (!tree.bus.is_empty() && tree.bus != "USB").then(|| Link {
            mbps: None,
            transport: tree.bus.clone(),
        });
    }

    let mbps = negotiated_mbps(&tree.hub, tree.port).or_else(|| tree.controller_ceiling());
    Some(Link { mbps, transport })
}

/// What [`TREE_SCRIPT`] found, one `key=value` per line.
///
/// Parsed on every platform so the tests run in CI, which is Linux.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Default, PartialEq)]
struct UsbTree {
    /// `Get-Disk`'s bus type: `USB`, `NVMe`, `SATA`.
    bus: String,
    /// The service on the USB device node: `UASPStor` or `USBSTOR`.
    service: String,
    /// Instance id of the hub the device is plugged into.
    hub: String,
    /// Port on that hub, 1-based.
    port: u32,
    /// The host controller's name.
    controller: String,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl UsbTree {
    fn parse(text: &str) -> Self {
        let mut tree = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.trim().split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key {
                "bus" => tree.bus = value.to_string(),
                "service" => tree.service = value.to_string(),
                "hub" => tree.hub = value.to_string(),
                "port" => tree.port = value.parse().unwrap_or(0),
                "controller" => tree.controller = value.to_string(),
                _ => {}
            }
        }
        tree
    }

    /// "UASP" or "BOT", named the way Linux names them; empty when neither.
    fn transport(&self) -> String {
        if self.service.eq_ignore_ascii_case("uaspstor") {
            "UASP".to_string()
        } else if self.service.eq_ignore_ascii_case("usbstor") {
            "BOT".to_string()
        } else {
            String::new()
        }
    }

    /// The most a link on this controller can be, when that is informative.
    ///
    /// Only the USB 2.0 and 1.1 controllers say anything definite. On
    /// 2026-08-20 a cartridge sat on an `AMD USB 2.0 eXtensible Host
    /// Controller` behind a hub called `ROOT_HUB30` and copied at 18 MB/s; the
    /// hub's name said USB 3, the controller's said the truth.
    fn controller_ceiling(&self) -> Option<u32> {
        let name = self.controller.to_ascii_lowercase();
        if name.contains("usb 2.0") || name.contains("enhanced host controller") {
            Some(480)
        } else if name.contains("open host controller")
            || name.contains("universal host controller")
        {
            Some(12)
        } else {
            None
        }
    }
}

/// The speed a hub port negotiated, from the hub's own answer.
///
/// `_EX_V2` says whether the device is running at SuperSpeed or SuperSpeedPlus;
/// the older `_EX` distinguishes the speeds below that. SuperSpeedPlus is 10 or
/// 20 Gbps and the hub does not say which, so it is reported as 10 — the lower
/// of the two, which is also the one no warning is given for.
#[cfg(windows)]
fn negotiated_mbps(hub: &str, port: u32) -> Option<u32> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
        CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CR_SUCCESS,
    };
    use windows_sys::Win32::Devices::Usb::{
        GUID_DEVINTERFACE_USB_HUB, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX,
        IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2, USB_NODE_CONNECTION_INFORMATION_EX,
        USB_NODE_CONNECTION_INFORMATION_EX_V2,
    };
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    if hub.is_empty() || port == 0 {
        return None;
    }
    let hub_id: Vec<u16> = hub.encode_utf16().chain(std::iter::once(0)).collect();

    // The hub's device interface path, which is what CreateFileW opens.
    let path = unsafe {
        let mut length = 0u32;
        if CM_Get_Device_Interface_List_SizeW(
            &mut length,
            &GUID_DEVINTERFACE_USB_HUB,
            hub_id.as_ptr(),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        ) != CR_SUCCESS
            || length <= 1
        {
            return None;
        }
        let mut list = vec![0u16; length as usize];
        if CM_Get_Device_Interface_ListW(
            &GUID_DEVINTERFACE_USB_HUB,
            hub_id.as_ptr(),
            list.as_mut_ptr(),
            length,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        ) != CR_SUCCESS
        {
            return None;
        }
        // A multi-string: the first entry, and its terminator.
        let end = list.iter().position(|c| *c == 0)?;
        list.truncate(end + 1);
        list
    };

    // No access rights asked for: both IOCTLs are FILE_ANY_ACCESS, and asking
    // for none is what lets this run unelevated.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    // SAFETY: both structs are packed(1), so a byte buffer of at least their
    // size is a valid place for them, and only fixed-offset fields are read.
    unsafe {
        let mut v2: USB_NODE_CONNECTION_INFORMATION_EX_V2 = std::mem::zeroed();
        v2.ConnectionIndex = port;
        v2.Length = std::mem::size_of::<USB_NODE_CONNECTION_INFORMATION_EX_V2>() as u32;
        // USB 1.1, 2.0 and 3.0: every protocol this caller understands.
        v2.SupportedUsbProtocols.ul = 0b111;
        let mut returned = 0u32;
        let v2_ok = DeviceIoControl(
            handle,
            IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2,
            std::ptr::addr_of!(v2).cast(),
            v2.Length,
            std::ptr::addr_of_mut!(v2).cast(),
            v2.Length,
            &mut returned,
            std::ptr::null_mut(),
        ) != 0;
        let flags = if v2_ok { v2.Flags.ul } else { 0 };

        // The pipe list on the end is variable length; room for plenty.
        let mut buffer = [0u8; 1024];
        let info = buffer
            .as_mut_ptr()
            .cast::<USB_NODE_CONNECTION_INFORMATION_EX>();
        std::ptr::addr_of_mut!((*info).ConnectionIndex).write_unaligned(port);
        let ex_ok = DeviceIoControl(
            handle,
            IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX,
            info.cast(),
            buffer.len() as u32,
            info.cast(),
            buffer.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        ) != 0;
        let ex_speed = ex_ok.then(|| (*info).Speed);

        CloseHandle(handle);
        speed_from_hub(v2_ok.then_some(flags), ex_speed)
    }
}

/// Turn the hub's two answers into megabits.
///
/// `flags` is `_EX_V2`'s: bit 0 operating at SuperSpeed or higher, bit 2
/// operating at SuperSpeedPlus or higher (bits 1 and 3 are only what the device
/// is *capable* of, which is not the question). `speed` is `_EX`'s
/// `USB_DEVICE_SPEED`: 0 low, 1 full, 2 high, 3 super.
#[cfg_attr(not(windows), allow(dead_code))]
fn speed_from_hub(flags: Option<u32>, speed: Option<u8>) -> Option<u32> {
    const OPERATING_AT_SUPER_SPEED: u32 = 1 << 0;
    const OPERATING_AT_SUPER_SPEED_PLUS: u32 = 1 << 2;

    if let Some(flags) = flags {
        if flags & OPERATING_AT_SUPER_SPEED_PLUS != 0 {
            return Some(10_000);
        }
        if flags & OPERATING_AT_SUPER_SPEED != 0 {
            return Some(5_000);
        }
    }
    match speed? {
        0 => Some(1),
        1 => Some(12),
        2 => Some(480),
        // SuperSpeed by the old call, but V2 did not answer to say which kind.
        // Calling that 5 Gbps would warn about a 10 Gbps link, so it is unknown.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_read_the_way_people_say_them() {
        assert_eq!(speed_label(480), "480 Mbps");
        assert_eq!(speed_label(5000), "5 Gbps");
        assert_eq!(speed_label(10_000), "10 Gbps");
        assert_eq!(speed_label(20_000), "20 Gbps");
        // Not every bridge reports a round number.
        assert_eq!(speed_label(1500), "1.5 Gbps");
    }

    #[test]
    fn a_good_connection_has_nothing_to_say() {
        let health = Health {
            link: "10 Gbps".into(),
            link_mbps: Some(10_000),
            transport: "UASP".into(),
            total_bytes: 128_000_000_000,
            free_bytes: 64_000_000_000,
            used_percent: 50,
            ..Default::default()
        };
        assert!(advise(&health).is_empty());
    }

    #[test]
    fn a_slow_port_is_named_as_the_port_not_the_drive() {
        let health = Health {
            link_mbps: Some(5000),
            transport: "UASP".into(),
            ..Default::default()
        };
        let said = advise(&health).join(" ");
        assert!(said.contains("5 Gbps"), "{said}");
        assert!(said.contains("front-panel"), "{said}");

        let health = Health {
            link_mbps: Some(480),
            ..Default::default()
        };
        let said = advise(&health).join(" ");
        assert!(said.contains("USB 2.0"), "{said}");
    }

    #[test]
    fn bot_and_a_full_drive_are_both_worth_saying() {
        let health = Health {
            link_mbps: Some(10_000),
            transport: "BOT".into(),
            used_percent: 92,
            ..Default::default()
        };
        let said = advise(&health);
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(said[0].contains("UASP"), "{said:?}");
        assert!(said[1].contains("92%"), "{said:?}");
    }

    #[test]
    fn the_windows_tree_is_read_into_transport_and_port() {
        // What TREE_SCRIPT printed for the RTL9210B-CG enclosure on 2026-09-12.
        // PowerShell ends its lines with CRLF, which the parse has to survive.
        let tree = UsbTree::parse(
            "bus=USB\r\n\
             service=UASPStor\r\n\
             hub=USB\\ROOT_HUB30\\9&1191aa4&0&0\r\n\
             port=1\r\n\
             controller=AMD USB 3.20 eXtensible Host Controller - 1.10 (Microsoft)\r\n",
        );
        assert_eq!(tree.transport(), "UASP");
        assert_eq!(tree.hub, "USB\\ROOT_HUB30\\9&1191aa4&0&0");
        assert_eq!(tree.port, 1);
        // A USB 3 controller is no ceiling worth reporting: the port can still
        // have fallen back.
        assert_eq!(tree.controller_ceiling(), None);

        let bot = UsbTree::parse("bus=USB\nservice=USBSTOR\n");
        assert_eq!(bot.transport(), "BOT");

        // An internal disk has no USB node at all.
        let nvme = UsbTree::parse("bus=NVMe\ncontroller=Standard NVM Express Controller\n");
        assert_eq!(nvme.transport(), "");
    }

    #[test]
    fn a_usb_2_controller_caps_the_link_whatever_the_hub_is_called() {
        // 2026-08-20: ROOT_HUB30, on this controller, at 18 MB/s.
        let tree = UsbTree {
            controller: "AMD USB 2.0 eXtensible Host Controller - 1.20".into(),
            ..Default::default()
        };
        assert_eq!(tree.controller_ceiling(), Some(480));
        let ehci = UsbTree {
            controller: "Intel(R) USB Enhanced Host Controller".into(),
            ..Default::default()
        };
        assert_eq!(ehci.controller_ceiling(), Some(480));
    }

    #[test]
    fn the_hub_answer_becomes_a_speed_only_when_it_is_definite() {
        // Operating flags win over capability flags.
        assert_eq!(speed_from_hub(Some(0b0101), Some(3)), Some(10_000));
        assert_eq!(speed_from_hub(Some(0b0001), Some(3)), Some(5_000));
        // Capable of SuperSpeedPlus, running at USB 2.0: the case to warn about.
        assert_eq!(speed_from_hub(Some(0b1010), Some(2)), Some(480));
        assert_eq!(speed_from_hub(None, Some(1)), Some(12));
        // SuperSpeed of an unknown kind is not reported as 5 Gbps.
        assert_eq!(speed_from_hub(None, Some(3)), None);
        assert_eq!(speed_from_hub(None, None), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn transports_are_named_the_way_the_docs_do() {
        assert_eq!(transport_name("uas"), "UASP");
        assert_eq!(transport_name("usb-storage"), "BOT");
        // Anything else is passed through rather than guessed at.
        assert_eq!(transport_name("ums-realtek"), "ums-realtek");
    }

    #[cfg(not(windows))]
    #[test]
    fn a_partition_resolves_to_the_disk_that_holds_it() {
        assert_eq!(block_name("/dev/sdb1").unwrap(), "sdb");
        assert_eq!(block_name("/dev/sdb").unwrap(), "sdb");
        assert_eq!(block_name("/dev/nvme0n1p3").unwrap(), "nvme0n1");
        assert_eq!(block_name("/dev/mmcblk0p1").unwrap(), "mmcblk0");
        assert_eq!(block_name("/dev/nvme0n1").unwrap(), "nvme0n1");
        assert!(block_name("tmpfs").is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn usb_interface_directories_are_recognised() {
        assert!(is_usb_interface("2-4:1.0"));
        assert!(is_usb_interface("1-1.2:1.0"));
        // The device itself, its bus, and PCI addresses are not interfaces.
        assert!(!is_usb_interface("2-4"));
        assert!(!is_usb_interface("usb2"));
        assert!(!is_usb_interface("0000:00:14.0"));
        assert!(!is_usb_interface("host6"));
    }

    #[cfg(not(windows))]
    #[test]
    fn finds_the_mount_in_proc_mounts() {
        let mounts = "\
/dev/sda2 / ext4 rw,relatime 0 0
/dev/sdb1 /run/media/harry/CINDER exfat rw,nosuid 0 0
tmpfs /run tmpfs rw 0 0
";
        assert_eq!(
            device_for_mount(mounts, Path::new("/run/media/harry/CINDER")).unwrap(),
            "/dev/sdb1"
        );
        assert!(device_for_mount(mounts, Path::new("/run")).is_none());
        assert!(device_for_mount(mounts, Path::new("/nowhere")).is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn reads_the_link_out_of_a_sysfs_tree() {
        // A miniature /sys, laid out the way the kernel does it.
        let scratch = crate::testutil::Scratch::new("sysfs");
        let sys = scratch.path();
        let device = sys.join("devices/pci0000:00/0000:00:14.0/usb2/2-4");
        let interface = device.join("2-4:1.0");
        let block = interface.join("host6/target6:0:0/6:0:0:0/block/sdb");
        std::fs::create_dir_all(&block).unwrap();
        std::fs::write(device.join("speed"), b"10000\n").unwrap();

        // The driver is a symlink into /sys/bus, and only its name matters.
        let driver = sys.join("bus/usb/drivers/uas");
        std::fs::create_dir_all(&driver).unwrap();
        std::os::unix::fs::symlink(&driver, interface.join("driver")).unwrap();

        std::fs::create_dir_all(sys.join("class/block")).unwrap();
        std::os::unix::fs::symlink(&block, sys.join("class/block/sdb")).unwrap();

        let link = usb_link(sys, "sdb").expect("the tree describes a USB disk");
        assert_eq!(link.mbps, Some(10_000));
        assert_eq!(link.transport, "UASP");
    }

    #[cfg(not(windows))]
    #[test]
    fn an_internal_disk_simply_has_no_usb_link() {
        let scratch = crate::testutil::Scratch::new("sysfs-internal");
        let sys = scratch.path();
        let block = sys.join("devices/pci0000:00/0000:00:1d.0/nvme/nvme0/nvme0n1");
        std::fs::create_dir_all(&block).unwrap();
        std::fs::create_dir_all(sys.join("class/block")).unwrap();
        std::os::unix::fs::symlink(&block, sys.join("class/block/nvme0n1")).unwrap();

        assert!(usb_link(sys, "nvme0n1").is_none());
    }
}
