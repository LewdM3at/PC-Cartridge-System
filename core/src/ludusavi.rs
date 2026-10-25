//! Where a game keeps its saves, from Ludusavi's manifest.
//!
//! Nothing on a PC says where a game writes its saves; PCGamingWiki does, and
//! [Ludusavi](https://github.com/mtkennerly/ludusavi-manifest) turns that into
//! one open YAML file covering tens of thousands of games. Off by default, like
//! every other lookup: switched on, the file is downloaded once, kept beside
//! `settings.json`, and refreshed when it is a month old.
//!
//! The file is 17 MB of very regular YAML, so it is read with a line scanner
//! rather than a YAML library — two-space indents, one game per top-level key:
//!
//! ```text
//! Stardew Valley:
//!   files:
//!     "<winAppData>/StardewValley/Saves":
//!       tags:
//!         - save
//!       when:
//!         - os: windows
//!   steam:
//!     id: 413150
//! ```
//!
//! What comes out is a `save.windows=` and a `save.linux=` line, each written
//! in the `{token}` form [`crate::saves`] resolves, so a cartridge made on one
//! machine syncs on the other kind too.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const URL: &str =
    "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";

/// How old the cached manifest may get before it is fetched again.
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// A game's save directory on each platform, as `save=` templates.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveLocations {
    pub windows: Option<String>,
    pub linux: Option<String>,
}

impl SaveLocations {
    pub fn is_empty(&self) -> bool {
        self.windows.is_none() && self.linux.is_none()
    }

    /// The template for the machine this is running on.
    pub fn here(&self) -> Option<&str> {
        if cfg!(windows) {
            self.windows.as_deref()
        } else {
            self.linux.as_deref()
        }
    }

    /// The conf lines, labelled so the two platform lines are one slot.
    pub fn conf_lines(&self, label: &str) -> Vec<String> {
        let label = label.replace(['|', '\n', '\r', '='], " ");
        [("windows", &self.windows), ("linux", &self.linux)]
            .into_iter()
            .filter_map(|(os, template)| {
                template
                    .as_ref()
                    .map(|template| format!("save.{os}={}|{template}", label.trim()))
            })
            .collect()
    }
}

fn cache_path() -> PathBuf {
    crate::settings::settings_dir()
        .join("ludusavi")
        .join("manifest.yaml")
}

/// Look a game up, downloading the manifest first if it is missing or stale.
///
/// `steam_id` wins over the title when the manifest has it: names drift
/// ("Hollow Knight" and "Hollow Knight: Voidheart Edition"), app ids do not.
pub fn lookup(title: &str, steam_id: Option<&str>) -> Result<Option<SaveLocations>, String> {
    let text = manifest()?;
    Ok(lookup_in(&text, title, steam_id).or_else(|| scan_here(title)))
}

/// A folder named after the game where games put saves, on this machine.
///
/// For a game the manifest does not list: most write to a folder with their
/// own name, directly under one of these or under their studio's. Only finds a
/// game that has saved here already, and only the platform it runs on.
pub fn scan_here(title: &str) -> Option<SaveLocations> {
    let wanted = normalise(title);
    // "Ys" or "Rez" would match any folder with those letters in it.
    if wanted.len() < 4 {
        return None;
    }
    let roots: &[(&str, &str)] = &[
        ("documents", "My Games"),
        ("documents", ""),
        ("savedgames", ""),
        ("appdata", ""),
        ("localappdata", ""),
        ("home", "AppData/LocalLow"),
    ];
    let named = |dir: &std::path::Path| {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>()
    };
    let is_it = |path: &std::path::Path| {
        path.file_name()
            .is_some_and(|name| normalise(&name.to_string_lossy()) == wanted)
    };
    for (token, under) in roots {
        let Some(base) = crate::saves::token_path(token).map(|p| p.join(under)) else {
            continue;
        };
        let top = named(&base);
        // The game's own folder, else one inside its studio's.
        let found = top.iter().find(|path| is_it(path)).cloned().or_else(|| {
            top.iter()
                .flat_map(|studio| named(studio))
                .find(|p| is_it(p))
        });
        if let Some(template) = found.and_then(|path| crate::saves::template_from_path(&path)) {
            let mut places = SaveLocations::default();
            if cfg!(windows) {
                places.windows = Some(template);
            } else {
                places.linux = Some(template);
            }
            return Some(places);
        }
    }
    None
}

