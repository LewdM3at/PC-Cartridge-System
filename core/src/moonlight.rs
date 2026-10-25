//! Moonlight's pairing, reused: start an app on a GameStream host (Apollo,
//! Sunshine) without streaming it.
//!
//! Moonlight's command line can stream an app and quit it, but not start one
//! and leave it running. GameStream itself can: a host told to `launch` an app
//! starts it at once and then waits for a stream to connect, and when none
//! comes within a few seconds it drops the waiting session and leaves the app
//! running. (Apollo, `nvhttp.cpp`: `proc::proc.execute` runs before
//! `launch_session_raise`; the session timer only pops the session.) A later
//! `moonlight stream` finds the app running and resumes it.
//!
//! So this sends that one request, the way Moonlight would, with the client
//! certificate Moonlight made when it was paired and the host certificate it
//! pinned then. Nothing new is paired or stored, and the host needs nothing
//! beyond the app being in its list.
//!
//! Moonlight keeps all of that in its settings: an INI file on Linux, the
//! registry on Windows, `Moonlight.ini` beside it when portable.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// GameStream's plain HTTP port, which Moonlight stores per host.
const DEFAULT_HTTP_PORT: u16 = 47989;
/// Its HTTPS port, unless the host's `serverinfo` says otherwise.
const DEFAULT_HTTPS_PORT: u16 = 47984;
/// Moonlight's own unique ID, which every GameStream request carries.
const UNIQUE_ID: &str = "0123456789ABCDEF";

// --------------------------------------------------------------------------
// Moonlight's settings
// --------------------------------------------------------------------------

/// The parts of Moonlight's settings this needs, flattened to `a/b/c` keys:
/// `certificate`, `hosts/1/hostname`, `hosts/1/apps/2/name`...
#[derive(Debug, Default, Clone)]
pub struct Settings {
    values: HashMap<String, Vec<u8>>,
}

/// A host Moonlight has paired with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub name: String,
    /// Where to reach it, best first: the address typed by hand, then the
    /// LAN one, then the remote one. Each `(address, http port)`.
    pub addresses: Vec<(String, u16)>,
    /// The certificate Moonlight pinned when it paired, PEM.
    pub server_cert: Vec<u8>,
    pub mac: Option<[u8; 6]>,
    /// Apps as Moonlight last saw them: `(name, id)`.
    pub apps: Vec<(String, u32)>,
}

impl Settings {
    /// Moonlight's settings on this machine. `PC_GAMEPAK_MOONLIGHT_CONF`
    /// names an INI file to read instead.
    pub fn load() -> Result<Settings, String> {
        if let Some(path) = std::env::var_os("PC_GAMEPAK_MOONLIGHT_CONF") {
            return Settings::from_ini_file(&PathBuf::from(path));
        }
        for path in ini_candidates() {
            if path.is_file() {
                return Settings::from_ini_file(&path);
            }
        }
        #[cfg(windows)]
        {
            let settings = registry::read();
            if !settings.values.is_empty() {
                return Ok(settings);
            }
        }
        Err("Moonlight's settings were not found: install Moonlight and pair it with the host first".into())
    }

