//! Generic GamePak actions: a stable `gp_` ID, a title and something to do.
//!
//! ```text
//! trigger -> gp_ id -> watcher -> registered action -> execute
//! ```
//!
//! A trigger — NFC, QR code, barcode, a button, an HTTP request, another
//! application — supplies *only* the ID. Paths, commands and host settings live
//! in the host registry, never in the trigger. NFC is just one transport for the
//! ID; see [`crate::nfc`].

use std::collections::HashSet;
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GamePakId(String);

impl GamePakId {
    pub fn parse(value: &str) -> Result<Self, String> {
        let Some(suffix) = value.strip_prefix("gp_") else {
            return Err("GamePak ID must start with gp_".into());
        };
        if suffix.is_empty()
            || suffix.len() > 64
            || !suffix.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte)
            })
        {
            return Err("GamePak ID must use lowercase letters, digits, _ or - after gp_".into());
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a GamePak does when its ID arrives.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    /// A GamePak folder (`cartridge.conf` or `autorun.inf`), opened in the
    /// launcher at READY. This is the original PC GamePak behaviour.
    #[serde(rename_all = "camelCase")]
    Cartridge { path: PathBuf },
    /// Run a program: a local executable, an emulator with a ROM, a local
    /// service, or a streaming client such as Moonlight.
    Exec {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Launch a Steam game by app ID.
    #[serde(rename_all = "camelCase")]
    Steam { app_id: u32 },
    /// Hand a URL or file to the operating system: a media page, a
    /// `moonlight://` link.
    Open { target: String },
    /// Open a local file (a movie, a document) with its default application.
    Local { path: PathBuf },
    /// Stream `app` from the Moonlight host `target` (a name or address) with
    /// the Moonlight client. Pair the host in Moonlight first.
    Moonlight { target: String, app: String },
    /// Start a named system service (systemd user unit on Linux, a Windows
    /// service on Windows).
    Service { target: String },
    /// Start the game on another PC now and stream it when Play is pressed,
    /// all through Moonlight's pairing: the host needs only the app in its
    /// list. Opened in the launcher at READY, like a cartridge. See
    /// [`crate::remote`].
    Remote {
        /// The host as Moonlight names it (or its address).
        moonlight: String,
        /// The app as the host lists it.
        app: String,
    },
}

/// The smallest useful action interface.
pub trait GamePakAction {
    fn execute(&self) -> Result<(), String>;
}

impl GamePakAction for Action {
    /// Start the action and return without waiting for it to finish.
    ///
    /// `Cartridge` is not executed here: it needs the launcher window, so the
    /// launcher handles it and this reports that.
    fn execute(&self) -> Result<(), String> {
        match self {
            Action::Cartridge { .. } => {
                Err("a cartridge action is opened by the launcher window".into())
            }
            Action::Remote { .. } => Err("a remote action is opened by the launcher window".into()),
            Action::Exec { program, args } => {
                if program.trim().is_empty() {
                    return Err("exec action has no program".into());
                }
                run(program, args)
            }
            Action::Steam { app_id } => open_target(&format!("steam://rungameid/{app_id}")),
            Action::Open { target } => open_target(target),
            Action::Local { path } => {
                if !path.is_file() {
                    return Err(format!("{} is not a file", path.display()));
                }
                open_target(&path.to_string_lossy())
            }
            Action::Moonlight { target, app } => {
                if target.trim().is_empty() || target.starts_with('-') || app.starts_with('-') {
                    return Err("moonlight action needs a host and an app name".into());
                }
                run(&moonlight_program(), &["stream", target, app])
            }
            Action::Service { target } => {
                if target.trim().is_empty() || target.starts_with('-') {
                    return Err("service action needs a service name".into());
                }
                #[cfg(windows)]
                return run("sc.exe", &["start", target]);
                #[cfg(not(windows))]
                return run("systemctl", &["--user", "start", target]);
            }
        }
    }
}

/// The Moonlight client: `PC_GAMEPAK_MOONLIGHT` if set, the installer's own
/// folder on Windows (it does not put itself on PATH), else `moonlight`.
pub fn moonlight_program() -> String {
    if let Some(chosen) = std::env::var_os("PC_GAMEPAK_MOONLIGHT") {
        return chosen.to_string_lossy().into_owned();
    }
    #[cfg(windows)]
    for base in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(base) {
            let exe = PathBuf::from(dir)
                .join("Moonlight Game Streaming")
                .join("Moonlight.exe");
            if exe.is_file() {
                return exe.to_string_lossy().into_owned();
            }
        }
    }
    "moonlight".to_string()
}

