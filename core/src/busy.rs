//! Who is still using the cartridge.
//!
//! Eject is the most destructive thing this project does on purpose. Everything
//! else either reads the drive or writes to it while watching; eject takes the
//! volume away, and if a game is still reading from it the game is gone —
//! along with, since saves started travelling on the cartridge, whatever it was
//! part-way through writing there.
//!
//! So before unmounting anything, ask the operating system who has the volume
//! open, and be specific about it. "Something is using the drive" is not
//! actionable. "TombRaider.exe is running from it, and Explorer's working
//! directory is inside it" tells somebody what to close.
//!
//! # What counts as holding the volume
//!
//! Four separate things, because they fail differently and a user can act on
//! each one:
//!
//! * **The program itself is on the drive.** Killing it is safe-ish and is
//!   usually what the user wants; the alternative is that the process dies
//!   anyway the moment the volume goes, at a time nobody chose.
//! * **A file is open.** A save being written is the case that matters, and the
//!   one where pulling the drive corrupts something rather than merely ending
//!   it.
//! * **A file is mapped into memory.** Game data is normally mapped, not read,
//!   and a mapped file whose device disappears takes the process down with a
//!   bus error rather than an error it can handle.
//! * **A working directory is inside it.** Holds the mount busy without holding
//!   any file, which is why an unmount can fail with nothing obviously open —
//!   the classic "device is busy" with no culprit in sight.
//!
//! # What this cannot see
//!
//! On Linux, another user's processes. `/proc/<pid>/fd` is readable by its owner
//! and root, so an unprivileged launcher can enumerate the user's own processes
//! and nothing else. That is reported rather than hidden: [`Holders::unchecked`]
//! counts the processes that could not be looked at, and a caller that finds no
//! holders and a non-zero count has learned less than it looks.
//!
//! On Windows this sees running programs whose executable is on the volume, and
//! not open handles. Handle enumeration there needs either the Restart Manager
//! against a file list or `NtQuerySystemInformation`, and the second is
//! undocumented. Windows also answers the underlying question itself — its
//! dismount fails with a veto — so the guard here is the part that can say
//! *what* to close, not the part that decides whether it is safe.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Why a process is keeping the volume busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    /// Its executable lives on the volume.
    Executable,
    /// It has a file on the volume open.
    OpenFile,
    /// It has a file on the volume mapped into memory.
    MappedFile,
    /// Its working directory is on the volume.
    WorkingDirectory,
}

impl Reason {
    /// How alarming this is, lowest first.
    ///
    /// An open file outranks everything because it is the only one where
    /// pulling the drive corrupts something rather than merely ending it. A
    /// working directory is last because closing a shell is nobody's emergency.
    pub fn severity(&self) -> u8 {
        match self {
            Reason::OpenFile => 0,
            Reason::MappedFile => 1,
            Reason::Executable => 2,
            Reason::WorkingDirectory => 3,
        }
    }

    /// The words a person sees.
    pub fn describe(&self) -> &'static str {
        match self {
            Reason::OpenFile => "has a file open on the cartridge",
            Reason::MappedFile => "is reading the cartridge",
            Reason::Executable => "is running from the cartridge",
            Reason::WorkingDirectory => "is sitting in a folder on the cartridge",
        }
    }
}

/// One process holding the volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holder {
    pub pid: u32,
    /// The process's own name, as the system reports it. Empty when unreadable.
    pub name: String,
    /// The strongest reason it is holding the volume.
    pub reason: Reason,
    /// The path that proved it, for the details sheet and the log.
    pub path: String,
}

impl Holder {
    /// A whole sentence, for a dialog.
    pub fn describe(&self) -> String {
        let name = if self.name.trim().is_empty() {
            format!("Process {}", self.pid)
        } else {
            format!("{} ({})", self.name, self.pid)
        };
        format!("{name} {}", self.reason.describe())
    }
}

/// What was found, and how much of the system could be looked at.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holders {
    pub holders: Vec<Holder>,
    /// Processes that exist and could not be inspected — another user's, or
    /// one that exited while being read.
    ///
    /// Reported because "nothing is using the drive" and "nothing that I am
    /// allowed to see is using the drive" are different answers, and only one
    /// of them justifies yanking a volume.
    pub unchecked: u32,
}