    pub fn from_ini_file(path: &std::path::Path) -> Result<Settings, String> {
        let text = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Settings::from_ini(&String::from_utf8_lossy(&text)))
    }

    /// QSettings' INI format: `[section]`, `key=value`, `\` between key
    /// levels, `[General]` for the top level, values quoted and escaped.
    pub fn from_ini(text: &str) -> Settings {
        let mut values = HashMap::new();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = if name == "General" {
                    String::new()
                } else {
                    name.to_string()
                };
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().replace('\\', "/");
            let key = if section.is_empty() {
                key
            } else {
                format!("{section}/{key}")
            };
            values.insert(key, decode_ini_value(value.trim()));
        }
        Settings { values }
    }

    fn text(&self, key: &str) -> Option<String> {
        self.values
            .get(key)
            .map(|v| String::from_utf8_lossy(v).into_owned())
            .filter(|v| !v.is_empty())
    }

    /// Moonlight's client certificate and key, PEM.
    pub fn identity(&self) -> Result<(Vec<u8>, Vec<u8>), String> {
        match (self.values.get("certificate"), self.values.get("key")) {
            (Some(cert), Some(key)) if !cert.is_empty() && !key.is_empty() => {
                Ok((cert.clone(), key.clone()))
            }
            _ => Err("Moonlight has no client certificate yet: pair it with the host once".into()),
        }
    }

    /// The resolution and frame rate Moonlight streams at, for the virtual
    /// display Apollo makes at launch. 1080p60 if it has never been set.
    pub fn mode(&self) -> (u32, u32, u32) {
        let number = |key: &str, fallback: u32| {
            self.text(key)
                .and_then(|v| v.trim().parse().ok())
                .filter(|v: &u32| *v > 0)
                .unwrap_or(fallback)
        };
        (
            number("width", 1920),
            number("height", 1080),
            number("fps", 60),
        )
    }

    /// Every host Moonlight has paired with.
    pub fn hosts(&self) -> Vec<Host> {
        let count = self
            .text("hosts/size")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        (1..=count)
            .filter_map(|n| {
                let get = |key: &str| self.text(&format!("hosts/{n}/{key}"));
                let name = get("hostname")?;
                let server_cert = self.values.get(&format!("hosts/{n}/srvcert"))?.clone();
                let mut addresses = Vec::new();
                for (address, port) in [
                    ("manualaddress", "manualport"),
                    ("localaddress", "localport"),
                    ("remoteaddress", "remoteport"),
                ] {
                    if let Some(address) = get(address) {
                        let port = get(port)
                            .and_then(|p| p.parse().ok())
                            .unwrap_or(DEFAULT_HTTP_PORT);
                        if !addresses.iter().any(|(a, _)| a == &address) {
                            addresses.push((address, port));
                        }
                    }
                }
                let mac = self
                    .values
                    .get(&format!("hosts/{n}/mac"))
                    .and_then(|bytes| <[u8; 6]>::try_from(bytes.as_slice()).ok());
                let app_count = get("apps/size")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                let apps = (1..=app_count)
                    .filter_map(|a| {
                        let app = |key: &str| self.text(&format!("hosts/{n}/apps/{a}/{key}"));
                        Some((app("name")?, app("id")?.parse().ok()?))
                    })
                    .collect();
                Some(Host {
                    name,
                    addresses,
                    server_cert,
                    mac,
                    apps,
                })
            })
            .collect()
    }

    /// The host `wanted` names: its name as Moonlight shows it, or any of
    /// its addresses.
    pub fn host(&self, wanted: &str) -> Result<Host, String> {
        let wanted = wanted.trim();
        self.hosts()
            .into_iter()
            .find(|host| {
                host.name.eq_ignore_ascii_case(wanted)
                    || host
                        .addresses
                        .iter()
                        .any(|(a, _)| a.eq_ignore_ascii_case(wanted))
            })
            .ok_or_else(|| format!("Moonlight has not paired with {wanted}"))
    }
}

/// Where Moonlight's INI lives on this platform, most likely first.
fn ini_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    // Portable Moonlight keeps it beside itself.
    let program = PathBuf::from(crate::gamepak::moonlight_program());
    if let Some(dir) = program.parent().filter(|d| !d.as_os_str().is_empty()) {
        out.push(
            dir.join("Moonlight Game Streaming Project")
                .join("Moonlight.ini"),
        );
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        out.push(
            config
                .join("Moonlight Game Streaming Project")
                .join("Moonlight.conf"),
        );
        // The Flatpak's own config directory.
        out.push(
            home.join(".var/app/com.moonlight_stream.Moonlight/config")
                .join("Moonlight Game Streaming Project")
                .join("Moonlight.conf"),
        );
    }
    out
}

/// A QSettings INI value as bytes: quotes and escapes undone, and the
/// `@ByteArray(...)` wrapper taken off.
fn decode_ini_value(raw: &str) -> Vec<u8> {
    let unquoted = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(raw);
    let mut out = Vec::with_capacity(unquoted.len());
    let mut chars = unquoted.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('n') => out.push(b'\n'),
            Some('r') => out.push(b'\r'),
            Some('t') => out.push(b'\t'),
            Some('0') => out.push(0),
            Some('x') => {
                let mut value = 0u32;
                let mut digits = 0;
                while digits < 2 {
                    match chars.peek().and_then(|c| c.to_digit(16)) {
                        Some(d) => {
                            value = value * 16 + d;
                            chars.next();
                            digits += 1;
                        }
                        None => break,
                    }
                }
                out.push(value as u8);
            }
            Some(other) => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            }
            None => out.push(b'\\'),
        }
    }
    strip_byte_array(out)
}