fn run<S: AsRef<std::ffi::OsStr>>(program: &str, args: &[S]) -> Result<(), String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start {program}: {error}"))
}

fn open_target(target: &str) -> Result<(), String> {
    if target.trim().is_empty() || target.starts_with('-') {
        return Err("open action needs a URL or file path".into());
    }
    #[cfg(windows)]
    let mut command = Command::new("explorer.exe");
    #[cfg(not(windows))]
    let mut command = Command::new("xdg-open");
    command
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open {target}: {error}"))
}

/// A registered GamePak.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamePak {
    pub id: GamePakId,
    pub title: Option<String>,
    pub action: Action,
}

#[derive(Debug, Deserialize, Serialize)]
struct Registry {
    #[serde(default)]
    gamepaks: Vec<Entry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action: Option<Action>,
    /// Shorthand for a `cartridge` action, kept so existing registries work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    #[serde(default)]
    host: Option<Host>,
    /// More IDs that select this GamePak: cards and barcodes learned onto it,
    /// so several of them can start the same game.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    triggers: Vec<String>,
}

impl Entry {
    fn answers_to(&self, id: &str) -> bool {
        self.id == id || self.triggers.iter().any(|trigger| trigger == id)
    }

    fn action(&self) -> Result<Action, String> {
        match (&self.action, &self.path) {
            (Some(action), None) => Ok(action.clone()),
            (None, Some(path)) => Ok(Action::Cartridge { path: path.clone() }),
            _ => Err(format!(
                "GamePak {} needs exactly one of action or path",
                self.id
            )),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Host {
    #[serde(skip_serializing_if = "Option::is_none")]
    wake_on_lan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ready_address: Option<String>,
}

pub fn registry_path() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
            .join("PC-GamePak")
            .join("gamepaks.json")
    }
    #[cfg(not(windows))]
    {
        let state_home = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        state_home.join("pc-gamepak").join("gamepaks.json")
    }
}

fn entries(registry: &Path) -> Result<Vec<Entry>, String> {
    let bytes = std::fs::read(registry).map_err(|error| {
        format!(
            "could not read GamePak registry {}: {error}",
            registry.display()
        )
    })?;
    let parsed: Registry = serde_json::from_slice(&bytes)
        .map_err(|error| format!("could not parse GamePak registry: {error}"))?;
    let mut ids = HashSet::new();
    for entry in &parsed.gamepaks {
        for id in std::iter::once(&entry.id).chain(&entry.triggers) {
            if !ids.insert(GamePakId::parse(id)?) {
                return Err(format!("duplicate GamePak ID in registry: {id}"));
            }
        }
    }
    Ok(parsed.gamepaks)
}

fn cartridge_path(registry: &Path, path: &Path) -> Result<PathBuf, String> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        registry
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    };
    let path = candidate.canonicalize().map_err(|error| {
        format!(
            "registered GamePak path {} is unavailable: {error}",
            candidate.display()
        )
    })?;
    if !path.is_dir()
        || !(path.join("cartridge.conf").is_file() || path.join("autorun.inf").is_file())
    {
        return Err("registered GamePak folder has no cartridge manifest".into());
    }
    Ok(path)
}

fn host_from(
    wake_on_lan: Option<&str>,
    ready_address: Option<&str>,
) -> Result<Option<Host>, String> {
    match (wake_on_lan, ready_address) {
        (None, None) => Ok(None),
        (Some(mac), Some(address)) => {
            magic_packet(mac)?;
            address
                .parse::<SocketAddr>()
                .map_err(|_| format!("invalid host readiness address: {address}"))?;
            Ok(Some(Host {
                wake_on_lan: Some(mac.to_owned()),
                ready_address: Some(address.to_owned()),
            }))
        }
        _ => Err("provide both Wake-on-LAN and readiness address, or neither".into()),
    }
}