impl Holders {
    pub fn is_empty(&self) -> bool {
        self.holders.is_empty()
    }

    /// Everything except the given process ids.
    ///
    /// The launcher excludes itself: by the time Eject is pressed it has read
    /// the cartridge's artwork and closed it, and a program refusing to eject a
    /// drive because of its own finished reads would be unusable.
    pub fn excluding(mut self, pids: &[u32]) -> Self {
        self.holders.retain(|holder| !pids.contains(&holder.pid));
        self
    }

    /// Worst first, then by pid so the order is stable between calls.
    ///
    /// Both platform implementations end with this rather than sorting for
    /// themselves, so a dialog's first line is the one that matters most
    /// wherever it was built.
    pub fn sort(&mut self) {
        self.holders
            .sort_by_key(|holder| (holder.reason.severity(), holder.pid));
    }

    /// One line naming up to `limit` of them, for a dialog or a log.
    pub fn summary(&self, limit: usize) -> String {
        if self.holders.is_empty() {
            return String::new();
        }
        let named: Vec<String> = self
            .holders
            .iter()
            .take(limit)
            .map(Holder::describe)
            .collect();
        let more = self.holders.len().saturating_sub(named.len());
        if more > 0 {
            format!("{}, and {more} more", named.join("; "))
        } else {
            named.join("; ")
        }
    }
}

/// Who is holding `root`, whoever they are.
///
/// Includes the calling process: this reports what is true, and deciding that
/// one's own handles do not count is the caller's business. See
/// [`Holders::excluding`].
pub fn holders(root: &Path) -> Holders {
    #[cfg(target_os = "linux")]
    {
        linux::holders(root)
    }
    #[cfg(target_os = "windows")]
    {
        windows::holders(root)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        // macOS would use libproc's PROC_PIDLISTFDS here; until that exists,
        // saying nothing is known beats implying the volume is idle.
        let _ = root;
        Holders {
            holders: Vec::new(),
            unchecked: u32::MAX,
        }
    }
}

/// Whether any process is running from one of these folders.
///
/// Its executable inside one, or — Linux only — its working directory. The
/// second is what finds a Windows game under Proton, whose process is a Wine
/// binary from outside the game folder with the game folder as its working
/// directory. Used by the play tracker once a tick, so it reads two links per
/// process and nothing else: no file-descriptor walk, which is the part of
/// [`holders`] that costs.
pub fn running_within(dirs: &[PathBuf]) -> bool {
    if dirs.is_empty() {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        linux::running_within(dirs)
    }
    #[cfg(target_os = "windows")]
    {
        windows::running_within(dirs)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

/// Processes whose executable matches `wanted`, wherever it lives.
///
/// [`holders`] answers "what is using this drive". This answers "is this
/// program running", for a game the cartridge started that is installed
/// somewhere else — a Steam game with a second copy in another library, which
/// Steam is free to run instead of the cartridge's. Executables only: it is a
/// question about which program is running, not about who has which file open.
pub fn running_where(wanted: &dyn Fn(&Path) -> bool) -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
                let exe = std::fs::read_link(entry.path().join("exe")).ok()?;
                wanted(&exe).then_some(pid)
            })
            .collect()
    }
    #[cfg(target_os = "windows")]
    {
        windows::running_where(wanted)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = wanted;
        Vec::new()
    }
}

/// Whether `path` is the root itself or inside it.
///
/// Component-wise, not a string prefix: `/run/media/you/CART` must not match
/// `/run/media/you/CARTRIDGE`, and a prefix test says it does. That bug would
/// refuse to eject one cartridge because a different one was in use.
pub fn is_within(path: &Path, root: &Path) -> bool {
    // Windows paths are case-insensitive, and the two sides rarely agree on
    // case: a process's image path comes back in the case the files have on
    // disk, the root in whatever case it was typed — `d:\` for `D:\`.
    let same = |a: std::path::Component, b: std::path::Component| {
        if cfg!(windows) {
            a.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
        } else {
            a == b
        }
    };
    let mut path = path.components();
    for part in root.components() {
        match path.next() {
            Some(theirs) if same(theirs, part) => {}
            _ => return false,
        }
    }
    true
}