fn strip_byte_array(value: Vec<u8>) -> Vec<u8> {
    match value
        .strip_prefix(b"@ByteArray(")
        .and_then(|v| v.strip_suffix(b")"))
    {
        Some(inner) => inner.to_vec(),
        None => value,
    }
}

#[cfg(windows)]
mod registry {
    //! Moonlight on Windows keeps QSettings in the registry: one subkey per
    //! level, `HKCU\Software\Moonlight Game Streaming Project\Moonlight\hosts\1`
    //! holding `hostname`, `srvcert`...

    use std::collections::HashMap;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
        KEY_READ, REG_BINARY, REG_DWORD, REG_QWORD, REG_SZ,
    };

    const ROOT: &str = r"Software\Moonlight Game Streaming Project\Moonlight";

    pub fn read() -> super::Settings {
        let mut values = HashMap::new();
        walk(ROOT, "", &mut values, 0);
        super::Settings { values }
    }

    /// A QSettings string value as bytes: UTF-16, and for a byte array
    /// `@ByteArray(...)` whose characters are the bytes.
    fn from_qt_text(data: &[u8], stop_at_nul: bool) -> Vec<u8> {
        let wide = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&c| !stop_at_nul || c != 0);
        let text: String = char::decode_utf16(wide)
            .map(|c| c.unwrap_or('\u{fffd}'))
            .collect();
        let bytes: Vec<u8> = if text.starts_with("@ByteArray(") {
            text.chars().map(|c| c as u32 as u8).collect()
        } else {
            text.into_bytes()
        };
        super::strip_byte_array(bytes)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn walk(path: &str, prefix: &str, values: &mut HashMap<String, Vec<u8>>, depth: usize) {
        if depth > 6 {
            return;
        }
        let mut key: HKEY = 0;
        // SAFETY: a valid wide string and an out pointer; the key is closed below.
        if unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(path).as_ptr(),
                0,
                KEY_READ,
                &mut key,
            )
        } != 0
        {
            return;
        }
        let mut index = 0;
        loop {
            let mut name = [0u16; 512];
            let mut name_len = name.len() as u32;
            let mut kind = 0u32;
            let mut data = vec![0u8; 64 * 1024];
            let mut data_len = data.len() as u32;
            // SAFETY: buffers sized as passed; the call writes within them.
            let status = unsafe {
                RegEnumValueW(
                    key,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut data_len,
                )
            };
            if status != 0 {
                break;
            }
            index += 1;
            data.truncate(data_len as usize);
            let name = String::from_utf16_lossy(&name[..name_len as usize]);
            let value = match kind {
                // QSettings writes text as REG_SZ, and text containing a NUL
                // (a byte array with a zero byte in it) as REG_BINARY holding
                // the same UTF-16; it is read back the same way here.
                REG_SZ => from_qt_text(&data, true),
                REG_BINARY => from_qt_text(&data, false),
                REG_DWORD if data.len() >= 4 => {
                    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
                        .to_string()
                        .into_bytes()
                }
                REG_QWORD if data.len() >= 8 => {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&data[..8]);
                    u64::from_le_bytes(b).to_string().into_bytes()
                }
                _ => continue,
            };
            values.insert(format!("{prefix}{name}"), value);
        }
        let mut index = 0;
        loop {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            // SAFETY: as above.
            let status = unsafe {
                RegEnumKeyExW(
                    key,
                    index,
                    name.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if status != 0 {
                break;
            }
            index += 1;
            let sub = String::from_utf16_lossy(&name[..name_len as usize]);
            walk(
                &format!(r"{path}\{sub}"),
                &format!("{prefix}{sub}/"),
                values,
                depth + 1,
            );
        }
        // SAFETY: opened above.
        unsafe { RegCloseKey(key) };
    }
}

// --------------------------------------------------------------------------
// GameStream
// --------------------------------------------------------------------------