/// The manifest's text, fetched if it has to be.
fn manifest() -> Result<String, String> {
    let path = cache_path();
    let fresh = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < MAX_AGE);
    if !fresh {
        if let Err(e) = download(&path) {
            // A stale copy is still a good answer; only no copy at all fails.
            if !path.is_file() {
                return Err(format!("Could not download Ludusavi's save list: {e}"));
            }
        }
    }
    std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn download(path: &PathBuf) -> Result<(), String> {
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(120))
        .user_agent("pc-gamepak")
        .build()
        .get(URL)
        .call()
        .map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Written beside and renamed over, so an interrupted download never leaves
    // half a manifest that parses as fewer games.
    let partial = path.with_extension("yaml.part");
    let mut file = std::fs::File::create(&partial).map_err(|e| e.to_string())?;
    std::io::copy(&mut response.into_reader(), &mut file).map_err(|e| e.to_string())?;
    drop(file);
    #[cfg(windows)]
    let _ = std::fs::remove_file(path);
    std::fs::rename(&partial, path).map_err(|e| e.to_string())
}

/// Find a game in manifest text and work out its save directories.
///
/// Tried in order of how sure each is: the Steam app id (its own or one of
/// its `steamExtra` ids), the name, then the name of the folder it installs
/// into — which is what a game scanned from a folder is called. A name that
/// is an `alias:` of another entry is looked up as that entry.
pub fn lookup_in(text: &str, title: &str, steam_id: Option<&str>) -> Option<SaveLocations> {
    let steam_id = steam_id.filter(|id| !id.is_empty());
    let wanted = normalise(title);
    let mut by_name = None;
    let mut by_folder = None;
    for (name, block) in games(text) {
        if steam_id.is_some_and(|id| steam_ids_of(&block).iter().any(|own| own == id)) {
            return Some(locations(&block)).filter(|found| !found.is_empty());
        }
        if by_name.is_none() && normalise(&name) == wanted {
            by_name = Some(block);
        } else if by_folder.is_none()
            && install_dirs_of(&block)
                .iter()
                .any(|dir| normalise(dir) == wanted)
        {
            by_folder = Some(block);
        }
    }
    let block = by_name.or(by_folder)?;
    if let Some(target) = alias_of(&block) {
        // One step only: an alias of an alias would be a manifest bug, and
        // following it could loop.
        let target = normalise(&target);
        let block = games(text).find(|(name, _)| normalise(name) == target)?.1;
        return Some(locations(&block)).filter(|found| !found.is_empty());
    }
    Some(locations(&block)).filter(|found| !found.is_empty())
}

/// Lower case, letters and digits only, accents dropped: "STAR WARS™:
/// Knights" matches "Star Wars Knights", and "Ragnarök" matches "Ragnarok".
fn normalise(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
}