/// How long to let a process put itself away before insisting.
///
/// Asked politely first, and the wait is the point rather than a courtesy: a
/// game told to quit writes its save, and since saves travel on the cartridge
/// now, killing it outright loses exactly the thing the eject was supposed to
/// preserve. Five seconds is longer than a save takes and shorter than anyone
/// will wait twice.
pub const GRACE_SECONDS: u64 = 5;

/// What came of asking everything on the volume to stop.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    /// Processes that quit when asked.
    pub asked: Vec<u32>,
    /// Processes that had to be killed.
    pub killed: Vec<u32>,
    /// Processes still holding the volume afterwards. Not empty means the eject
    /// should not go ahead.
    pub survived: Vec<u32>,
    pub notes: Vec<String>,
}

/// Ask everything holding `root` to stop, then insist.
///
/// Two rounds on purpose. The first is a polite signal every process can catch,
/// which is what gives a game the chance to flush its save to the cartridge
/// before the cartridge goes. Only what is left after [`GRACE_SECONDS`] is
/// killed outright, and what survives even that is reported rather than
/// pretended about — the caller must not eject on top of it.
///
/// Never called on its own: the launcher offers this as *Force quit and eject*,
/// after naming what it is about to close.
pub fn stop_all(root: &Path) -> Stopped {
    stop_all_within(root, GRACE_SECONDS)
}

/// [`stop_all`] with the grace period spelled out.
///
/// The launcher uses the default. This exists so the escalation from "asked" to
/// "killed" can be tested without five seconds of sleeping in every run.
pub fn stop_all_within(root: &Path, grace_seconds: u64) -> Stopped {
    let mut outcome = Stopped::default();
    let initial = holders(root).excluding(&[std::process::id()]);
    if initial.is_empty() {
        return outcome;
    }

    let mut pids: Vec<u32> = initial.holders.iter().map(|holder| holder.pid).collect();
    pids.sort_unstable();
    pids.dedup();

    for pid in &pids {
        match signal(*pid, Signal::Terminate) {
            Ok(()) => outcome.asked.push(*pid),
            Err(why) => outcome.notes.push(why),
        }
    }

    // Poll rather than sleep the whole grace period: a game that exits at once
    // should not cost five seconds of a spinner.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(grace_seconds);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if holders(root).excluding(&[std::process::id()]).is_empty() {
            return outcome;
        }
    }

    let stubborn = holders(root).excluding(&[std::process::id()]);
    for holder in &stubborn.holders {
        match signal(holder.pid, Signal::Kill) {
            Ok(()) => outcome.killed.push(holder.pid),
            Err(why) => outcome.notes.push(why),
        }
    }

    // One last look, because a kill is not instantaneous and because a process
    // this could not signal at all is the case that must not be papered over.
    std::thread::sleep(std::time::Duration::from_millis(300));
    outcome.survived = holders(root)
        .excluding(&[std::process::id()])
        .holders
        .iter()
        .map(|holder| holder.pid)
        .collect();
    outcome.survived.sort_unstable();
    outcome.survived.dedup();
    outcome
}

enum Signal {
    /// Catchable. The one that lets a game save.
    Terminate,
    /// Not catchable.
    Kill,
}

/// Signal one process, without a console window flashing up.
///
/// Through the system's own command rather than a `libc` or `windows-sys` call:
/// this is one process at a time at human speed, on a code path that already
/// shells out to eject a volume, and `crate::proc::command` is the thing that
/// keeps a windowed build from flashing a console while it happens.
fn signal(pid: u32, signal: Signal) -> Result<(), String> {
    #[cfg(windows)]
    let mut command = {
        let mut command = crate::proc::command("taskkill");
        command.args(["/PID", &pid.to_string()]);
        if matches!(signal, Signal::Kill) {
            // Without /F taskkill posts a close message, which is the
            // catchable half of this on Windows.
            command.arg("/F");
        }
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = crate::proc::command("kill");
        command.arg(match signal {
            Signal::Terminate => "-TERM",
            Signal::Kill => "-KILL",
        });
        command.arg(pid.to_string());
        command
    };

    match command.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
            // A process that exited on its own between being listed and being
            // signalled is not a failure; it is the outcome that was wanted.
            if text.contains("No such process") || text.contains("not found") {
                Ok(())
            } else {
                Err(format!("could not stop {pid}: {text}"))
            }
        }
        Err(e) => Err(format!("could not stop {pid}: {e}")),
    }
}