/// Register a GamePak folder under `id`. Used by the wizard.
pub fn register_from(
    registry: &Path,
    id: &str,
    path: &Path,
    wake_on_lan: Option<&str>,
    ready_address: Option<&str>,
) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("GamePak path {} is unavailable: {error}", path.display()))?;
    if !path.is_dir()
        || !(path.join("cartridge.conf").is_file() || path.join("autorun.inf").is_file())
    {
        return Err("selected folder does not contain a GamePak manifest".into());
    }
    register_action_from(
        registry,
        id,
        None,
        Action::Cartridge { path },
        wake_on_lan,
        ready_address,
    )
}

/// Register any action under `id`. An ID already bound to a different action
/// is refused: IDs are stable, so they are never silently reassigned.
pub fn register_action_from(
    registry: &Path,
    id: &str,
    title: Option<&str>,
    action: Action,
    wake_on_lan: Option<&str>,
    ready_address: Option<&str>,
) -> Result<(), String> {
    let id = GamePakId::parse(id)?;
    let host = host_from(wake_on_lan, ready_address)?;
    let mut gamepaks = if registry.exists() {
        entries(registry)?
    } else {
        Vec::new()
    };
    if gamepaks
        .iter()
        .any(|entry| entry.id != id.as_str() && entry.answers_to(id.as_str()))
    {
        return Err(format!("{} already starts another GamePak", id.as_str()));
    }
    if let Some(existing) = gamepaks.iter_mut().find(|entry| entry.id == id.as_str()) {
        let same = match (existing.action()?, &action) {
            (Action::Cartridge { path: old }, Action::Cartridge { path: new }) => {
                cartridge_path(registry, &old)? == cartridge_path(registry, new)?
            }
            (old, new) => old == *new,
        };
        if !same {
            return Err(format!(
                "GamePak ID {} is already registered to another action",
                id.as_str()
            ));
        }
        existing.host = host;
    } else {
        let (action, path) = match action {
            Action::Cartridge { path } => (None, Some(path)),
            other => (Some(other), None),
        };
        gamepaks.push(Entry {
            id: id.as_str().to_owned(),
            title: title.map(str::to_owned),
            action,
            path,
            host,
            triggers: Vec::new(),
        });
    }
    save(registry, gamepaks)
}

/// Make `trigger` — a learned card or barcode — start the GamePak `id` too.
/// Adding one it already has is fine; one that starts another is refused.
pub fn add_trigger_from(registry: &Path, id: &str, trigger: &str) -> Result<(), String> {
    let trigger = GamePakId::parse(trigger)?;
    let mut gamepaks = entries(registry)?;
    if let Some(owner) = gamepaks
        .iter()
        .find(|entry| entry.answers_to(trigger.as_str()))
    {
        return if owner.id == id {
            Ok(())
        } else {
            Err(format!(
                "{} already starts {}",
                trigger.as_str(),
                owner.title.as_deref().unwrap_or(&owner.id)
            ))
        };
    }
    let entry = gamepaks
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| format!("GamePak {id} is not registered on this host"))?;
    entry.triggers.push(trigger.as_str().to_owned());
    save(registry, gamepaks)
}

fn save(registry: &Path, gamepaks: Vec<Entry>) -> Result<(), String> {
    if let Some(parent) = registry.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(&Registry { gamepaks })
        .map_err(|error| format!("could not serialize GamePak registry: {error}"))?;
    std::fs::write(registry, bytes).map_err(|error| {
        format!(
            "could not write GamePak registry {}: {error}",
            registry.display()
        )
    })
}

/// One GamePak as the wizard lists it: what it starts and what starts it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
    pub id: String,
    pub title: Option<String>,
    pub action: Action,
    pub triggers: Vec<String>,
}

/// Every registered GamePak, in registry order. No registry yet is an empty
/// list, not an error.
pub fn list_from(registry: &Path) -> Result<Vec<Listed>, String> {
    if !registry.exists() {
        return Ok(Vec::new());
    }
    entries(registry)?
        .into_iter()
        .map(|entry| {
            Ok(Listed {
                action: entry.action()?,
                id: entry.id,
                title: entry.title,
                triggers: entry.triggers,
            })
        })
        .collect()
}