/// Every top-level key with the lines that belong to it.
fn games(text: &str) -> impl Iterator<Item = (String, Vec<&str>)> {
    let mut lines = text.lines().peekable();
    std::iter::from_fn(move || loop {
        let line = lines.next()?;
        if line.starts_with(' ') || line.starts_with('#') || line.starts_with("---") {
            continue;
        }
        let Some(key) = line.strip_suffix(':') else {
            continue;
        };
        let name = unquote(key);
        let mut block = Vec::new();
        while let Some(next) = lines.peek() {
            if !next.is_empty() && !next.starts_with(' ') {
                break;
            }
            block.push(lines.next().unwrap_or_default());
        }
        return Some((name, block));
    })
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .map(|v| v.replace("\\\"", "\""))
        .unwrap_or_else(|| value.to_string())
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The lines under one top-level field of a game (`  steam:`), trimmed.
fn field<'a>(block: &'a [&'a str], name: &str) -> impl Iterator<Item = &'a str> {
    let at = block
        .iter()
        .position(|line| line.trim_end() == format!("  {name}:"));
    at.map_or(&[][..], |at| &block[at + 1..])
        .iter()
        .take_while(|line| indent(line) > 2)
        .map(|line| line.trim())
}

/// `steam: id:` and every `id: steamExtra:` — a game sold as several
/// editions or bundles lists the other app ids there.
fn steam_ids_of(block: &[&str]) -> Vec<String> {
    let own = field(block, "steam").filter_map(|line| line.strip_prefix("id:"));
    let mut in_extra = false;
    let extra = field(block, "id").filter_map(move |line| {
        if line.ends_with(':') {
            in_extra = line == "steamExtra:";
            return None;
        }
        line.strip_prefix("- ").filter(|_| in_extra)
    });
    own.chain(extra).map(|id| id.trim().to_string()).collect()
}

/// The folder names the game installs into, from `installDir:`.
fn install_dirs_of(block: &[&str]) -> Vec<String> {
    block
        .iter()
        .skip_while(|line| line.trim_end() != "  installDir:")
        .skip(1)
        .take_while(|line| indent(line) > 2)
        .filter(|line| indent(line) == 4)
        .map(|line| {
            let key = line.trim();
            // `Name: {}` on one line, or `Name:` with fields under it.
            let key = key.strip_suffix(" {}").unwrap_or(key);
            unquote(key.strip_suffix(':').unwrap_or(key))
        })
        .collect()
}

/// `alias: Other Name`: this entry is only a pointer to that one.
fn alias_of(block: &[&str]) -> Option<String> {
    block
        .iter()
        .find_map(|line| line.strip_prefix("  alias:"))
        .map(unquote)
}

/// One entry under `files:`.
#[derive(Default)]
struct Entry {
    path: String,
    save: bool,
    /// Any tag at all. An untagged entry is backed up by Ludusavi as a save.
    tagged: bool,
    /// `(os, store)` per `when` item; empty means everywhere.
    when: Vec<(Option<String>, Option<String>)>,
}

fn entries(block: &[&str]) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    // Files, then registry keys, each written as a path: a key becomes
    // `<registry>/HKEY_CURRENT_USER/...` and maps to the `{registry}` token.
    let mut section = "-";
    let mut list = "";
    for line in block {
        let depth = indent(line);
        let text = line.trim();
        if depth == 2 {
            section = match text {
                "files:" => "",
                "registry:" => "<registry>/",
                _ => "-",
            };
            continue;
        }
        if section == "-" || text.is_empty() {
            continue;
        }
        match depth {
            4 => out.push(Entry {
                path: format!(
                    "{section}{}",
                    unquote(text.strip_suffix(':').unwrap_or(text))
                ),
                ..Default::default()
            }),
            6 => list = text.trim_end_matches(':'),
            _ => {
                let Some(entry) = out.last_mut() else {
                    continue;
                };
                let item = text.strip_prefix("- ");
                match list {
                    "tags" => {
                        entry.tagged = true;
                        entry.save |= item == Some("save");
                    }
                    "when" => {
                        let (field, starts) = match item {
                            Some(rest) => (rest, true),
                            None => (text, false),
                        };
                        if starts {
                            entry.when.push((None, None));
                        }
                        let Some(last) = entry.when.last_mut() else {
                            continue;
                        };
                        if let Some(os) = field.strip_prefix("os:") {
                            last.0 = Some(os.trim().to_string());
                        } else if let Some(store) = field.strip_prefix("store:") {
                            last.1 = Some(store.trim().to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

fn locations(block: &[&str]) -> SaveLocations {
    let entries = entries(block);
    let first = |os: &str| {
        entries
            .iter()
            .filter(|entry| (entry.save || !entry.tagged) && applies(entry, os))
            .find_map(|entry| template_for(&entry.path, os))
    };
    SaveLocations {
        windows: first("windows"),
        linux: first("linux"),
    }
}

/// Whether an entry is for this platform, installed the ordinary way. A
/// Microsoft Store copy keeps its saves somewhere else entirely.
fn applies(entry: &Entry, os: &str) -> bool {
    entry.when.is_empty()
        || entry.when.iter().any(|(when_os, store)| {
            when_os.as_deref().is_none_or(|w| w == os) && store.as_deref() != Some("microsoft")
        })
}

/// Ludusavi's placeholders in [`crate::saves`] tokens, or `None` for a path
/// that needs something only Ludusavi knows (the install directory, a store
/// user id).
fn template_for(path: &str, os: &str) -> Option<String> {
    let map: &[(&str, &str)] = match os {
        "windows" => &[
            ("<root>/userdata/<storeUserId>", "{steamuserdata}"),
            ("<registry>/HKEY_CURRENT_USER", "{registry}/HKCU"),
            ("<winAppData>", "{appdata}"),
            ("<winLocalAppDataLow>", "{home}/AppData/LocalLow"),
            ("<winLocalAppData>", "{localappdata}"),
            ("<winDocuments>", "{documents}"),
            ("<home>", "{home}"),
        ],
        _ => &[
            ("<root>/userdata/<storeUserId>", "{steamuserdata}"),
            ("<xdgConfig>", "{appdata}"),
            ("<xdgData>", "{localappdata}"),
            ("<home>", "{home}"),
        ],
    };
    let (from, to) = map.iter().find(|(from, _)| path.starts_with(from))?;
    // The folder is what gets carried, so a path that goes on into a pattern
    // (`*.dat`) or a placeholder only Ludusavi can fill (`<storeUserId>`) is
    // cut at the last folder before it. At least one folder of the game's own
    // has to remain: carrying the whole of AppData is not carrying a save.
    let rest: Vec<&str> = path[from.len()..]
        .split('/')
        .filter(|part| !part.is_empty())
        .take_while(|part| !part.contains('*') && !part.contains('<'))
        .collect();
    (!rest.is_empty()).then(|| format!("{to}/{}", rest.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"---
"!Hidden Game":
  files:
    "<base>/save.dat":
      tags:
        - save
Hollow Knight:
  files:
    "<base>/hollow_knight_Data/Config.ini":
      tags:
        - config
    "<home>/AppData/LocalLow/Team Cherry/Hollow Knight/*.dat":
      tags:
        - save
      when:
        - os: windows
    "<winProgramData>/Packages/TeamCherry/SystemAppData/wgs":
      tags:
        - save
      when:
        - os: windows
          store: microsoft
    "<xdgConfig>/unity3d/Team Cherry/Hollow Knight/*.dat":
      tags:
        - save
      when:
        - os: linux
  steam:
    id: 367520
Stardew Valley:
  cloud:
    steam: true
  files:
    "<winAppData>/StardewValley/Saves":
      tags:
        - save
      when:
        - os: windows
    "<xdgConfig>/StardewValley/Saves":
      tags:
        - save
      when:
        - os: linux
  steam:
    id: 413150
"#;

    #[test]
    fn finds_by_steam_id_before_name() {
        let found = lookup_in(SAMPLE, "Something Else Entirely", Some("413150")).unwrap();
        assert_eq!(
            found.windows.as_deref(),
            Some("{appdata}/StardewValley/Saves")
        );
        assert_eq!(
            found.linux.as_deref(),
            Some("{appdata}/StardewValley/Saves")
        );
    }

    #[test]
    fn finds_by_name_and_carries_the_folder_of_a_pattern() {
        let found = lookup_in(SAMPLE, "HOLLOW KNIGHT", None).unwrap();
        assert_eq!(
            found.windows.as_deref(),
            Some("{home}/AppData/LocalLow/Team Cherry/Hollow Knight")
        );
        assert_eq!(
            found.linux.as_deref(),
            Some("{appdata}/unity3d/Team Cherry/Hollow Knight")
        );
    }

    #[test]
    fn a_game_only_its_install_folder_knows_is_not_a_match() {
        assert_eq!(lookup_in(SAMPLE, "!Hidden Game", None), None);
        assert_eq!(lookup_in(SAMPLE, "Not In The List", None), None);
    }

    const MORE: &str = r#"---
Castle Crashers:
  files:
    "<base>/data/config.xml":
      tags:
        - config
    "<root>/userdata/<storeUserId>/204360/remote":
      tags:
        - save
      when:
        - store: steam
  id:
    steamExtra:
      - 999001
  installDir:
    CastleCrashers: {}
  steam:
    id: 204360
"God of War Ragnarök":
  files:
    "<winDocuments>/God of War Ragnarök":
      tags:
        - save
      when:
        - os: windows
Klaus Old Name:
  alias: Castle Crashers
"Bluey: The Videogame":
  installDir:
    Biscuits: {}
  registry:
    HKEY_CURRENT_USER/Software/Outright Games Ltd/Bluey The Videogame:
      tags:
        - save
  steam:
    id: 2078350
"#;

    #[test]
    fn finds_steam_cloud_saves_extra_ids_folders_aliases_and_accents() {
        let cloud = Some("{steamuserdata}/204360/remote".to_string());
        assert_eq!(lookup_in(MORE, "x", Some("204360")).unwrap().windows, cloud);
        assert_eq!(lookup_in(MORE, "x", Some("999001")).unwrap().windows, cloud);
        assert_eq!(
            lookup_in(MORE, "CastleCrashers", None).unwrap().windows,
            cloud
        );
        assert_eq!(
            lookup_in(MORE, "klaus old name", None).unwrap().windows,
            cloud
        );
        assert_eq!(
            lookup_in(MORE, "God of War - Ragnarok", None)
                .unwrap()
                .windows
                .as_deref(),
            Some("{documents}/God of War Ragnarök")
        );
        // A save kept in the registry, on Windows only.
        let bluey = lookup_in(MORE, "x", Some("2078350")).unwrap();
        assert_eq!(
            bluey.windows.as_deref(),
            Some("{registry}/HKCU/Software/Outright Games Ltd/Bluey The Videogame")
        );
        assert_eq!(bluey.linux, None);
    }

    #[test]
    fn writes_one_slot_for_both_platforms() {
        let found = lookup_in(SAMPLE, "Stardew Valley", None).unwrap();
        assert_eq!(
            found.conf_lines("Stardew | Valley"),
            [
                "save.windows=Stardew   Valley|{appdata}/StardewValley/Saves",
                "save.linux=Stardew   Valley|{appdata}/StardewValley/Saves",
            ]
        );
        // And the saves module reads them back as one slot on this machine.
        let conf = format!("title=S\n{}\n", found.conf_lines("Stardew").join("\n"));
        let slots = crate::saves::parse_declarations(&conf);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].template, "{appdata}/StardewValley/Saves");
    }
}