// --------------------------------------------------------------------------
// Linux: /proc, and nothing else
// --------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    pub fn holders(root: &Path) -> Holders {
        let mut found = Holders::default();
        let Ok(entries) = std::fs::read_dir("/proc") else {
            // No /proc at all: a container without it, and nothing can be said.
            return Holders {
                holders: Vec::new(),
                unchecked: u32::MAX,
            };
        };

        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
                continue; // /proc/self, /proc/meminfo, and the rest
            };
            match inspect(pid, root) {
                Inspected::Holding(holder) => found.holders.push(holder),
                Inspected::Idle => {}
                Inspected::Unreadable => found.unchecked += 1,
            }
        }
        found.sort();
        found
    }

    pub fn running_within(dirs: &[PathBuf]) -> bool {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return false;
        };
        let own = std::process::id();
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
                return false;
            };
            // The launcher's own working directory may well be the cartridge.
            if pid == own {
                return false;
            }
            let dir = entry.path();
            ["exe", "cwd"].iter().any(|link| {
                std::fs::read_link(dir.join(link))
                    .is_ok_and(|target| dirs.iter().any(|d| is_within(&target, d)))
            })
        })
    }

    enum Inspected {
        Holding(Holder),
        Idle,
        Unreadable,
    }

    fn inspect(pid: u32, root: &Path) -> Inspected {
        let dir = PathBuf::from(format!("/proc/{pid}"));
        if !dir.is_dir() {
            return Inspected::Idle; // exited between the listing and now
        }

        // Cheapest first, and in the order a user would want to hear about.
        // `fd` is the one that needs permission, so a denial there is what
        // "unchecked" is really counting.
        let mut denied = false;

        if let Some(path) = open_files(&dir, root, &mut denied) {
            return Inspected::Holding(holder(pid, Reason::OpenFile, path));
        }
        if let Some(path) = mapped_files(&dir, root) {
            return Inspected::Holding(holder(pid, Reason::MappedFile, path));
        }
        if let Some(path) = link_within(&dir.join("exe"), root) {
            return Inspected::Holding(holder(pid, Reason::Executable, path));
        }
        if let Some(path) = link_within(&dir.join("cwd"), root) {
            return Inspected::Holding(holder(pid, Reason::WorkingDirectory, path));
        }

        if denied {
            Inspected::Unreadable
        } else {
            Inspected::Idle
        }
    }

    fn holder(pid: u32, reason: Reason, path: PathBuf) -> Holder {
        Holder {
            pid,
            name: process_name(pid),
            reason,
            path: path.display().to_string(),
        }
    }

    /// `/proc/<pid>/comm`, which is the name the kernel knows it by.
    fn process_name(pid: u32) -> String {
        std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    }

    fn link_within(link: &Path, root: &Path) -> Option<PathBuf> {
        let target = std::fs::read_link(link).ok()?;
        is_within(&target, root).then_some(target)
    }

    fn open_files(dir: &Path, root: &Path, denied: &mut bool) -> Option<PathBuf> {
        let entries = match std::fs::read_dir(dir.join("fd")) {
            Ok(entries) => entries,
            Err(e) => {
                // Another user's process, or one that just exited. Only the
                // first is worth counting, but they are indistinguishable here
                // and over-reporting is the safe direction.
                *denied = e.kind() == std::io::ErrorKind::PermissionDenied;
                return None;
            }
        };
        for entry in entries.flatten() {
            if let Some(path) = link_within(&entry.path(), root) {
                return Some(path);
            }
        }
        None
    }

    /// `/proc/<pid>/maps`, for files mapped rather than read.
    ///
    /// A game's data normally arrives this way, and a mapping whose device goes
    /// away kills the process with a bus error it cannot catch — so this is not
    /// a lesser case than an open file, only a quieter one.
    fn mapped_files(dir: &Path, root: &Path) -> Option<PathBuf> {
        let text = std::fs::read_to_string(dir.join("maps")).ok()?;
        text.lines()
            .filter_map(path_in_maps_line)
            .map(PathBuf::from)
            .find(|path| is_within(path, root))
    }

    /// The filename out of one `maps` line, if it names one.
    ///
    /// A line is `address perms offset dev inode pathname`, and the path is
    /// everything from the first `/` — none of the five fields before it can
    /// contain one (`7f..-7f..`, `r-xp`, an offset, `08:01`, a number). That is
    /// simpler than counting fields and, unlike counting, it survives a
    /// filename with a space in it.
    ///
    /// `[heap]`, `[stack]` and anonymous mappings have no `/` and so are
    /// skipped. A mapping the kernel marks `(deleted)` is kept: an unlinked file
    /// that is still mapped still holds the mount, which is one of the harder
    /// "device is busy" cases to explain without it.
    pub(super) fn path_in_maps_line(line: &str) -> Option<&str> {
        let start = line.find('/')?;
        let path = line[start..].trim_end();
        (!path.is_empty()).then_some(path)
    }
}