/// Forget the GamePak `id` and every trigger that started it.
pub fn remove_from(registry: &Path, id: &str) -> Result<(), String> {
    let mut gamepaks = entries(registry)?;
    let before = gamepaks.len();
    gamepaks.retain(|entry| entry.id != id);
    if gamepaks.len() == before {
        return Err(format!("GamePak {id} is not registered on this host"));
    }
    save(registry, gamepaks)
}

/// Stop `trigger` starting the GamePak `id`.
pub fn remove_trigger_from(registry: &Path, id: &str, trigger: &str) -> Result<(), String> {
    let mut gamepaks = entries(registry)?;
    let entry = gamepaks
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| format!("GamePak {id} is not registered on this host"))?;
    entry.triggers.retain(|own| own != trigger);
    save(registry, gamepaks)
}

/// Resolve an ID — the GamePak's own or one of its triggers — to the GamePak.
/// The result carries the GamePak's own ID. Cartridge paths are verified.
pub fn lookup_from(registry: &Path, id: &GamePakId) -> Result<GamePak, String> {
    let entry = find(registry, id)?;
    let action = match entry.action()? {
        Action::Cartridge { path } => Action::Cartridge {
            path: cartridge_path(registry, &path)?,
        },
        other => other,
    };
    Ok(GamePak {
        id: GamePakId::parse(&entry.id)?,
        title: entry.title,
        action,
    })
}

fn find(registry: &Path, id: &GamePakId) -> Result<Entry, String> {
    entries(registry)?
        .into_iter()
        .find(|entry| entry.answers_to(id.as_str()))
        .ok_or_else(|| format!("GamePak {} is not registered on this host", id.as_str()))
}

/// The result of a trigger.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A non-cartridge action was started.
    Executed,
    /// The ID names a cartridge; the launcher should open this folder at READY.
    OpenCartridge(PathBuf),
    /// The ID names a game on another PC; the launcher should open at READY
    /// and prime it there. See [`crate::remote`].
    OpenRemote(GamePakId),
}

/// The one path every trigger takes: `gp_id` -> lookup -> host wake -> execute.
///
/// The input is only an ID. Nothing a trigger sends is ever run; an unknown or
/// malformed ID is an error, never an action.
pub fn trigger_from(registry: &Path, id: &str) -> Result<Outcome, String> {
    let id = GamePakId::parse(id.trim())?;
    let pak = lookup_from(registry, &id)?;
    prepare_host(registry, &pak.id)?;
    match pak.action {
        Action::Cartridge { path } => Ok(Outcome::OpenCartridge(path)),
        // The GamePak's own ID, not the card's: it is the one the host knows.
        Action::Remote { .. } => Ok(Outcome::OpenRemote(pak.id)),
        action => action.execute().map(|()| Outcome::Executed),
    }
}

/// A fresh stable ID: `gp_` plus 16 hex digits from the clock and a counter.
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mixed = nanos
        ^ (u64::from(std::process::id()) << 40)
        ^ (u64::from(COUNTER.fetch_add(1, Ordering::Relaxed)) << 56);
    format!("gp_{mixed:016x}")
}

/// Wake the GamePak's host, if it has one, and wait until it answers.
pub fn prepare_host(registry: &Path, id: &GamePakId) -> Result<(), String> {
    let entry = find(registry, id)?;
    let Some(host) = entry.host else {
        return Ok(());
    };
    let (Some(mac), Some(address)) = (host.wake_on_lan, host.ready_address) else {
        return Err("host registry entries must provide both wakeOnLan and readyAddress".into());
    };
    let address: SocketAddr = address
        .parse()
        .map_err(|_| format!("invalid host readiness address: {address}"))?;
    send_packet(&magic_packet(&mac)?)?;
    wait_until_ready(address)
}

/// Wake the machine with this MAC address.
pub fn send_magic_packet(mac: &[u8; 6]) -> Result<(), String> {
    let text: Vec<String> = mac.iter().map(|b| format!("{b:02x}")).collect();
    send_packet(&magic_packet(&text.join(":"))?)
}

