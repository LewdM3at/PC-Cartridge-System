use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://www.steamgriddb.com/api/v2";
const DEFAULT_MAX_AGE_DAYS: u64 = 30;
const DEFAULT_RETRIES: usize = 4;
const USER_AGENT: &str = "pc-gamepak/0.1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtworkType {
    Grid,
    Hero,
    Icon,
    Logo,
    Cover,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamGridGame {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artwork {
    pub id: u32,
    pub url: String,
    pub thumb: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedArtwork {
    pub path: String,
    pub data_uri: String,
}

#[derive(Debug, Deserialize)]
struct GamesEnvelope {
    success: bool,
    #[serde(default)]
    data: Vec<RawGame>,
}

#[derive(Debug, Deserialize)]
struct ArtworkEnvelope {
    success: bool,
    #[serde(default)]
    data: Vec<RawArtwork>,
}

#[derive(Debug, Deserialize)]
struct RawGame {
    id: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawArtwork {
    id: u32,
    #[serde(default)]
    url: String,
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LastUsedMap {
    #[serde(default)]
    by_game: HashMap<String, String>,
}

fn cache_root() -> PathBuf {
    if let Ok(override_dir) = std::env::var("SGDB_CACHE_DIR") {
        if !override_dir.trim().is_empty() {
            return PathBuf::from(override_dir);
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            if !local.trim().is_empty() {
                return PathBuf::from(local).join("PC-GamePak").join("sgdb-cache");
            }
        }
        PathBuf::from(".").join("sgdb-cache")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("pc-gamepak")
            .join("sgdb-cache")
    }
}

fn cache_images_dir() -> PathBuf {
    cache_root().join("images")
}

fn cache_index_path() -> PathBuf {
    cache_root().join("last-used.json")
}

fn max_age() -> Duration {
    let days = std::env::var("SGDB_CACHE_MAX_AGE_DAYS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_AGE_DAYS);
    Duration::from_secs(days * 24 * 60 * 60)
}

fn ensure_cache_dirs() -> Result<(), String> {
    std::fs::create_dir_all(cache_images_dir())
        .map_err(|e| format!("could not create SGDB cache directory: {e}"))
}

fn ext_from_url(url: &str) -> &'static str {
    let trimmed = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    if trimmed.ends_with(".png") {
        "png"
    } else if trimmed.ends_with(".webp") {
        "webp"
    } else if trimmed.ends_with(".bmp") {
        "bmp"
    } else if trimmed.ends_with(".ico") {
        // The icons tab serves plenty of these, and calling one `.jpg` on the
        // way in is how a perfectly good icon reached the cartridge unusable.
        "ico"
    } else {
        "jpg"
    }
}

fn cache_filename(cache_key: &str, url: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cache_key.hash(&mut hasher);
    url.hash(&mut hasher);
    let hash = hasher.finish();
    format!(
        "{}-{hash:016x}.{}",
        sanitize_key(cache_key),
        ext_from_url(url)
    )
}

fn sanitize_key(input: &str) -> String {
    let compact: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let folded = compact
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if folded.is_empty() {
        "artwork".to_string()
    } else {
        folded.chars().take(80).collect()
    }
}

fn is_fresh(path: &Path) -> bool {
    let age = max_age();
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
        return false;
    };
    elapsed <= age
}

fn cleanup_stale_cache() {
    let dir = cache_images_dir();
    let max = max_age();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
            continue;
        };
        if elapsed > max {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn read_last_used_map() -> LastUsedMap {
    let path = cache_index_path();
    let bytes = std::fs::read(path).ok();
    bytes
        .and_then(|b| serde_json::from_slice::<LastUsedMap>(&b).ok())
        .unwrap_or_else(|| LastUsedMap {
            by_game: HashMap::new(),
        })
}

fn write_last_used_map(map: &LastUsedMap) -> Result<(), String> {
    ensure_cache_dirs()?;
    let bytes = serde_json::to_vec_pretty(map)
        .map_err(|e| format!("could not serialize cache map: {e}"))?;
    std::fs::write(cache_index_path(), bytes)
        .map_err(|e| format!("could not write SGDB cache index: {e}"))
}

pub fn last_used_artwork(game_key: &str) -> Option<PathBuf> {
    let key = sanitize_key(game_key);
    let map = read_last_used_map();
    map.by_game
        .get(&key)
        .map(PathBuf::from)
        .filter(|path| path.is_file() && is_fresh(path))
}

fn mime_from_ext(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        // The icons tab serves these, and the wizard previews whatever was
        // picked - so getting this wrong is a chosen icon that shows as a
        // broken image in the preview it was chosen from.
        "ico" => "image/vnd.microsoft.icon",
        _ => "image/jpeg",
    }
}

/// Inline a picture for the window to show.
///
/// Capped at the same size a cartridge cover is: this is handed arbitrary files
/// once the user can pick one from disk, and base64 costs a third again on top
/// of whatever they chose.
pub fn read_as_data_uri(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > crate::cartridge::MAX_COVER_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!(
        "data:{};base64,{}",
        mime_from_ext(path),
        crate::base64_encode(&bytes)
    ))
}

pub fn last_used_artwork_data_uri(game_key: &str) -> Option<CachedArtwork> {
    let path = last_used_artwork(game_key)?;
    let data_uri = read_as_data_uri(&path)?;
    Some(CachedArtwork {
        path: path.to_string_lossy().to_string(),
        data_uri,
    })
}

pub fn remember_last_used(game_key: &str, path: &Path) -> Result<(), String> {
    let key = sanitize_key(game_key);
    let mut map = read_last_used_map();
    map.by_game.insert(key, path.to_string_lossy().to_string());
    write_last_used_map(&map)
}

/// Portrait grid sizes, in the order SteamGridDB itself lists them.
///
/// These are what a cartridge cover wants: box art, taller than it is wide.
const COVER_DIMENSIONS: &str = "600x900,342x482,660x930";

fn endpoint_for(game_id: u32, art_type: ArtworkType) -> String {
    match art_type {
        ArtworkType::Grid => format!("{API_BASE}/grids/game/{game_id}?dimensions=600x900,460x215"),
        ArtworkType::Hero => format!("{API_BASE}/heroes/game/{game_id}"),
        ArtworkType::Icon => format!("{API_BASE}/icons/game/{game_id}"),
        ArtworkType::Logo => format!("{API_BASE}/logos/game/{game_id}"),
        // There is no /covers endpoint. The v2 API serves grids, heroes, logos
        // and icons, and a "cover" is a grid in one of the portrait sizes — so
        // asking for /covers/game/<id> is a 404 every time, for every game.
        ArtworkType::Cover => {
            format!("{API_BASE}/grids/game/{game_id}?dimensions={COVER_DIMENSIONS}")
        }
    }
}

/// The one gate every request passes through.
///
/// SteamGridDB is the only thing in this project that talks to the network, so
/// it stays switched off until someone turns it on, and it is refused here
/// rather than in the window — a setting the UI merely hides is not a setting.
fn api_key() -> Result<String, String> {
    api_key_from(&crate::settings::load())
}

fn api_key_from(settings: &crate::settings::Settings) -> Result<String, String> {
    if !settings.steamgriddb_enabled {
        return Err(
            "SteamGridDB lookup is switched off. Turn it on in the wizard's settings.".to_string(),
        );
    }
    settings
        .steamgriddb_key()
        .map(str::to_string)
        .ok_or_else(|| {
            "SteamGridDB needs a personal API key. Add one in the wizard's settings.".to_string()
        })
}

/// What a 404 is turned into: a successful response carrying nothing.
///
/// Both envelopes this module parses are `{ success, data }` with `data` a
/// list, so one body serves for either.
const NOT_FOUND_BODY: &[u8] = br#"{"success":true,"data":[]}"#;

/// A call to the v2 API, which refuses anything unauthenticated.
fn request_with_retry(url: &str) -> Result<Vec<u8>, String> {
    let key = api_key()?;
    get_with_retry(url, Some(&key), true)
}

/// A fetch of one image from the CDN.
///
/// Deliberately unauthenticated. The images are served from a different host to
/// the API — cdn2.steamgriddb.com rather than www.steamgriddb.com — and it does
/// not take the API key: a request carrying one is answered with 401, so every
/// download failed while every search succeeded.
///
/// It also means the "paste a URL" field works with no API key at all, which is
/// the whole point of offering it to someone who has lookup switched off.
fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
    get_with_retry(url, None, false)
}

/// One GET, retrying the failures worth retrying.
///
/// `not_found_is_empty` separates the two callers' idea of a 404. For the API it
/// is an ordinary answer — this game has no artwork of that kind — and an empty
/// envelope lets the caller carry on. For an image it is a real failure, and
/// handing back a JSON envelope pretending to be a picture would only move the
/// error somewhere harder to read.
fn get_with_retry(
    url: &str,
    key: Option<&str>,
    not_found_is_empty: bool,
) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build();

    for attempt in 0..DEFAULT_RETRIES {
        let mut request = agent.get(url);
        if let Some(key) = key {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
        let response = request.call();
        match response {
            Ok(resp) => {
                let mut reader = resp.into_reader();
                let mut bytes = Vec::new();
                std::io::copy(&mut reader, &mut bytes)
                    .map_err(|e| format!("could not read response body: {e}"))?;
                return Ok(bytes);
            }
            Err(ureq::Error::Status(code, resp)) => {
                if code == 429 || code >= 500 {
                    let retry_after = resp
                        .header("retry-after")
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(0);
                    let base = if retry_after > 0 {
                        Duration::from_secs(retry_after)
                    } else {
                        Duration::from_millis(250 * (1u64 << attempt))
                    };
                    std::thread::sleep(base);
                    continue;
                }
                if code == 404 && not_found_is_empty {
                    // Nothing there. Not an error worth showing: a game with no
                    // artwork of the requested kind is an ordinary result, and
                    // the caller can still fall back to another kind.
                    return Ok(NOT_FOUND_BODY.to_vec());
                }
                return Err(format!("SteamGridDB returned HTTP {code} for {url}"));
            }
            Err(ureq::Error::Transport(e)) => {
                if attempt + 1 == DEFAULT_RETRIES {
                    return Err(format!("SteamGridDB request failed: {e}"));
                }
                std::thread::sleep(Duration::from_millis(250 * (1u64 << attempt)));
            }
        }
    }
    Err("SteamGridDB request failed after retries".to_string())
}

pub fn search_games(query: &str) -> Result<Vec<SteamGridGame>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    ensure_cache_dirs()?;
    cleanup_stale_cache();

    let url = format!(
        "{API_BASE}/search/autocomplete/{}",
        urlencoding::encode(trimmed)
    );
    let body = request_with_retry(&url)?;
    let envelope: GamesEnvelope =
        serde_json::from_slice(&body).map_err(|e| format!("invalid SteamGridDB response: {e}"))?;
    if !envelope.success {
        return Err("SteamGridDB search failed.".to_string());
    }
    Ok(envelope
        .data
        .into_iter()
        .map(|g| SteamGridGame {
            id: g.id,
            name: g.name,
        })
        .collect())
}