// --------------------------------------------------------------------------
// Windows: which programs are running from the volume
// --------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, MAX_PATH};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub fn holders(root: &Path) -> Holders {
        let mut found = Holders::default();
        // SAFETY: a snapshot handle is opened, walked, and closed on every
        // path out. Every struct passed in is zeroed and has its dwSize set,
        // which is what the API checks before writing to it.
        unsafe {
            // A `HANDLE` here is an `isize`, not a pointer, so there is no
            // `is_null` to call — and the two calls below fail differently:
            // a snapshot reports INVALID_HANDLE_VALUE and OpenProcess returns a
            // null handle, which is zero.
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Holders {
                    holders: Vec::new(),
                    unchecked: u32::MAX,
                };
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            // ToolHelp will not walk a snapshot that has not been started with
            // First: Next alone returns ERROR_NO_MORE_FILES and the list comes
            // back empty, which would read as "nothing is holding the drive".
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                let pid = entry.th32ProcessID;
                match image_path(pid) {
                    Some(path) if is_within(&path, root) => {
                        found.holders.push(Holder {
                            pid,
                            name: wide_to_string(&entry.szExeFile),
                            reason: Reason::Executable,
                            path: path.display().to_string(),
                        });
                    }
                    Some(_) => {}
                    // A process whose image cannot be asked for: the kernel's
                    // own, or one at a higher integrity level.
                    None => found.unchecked += 1,
                }
                more = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
        }
        found.sort();
        found
    }

    pub fn running_within(dirs: &[PathBuf]) -> bool {
        // Windows paths compare without regard to case, and the image path and
        // the drive root can disagree about the letter's.
        let dirs: Vec<PathBuf> = dirs.iter().map(|d| lowercase(d)).collect();
        let own = std::process::id();
        // SAFETY: as in `holders` — one snapshot, walked and closed.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = false;
            while Process32NextW(snapshot, &mut entry) != 0 {
                if entry.th32ProcessID == own {
                    continue;
                }
                if let Some(path) = image_path(entry.th32ProcessID) {
                    let path = lowercase(&path);
                    if dirs.iter().any(|d| is_within(&path, d)) {
                        found = true;
                        break;
                    }
                }
            }
            CloseHandle(snapshot);
            found
        }
    }

    fn lowercase(path: &Path) -> PathBuf {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    }

    pub fn running_where(wanted: &dyn Fn(&Path) -> bool) -> Vec<u32> {
        let mut found = Vec::new();
        // SAFETY: as in `holders` — one snapshot, walked from First, closed.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return found;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                if let Some(path) = image_path(entry.th32ProcessID) {
                    if wanted(&path) {
                        found.push(entry.th32ProcessID);
                    }
                }
                more = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
        }
        found
    }

    /// The full path of a process's executable, if it can be asked for.
    unsafe fn image_path(pid: u32) -> Option<PathBuf> {
        // The limited variant is the one an unprivileged process is allowed to
        // ask for, and it answers exactly this question.
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return None;
        }
        let mut buffer = [0u16; MAX_PATH as usize];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(
            &buffer[..length as usize],
        )))
    }

    fn wide_to_string(wide: &[u16]) -> String {
        let end = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
        String::from_utf16_lossy(&wide[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the tests that spawn a process or open a file need a scratch
    // directory, and all of those are Linux-only — `/proc` is what they read.
    // Imported unconditionally, this is an unused import on Windows, which
    // `-D warnings` turns into a failed build.
    #[cfg(target_os = "linux")]
    use crate::testutil::Scratch;

    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_working_inside_a_folder_is_running_from_it() {
        let scratch = Scratch::new("busy-running");
        let game = scratch.path().join("Game");
        std::fs::create_dir_all(&game).unwrap();
        let other = scratch.path().join("Other");
        std::fs::create_dir_all(&other).unwrap();

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .current_dir(&game)
            .spawn()
            .expect("sleep");
        // /proc/<pid>/cwd is set by the time spawn returns.
        let seen = running_within(std::slice::from_ref(&game));
        let elsewhere = running_within(&[other]);
        child.kill().ok();
        child.wait().ok();

        assert!(seen, "a process with its working directory in the folder");
        assert!(!elsewhere);
        assert!(!running_within(&[game]), "and not once it has gone");
    }

    #[test]
    fn a_sibling_directory_with_a_shared_prefix_is_not_inside() {
        // The bug a string prefix would have: refusing to eject CART because
        // CARTRIDGE2 is busy.
        let root = Path::new("/run/media/you/CART");
        assert!(is_within(Path::new("/run/media/you/CART"), root));
        assert!(is_within(
            Path::new("/run/media/you/CART/Games/a.exe"),
            root
        ));
        assert!(!is_within(
            Path::new("/run/media/you/CARTRIDGE2/a.exe"),
            root
        ));
        assert!(!is_within(Path::new("/run/media/you"), root));
        assert!(!is_within(Path::new("/etc/passwd"), root));
    }

    #[test]
    fn describing_a_holder_says_what_to_close() {
        let holder = Holder {
            pid: 1234,
            name: "TombRaider.exe".to_string(),
            reason: Reason::Executable,
            path: "/run/media/you/CART/Games/TombRaider.exe".to_string(),
        };
        assert_eq!(
            holder.describe(),
            "TombRaider.exe (1234) is running from the cartridge"
        );

        // A process whose name could not be read is still identified.
        let nameless = Holder {
            name: String::new(),
            ..holder
        };
        assert!(nameless.describe().starts_with("Process 1234"));
    }

    #[test]
    fn a_summary_names_a_few_and_counts_the_rest() {
        let mut found = Holders::default();
        for pid in 1..=5 {
            found.holders.push(Holder {
                pid,
                name: format!("game{pid}"),
                reason: Reason::OpenFile,
                path: "/x".to_string(),
            });
        }
        let summary = found.summary(2);
        assert!(summary.contains("game1"), "{summary}");
        assert!(summary.contains("and 3 more"), "{summary}");
        assert_eq!(Holders::default().summary(2), "");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_open_file_on_the_volume_is_found() {
        // The strongest test available without a real drive: this process opens
        // a file under a directory and then finds itself through /proc.
        let scratch = Scratch::new("busy-open");
        let path = scratch.write("Games/save.dat", b"mid-write");
        let handle = std::fs::File::open(&path).expect("open");

        let found = holders(scratch.path());
        let me = std::process::id();
        let mine = found
            .holders
            .iter()
            .find(|holder| holder.pid == me)
            .unwrap_or_else(|| panic!("this process was not found: {found:?}"));
        assert_eq!(mine.reason, Reason::OpenFile);
        assert!(mine.path.ends_with("save.dat"), "{}", mine.path);
        assert!(!mine.name.is_empty(), "comm should name the test binary");

        drop(handle);
        // And once it is closed, it is not a holder any more.
        let after = holders(scratch.path());
        assert!(
            !after.holders.iter().any(|holder| holder.pid == me),
            "{after:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_working_directory_on_the_volume_is_found() {
        // The "device is busy with nothing open" case. Not run in parallel with
        // anything else that cares: set_current_dir is process-wide.
        let scratch = Scratch::new("busy-cwd");
        std::fs::create_dir_all(scratch.join("Games")).expect("mkdir");
        let original = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(scratch.join("Games")).expect("chdir");

        let found = holders(scratch.path());
        let me = std::process::id();
        let mine = found
            .holders
            .iter()
            .find(|holder| holder.pid == me)
            .cloned();

        std::env::set_current_dir(original).expect("chdir back");

        let mine = mine.unwrap_or_else(|| panic!("this process was not found"));
        assert_eq!(mine.reason, Reason::WorkingDirectory);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_maps_line_gives_up_its_filename() {
        use linux::path_in_maps_line as path_of;

        // Real lines, copied from /proc/self/maps.
        assert_eq!(
            path_of("55a4f7e00000-55a4f7e21000 r--p 00000000 08:01 1443330  /usr/bin/bash"),
            Some("/usr/bin/bash")
        );
        // A filename with a space in it, which counting fields gets wrong.
        assert_eq!(
            path_of(
                "7f1c00000000-7f1c00021000 r--s 00000000 08:01 99  /run/media/you/C/My Game/d.pak"
            ),
            Some("/run/media/you/C/My Game/d.pak")
        );
        // Unlinked and still mapped: still holds the mount.
        assert_eq!(
            path_of("7f1c00000000-7f1c00021000 rw-p 00000000 08:01 99  /run/media/you/C/g.dat (deleted)"),
            Some("/run/media/you/C/g.dat (deleted)")
        );
        // Nothing that holds a volume.
        assert_eq!(
            path_of("55a4f8000000-55a4f8021000 rw-p 00000000 00:00 0  [heap]"),
            None
        );
        assert_eq!(
            path_of("7ffd1c000000-7ffd1c021000 rw-p 00000000 00:00 0"),
            None
        );
        assert_eq!(path_of(""), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_maps_its_own_binary_and_is_found_by_it() {
        // The mapped-file path, exercised for real: every running program has
        // its own executable mapped, so pointing the search at the directory
        // the test binary lives in must find this process.
        let exe = std::env::current_exe().expect("exe");
        let dir = exe.parent().expect("parent");
        let found = holders(dir);
        let me = std::process::id();
        let mine = found
            .holders
            .iter()
            .find(|holder| holder.pid == me)
            .unwrap_or_else(|| panic!("this process was not found under {}", dir.display()));
        // An open file outranks a mapping, and cargo keeps none open here, so
        // this arrives as the executable or the mapping of it.
        assert!(
            matches!(
                mine.reason,
                Reason::MappedFile | Reason::Executable | Reason::OpenFile
            ),
            "{mine:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_idle_directory_holds_nobody() {
        let scratch = Scratch::new("busy-idle");
        scratch.write("Games/unopened.dat", b"nobody has this");
        let found = holders(scratch.path());
        assert!(found.is_empty(), "{found:?}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn other_users_processes_are_counted_rather_than_ignored() {
        // "Nothing is using it" and "nothing I am allowed to see is using it"
        // must not look the same to a caller, because only one of them justifies
        // taking a volume away.
        //
        // Which of those this machine can demonstrate depends on who is running
        // it: as an ordinary user there is always a root process that cannot be
        // read, and as root there is nothing that cannot. Both are asserted,
        // rather than assuming the friendlier one — CI runs as a user, and this
        // project's container runs as root.
        let scratch = Scratch::new("busy-unchecked");
        let found = holders(scratch.path());
        if effective_uid() == Some(0) {
            assert_eq!(
                found.unchecked, 0,
                "root can read every process, so nothing should be unchecked: {found:?}"
            );
        } else {
            assert!(
                found.unchecked > 0,
                "expected at least one of root's processes to be unreadable: {found:?}"
            );
        }
    }

    /// The `Uid:` line of `/proc/self/status`, whose third field is the
    /// effective uid. Read rather than asked for, to stay off libc.
    #[cfg(target_os = "linux")]
    fn effective_uid() -> Option<u32> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    }

    /// Wait for a child to appear as a holder, or give up.
    #[cfg(target_os = "linux")]
    fn wait_for_holder(root: &Path, pid: u32) -> bool {
        for _ in 0..50 {
            if holders(root).holders.iter().any(|h| h.pid == pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        false
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_asked_to_stop_lets_go_of_the_volume() {
        // A real child process holding a real open file, asked to quit the way
        // the launcher's Eject asks. `exec` so the pid that is spawned is the
        // pid that holds the file.
        let scratch = Scratch::new("busy-stop");
        let path = scratch.write("Games/save.dat", b"mid-write");
        let mut child = crate::proc::command("sh")
            .arg("-c")
            .arg(format!("exec sleep 60 < '{}'", path.display()))
            .spawn()
            .expect("spawn");
        let pid = child.id();
        assert!(
            wait_for_holder(scratch.path(), pid),
            "the child never appeared"
        );

        let outcome = stop_all_within(scratch.path(), 2);
        assert!(outcome.asked.contains(&pid), "{outcome:?}");
        assert!(
            outcome.killed.is_empty(),
            "a polite signal should have been enough: {outcome:?}"
        );
        assert!(outcome.survived.is_empty(), "{outcome:?}");
        assert!(holders(scratch.path())
            .excluding(&[std::process::id()])
            .is_empty());

        let _ = child.wait();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_that_ignores_being_asked_is_killed() {
        // The escalation. `exec 3<` so the shell itself holds the file, and a
        // trap so it ignores the polite signal the way a hung game would.
        let scratch = Scratch::new("busy-kill");
        let path = scratch.write("Games/save.dat", b"mid-write");
        let mut child = crate::proc::command("sh")
            .arg("-c")
            .arg(format!(
                "exec 3< '{}'; trap '' TERM; sleep 60",
                path.display()
            ))
            .spawn()
            .expect("spawn");
        let pid = child.id();
        assert!(
            wait_for_holder(scratch.path(), pid),
            "the child never appeared"
        );

        let outcome = stop_all_within(scratch.path(), 1);
        assert!(outcome.asked.contains(&pid), "{outcome:?}");
        assert!(
            outcome.killed.contains(&pid),
            "it ignored the request, so it should have been killed: {outcome:?}"
        );
        assert!(
            outcome.survived.is_empty(),
            "nothing should survive a kill: {outcome:?}"
        );

        let _ = child.wait();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stopping_an_idle_volume_does_nothing_at_all() {
        let scratch = Scratch::new("busy-stop-idle");
        scratch.write("Games/unopened.dat", b"nobody has this");
        assert_eq!(stop_all_within(scratch.path(), 1), Stopped::default());
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_path_is_within_its_root_whatever_the_case() {
        assert!(is_within(
            Path::new(r"C:\Users\h\documents-github\cart\Game\x.exe"),
            Path::new(r"c:\Users\H\Documents-GitHub\cart"),
        ));
        assert!(!is_within(
            Path::new(r"C:\Other\x.exe"),
            Path::new(r"C:\cart")
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn excluding_a_pid_removes_it() {
        let scratch = Scratch::new("busy-exclude");
        let path = scratch.write("save.dat", b"x");
        let _handle = std::fs::File::open(&path).expect("open");
        let me = std::process::id();

        assert!(holders(scratch.path()).holders.iter().any(|h| h.pid == me));
        assert!(!holders(scratch.path())
            .excluding(&[me])
            .holders
            .iter()
            .any(|h| h.pid == me));
    }

    #[test]
    fn the_most_alarming_reason_is_reported_first() {
        let holder = |pid, reason| Holder {
            pid,
            name: format!("p{pid}"),
            reason,
            path: "/x".to_string(),
        };
        let mut found = Holders {
            holders: vec![
                holder(2, Reason::WorkingDirectory),
                holder(9, Reason::OpenFile),
                holder(4, Reason::Executable),
                holder(3, Reason::OpenFile),
                holder(5, Reason::MappedFile),
            ],
            unchecked: 0,
        };
        found.sort();
        let order: Vec<(Reason, u32)> = found
            .holders
            .iter()
            .map(|holder| (holder.reason, holder.pid))
            .collect();
        assert_eq!(
            order,
            vec![
                (Reason::OpenFile, 3),
                (Reason::OpenFile, 9),
                (Reason::MappedFile, 5),
                (Reason::Executable, 4),
                (Reason::WorkingDirectory, 2),
            ]
        );
    }
}