fn send_packet(packet: &[u8; 102]) -> Result<(), String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|error| error.to_string())?;
    socket
        .set_broadcast(true)
        .map_err(|error| error.to_string())?;
    socket
        .send_to(packet, "255.255.255.255:9")
        .map_err(|error| format!("could not send Wake-on-LAN packet: {error}"))?;
    Ok(())
}

fn magic_packet(mac: &str) -> Result<[u8; 102], String> {
    let bytes = mac
        .split([':', '-'])
        .map(|part| {
            if part.len() != 2 {
                return Err("Wake-on-LAN address must contain six hex byte pairs".to_string());
            }
            u8::from_str_radix(part, 16)
                .map_err(|_| "Wake-on-LAN address must contain six hex byte pairs".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if bytes.len() != 6 {
        return Err("Wake-on-LAN address must contain six hex byte pairs".into());
    }
    let mut packet = [0xff; 102];
    let (chunks, remainder) = packet[6..].as_chunks_mut::<6>();
    debug_assert!(remainder.is_empty());
    for chunk in chunks {
        chunk.copy_from_slice(&bytes);
    }
    Ok(packet)
}

fn wait_until_ready(address: SocketAddr) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(90) {
        if TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(format!(
        "GamePak host at {address} did not become ready within 90 seconds"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_packet_contains_the_repeated_mac() {
        let packet = magic_packet("00:11:22:33:44:55").unwrap();
        assert_eq!(&packet[..6], &[0xff; 6]);
        let (chunks, remainder) = packet[6..].as_chunks::<6>();
        assert!(remainder.is_empty());
        assert!(chunks
            .iter()
            .all(|part| *part == [0x00, 0x11, 0x22, 0x33, 0x44, 0x55]));
    }

    #[test]
    fn lists_removes_and_tolerates_no_registry() {
        let scratch = crate::testutil::Scratch::new("gamepak-list");
        let registry = scratch.path().join("gamepaks.json");
        assert_eq!(list_from(&registry).unwrap(), vec![]);
        std::fs::write(
            &registry,
            r#"{"gamepaks":[{"id":"gp_a","title":"A","action":{"type":"open","target":"x"},
                             "triggers":["gp_nfc-01","gp_2"]},
                            {"id":"gp_b","path":"games/b"}]}"#,
        )
        .unwrap();
        let listed = list_from(&registry).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].title.as_deref(), Some("A"));
        assert_eq!(listed[0].triggers, ["gp_nfc-01", "gp_2"]);
        assert_eq!(
            listed[1].action,
            Action::Cartridge {
                path: "games/b".into()
            }
        );

        remove_trigger_from(&registry, "gp_a", "gp_nfc-01").unwrap();
        assert_eq!(list_from(&registry).unwrap()[0].triggers, ["gp_2"]);
        remove_from(&registry, "gp_b").unwrap();
        assert_eq!(list_from(&registry).unwrap().len(), 1);
        assert!(remove_from(&registry, "gp_b").is_err());
    }

    #[test]
    fn learned_triggers_start_their_gamepak_and_only_theirs() {
        let scratch = crate::testutil::Scratch::new("gamepak-triggers");
        let registry = scratch.path().join("gamepaks.json");
        let open = |target: &str| Action::Open {
            target: target.into(),
        };
        register_action_from(&registry, "gp_a", Some("A"), open("a"), None, None).unwrap();
        register_action_from(&registry, "gp_b", None, open("b"), None, None).unwrap();
        add_trigger_from(&registry, "gp_a", "gp_nfc-e9105207").unwrap();
        add_trigger_from(&registry, "gp_a", "gp_nfc-4726aa64").unwrap();
        add_trigger_from(&registry, "gp_a", "gp_nfc-4726aa64").unwrap(); // again: fine

        let card = GamePakId::parse("gp_nfc-4726aa64").unwrap();
        let pak = lookup_from(&registry, &card).unwrap();
        assert_eq!(pak.id.as_str(), "gp_a");
        assert_eq!(pak.action, open("a"));

        assert!(add_trigger_from(&registry, "gp_b", "gp_nfc-e9105207").is_err());
        assert!(add_trigger_from(&registry, "gp_b", "gp_a").is_err());
        assert!(
            register_action_from(&registry, "gp_nfc-e9105207", None, open("c"), None, None)
                .is_err()
        );
    }

    #[test]
    fn resolves_a_registered_gamepak_directory() {
        let scratch = crate::testutil::Scratch::new("gamepak-registry");
        let root = scratch.path().join("games/demo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("cartridge.conf"), "title=Demo\n").unwrap();
        let registry = scratch.path().join("gamepaks.json");
        std::fs::write(
            &registry,
            r#"{"gamepaks":[{"id":"gp_demo","path":"games/demo"}]}"#,
        )
        .unwrap();
        let id = GamePakId::parse("gp_demo").unwrap();
        assert_eq!(
            lookup_from(&registry, &id).unwrap().action,
            Action::Cartridge {
                path: root.canonicalize().unwrap()
            }
        );
    }

    #[test]
    fn registers_a_gamepak_and_refuses_to_reassign_its_id() {
        let scratch = crate::testutil::Scratch::new("gamepak-register");
        let root = scratch.path().join("games/demo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("cartridge.conf"), "title=Demo\n").unwrap();
        let other = scratch.path().join("games/other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("cartridge.conf"), "title=Other\n").unwrap();
        let registry = scratch.path().join("state/gamepaks.json");

        register_from(&registry, "gp_demo", &root, None, None).unwrap();
        let id = GamePakId::parse("gp_demo").unwrap();
        assert_eq!(
            lookup_from(&registry, &id).unwrap().action,
            Action::Cartridge {
                path: root.canonicalize().unwrap()
            }
        );
        assert!(register_from(&registry, "gp_demo", &other, None, None).is_err());
    }

    #[test]
    fn resolves_non_cartridge_actions_and_refuses_reassignment() {
        let scratch = crate::testutil::Scratch::new("gamepak-actions");
        let registry = scratch.path().join("gamepaks.json");
        std::fs::write(
            &registry,
            r#"{"gamepaks":[
                {"id":"gp_cp","title":"Cyberpunk 2077","action":{"type":"steam","appId":1091500}},
                {"id":"gp_film","action":{"type":"open","target":"/media/film.mkv"}}
            ]}"#,
        )
        .unwrap();
        let pak = lookup_from(&registry, &GamePakId::parse("gp_cp").unwrap()).unwrap();
        assert_eq!(pak.title.as_deref(), Some("Cyberpunk 2077"));
        assert_eq!(pak.action, Action::Steam { app_id: 1091500 });

        let exec = Action::Exec {
            program: "true".into(),
            args: vec![],
        };
        register_action_from(&registry, "gp_svc", None, exec.clone(), None, None).unwrap();
        register_action_from(&registry, "gp_svc", None, exec, None, None).unwrap();
        let other = Action::Open { target: "x".into() };
        assert!(register_action_from(&registry, "gp_svc", None, other, None, None).is_err());
        assert!(lookup_from(&registry, &GamePakId::parse("gp_none").unwrap()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn trigger_executes_only_registered_actions() {
        let scratch = crate::testutil::Scratch::new("gamepak-trigger");
        let registry = scratch.path().join("gamepaks.json");
        let marker = scratch.path().join("marker");
        let exec = Action::Exec {
            program: "touch".into(),
            args: vec![marker.to_string_lossy().into_owned()],
        };
        register_action_from(&registry, "gp_test", Some("Test"), exec, None, None).unwrap();
        assert!(trigger_from(&registry, "gp_unknown").is_err());
        assert!(trigger_from(&registry, "touch /etc/passwd").is_err());
        assert_eq!(trigger_from(&registry, "gp_test\n"), Ok(Outcome::Executed));
        for _ in 0..50 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(marker.exists());
    }

    #[test]
    fn new_ids_are_valid_and_distinct() {
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert!(GamePakId::parse(&a).is_ok());
    }

    #[test]
    fn entry_needs_exactly_one_of_action_or_path() {
        let scratch = crate::testutil::Scratch::new("gamepak-bad");
        let registry = scratch.path().join("gamepaks.json");
        std::fs::write(&registry, r#"{"gamepaks":[{"id":"gp_bad"}]}"#).unwrap();
        assert!(lookup_from(&registry, &GamePakId::parse("gp_bad").unwrap()).is_err());
    }
}