pub fn get_artwork(game_id: u32, art_type: ArtworkType) -> Result<Vec<Artwork>, String> {
    ensure_cache_dirs()?;
    cleanup_stale_cache();

    let primary_url = endpoint_for(game_id, art_type);
    let mut body = request_with_retry(&primary_url)?;
    let mut parsed: ArtworkEnvelope = serde_json::from_slice(&body)
        .map_err(|e| format!("invalid SteamGridDB artwork response: {e}"))?;

    // Not every game has portrait box art. Widening to the unrestricted grid
    // query picks up the landscape banners rather than showing nothing.
    if parsed.data.is_empty() && art_type == ArtworkType::Cover {
        let fallback = endpoint_for(game_id, ArtworkType::Grid);
        body = request_with_retry(&fallback)?;
        parsed = serde_json::from_slice(&body)
            .map_err(|e| format!("invalid SteamGridDB fallback response: {e}"))?;
    }
    if !parsed.success {
        return Err("SteamGridDB artwork query failed.".to_string());
    }

    Ok(parsed
        .data
        .into_iter()
        .filter(|a| !a.url.trim().is_empty())
        .map(|a| Artwork {
            id: a.id,
            url: a.url,
            thumb: a.thumb.unwrap_or_default(),
            width: a.width.unwrap_or(0),
            height: a.height.unwrap_or(0),
        })
        .collect())
}