/// The paired host's certificate, and nothing else, is trusted: a host is
/// self-signed, so the usual chain of trust has nothing to say about it.
#[derive(Debug)]
struct Pinned {
    cert: CertificateDer<'static>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.cert.as_ref() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "this is not the host Moonlight paired with".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A connection to one host, as Moonlight would make it.
pub struct Client {
    agent: ureq::Agent,
    plain: ureq::Agent,
    host: Host,
    /// Which of the host's addresses answered.
    address: Option<(String, u16)>,
}

impl Client {
    pub fn new(host: Host, cert_pem: &[u8], key_pem: &[u8]) -> Result<Client, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let client_cert = CertificateDer::from_pem_slice(cert_pem)
            .map_err(|e| format!("Moonlight's client certificate: {e}"))?;
        let key = PrivateKeyDer::from_pem_slice(key_pem)
            .map_err(|e| format!("Moonlight's client key: {e}"))?;
        let server_cert = CertificateDer::from_pem_slice(&host.server_cert)
            .map_err(|e| format!("the certificate Moonlight pinned for {}: {e}", host.name))?
            .into_owned();
        let config = rustls::ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Pinned {
                cert: server_cert,
                provider,
            }))
            .with_client_auth_cert(vec![client_cert.into_owned()], key)
            .map_err(|e| format!("Moonlight's client certificate: {e}"))?;
        Ok(Client {
            agent: ureq::AgentBuilder::new()
                .tls_config(Arc::new(config))
                .timeout(Duration::from_secs(15))
                .build(),
            plain: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(3))
                .build(),
            host,
            address: None,
        })
    }

    /// The host's unauthenticated `serverinfo`, from whichever address
    /// answers first. Remembers that address.
    fn server_info(&mut self) -> Option<String> {
        for (address, port) in self.host.addresses.clone() {
            let url = format!(
                "http://{}:{port}/serverinfo?uniqueid={UNIQUE_ID}",
                bracket(&address)
            );
            let body = self
                .plain
                .get(&url)
                .call()
                .ok()
                .and_then(|r| r.into_string().ok());
            if let Some(body) = body {
                self.address = Some((address, port));
                return Some(body);
            }
        }
        None
    }

    /// Make sure the host is up: wake it with Moonlight's stored MAC address
    /// if it does not answer, and wait for it.
    pub fn wake(&mut self) -> Result<String, String> {
        if let Some(info) = self.server_info() {
            return Ok(info);
        }
        let Some(mac) = self.host.mac else {
            return Err(format!(
                "{} does not answer, and Moonlight has no MAC address to wake it with",
                self.host.name
            ));
        };
        crate::gamepak::send_magic_packet(&mac)?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(90) {
            std::thread::sleep(Duration::from_secs(2));
            if let Some(info) = self.server_info() {
                return Ok(info);
            }
        }
        Err(format!("{} did not wake within 90 seconds", self.host.name))
    }

    fn https(&self, info: &str, verb: &str, query: &str) -> Result<ureq::Response, String> {
        let (address, _) = self
            .address
            .clone()
            .ok_or_else(|| format!("{} has not answered yet", self.host.name))?;
        let port = xml_text(info, "HttpsPort")
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_HTTPS_PORT);
        let url = format!(
            "https://{}:{port}/{verb}?uniqueid={UNIQUE_ID}&uuid={}{query}",
            bracket(&address),
            random_hex(16)?
        );
        self.agent.get(&url).call().map_err(|e| match e {
            ureq::Error::Transport(t) => format!("{}: {t}", self.host.name),
            ureq::Error::Status(code, _) => format!("{} answered {code}", self.host.name),
        })
    }

    /// The host's apps, fresh, `(name, id)`.
    pub fn apps(&self, info: &str) -> Result<Vec<(String, u32)>, String> {
        let body = read_body(self.https(info, "applist", "")?)?;
        check_status(&body)?;
        Ok(body
            .split("<App>")
            .skip(1)
            .filter_map(|app| {
                Some((
                    xml_unescape(&xml_text(app, "AppTitle")?),
                    xml_text(app, "ID")?.parse().ok()?,
                ))
            })
            .collect())
    }

    /// Start `app` on the host without streaming it. Returns its ID. An app
    /// already running is left running.
    pub fn launch(&mut self, app: &str, mode: (u32, u32, u32)) -> Result<Launched, String> {
        let info = self.wake()?;
        // The host's list as it is now; Moonlight's copy if the host will not
        // say, and why not if there is no copy either.
        let apps = match self.apps(&info) {
            Ok(apps) => apps,
            Err(_) if !self.host.apps.is_empty() => self.host.apps.clone(),
            Err(why) => return Err(why),
        };
        let (title, id) = apps
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(app.trim()))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "{} has no app called {app}. It has: {}",
                    self.host.name,
                    apps.iter()
                        .map(|(n, _)| n.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;

        let running = xml_text(&info, "currentgame")
            .and_then(|g| g.parse::<u32>().ok())
            .unwrap_or(0);
        if running != id {
            if running != 0 {
                return Err(format!(
                    "{} is already running something else; eject it first",
                    self.host.name
                ));
            }
            let (width, height, fps) = mode;
            // The input key is for a stream that this request never opens;
            // Moonlight makes a new one when it resumes.
            let rikeyid = u32::from_be_bytes(random_bytes::<4>()?);
            let query = format!(
                "&appid={id}&mode={width}x{height}x{fps}&additionalStates=1&sops=0\
                 &rikey={}&rikeyid={rikeyid}&localAudioPlayMode=0\
                 &surroundAudioInfo=196610&remoteControllersBitmap=0&gcmap=0",
                random_hex(16)?
            );
            let body = read_body(self.https(&info, "launch", &query)?)?;
            check_status(&body)?;
        }

        let cover = self
            .https(
                &info,
                "appasset",
                &format!("&appid={id}&AssetType=2&AssetIdx=0"),
            )
            .ok()
            .and_then(|response| read_bytes(response, 8 * 1024 * 1024).ok())
            .filter(|bytes| bytes.starts_with(b"\x89PNG"))
            .map(|bytes| {
                format!(
                    "data:image/png;base64,{}",
                    crate::cartridge::base64_encode(&bytes)
                )
            });
        Ok(Launched {
            title,
            id,
            already_running: running == id,
            cover,
        })
    }
}