pub fn download_artwork(url: &str, cache_key: &str) -> Result<PathBuf, String> {
    let trimmed_url = url.trim();
    if trimmed_url.is_empty() {
        return Err("Artwork URL is empty.".to_string());
    }
    let key = sanitize_key(cache_key);
    ensure_cache_dirs()?;
    cleanup_stale_cache();

    let destination = cache_images_dir().join(cache_filename(&key, trimmed_url));
    if destination.is_file() && is_fresh(&destination) {
        return Ok(destination);
    }

    let bytes = download_bytes(trimmed_url)?;
    if bytes.is_empty() {
        return Err("SteamGridDB returned an empty image.".to_string());
    }

    let temp = destination.with_extension("tmp");
    let mut file = std::fs::File::create(&temp)
        .map_err(|e| format!("could not write SGDB cache file {}: {e}", temp.display()))?;
    file.write_all(&bytes)
        .map_err(|e| format!("could not write SGDB image: {e}"))?;
    std::fs::rename(&temp, &destination)
        .map_err(|e| format!("could not finalize SGDB cache image: {e}"))?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn every_artwork_request_goes_to_an_endpoint_that_exists() {
        // The v2 API serves grids, heroes, logos and icons. There is no
        // /covers, so asking for one 404s for every game — which is exactly
        // what it did, on every cover lookup, until this was pinned down.
        for art_type in [ArtworkType::Grid, ArtworkType::Logo, ArtworkType::Cover] {
            let url = endpoint_for(4997889, art_type);
            assert!(
                !url.contains("/covers/"),
                "{art_type:?} asks for a route that does not exist: {url}"
            );
            let route = url
                .trim_start_matches(API_BASE)
                .split('/')
                .nth(1)
                .unwrap_or_default();
            assert!(
                matches!(route, "grids" | "heroes" | "logos" | "icons"),
                "{art_type:?} asks for /{route}/, which is not a v2 route: {url}"
            );
        }

        // A cover is a grid in a portrait size, so it must say which sizes.
        let cover = endpoint_for(4997889, ArtworkType::Cover);
        assert!(cover.contains("dimensions=600x900"), "{cover}");
    }

    #[test]
    fn nothing_there_is_an_empty_list_rather_than_a_failure() {
        // A 404 is turned into an empty success so a game with no artwork of
        // one kind can still fall back to another.
        let parsed: ArtworkEnvelope = serde_json::from_slice(NOT_FOUND_BODY).unwrap();
        assert!(parsed.success);
        assert!(parsed.data.is_empty());

        let parsed: GamesEnvelope = serde_json::from_slice(NOT_FOUND_BODY).unwrap();
        assert!(parsed.success);
        assert!(parsed.data.is_empty());
    }

    #[test]
    fn no_request_is_made_until_the_user_opts_in() {
        use crate::settings::Settings;

        // The default install is offline, and says why rather than failing
        // somewhere in the middle of a request.
        let err = api_key_from(&Settings::default()).unwrap_err();
        assert!(err.contains("switched off"), "{err}");

        // On but unkeyed is its own message: their v2 API refuses anything
        // unauthenticated, so "on" alone would just produce a 401.
        let err = api_key_from(&Settings {
            steamgriddb_enabled: true,
            steamgriddb_api_key: String::new(),
            ..Settings::default()
        })
        .unwrap_err();
        assert!(err.contains("API key"), "{err}");

        assert_eq!(
            api_key_from(&Settings {
                steamgriddb_enabled: true,
                steamgriddb_api_key: "abc123".into(),
                ..Settings::default()
            }),
            Ok("abc123".to_string())
        );
    }

    #[test]
    fn sanitizes_cache_keys() {
        assert_eq!(sanitize_key("God of War: Ragnarök"), "god-of-war-ragnar-k");
        assert_eq!(sanitize_key("   "), "artwork");
        assert_eq!(sanitize_key("A_B_C"), "a-b-c");
    }

    #[test]
    fn infers_extension_from_url() {
        assert_eq!(ext_from_url("https://x/y.png"), "png");
        assert_eq!(ext_from_url("https://x/y.webp?token=1"), "webp");
        assert_eq!(ext_from_url("https://x/y.jpg"), "jpg");
    }

    #[test]
    fn stale_files_are_not_reused() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let scratch = crate::testutil::Scratch::new("sgdb-fresh");
        std::env::set_var("SGDB_CACHE_DIR", scratch.path());
        std::env::set_var("SGDB_CACHE_MAX_AGE_DAYS", "1");
        ensure_cache_dirs().expect("cache dirs");
        let old = cache_images_dir().join("old.jpg");
        std::fs::write(&old, b"x").expect("write old file");
        assert!(is_fresh(&old));
        cleanup_stale_cache();
        assert!(old.exists());
        std::env::remove_var("SGDB_CACHE_DIR");
        std::env::remove_var("SGDB_CACHE_MAX_AGE_DAYS");
    }

    #[test]
    fn remembers_and_reads_last_used_artwork() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let scratch = crate::testutil::Scratch::new("sgdb-last");
        std::env::set_var("SGDB_CACHE_DIR", scratch.path());
        ensure_cache_dirs().expect("cache dirs");
        let art = scratch.write("picked.jpg", b"img");
        remember_last_used("steam:367520", &art).expect("remember");
        let loaded = last_used_artwork("steam:367520").expect("load");
        assert_eq!(loaded, art);
        std::env::remove_var("SGDB_CACHE_DIR");
    }
}