use std::io::Read;

/// A reply's body as text. A host that closes the connection without TLS's
/// goodbye, after sending what it had to send, is taken at its word: some
/// GameStream servers do, and the reply is complete.
fn read_body(response: ureq::Response) -> Result<String, String> {
    read_bytes(response, 1024 * 1024).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn read_bytes(response: ureq::Response, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    match response.into_reader().take(limit).read_to_end(&mut bytes) {
        Ok(_) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !bytes.is_empty() => Ok(bytes),
        Err(e) => Err(e.to_string()),
    }
}

/// What `launch` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launched {
    pub title: String,
    pub id: u32,
    pub already_running: bool,
    /// The app's box art from the host, as a `data:` URI, when it has one.
    pub cover: Option<String>,
}

fn bracket(address: &str) -> String {
    if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]")
    } else {
        address.to_string()
    }
}

/// The text of the first `<tag>…</tag>`.
fn xml_text(xml: &str, tag: &str) -> Option<String> {
    let start = xml.find(&format!("<{tag}>"))? + tag.len() + 2;
    let end = start + xml[start..].find(&format!("</{tag}>"))?;
    Some(xml[start..end].trim().to_string())
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// A GameStream reply's `status_code`, as an error unless it is 200.
fn check_status(body: &str) -> Result<(), String> {
    let attribute = |name: &str| {
        let at = body.find(&format!("{name}=\""))? + name.len() + 2;
        Some(body[at..at + body[at..].find('"')?].to_string())
    };
    match attribute("status_code").as_deref() {
        Some("200") => Ok(()),
        Some(code) => Err(format!(
            "the host refused ({code}): {}",
            attribute("status_message").unwrap_or_default()
        )),
        None => Err("the host's answer had no status".into()),
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn random_hex(len: usize) -> Result<String, String> {
    let mut bytes = vec![0u8; len];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: &str = r#"[General]
certificate="@ByteArray(-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n)"
key="@ByteArray(-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----\n)"
width=2560
height=1440
fps=120

[hosts]
1\apps\1\id=881448767
1\apps\1\name=Desktop
1\apps\2\id=1093255277
1\apps\2\name=Cyberpunk 2077
1\apps\size=2
1\hostname=GAMING-PC
1\localaddress=192.168.1.20
1\localport=47989
1\mac=@ByteArray(\x1c\x1b\r\x10 \x30)
1\manualaddress=gaming-pc.lan
1\srvcert="@ByteArray(-----BEGIN CERTIFICATE-----\nMIIC\n-----END CERTIFICATE-----\n)"
1\uuid=1a2b
size=1
"#;

    #[test]
    fn moonlight_ini_reads_into_identity_mode_and_hosts() {
        let settings = Settings::from_ini(CONF);
        let (cert, key) = settings.identity().unwrap();
        assert!(cert.starts_with(b"-----BEGIN CERTIFICATE-----\nMIIB\n"));
        assert!(key.ends_with(b"-----END PRIVATE KEY-----\n"));
        assert_eq!(settings.mode(), (2560, 1440, 120));

        let host = settings.host("gaming-pc").unwrap();
        assert_eq!(host.name, "GAMING-PC");
        assert_eq!(
            host.addresses,
            [
                ("gaming-pc.lan".to_string(), 47989),
                ("192.168.1.20".to_string(), 47989)
            ]
        );
        assert_eq!(host.mac, Some([0x1c, 0x1b, b'\r', 0x10, b' ', 0x30]));
        assert_eq!(host.apps[1], ("Cyberpunk 2077".to_string(), 1093255277));
        assert!(host
            .server_cert
            .starts_with(b"-----BEGIN CERTIFICATE-----\nMIIC"));

        // Found by address too, and a stranger is a clear error.
        assert_eq!(settings.host("192.168.1.20").unwrap().name, "GAMING-PC");
        assert!(settings
            .host("other-pc")
            .unwrap_err()
            .contains("not paired"));
    }

    #[test]
    fn an_unpaired_moonlight_says_so() {
        let settings = Settings::from_ini("[General]\nwidth=1280\n");
        assert!(settings.identity().unwrap_err().contains("pair"));
        assert_eq!(settings.mode(), (1280, 1080, 60));
        assert!(settings.hosts().is_empty());
    }

    /// What this machine's Moonlight settings hold:
    /// `cargo test -- --ignored moonlight_settings_here --nocapture`.
    #[test]
    #[ignore]
    fn moonlight_settings_here() {
        let settings = Settings::load().unwrap();
        let (cert, key) = settings.identity().unwrap();
        println!(
            "SETTINGS cert {} key {} mode {:?}",
            String::from_utf8_lossy(&cert[..27.min(cert.len())]),
            key.len(),
            settings.mode()
        );
        for host in settings.hosts() {
            println!(
                "SETTINGS host {} {:?} mac {:?} apps {:?} srvcert {}",
                host.name,
                host.addresses,
                host.mac,
                host.apps,
                host.server_cert.len()
            );
        }
    }

    /// Against a real (or mock) host, named by a Moonlight INI in
    /// `PC_GAMEPAK_E2E_CONF`: `cargo test -- --ignored gamestream_end_to_end`.
    #[test]
    #[ignore]
    fn gamestream_end_to_end() {
        let conf = std::env::var("PC_GAMEPAK_E2E_CONF").expect("PC_GAMEPAK_E2E_CONF");
        let app = std::env::var("PC_GAMEPAK_E2E_APP").unwrap_or_else(|_| "Desktop".into());
        let settings = Settings::from_ini_file(std::path::Path::new(&conf)).unwrap();
        let (cert, key) = settings.identity().unwrap();
        let host = settings.hosts().remove(0);
        let mut client = Client::new(host, &cert, &key).unwrap();
        match client.launch(&app, settings.mode()) {
            Ok(launched) => println!("E2E OK {launched:?}"),
            Err(why) => println!("E2E ERR {why}"),
        }
    }

    #[test]
    fn gamestream_replies_are_read() {
        assert!(
            check_status(r#"<root status_code="200"><gamesession>1</gamesession></root>"#).is_ok()
        );
        let refused = check_status(
            r#"<root status_code="400" status_message="An app is already running on this host"/>"#,
        )
        .unwrap_err();
        assert!(refused.contains("400") && refused.contains("already running"));
        assert_eq!(
            xml_text("<root><HttpsPort>47984</HttpsPort></root>", "HttpsPort").as_deref(),
            Some("47984")
        );
        assert_eq!(xml_unescape("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(bracket("fe80::1"), "[fe80::1]");
        assert_eq!(bracket("10.0.0.2"), "10.0.0.2");
    }
}
