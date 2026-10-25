//! Saves that travel with the cartridge.
//!
//! A cartridge already carries the game. What it does not carry, and what
//! makes playing the same game on a second machine feel like starting over, is
//! the save. Steam Cloud solves this for the games that are in it; nothing
//! solves it for a GOG game, an emulator, or anything copied onto a drive by
//! hand. This module does, for any game whose cartridge says where its saves
//! live.
//!
//! # What a cartridge declares
//!
//! `save=` lines in `cartridge.conf`, beside the keys that were already there:
//!
//! ```text
//! executable=steam://rungameid/413150
//! title=Stardew Valley
//! save=Stardew Valley|{appdata}/StardewValley/Saves
//! ```
//!
//! The `{appdata}` part is the point. A save path is the one thing in a
//! cartridge that cannot be written down literally, because the three
//! platforms disagree about where it goes and two of them disagree with
//! themselves depending on how the user's account is set up. A cartridge names
//! a *role* — see [`TOKENS`] — and the host resolves it.
//!
//! # How the two copies are reconciled
//!
//! Not by trusting timestamps against each other. Both sides are compared
//! against the last sync this cartridge recorded, which turns "which is newer"
//! into three answerable questions: has the host's copy changed since then,
//! has the cartridge's, and have both. Only the last one is a conflict, and a
//! conflict is *refused* rather than resolved. Silently picking a winner is
//! how a save-sync tool eats somebody's eighty-hour run.
//!
//! Every write is preceded by a backup of whatever is about to be replaced,
//! and backups are kept. Disk is cheap; the save is not.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where slot payloads and the sync index live, under
/// [`crate::create::ASSET_DIR`].
pub const SAVES_DIR: &str = "saves";

/// The index file inside [`SAVES_DIR`].
pub const INDEX_FILE: &str = "index.json";

/// Suffix for a directory moved aside before being replaced.
pub const BACKUP_SUFFIX: &str = ".gamepak-backup";

/// How many backups of one slot to keep on each side before pruning.
pub const BACKUPS_KEPT: usize = 3;

/// Slack allowed when comparing a modification time against the last sync.
///
/// exFAT stores modification times to two seconds, and the host clock that
/// wrote the index is not necessarily the clock that stamped the files. Three
/// seconds is enough to stop that arithmetic from reporting a change nobody
/// made, and far too short to hide a real edit.
pub const MTIME_SLACK_SECONDS: u64 = 3;

// --------------------------------------------------------------------------
// Declarations
// --------------------------------------------------------------------------

/// How a slot keeps the two copies together.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    /// Copy in whichever direction changed. Works on every filesystem and
    /// every platform, and leaves the host with a working save after the
    /// cartridge is gone. The default, for both reasons.
    #[default]
    Copy,
    /// Replace the host's directory with a symlink onto the cartridge, so the
    /// game writes straight to the drive and there is only ever one copy.
    ///
    /// Faster and exact, and it has a cost that is easy to underestimate: the
    /// link dangles the moment the cartridge leaves. (On Windows without
    /// Developer Mode it is a directory junction rather than a symlink, which
    /// needs no privilege and behaves the same.) A
    /// clean eject turns it back into a real directory; a drive pulled out of
    /// the port does not, and the game will find its save directory missing.
    Link,
}

/// One declared save location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSlot {
    /// Directory name on the cartridge, derived from the label.
    pub id: String,
    /// What to call this in the interface.
    pub label: String,
    /// The declared path, tokens and all, before any host touches it.
    pub template: String,
    /// The game this belongs to, keyed as [`crate::stats::key_for`] keys it.
    /// `None` for a cartridge-wide slot.
    pub game: Option<String>,
    pub mode: SyncMode,
}

/// The tokens a `save=` line may use, and what each one means per platform.
///
/// Only roles that exist on all three platforms are here. A path that genuinely
/// only exists on one — a Proton prefix, a Windows registry-derived directory —
/// is written as a platform-specific line (`save.linux=`) instead of being
/// forced into a token that lies on the other two.
pub const TOKENS: &[&str] = &[
    "home",
    "documents",
    "savedgames",
    "appdata",
    "localappdata",
    "appsupport",
    "prefs",
];

/// Tokens that only resolve where a store is installed, so they are not in
/// [`TOKENS`], which every host resolves.
///
/// `{steamuserdata}` is Steam's `userdata/<account id>`: where every Steam
/// Cloud game keeps its saves, as `{steamuserdata}/<app id>/remote`.
pub const STORE_TOKENS: &[&str] = &["steamuserdata"];

/// The signed-in Steam account's `userdata` folder: the most recently written
/// one, when more than one account has used this PC.
fn steam_userdata() -> Option<PathBuf> {
    // ponytail: newest account wins; a per-cartridge account id if people share PCs.
    std::fs::read_dir(crate::steam::steam_root()?.join("userdata"))
        .ok()?
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name != "0" && name.chars().all(|c| c.is_ascii_digit())
        })
        .max_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok())
        .map(|entry| entry.path())
}

/// Resolve one token to a directory on this host.
///
/// The macOS column is the reason this exists as a table rather than a pair of
/// `#[cfg]` branches: `~/Library/Application Support` stands in for both of
/// Windows' roaming and local application directories, and `~/Library/
/// Preferences` — which has no Windows or Linux equivalent at all — is where a
/// native Mac game's settings actually are.
pub fn token_path(token: &str) -> Option<PathBuf> {
    let home = home_dir()?;
    let path = match token {
        "home" => home,
        "documents" => {
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                xdg_user_dir("XDG_DOCUMENTS_DIR").unwrap_or_else(|| home.join("Documents"))
            }
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            {
                home.join("Documents")
            }
        }
        "savedgames" => home.join("Saved Games"),
        "appdata" => {
            #[cfg(target_os = "windows")]
            {
                env_path("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming"))
            }
            #[cfg(target_os = "macos")]
            {
                home.join("Library").join("Application Support")
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"))
            }
        }
        "localappdata" => {
            #[cfg(target_os = "windows")]
            {
                env_path("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local"))
            }
            #[cfg(target_os = "macos")]
            {
                home.join("Library").join("Application Support")
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"))
            }
        }
        "appsupport" => {
            #[cfg(target_os = "windows")]
            {
                env_path("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming"))
            }
            #[cfg(target_os = "macos")]
            {
                home.join("Library").join("Application Support")
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"))
            }
        }
        "prefs" => {
            #[cfg(target_os = "windows")]
            {
                env_path("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming"))
            }
            #[cfg(target_os = "macos")]
            {
                home.join("Library").join("Preferences")
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"))
            }
        }
        "steamuserdata" => return steam_userdata(),
        // A registry key, carried as an exported file: see [`crate::registry`].
        "registry" if cfg!(windows) => crate::registry::staging_root(),
        _ => return None,
    };
    Some(path)
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn home_dir() -> Option<PathBuf> {
    env_path("PC_GAMEPAK_HOME") // tests, and nothing else
        .or_else(|| env_path("HOME"))
        .or_else(|| env_path("USERPROFILE"))
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            let mut joined = PathBuf::from(drive);
            joined.push(PathBuf::from(path));
            Some(joined)
        })
}

/// `~/.config/user-dirs.dirs`, which is where a localised desktop records that
/// Documents is actually called `Dokument`.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn xdg_user_dir(key: &str) -> Option<PathBuf> {
    if let Some(direct) = env_path(key) {
        return Some(direct);
    }
    let home = home_dir()?;
    let text = std::fs::read_to_string(home.join(".config").join("user-dirs.dirs")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        let expanded = match value.strip_prefix("$HOME/") {
            Some(rel) => home.join(rel),
            None => PathBuf::from(value),
        };
        if !expanded.as_os_str().is_empty() {
            return Some(expanded);
        }
    }
    None
}

/// Why a declared path cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Unusable {
    /// The line names a token this host does not know.
    UnknownToken(String),
    /// The line names no token at all. A save path has to be relative to
    /// something the host resolves, or it is a path to one particular machine.
    NoToken,
    /// The path escapes its own root via `..`.
    Escapes,
    /// The path resolves to a whole home, library or documents directory
    /// rather than to one game's saves inside it.
    TooBroad,
}

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unusable::UnknownToken(t) => write!(f, "unknown token {{{t}}}"),
            Unusable::NoToken => write!(f, "no {{token}} — a literal path is host-specific"),
            Unusable::Escapes => write!(f, "the path escapes its own root"),
            Unusable::TooBroad => write!(f, "names a whole user directory, not one game's saves"),
        }
    }
}

/// Turn a declared template into a path on this host.
///
/// Refuses more than it accepts, on purpose. A cartridge is a file somebody
/// else may have written, and this is the one part of the format that names a
/// directory *off* the cartridge. `{home}` on its own, or `{appdata}` on its
/// own, would make "sync my saves" mean "copy my home directory onto this
/// drive", so a template has to reach at least one level inside the root it
/// names.
pub fn resolve_template(template: &str) -> Result<PathBuf, Unusable> {
    let template = template.trim().replace('\\', "/");
    let Some(rest) = template.strip_prefix('{') else {
        return Err(Unusable::NoToken);
    };
    let Some(close) = rest.find('}') else {
        return Err(Unusable::NoToken);
    };
    let token = rest[..close].trim().to_lowercase();
    let tail = rest[close + 1..].trim_start_matches('/');

    let root = token_path(&token).ok_or_else(|| Unusable::UnknownToken(token.clone()))?;

    if token == "registry" {
        let parts: Vec<&str> = tail.split('/').filter(|p| !p.is_empty()).collect();
        if parts.contains(&"..") {
            return Err(Unusable::Escapes);
        }
        if !crate::registry::allowed(&parts) {
            return Err(Unusable::TooBroad);
        }
    }

    let mut depth = 0usize;
    let mut path = root;
    for part in tail.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(Unusable::Escapes),
            other => {
                depth += 1;
                path.push(other);
            }
        }
    }

    // One component inside `{home}` is still most of a home directory —
    // `{home}/Documents` and `{home}/Library` both are. Tokens that already
    // name a user directory need one component; `{home}` needs two.
    let needed = if token == "home" { 2 } else { 1 };
    if depth < needed {
        return Err(Unusable::TooBroad);
    }
    Ok(path)
}

/// Read every `save=` line on the cartridge, in declaration order.
///
/// Duplicate keys are the reason this is a line scanner rather than
/// [`crate::cartridge::parse_ini`]: a game with two save locations writes two
/// `save=` lines, and a map keeps one of them.
pub fn declared(root: &Path) -> Vec<SaveSlot> {
    // A memory card declares its saves the same way, in its own file, and a
    // combo drive has both. Read as one text, so slot names stay unique across
    // the two; the card's part starts in a section of its own, so nothing it
    // declares is taken for the cartridge's last game.
    let cartridge = std::fs::read_to_string(root.join("cartridge.conf")).unwrap_or_default();
    let card = std::fs::read_to_string(root.join(crate::memcard::CONF)).unwrap_or_default();
    if card.is_empty() {
        return parse_declarations(&cartridge);
    }
    let mut slots = parse_declarations(&format!("{cartridge}\n[memorycard]\n{card}"));
    // One folder declared in both files is one save, not two blocks that
    // would sync over each other. The cartridge's own line comes first; keep it.
    let mut seen = std::collections::HashSet::new();
    slots.retain(|slot| seen.insert(slot.template.to_lowercase()));
    slots
}

/// A save folder somebody pointed at, written the portable way.
///
/// The reverse of [`resolve_template`]: whichever token's directory holds the
/// folder most closely wins, so `C:\Users\h\AppData\Roaming\Foo\Saves` is
/// `{appdata}/Foo/Saves` rather than `{home}/AppData/Roaming/Foo/Saves`, which
/// would only ever work on Windows. `None` for a folder outside every token —
/// one inside a game's install folder, say — because there is no way to write
/// that down that another machine would understand.
pub fn template_from_path(path: &Path) -> Option<String> {
    // Reversed because `max_by_key` keeps the last of equals, and on Windows
    // three tokens share AppData\Roaming: `appdata`, listed first, should win.
    let (token, base) = TOKENS
        .iter()
        .chain(STORE_TOKENS)
        .rev()
        .filter_map(|token| Some((*token, token_path(token)?)))
        .filter(|(_, base)| path.starts_with(base))
        .max_by_key(|(_, base)| base.components().count())?;
    let rest: Vec<String> = path
        .strip_prefix(&base)
        .ok()?
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    // The token's own directory is not a game's save folder.
    (!rest.is_empty()).then(|| format!("{{{token}}}/{}", rest.join("/")))
}

/// The platform suffix this host answers to on a `save.<os>=` line.
pub fn host_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Parse the `save=` lines out of a `cartridge.conf`.
///
/// Sections matter: a line in `[game]` belongs to that game, and a line
/// anywhere else belongs to the cartridge. A `save.<os>=` line is taken only
/// on that platform, and where both a plain and a platform line exist for the
/// same game, the platform one replaces it — which is how a cartridge says
/// "everywhere it is here, but under Proton it is in the prefix".
pub fn parse_declarations(content: &str) -> Vec<SaveSlot> {
    let suffix = host_suffix();
    let mut section = String::from("general");
    let mut game_index: usize = 0;
    let mut current_exe = String::new();
    let mut mode = SyncMode::Copy;

    // (game key, label, template, specific?) in declaration order.
    let mut raw: Vec<(Option<String>, String, String, bool)> = Vec::new();
    // Executables are only known once the section has been read through, so
    // slots are stamped with the section they came from and filled in after.
    let mut section_of: Vec<usize> = Vec::new();
    let mut exe_of_section: BTreeMap<usize, String> = BTreeMap::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                let name = line[1..end].trim().to_lowercase();
                if name == "game" {
                    game_index += 1;
                }
                section = name;
                current_exe.clear();
            }
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim().to_lowercase();
        let value = line[eq + 1..].trim().to_string();

        if key == "executable" || key == "open" {
            current_exe = value.clone();
            let index = if section == "game" { game_index } else { 0 };
            exe_of_section.insert(index, crate::stats::key_for(&current_exe));
            continue;
        }
        if key == "savemode" {
            mode = match value.to_lowercase().as_str() {
                "link" | "symlink" => SyncMode::Link,
                _ => SyncMode::Copy,
            };
            continue;
        }

        let specific = match key.as_str() {
            "save" => false,
            other => match other.strip_prefix("save.") {
                Some(os) if os == suffix => true,
                // A `save.windows=` line read on Linux is not an error: it is
                // a line for another machine, and the cartridge is portable
                // precisely because it carries all three.
                Some(_) => continue,
                None => continue,
            },
        };

        let (label, template) = split_label(&value);
        raw.push((None, label, template, specific));
        section_of.push(if section == "game" { game_index } else { 0 });
    }

    // Attach each slot to its section's executable, now that they are known.
    let mut slots: Vec<SaveSlot> = Vec::new();
    let mut specific_labels: Vec<String> = Vec::new();
    for (index, (_, label, template, specific)) in raw.iter().enumerate() {
        let section_index = section_of[index];
        let game = exe_of_section.get(&section_index).cloned();
        if *specific {
            specific_labels.push(slot_key(&game, label));
        }
        slots.push(SaveSlot {
            id: String::new(),
            label: label.clone(),
            template: template.clone(),
            game,
            mode,
        });
    }

    // A platform line overrides the plain line it shares a label with.
    let mut kept: Vec<SaveSlot> = Vec::new();
    for (index, slot) in slots.into_iter().enumerate() {
        let overridden =
            !raw[index].3 && specific_labels.contains(&slot_key(&slot.game, &slot.label));
        if !overridden {
            kept.push(slot);
        }
    }

    assign_ids(&mut kept);
    kept
}

fn slot_key(game: &Option<String>, label: &str) -> String {
    format!(
        "{}\u{1}{}",
        game.clone().unwrap_or_default(),
        label.to_lowercase()
    )
}

/// `Label|path` or just `path`, in which case the last component names it.
fn split_label(value: &str) -> (String, String) {
    match value.split_once('|') {
        Some((label, template)) if !label.trim().is_empty() => {
            (label.trim().to_string(), template.trim().to_string())
        }
        Some((_, template)) => (derived_label(template.trim()), template.trim().to_string()),
        None => (derived_label(value.trim()), value.trim().to_string()),
    }
}

fn derived_label(template: &str) -> String {
    template
        .replace('\\', "/")
        .rsplit('/')
        .find(|part| !part.trim().is_empty())
        .unwrap_or("Saves")
        .trim()
        .to_string()
}

/// Give every slot a directory name: a slug of its label, made unique.
///
/// The cartridge may be exFAT, so this is conservative about what a directory
/// may be called — ASCII, digits and dashes — rather than assuming the label
/// itself is a legal filename.
fn assign_ids(slots: &mut [SaveSlot]) {
    let mut used: Vec<String> = Vec::new();
    for slot in slots.iter_mut() {
        let base = slugify(&slot.label);
        let mut id = base.clone();
        let mut n = 2;
        while used.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        used.push(id.clone());
        slot.id = id;
    }
}

fn slugify(label: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "saves".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

/// Carry a cartridge's save declarations across a rewrite of its `cartridge.conf`.
///
/// The wizard's editor renders the conf from scratch — that is how it drops
/// whatever was left over from an earlier write — which would also drop every
/// `save=` line somebody typed in by hand, and with it the cartridge's link to
/// the saves already sitting in `.gamepak/saves/`. Renaming a cartridge would
/// quietly orphan them.
///
/// So the declarations are lifted out of the old text and put back into the
/// new one, in the section they belong to. A `[game]` section is matched by its
/// `executable`, not by its position: the editor exists partly to reorder
/// games, and a line that followed the wrong one would point a game at another
/// game's saves.
///
/// Lines are re-emitted verbatim. Anything for a game that is no longer on the
/// cartridge is dropped, which is the right answer — the game is gone.
pub fn preserve(old: &str, new: &str) -> String {
    let mut carried: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (owner, line) in declaration_lines(old) {
        carried.entry(owner).or_default().push(line);
    }
    if carried.is_empty() {
        return new.to_string();
    }

    let mut out = String::new();
    for block in blocks(new) {
        let owner = owner_of(&block);
        let carried_here = carried.get(&owner).cloned().unwrap_or_default();
        if carried_here.is_empty() {
            out.push_str(&block);
            continue;
        }
        // After the section's own keys, before the blank line that ends it, so
        // the result reads the way a hand-written conf does — and so the next
        // `[game]` is still separated from this one.
        let trimmed = block.trim_end_matches('\n');
        let stripped = block.len() - trimmed.len();
        out.push_str(trimmed);
        out.push('\n');
        for line in carried_here {
            out.push_str(&line);
            out.push('\n');
        }
        // One of the stripped newlines has just been put back by the line
        // above; the rest were the blank line between sections. A final block
        // with no newline at all gains one, which a conf should have anyway.
        for _ in 1..stripped {
            out.push('\n');
        }
    }
    out
}

/// The save lines in a conf, each with the owner it belongs to.
///
/// The owner is a `[game]` section's executable key, or the empty string for
/// anything outside one — which is where `savemode` always lives and where a
/// single-game cartridge's `save=` lines live too.
fn declaration_lines(conf: &str) -> Vec<(String, String)> {
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut owner_of_section: BTreeMap<usize, String> = BTreeMap::new();
    let mut section = 0usize;
    let mut in_game = false;

    for raw in conf.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                let name = line[1..end].trim().to_lowercase();
                in_game = name == "game";
                if in_game {
                    section += 1;
                }
            }
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim().to_lowercase();
        let index = if in_game { section } else { 0 };
        if key == "executable" || key == "open" {
            owner_of_section.insert(index, crate::stats::key_for(line[eq + 1..].trim()));
            continue;
        }
        // HowLongToBeat's figures ride along too: they were looked up once,
        // when the cartridge was made, and a rename is no reason to lose them.
        // (The play history needs no carrying — `playlog` rewrites it from
        // the stats file after every write.)
        if key == "savemode"
            || key == "save"
            || key.starts_with("save.")
            || key.starts_with("hltb_")
            // Nothing in the editor sets the platform yet, so a rewrite would drop it.
            || key == "platform"
        {
            // A cartridge-wide key even when it is written inside a section.
            let index = if key == "savemode" { 0 } else { index };
            found.push((index, raw.trim_end().to_string()));
        }
    }

    found
        .into_iter()
        .map(|(index, line)| {
            let owner = if index == 0 {
                String::new()
            } else {
                owner_of_section.get(&index).cloned().unwrap_or_default()
            };
            (owner, line)
        })
        .collect()
}

/// Split a conf into its sections, each keeping its own trailing blank lines.
fn blocks(conf: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in conf.split_inclusive('\n') {
        if line.trim_start().starts_with('[') && !current.trim().is_empty() {
            out.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Which owner a rendered section belongs to, by the same rule.
fn owner_of(block: &str) -> String {
    let is_game = block
        .lines()
        .find(|line| line.trim_start().starts_with('['))
        .map(|line| line.trim().to_lowercase().starts_with("[game"))
        .unwrap_or(false);
    if !is_game {
        return String::new();
    }
    for line in block.lines() {
        let line = line.trim();
        let Some(eq) = line.find('=') else { continue };
        if matches!(
            line[..eq].trim().to_lowercase().as_str(),
            "executable" | "open"
        ) {
            return crate::stats::key_for(line[eq + 1..].trim());
        }
    }
    String::new()
}

// --------------------------------------------------------------------------
// The sync index
// --------------------------------------------------------------------------

/// What the cartridge remembers about syncing, so a later comparison has
/// something to be relative to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Index {
    pub version: u32,
    pub slots: BTreeMap<String, SlotRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlotRecord {
    /// What each machine knew when it last synced this slot.
    ///
    /// Per host, and it has to be. A modification time written on a desktop
    /// says nothing on a Steam Deck: the two clocks are not the same clock,
    /// and more to the point the two *save directories* are not the same
    /// directory. One shared "last synced at" makes a cartridge that A ejected
    /// five seconds ago look settled to B, which has never seen the save at
    /// all — which is exactly the bug this shape replaced.
    pub hosts: BTreeMap<String, HostSync>,
    /// The most recent sync by anyone, for the interface to display. Never
    /// read by [`decide`].
    pub last_sync: u64,
    pub last_direction: String,
    pub last_host: String,
    pub last_host_path: String,
}

/// One machine's memory of this slot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostSync {
    /// Newest modification time on *this host's* copy as of its last sync.
    pub last_sync: u64,
    /// Newest modification time on the *cartridge's* copy as of the same
    /// moment. This is what makes "somebody else played since I last saw it"
    /// an answerable question rather than a guess.
    pub cartridge_seen: u64,
    /// Where this host resolved the slot to. Also half of the key — see
    /// [`host_key`] — and kept as a field so the file reads without decoding
    /// its own keys.
    pub host_path: String,
}

/// How one machine's baseline is addressed in [`SlotRecord::hosts`].
///
/// Machine *and* path, because neither alone identifies a save directory. Two
/// people on one PC have the same hostname and different saves; two PCs set up
/// the same way have the same path and different saves. A baseline that
/// applied to the wrong one of those would report the other's edits as this
/// one's, which is a conflict invented out of nothing — or worse, a change
/// missed.
pub fn host_key(host_path: &str) -> String {
    format!("{}:{host_path}", crate::stats::host_name())
}

pub fn index_path(root: &Path) -> PathBuf {
    saves_root(root).join(INDEX_FILE)
}

pub fn saves_root(root: &Path) -> PathBuf {
    root.join(crate::create::ASSET_DIR).join(SAVES_DIR)
}

/// The cartridge-side directory for one slot.
pub fn slot_path(root: &Path, slot: &SaveSlot) -> PathBuf {
    saves_root(root).join(&slot.id)
}

pub fn read_index(root: &Path) -> Index {
    let Ok(text) = std::fs::read_to_string(index_path(root)) else {
        return Index::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn write_index(root: &Path, index: &Index) -> Result<(), String> {
    let path = index_path(root);
    let dir = path
        .parent()
        .ok_or_else(|| "index path has no parent".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let mut out = index.clone();
    out.version = 1;
    let text = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(windows)]
    let _ = std::fs::remove_file(&path);
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(format!("{}: {e}", path.display()))
        }
    }
}

// --------------------------------------------------------------------------
// Deciding
// --------------------------------------------------------------------------

/// What this slot needs, worked out but not yet done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    /// The host's copy has changed: it goes to the cartridge.
    Push,
    /// The cartridge's copy has changed: it comes to the host.
    Pull,
    /// Neither has changed since the last sync.
    InSync,
    /// Both have changed since the last sync. Nothing is written.
    Conflict,
    /// Neither side has a save yet. Nothing is written, and that is not a
    /// problem — it is a game that has not been played.
    Empty,
    /// The declaration cannot be used on this host.
    Unusable,
    /// The host's directory is a symlink onto this cartridge already, which is
    /// [`SyncMode::Link`] working as intended.
    Linked,
}

/// A slot, resolved against this host, with the answer worked out.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotStatus {
    pub slot: SaveSlot,
    /// Where the declaration resolved to, or empty when it did not.
    pub host_path: String,
    pub direction: Direction,
    /// Newest modification time on each side, unix seconds; zero for absent.
    pub host_newest: u64,
    pub cartridge_newest: u64,
    pub host_bytes: u64,
    pub cartridge_bytes: u64,
    pub last_sync: u64,
    /// Human-readable reason, for the states that have one.
    pub detail: String,
}

/// A save kept in the registry rather than in a folder.
fn is_registry(template: &str) -> bool {
    template.trim().to_lowercase().starts_with("{registry}")
}

/// Look at every declared slot and say what would happen.
///
/// Read-only, but for one thing: a registry save's export is refreshed in the
/// app's own folder, since that export is how it is compared. Nothing in the
/// user's folders is created, and that matters: the launcher
/// calls this to draw a panel on every insert, including for cartridges whose
/// owner has never turned save syncing on.
pub fn status(root: &Path) -> Vec<SlotStatus> {
    let index = read_index(root);
    declared(root)
        .into_iter()
        .map(|slot| status_of(root, slot, &index))
        .collect()
}

fn status_of(root: &Path, slot: SaveSlot, index: &Index) -> SlotStatus {
    let record = index.slots.get(&slot.id).cloned().unwrap_or_default();
    let cart = slot_path(root, &slot);

    let host = match resolve_template(&slot.template) {
        Ok(path) => path,
        Err(why) => {
            return SlotStatus {
                slot,
                host_path: String::new(),
                direction: Direction::Unusable,
                host_newest: 0,
                cartridge_newest: 0,
                host_bytes: 0,
                cartridge_bytes: 0,
                last_sync: record.last_sync,
                detail: why.to_string(),
            }
        }
    };

    // A registry save's host side is its export, brought up to date first.
    if is_registry(&slot.template) {
        if let Err(why) = crate::registry::stage(&host) {
            return SlotStatus {
                slot,
                host_path: String::new(),
                direction: Direction::Unusable,
                host_newest: 0,
                cartridge_newest: 0,
                host_bytes: 0,
                cartridge_bytes: 0,
                last_sync: record.last_sync,
                detail: why,
            };
        }
    }

    let mine = record
        .hosts
        .get(&host_key(&host.display().to_string()))
        .cloned()
        .unwrap_or_default();

    let (host_newest, host_bytes) = tree_summary(&host);
    let (cart_newest, cart_bytes) = tree_summary(&cart);

    let linked = links_into(&host, &cart);
    let direction = if linked {
        Direction::Linked
    } else {
        decide(
            mine.last_sync,
            mine.cartridge_seen,
            host_newest,
            cart_newest,
            host_bytes > 0 || host.is_dir(),
            cart_bytes > 0 || cart.is_dir(),
        )
    };

    let detail = match direction {
        Direction::Conflict => {
            "both copies changed since the last sync — pick one, nothing was written".to_string()
        }
        Direction::Linked => format!("{} is a link onto the cartridge", host.display()),
        _ => String::new(),
    };

    SlotStatus {
        slot,
        host_path: host.display().to_string(),
        direction,
        host_newest,
        cartridge_newest: cart_newest,
        host_bytes,
        cartridge_bytes: cart_bytes,
        last_sync: mine.last_sync,
        detail,
    }
}

/// The rule, with the filesystem taken out of it so it can be tested directly.
///
/// Both baselines are *this host's*: what its own copy looked like when it last
/// synced, and what the cartridge looked like at the same moment. Each side is
/// compared against its own baseline, which is what makes "changed since I last
/// saw it" mean the same thing on a machine that synced an hour ago and one
/// that synced in March.
pub fn decide(
    host_baseline: u64,
    cartridge_baseline: u64,
    host_newest: u64,
    cart_newest: u64,
    host_exists: bool,
    cart_exists: bool,
) -> Direction {
    if !host_exists && !cart_exists {
        return Direction::Empty;
    }
    // Nothing on this machine to lose. A cartridge carried to a new PC is
    // overwhelmingly this case, and it is the one the feature is *for*.
    //
    // It also means a save deleted on the host comes back from the cartridge
    // rather than the deletion propagating. That is the deliberate choice: a
    // cartridge is a carrier, and restoring a save somebody did not want is an
    // annoyance where erasing one they did is the end of a playthrough.
    if !host_exists && cart_newest > 0 {
        return Direction::Pull;
    }
    if host_baseline == 0 {
        // This host has never synced. One side having something and the other
        // not is the ordinary first run; both having something is two
        // histories meeting, and nobody but the user can say which is wanted.
        return match (host_newest > 0, cart_newest > 0) {
            (true, false) => Direction::Push,
            (false, true) => Direction::Pull,
            (true, true) => Direction::Conflict,
            (false, false) => Direction::Empty,
        };
    }
    let host_changed = host_newest > host_baseline.saturating_add(MTIME_SLACK_SECONDS);
    let cart_changed = cart_newest > cartridge_baseline.saturating_add(MTIME_SLACK_SECONDS);
    match (host_changed, cart_changed) {
        (true, true) => Direction::Conflict,
        (true, false) => Direction::Push,
        (false, true) => Direction::Pull,
        (false, false) => Direction::InSync,
    }
}

/// Newest modification time and total size under a directory.
///
/// Absent, empty and unreadable all answer `(0, 0)`. Symlinks are not followed:
/// a save directory that is a link onto the cartridge would otherwise report
/// the cartridge's own times as the host's, and every comparison after that is
/// meaningless.
pub fn tree_summary(dir: &Path) -> (u64, u64) {
    fn walk(dir: &Path, newest: &mut u64, bytes: &mut u64, depth: usize) {
        if depth > 32 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                walk(&entry.path(), newest, bytes, depth + 1);
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            *bytes = bytes.saturating_add(meta.len());
            if let Ok(modified) = meta.modified() {
                if let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH) {
                    *newest = (*newest).max(since.as_secs());
                }
            }
        }
    }
    let mut newest = 0;
    let mut bytes = 0;
    if std::fs::symlink_metadata(dir)
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        walk(dir, &mut newest, &mut bytes, 0);
    }
    (newest, bytes)
}

/// Whether the host path is a symlink pointing at this cartridge's slot.
fn links_into(host: &Path, cart: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(host) else {
        return false;
    };
    if !meta.file_type().is_symlink() {
        return false;
    }
    let Ok(target) = std::fs::read_link(host) else {
        return false;
    };
    // A junction reads back as `\\?\D:\...`; the same place as `D:\...`.
    let target = match target.to_str().and_then(|t| t.strip_prefix(r"\\?\")) {
        Some(plain) => PathBuf::from(plain),
        None => target,
    };
    let target = if target.is_absolute() {
        target
    } else {
        host.parent().unwrap_or(Path::new("")).join(target)
    };
    normalise(&target) == normalise(cart)
}

/// Lexical cleanup, because `canonicalize` needs the path to exist and this is
/// asked about links whose target may have gone with the drive.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// --------------------------------------------------------------------------
// Doing
// --------------------------------------------------------------------------

/// What a sync actually did.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub slot_id: String,
    pub label: String,
    pub direction: Direction,
    pub files: u64,
    pub bytes: u64,
    /// Where the replaced copy was moved to, if anything was replaced.
    pub backup: String,
    pub detail: String,
}

/// Sync one slot in the direction its status asked for.
///
/// `Conflict`, `Unusable`, `InSync`, `Empty` and `Linked` all do nothing and
/// say so. Only `Push` and `Pull` write, and both of them back the destination
/// up first.
pub fn sync_slot(root: &Path, status: &SlotStatus) -> Result<SyncOutcome, String> {
    let mut outcome = SyncOutcome {
        slot_id: status.slot.id.clone(),
        label: status.slot.label.clone(),
        direction: status.direction,
        files: 0,
        bytes: 0,
        backup: String::new(),
        detail: status.detail.clone(),
    };
    let (source, destination) = match status.direction {
        Direction::Push => (
            PathBuf::from(&status.host_path),
            slot_path(root, &status.slot),
        ),
        Direction::Pull => (
            slot_path(root, &status.slot),
            PathBuf::from(&status.host_path),
        ),
        _ => return Ok(outcome),
    };

    if let Some(backup) = back_up(&destination)? {
        outcome.backup = backup.display().to_string();
    }
    let (files, bytes) = copy_tree(&source, &destination)?;
    outcome.files = files;
    outcome.bytes = bytes;
    if matches!(status.direction, Direction::Pull) && is_registry(&status.slot.template) {
        crate::registry::apply(&destination)?;
    }

    record_sync(root, status, &destination)?;
    Ok(outcome)
}

/// Sync every slot that has something to do. Errors are per slot: one save
/// directory the host cannot write must not stop the rest.
pub fn sync_all(root: &Path) -> Vec<Result<SyncOutcome, String>> {
    status(root)
        .iter()
        .map(|status| sync_slot(root, status))
        .collect()
}

/// Push every slot to the cartridge, for a clean eject.
///
/// Deliberately not "sync": on the way out, the host's copy is the one that
/// just had a game writing to it, and a slot that reads as `InSync` because
/// the game exited within the modification-time slack should still go. A
/// `Conflict` is still refused — the cartridge holding a change the host never
/// saw means something else wrote to the drive, and overwriting that blind is
/// exactly the mistake this module exists to avoid.
pub fn push_all(root: &Path) -> Vec<Result<SyncOutcome, String>> {
    status(root)
        .into_iter()
        .map(|status| sync_slot(root, &on_the_way_out(status)))
        .collect()
}

/// Promote a slot that reads as settled to a push, for an eject.
///
/// A `Conflict` is left exactly as it is: the cartridge holding a change this
/// host never saw means something else wrote to the drive, and overwriting
/// that blind is the mistake this module exists to avoid.
fn on_the_way_out(mut status: SlotStatus) -> SlotStatus {
    if matches!(status.direction, Direction::InSync | Direction::Push) && status.host_bytes > 0 {
        status.direction = Direction::Push;
    }
    status
}

/// Everything a cartridge being plugged in owes its saves.
///
/// One call for the launcher to make on insert, so the decision about what
/// link mode means lives here with the rest of it rather than in the window.
/// Per-slot results: a save directory this host cannot write must not stop the
/// other three from arriving.
pub fn attach_all(root: &Path) -> Vec<Result<SyncOutcome, String>> {
    status(root)
        .into_iter()
        .map(|status| match status.slot.mode {
            SyncMode::Link => match link_slot(root, &status) {
                Ok(outcome) => Ok(outcome),
                // The platform refused the link — Windows without Developer
                // Mode is the whole of this case. Copying is what link mode
                // was an optimisation over, so fall back to it rather than
                // leaving the cartridge with no save at all. `link_slot` backs
                // the host copy up before it tries, so nothing is lost by
                // having got this far.
                Err(why) => sync_slot(root, &status).map(|mut outcome| {
                    outcome.detail = format!("linking failed, copied instead: {why}");
                    outcome
                }),
            },
            SyncMode::Copy => sync_slot(root, &status),
        })
        .collect()
}

/// Everything a clean eject owes them.
///
/// A linked slot becomes a real directory again, so the host is left with a
/// working save rather than a link into a drive that is no longer there. A
/// copied slot is pushed, on the terms [`push_all`] describes.
pub fn detach_all(root: &Path) -> Vec<Result<SyncOutcome, String>> {
    let statuses = status(root);
    statuses
        .into_iter()
        .map(|status| match status.slot.mode {
            SyncMode::Link => unlink_slot(root, &status.slot),
            SyncMode::Copy => sync_slot(root, &on_the_way_out(status)),
        })
        .collect()
}

fn record_sync(root: &Path, status: &SlotStatus, _destination: &Path) -> Result<(), String> {
    let host = PathBuf::from(&status.host_path);
    let cart = slot_path(root, &status.slot);

    // Stamped from the files themselves rather than from the clock. The
    // comparison that reads these back compares them against modification
    // times, and a host whose clock is behind the drive's would otherwise
    // write a baseline that every file on the cartridge is already newer than.
    let (host_newest, _) = tree_summary(&host);
    let (cart_newest, _) = tree_summary(&cart);
    let now = crate::stats::now_unix();

    let mut index = read_index(root);
    let record = index.slots.entry(status.slot.id.clone()).or_default();
    record.hosts.insert(
        host_key(&status.host_path),
        HostSync {
            last_sync: host_newest.max(now),
            cartridge_seen: cart_newest.max(now),
            host_path: status.host_path.clone(),
        },
    );
    record.last_sync = now;
    record.last_direction = match status.direction {
        Direction::Push => "push",
        Direction::Pull => "pull",
        _ => "none",
    }
    .to_string();
    record.last_host = crate::stats::host_name();
    record.last_host_path = status.host_path.clone();
    write_index(root, &index)
}

/// Move a directory aside, keeping the most recent [`BACKUPS_KEPT`].
///
/// Returns `None` when there was nothing there to preserve — a first sync onto
/// a machine that has never run the game is the common case, and it should not
/// leave an empty backup directory behind to explain later.
pub fn back_up(target: &Path) -> Result<Option<PathBuf>, String> {
    let Ok(meta) = std::fs::symlink_metadata(target) else {
        return Ok(None);
    };
    if meta.file_type().is_symlink() {
        // A link is not data. Removing it loses nothing, and keeping it would
        // make the copy that follows write through it to the other side.
        std::fs::remove_file(target).map_err(|e| format!("{}: {e}", target.display()))?;
        return Ok(None);
    }
    let (_, bytes) = tree_summary(target);
    if bytes == 0 {
        std::fs::remove_dir_all(target).ok();
        return Ok(None);
    }

    let stamp = crate::stats::now_unix();
    let mut backup = sibling(target, &format!("{BACKUP_SUFFIX}-{stamp}"));
    let mut n = 2;
    while backup.exists() {
        backup = sibling(target, &format!("{BACKUP_SUFFIX}-{stamp}-{n}"));
        n += 1;
    }
    std::fs::rename(target, &backup).map_err(|e| {
        format!(
            "could not move {} aside to {}: {e}",
            target.display(),
            backup.display()
        )
    })?;
    prune_backups(target);
    Ok(Some(backup))
}

fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "saves".to_string());
    target
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{name}{suffix}"))
}

/// Keep the newest [`BACKUPS_KEPT`] backups of one directory and delete the rest.
fn prune_backups(target: &Path) {
    let Some(parent) = target.parent() else {
        return;
    };
    let Some(name) = target.file_name().map(|n| n.to_string_lossy().to_string()) else {
        return;
    };
    let prefix = format!("{name}{BACKUP_SUFFIX}-");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    let mut found: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(&prefix))
        .collect();
    // The suffix is a unix timestamp, so sorting the names sorts by age until
    // the year 2286 — every stamp is the same number of digits until then.
    found.sort();
    while found.len() > BACKUPS_KEPT {
        let oldest = found.remove(0);
        std::fs::remove_dir_all(parent.join(oldest)).ok();
    }
}

/// Copy a directory tree, making the destination match the source exactly.
///
/// The destination is created fresh — the caller has already moved anything
/// that was there out of the way — so a file the player deleted in game stays
/// deleted rather than coming back from the other side on the next sync.
pub fn copy_tree(source: &Path, destination: &Path) -> Result<(u64, u64), String> {
    std::fs::create_dir_all(destination).map_err(|e| format!("{}: {e}", destination.display()))?;
    let mut files = 0u64;
    let mut bytes = 0u64;
    copy_into(source, destination, &mut files, &mut bytes, 0)?;
    Ok((files, bytes))
}

fn copy_into(
    source: &Path,
    destination: &Path,
    files: &mut u64,
    bytes: &mut u64,
    depth: usize,
) -> Result<(), String> {
    if depth > 32 {
        return Err(format!(
            "{}: nested deeper than 32 directories",
            source.display()
        ));
    }
    let entries = match std::fs::read_dir(source) {
        Ok(entries) => entries,
        // A source that is not there is an empty source, which is a legitimate
        // thing to copy: it makes the destination empty, which is correct.
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if kind.is_symlink() {
            // Not followed, not recreated. exFAT cannot hold one, and a save
            // directory containing a link out to somewhere else on the host is
            // not something to carry to another machine.
            continue;
        }
        if kind.is_dir() {
            std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
            copy_into(&from, &to, files, bytes, depth + 1)?;
            continue;
        }
        let copied = std::fs::copy(&from, &to)
            .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
        *files += 1;
        *bytes += copied;
    }
    Ok(())
}

// --------------------------------------------------------------------------
// Link mode
// --------------------------------------------------------------------------

/// Replace the host's save directory with a link onto the cartridge.
///
/// The host's copy is seeded onto the cartridge first if the cartridge has
/// nothing, and backed up either way, because this deletes a directory the
/// player's saves are in and there is no version of that which should be done
/// without a copy somewhere else.
///
/// Returns the error the platform gave if it will not make the link — on
/// Windows, an unprivileged process without Developer Mode — and leaves the
/// backup in place so the caller can fall back to [`SyncMode::Copy`] rather
/// than being left with nothing.
pub fn link_slot(root: &Path, status: &SlotStatus) -> Result<SyncOutcome, String> {
    if is_registry(&status.slot.template) {
        return Err("a registry save cannot be linked".to_string());
    }
    if status.host_path.is_empty() {
        return Err(format!("{}: {}", status.slot.label, status.detail));
    }
    if matches!(status.direction, Direction::Linked) {
        return Ok(SyncOutcome {
            slot_id: status.slot.id.clone(),
            label: status.slot.label.clone(),
            direction: Direction::Linked,
            files: 0,
            bytes: 0,
            backup: String::new(),
            detail: status.detail.clone(),
        });
    }
    if matches!(status.direction, Direction::Conflict) {
        return Err(format!(
            "{}: both copies changed since the last sync, so linking would discard one",
            status.slot.label
        ));
    }

    let host = PathBuf::from(&status.host_path);
    let cart = slot_path(root, &status.slot);
    std::fs::create_dir_all(&cart).map_err(|e| format!("{}: {e}", cart.display()))?;

    // Seed the cartridge from the host unless the cartridge is the fresher of
    // the two, in which case the host's copy is the one being superseded.
    let mut files = 0;
    let mut bytes = 0;
    if !matches!(status.direction, Direction::Pull) {
        let (f, b) = copy_tree(&host, &cart)?;
        files = f;
        bytes = b;
    }

    let backup = back_up(&host)?;
    if let Some(parent) = host.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::remove_dir_all(&host).ok();

    symlink_dir(&cart, &host).map_err(|e| {
        format!(
            "could not link {} to {}: {e}{}",
            host.display(),
            cart.display(),
            match &backup {
                Some(path) => format!(" (the host's copy is at {})", path.display()),
                None => String::new(),
            }
        )
    })?;

    record_sync(root, status, &cart)?;
    Ok(SyncOutcome {
        slot_id: status.slot.id.clone(),
        label: status.slot.label.clone(),
        direction: Direction::Linked,
        files,
        bytes,
        backup: backup.map(|p| p.display().to_string()).unwrap_or_default(),
        detail: String::new(),
    })
}

/// Turn a link back into a real directory on the host, for a clean eject.
///
/// Only ever acts on a link that points at this cartridge. A directory, or a
/// link pointing somewhere else, is left exactly as it is.
pub fn unlink_slot(root: &Path, slot: &SaveSlot) -> Result<SyncOutcome, String> {
    let host = resolve_template(&slot.template).map_err(|e| e.to_string())?;
    let cart = slot_path(root, slot);
    let mut outcome = SyncOutcome {
        slot_id: slot.id.clone(),
        label: slot.label.clone(),
        direction: Direction::Pull,
        files: 0,
        bytes: 0,
        backup: String::new(),
        detail: String::new(),
    };
    if !links_into(&host, &cart) {
        outcome.direction = Direction::InSync;
        outcome.detail = "not linked to this cartridge".to_string();
        return Ok(outcome);
    }
    remove_link(&host).map_err(|e| format!("{}: {e}", host.display()))?;
    let (files, bytes) = copy_tree(&cart, &host)?;
    outcome.files = files;
    outcome.bytes = bytes;
    Ok(outcome)
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// A symlink when Windows allows one (Developer Mode, or elevated), else a
/// directory junction, which any user may make. A junction only reaches a
/// local volume, which a cartridge always is, and to everything reading it —
/// the game, `read_link`, `remove_dir` — it behaves as the symlink would.
#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    let refused = match std::os::windows::fs::symlink_dir(target, link) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    junction(target, link).map_err(|_| refused)
}

#[cfg(windows)]
fn junction(target: &Path, link: &Path) -> std::io::Result<()> {
    // std has no junction call; mklink is a cmd built-in.
    let made = crate::proc::command("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(link)
        .arg(target)
        .output()?;
    if made.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&made.stderr).trim().to_string(),
        ))
    }
}

/// Remove a symlink to a directory.
///
/// Unix unlinks it as a file; Windows holds a directory symlink in a directory
/// entry, so `remove_file` is the wrong call there and `remove_dir` is the
/// right one. Neither follows the link, so the cartridge is untouched.
fn remove_link(link: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        std::fs::remove_dir(link)
    }
    #[cfg(not(windows))]
    {
        std::fs::remove_file(link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    #[cfg(windows)]
    #[test]
    fn a_junction_counts_as_a_link_into_the_cartridge_and_unlinks_cleanly() {
        let scratch = Scratch::new("saves-junction");
        let cart = scratch.join("cart");
        std::fs::create_dir_all(&cart).unwrap();
        scratch.write("cart/save.dat", b"x");
        let host = scratch.join("host");
        junction(&cart, &host).unwrap();
        assert!(host.join("save.dat").is_file());
        assert!(links_into(&host, &cart));
        remove_link(&host).unwrap();
        assert!(!host.exists());
        assert!(
            cart.join("save.dat").is_file(),
            "the cartridge is untouched"
        );
    }

    /// Point the token table at a scratch directory for the duration of a test.
    ///
    /// The environment is process-wide, so these tests cannot run in parallel
    /// with each other; they take a lock rather than being marked serial by
    /// hand, which would only work until somebody added another one.
    fn with_home<T>(home: &Path, body: impl FnOnce() -> T) -> T {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let guard = LOCK.get_or_init(|| Mutex::new(())).lock();
        let _guard = match guard {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        std::env::set_var("PC_GAMEPAK_HOME", home);
        let out = body();
        std::env::remove_var("PC_GAMEPAK_HOME");
        out
    }

    fn conf(scratch: &Scratch, body: &str) {
        scratch.write("cartridge.conf", body.as_bytes());
    }

    #[test]
    fn a_picked_folder_is_written_the_portable_way() {
        let scratch = Scratch::new("saves-template");
        with_home(scratch.path(), || {
            let home = scratch.path();
            // The closest token wins over the home directory it sits in.
            assert_eq!(
                template_from_path(&home.join("Documents").join("My Games").join("Foo")),
                Some("{documents}/My Games/Foo".to_string())
            );
            let appdata = token_path("appdata").unwrap();
            assert_eq!(
                template_from_path(&appdata.join("StardewValley").join("Saves")),
                Some("{appdata}/StardewValley/Saves".to_string())
            );
            assert_eq!(
                template_from_path(&home.join("Odd").join("Place")),
                Some("{home}/Odd/Place".to_string())
            );
            // A token's own directory, or somewhere no token reaches, cannot be
            // carried.
            assert_eq!(template_from_path(home), None);
            assert_eq!(template_from_path(Path::new("/nowhere/at/all")), None);
        });
    }

    #[test]
    fn a_memory_card_declares_its_saves_in_its_own_file() {
        let scratch = Scratch::new("saves-memcard");
        scratch.write(
            crate::memcard::CONF,
            b"title=Card\n\n[game]\ntitle=Stardew\nsave=Stardew|{appdata}/StardewValley/Saves\n",
        );
        let slots = declared(scratch.path());
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].label, "Stardew");
    }

    /// Say when a tree was last written, rather than sleeping until it is true.
    fn stamp(dir: &Path, unix: u64) {
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix);
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stamp(&path, unix);
                continue;
            }
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(when).unwrap();
        }
    }

    // ---- declarations ----------------------------------------------------

    #[test]
    fn a_cartridge_with_no_save_lines_declares_nothing() {
        let scratch = Scratch::new("saves-none");
        conf(&scratch, "executable=steam://rungameid/1\ntitle=X\n");
        assert!(declared(scratch.path()).is_empty());
    }

    #[test]
    fn a_label_may_be_given_or_derived() {
        let slots = parse_declarations(
            "executable=steam://rungameid/413150\n\
             save=Stardew|{appdata}/StardewValley/Saves\n\
             save={documents}/My Games/Terraria\n",
        );
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].label, "Stardew");
        assert_eq!(slots[0].id, "stardew");
        assert_eq!(slots[1].label, "Terraria");
        assert_eq!(slots[1].id, "terraria");
    }

    #[test]
    fn two_save_lines_for_one_game_both_survive() {
        // The map-based parser would have kept one of these.
        let slots = parse_declarations(
            "executable=x://1\nsave={appdata}/Foo/Saves\nsave={documents}/Foo/Config\n",
        );
        assert_eq!(slots.len(), 2);
    }

    #[test]
    fn identical_labels_get_distinct_directories() {
        let slots =
            parse_declarations("save=Saves|{appdata}/A/Saves\nsave=Saves|{appdata}/B/Saves\n");
        assert_eq!(slots[0].id, "saves");
        assert_eq!(slots[1].id, "saves-2");
    }

    #[test]
    fn a_bundle_attaches_each_slot_to_its_own_game() {
        let slots = parse_declarations(
            "[collection]\ntitle=Two\n\
             [game]\ntitle=A\nexecutable=steam://rungameid/1\nsave=A|{appdata}/A\n\
             [game]\ntitle=B\nexecutable=steam://rungameid/2\nsave=B|{appdata}/B\n",
        );
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].game.as_deref(), Some("steam://rungameid/1"));
        assert_eq!(slots[1].game.as_deref(), Some("steam://rungameid/2"));
    }

    #[test]
    fn a_platform_line_replaces_the_plain_one_on_that_platform() {
        let host = host_suffix();
        let slots = parse_declarations(&format!(
            "executable=x://1\nsave=Saves|{{appdata}}/Everywhere\nsave.{host}=Saves|{{appdata}}/Here\n"
        ));
        assert_eq!(slots.len(), 1, "{slots:?}");
        assert_eq!(slots[0].template, "{appdata}/Here");
    }

    #[test]
    fn a_line_for_another_platform_is_carried_and_ignored() {
        let other = if host_suffix() == "windows" {
            "linux"
        } else {
            "windows"
        };
        let slots = parse_declarations(&format!(
            "executable=x://1\nsave=Saves|{{appdata}}/Everywhere\nsave.{other}=Saves|{{appdata}}/There\n"
        ));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].template, "{appdata}/Everywhere");
    }

    #[test]
    fn the_mode_defaults_to_copy_and_can_be_asked_for() {
        assert_eq!(
            parse_declarations("save={appdata}/A/S\n")[0].mode,
            SyncMode::Copy
        );
        assert_eq!(
            parse_declarations("savemode=link\nsave={appdata}/A/S\n")[0].mode,
            SyncMode::Link
        );
    }

    // ---- resolution ------------------------------------------------------

    #[test]
    fn every_token_resolves_somewhere_on_this_host() {
        let scratch = Scratch::new("saves-tokens");
        with_home(scratch.path(), || {
            for token in TOKENS {
                assert!(token_path(token).is_some(), "{token} did not resolve");
            }
            assert!(token_path("nonsense").is_none());
        });
    }

    #[test]
    fn a_template_resolves_under_the_token_it_names() {
        let scratch = Scratch::new("saves-resolve");
        with_home(scratch.path(), || {
            let path = resolve_template("{home}/Games/Foo").expect("resolve");
            assert!(path.starts_with(scratch.path()), "{}", path.display());
            assert!(path.ends_with("Games/Foo"));
        });
    }

    #[test]
    fn backslashes_in_a_template_are_separators_not_characters() {
        let scratch = Scratch::new("saves-backslash");
        with_home(scratch.path(), || {
            assert_eq!(
                resolve_template("{home}\\Games\\Foo").expect("resolve"),
                resolve_template("{home}/Games/Foo").expect("resolve")
            );
        });
    }

    #[test]
    fn a_whole_user_directory_is_refused() {
        let scratch = Scratch::new("saves-broad");
        with_home(scratch.path(), || {
            assert_eq!(resolve_template("{home}"), Err(Unusable::TooBroad));
            assert_eq!(
                resolve_template("{home}/Documents"),
                Err(Unusable::TooBroad)
            );
            assert_eq!(resolve_template("{appdata}"), Err(Unusable::TooBroad));
            assert_eq!(resolve_template("{documents}/"), Err(Unusable::TooBroad));
            // One level inside a user directory is the real case, and passes.
            assert!(resolve_template("{appdata}/Foo").is_ok());
        });
    }

    #[test]
    fn a_template_cannot_climb_out_of_its_token() {
        let scratch = Scratch::new("saves-escape");
        with_home(scratch.path(), || {
            assert_eq!(
                resolve_template("{appdata}/../../etc"),
                Err(Unusable::Escapes)
            );
        });
    }

    #[test]
    fn a_literal_path_is_refused_as_host_specific() {
        assert_eq!(
            resolve_template("/home/someone/.config/Foo"),
            Err(Unusable::NoToken)
        );
        assert_eq!(
            resolve_template("C:/Users/Someone/Foo"),
            Err(Unusable::NoToken)
        );
    }

    #[test]
    fn an_unknown_token_names_itself_in_the_error() {
        let scratch = Scratch::new("saves-unknown");
        with_home(scratch.path(), || {
            let err = resolve_template("{proton}/Foo").expect_err("must refuse");
            assert_eq!(err, Unusable::UnknownToken("proton".into()));
            assert!(err.to_string().contains("proton"));
        });
    }

    // ---- the rule --------------------------------------------------------

    #[test]
    fn a_first_sync_goes_whichever_way_has_data() {
        assert_eq!(decide(0, 0, 100, 0, true, false), Direction::Push);
        assert_eq!(decide(0, 0, 0, 100, false, true), Direction::Pull);
        assert_eq!(decide(0, 0, 0, 0, false, false), Direction::Empty);
    }

    #[test]
    fn two_histories_meeting_for_the_first_time_are_a_conflict() {
        assert_eq!(decide(0, 0, 100, 200, true, true), Direction::Conflict);
    }

    #[test]
    fn only_the_side_that_changed_is_copied() {
        assert_eq!(decide(1000, 1000, 2000, 900, true, true), Direction::Push);
        assert_eq!(decide(1000, 1000, 900, 2000, true, true), Direction::Pull);
        assert_eq!(decide(1000, 1000, 900, 900, true, true), Direction::InSync);
        assert_eq!(
            decide(1000, 1000, 2000, 2000, true, true),
            Direction::Conflict
        );
    }

    #[test]
    fn a_machine_that_synced_in_march_is_not_told_the_cartridge_is_settled() {
        // The bug the per-host baseline exists for. This host last synced at
        // 1000 and the cartridge has been played on another machine since;
        // a single shared "last synced" would have been stamped at 5000 by
        // that machine and this one would have seen nothing to do.
        assert_eq!(decide(1000, 1000, 1000, 5000, true, true), Direction::Pull);
    }

    #[test]
    fn a_host_with_no_copy_at_all_pulls_whatever_the_baseline_says() {
        assert_eq!(decide(9000, 9000, 0, 100, false, true), Direction::Pull);
    }

    #[test]
    fn a_two_second_filesystem_does_not_invent_a_change() {
        // exFAT rounds to two seconds, so a file written at the moment of the
        // sync can come back stamped later than it.
        assert_eq!(
            decide(1000, 1000, 1002, 1002, true, true),
            Direction::InSync
        );
        assert_eq!(decide(1000, 1000, 1004, 1000, true, true), Direction::Push);
    }

    #[test]
    fn a_second_account_on_one_machine_has_its_own_baseline() {
        assert_ne!(
            host_key("/home/a/.config/Foo"),
            host_key("/home/b/.config/Foo")
        );
    }

    // ---- copying ---------------------------------------------------------

    #[test]
    fn a_first_sync_carries_the_host_copy_onto_the_cartridge() {
        let scratch = Scratch::new("saves-push");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(&scratch, "executable=x://1\nsave=Saves|{home}/Foo/Saves\n");
        scratch.write("home/Foo/Saves/slot1.sav", b"chapter one");
        scratch.write("home/Foo/Saves/deep/extra.sav", b"side quest");

        with_home(&home, || {
            let statuses = status(scratch.path());
            assert_eq!(statuses.len(), 1);
            assert_eq!(statuses[0].direction, Direction::Push);

            let outcome = sync_slot(scratch.path(), &statuses[0]).expect("sync");
            assert_eq!(outcome.files, 2);
            assert_eq!(
                std::fs::read_to_string(scratch.join(".gamepak/saves/saves/slot1.sav")).unwrap(),
                "chapter one"
            );
            assert!(scratch
                .join(".gamepak/saves/saves/deep/extra.sav")
                .is_file());

            // And the cartridge now knows it has been synced.
            let index = read_index(scratch.path());
            let record = &index.slots["saves"];
            assert_eq!(record.last_direction, "push");
            assert!(record.last_sync > 0);
            assert!(!record.last_host_path.is_empty());
            assert_eq!(record.hosts.len(), 1, "one machine has synced it");
        });
    }

    #[test]
    fn a_second_machine_gets_the_save_the_first_one_wrote() {
        let scratch = Scratch::new("saves-pull");
        let home = scratch.join("home2");
        std::fs::create_dir_all(&home).unwrap();
        conf(&scratch, "executable=x://1\nsave=Saves|{home}/Foo/Saves\n");
        // The cartridge arrives carrying a save; this host has never played it.
        scratch.write(".gamepak/saves/saves/slot1.sav", b"chapter four");

        with_home(&home, || {
            let statuses = status(scratch.path());
            assert_eq!(statuses[0].direction, Direction::Pull);
            sync_slot(scratch.path(), &statuses[0]).expect("sync");
            assert_eq!(
                std::fs::read_to_string(home.join("Foo/Saves/slot1.sav")).unwrap(),
                "chapter four"
            );
        });
    }

    #[test]
    fn nothing_is_written_when_both_sides_changed() {
        let scratch = Scratch::new("saves-conflict");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(&scratch, "executable=x://1\nsave=Saves|{home}/Foo/Saves\n");
        scratch.write("home/Foo/Saves/slot1.sav", b"eighty hours");
        scratch.write(".gamepak/saves/saves/slot1.sav", b"somebody else's run");

        with_home(&home, || {
            let statuses = status(scratch.path());
            assert_eq!(statuses[0].direction, Direction::Conflict);
            assert!(!statuses[0].detail.is_empty());

            let outcome = sync_slot(scratch.path(), &statuses[0]).expect("sync");
            assert_eq!(outcome.files, 0);
            // Both survive, untouched.
            assert_eq!(
                std::fs::read_to_string(home.join("Foo/Saves/slot1.sav")).unwrap(),
                "eighty hours"
            );
            assert_eq!(
                std::fs::read_to_string(scratch.join(".gamepak/saves/saves/slot1.sav")).unwrap(),
                "somebody else's run"
            );
        });
    }

    #[test]
    fn what_is_replaced_is_kept() {
        let scratch = Scratch::new("saves-backup");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(&scratch, "executable=x://1\nsave=Saves|{home}/Foo/Saves\n");
        scratch.write(".gamepak/saves/saves/new.sav", b"from the cartridge");
        scratch.write("home/Foo/Saves/old.sav", b"what was here");

        with_home(&home, || {
            // Force a pull over the top of a host copy, the way a user
            // resolving a conflict by hand would.
            let mut status = status(scratch.path()).remove(0);
            status.direction = Direction::Pull;
            let outcome = sync_slot(scratch.path(), &status).expect("sync");

            assert!(!outcome.backup.is_empty(), "nothing was preserved");
            let backup = PathBuf::from(&outcome.backup);
            assert_eq!(
                std::fs::read_to_string(backup.join("old.sav")).unwrap(),
                "what was here"
            );
            assert!(home.join("Foo/Saves/new.sav").is_file());
            assert!(
                !home.join("Foo/Saves/old.sav").exists(),
                "the copy is a mirror"
            );
        });
    }

    #[test]
    fn only_three_backups_are_kept() {
        let scratch = Scratch::new("saves-prune");
        let target = scratch.join("Saves");
        for n in 0..5 {
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("a.sav"), format!("{n}")).unwrap();
            back_up(&target).expect("back up");
        }
        let kept = std::fs::read_dir(scratch.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(BACKUP_SUFFIX))
            .count();
        assert_eq!(kept, BACKUPS_KEPT);
    }

    #[test]
    fn an_empty_directory_is_not_backed_up() {
        let scratch = Scratch::new("saves-empty-backup");
        let target = scratch.join("Saves");
        std::fs::create_dir_all(&target).unwrap();
        assert_eq!(back_up(&target).expect("back up"), None);
    }

    #[test]
    fn an_unusable_declaration_is_reported_rather_than_guessed_at() {
        let scratch = Scratch::new("saves-unusable");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(&scratch, "executable=x://1\nsave=Everything|{home}\n");

        with_home(&home, || {
            let statuses = status(scratch.path());
            assert_eq!(statuses[0].direction, Direction::Unusable);
            assert!(statuses[0].host_path.is_empty());
            assert!(statuses[0].detail.contains("not one game's saves"));
            // And syncing it does nothing at all.
            assert_eq!(sync_slot(scratch.path(), &statuses[0]).unwrap().files, 0);
        });
    }

    #[test]
    fn an_eject_pushes_a_save_the_game_only_just_wrote() {
        let scratch = Scratch::new("saves-eject");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(&scratch, "executable=x://1\nsave=Saves|{home}/Foo/Saves\n");
        scratch.write("home/Foo/Saves/slot1.sav", b"before");

        with_home(&home, || {
            // A first sync, so both sides agree.
            let statuses = status(scratch.path());
            sync_slot(scratch.path(), &statuses[0]).expect("sync");

            // The game writes again, within the slack that would read as
            // in-sync. On the way out that still has to go.
            std::fs::write(home.join("Foo/Saves/slot1.sav"), b"after").unwrap();
            let results = push_all(scratch.path());
            assert!(results[0].is_ok(), "{:?}", results[0]);
            assert_eq!(
                std::fs::read_to_string(scratch.join(".gamepak/saves/saves/slot1.sav")).unwrap(),
                "after"
            );
        });
    }

    #[test]
    fn an_eject_still_refuses_a_conflict() {
        let scratch = Scratch::new("saves-eject-conflict");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(&scratch, "executable=x://1\nsave=Saves|{home}/Foo/Saves\n");
        scratch.write("home/Foo/Saves/a.sav", b"host");
        scratch.write(".gamepak/saves/saves/b.sav", b"cartridge");

        with_home(&home, || {
            push_all(scratch.path());
            assert!(
                scratch.join(".gamepak/saves/saves/b.sav").is_file(),
                "the cartridge's own copy was overwritten"
            );
        });
    }

    // ---- link mode -------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn linking_puts_the_save_on_the_cartridge_and_leaves_a_copy_behind() {
        let scratch = Scratch::new("saves-link");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(
            &scratch,
            "executable=x://1\nsavemode=link\nsave=Saves|{home}/Foo/Saves\n",
        );
        scratch.write("home/Foo/Saves/slot1.sav", b"chapter one");

        with_home(&home, || {
            let statuses = status(scratch.path());
            let outcome = link_slot(scratch.path(), &statuses[0]).expect("link");
            assert!(!outcome.backup.is_empty(), "the host copy was not kept");

            let host = home.join("Foo/Saves");
            assert!(std::fs::symlink_metadata(&host)
                .unwrap()
                .file_type()
                .is_symlink());
            // Writing through the link lands on the cartridge.
            std::fs::write(host.join("slot2.sav"), b"chapter two").unwrap();
            assert!(scratch.join(".gamepak/saves/saves/slot2.sav").is_file());

            // And a second look knows it is already linked.
            assert_eq!(status(scratch.path())[0].direction, Direction::Linked);
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_clean_eject_turns_the_link_back_into_a_directory() {
        let scratch = Scratch::new("saves-unlink");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(
            &scratch,
            "executable=x://1\nsavemode=link\nsave=Saves|{home}/Foo/Saves\n",
        );
        scratch.write("home/Foo/Saves/slot1.sav", b"chapter one");

        with_home(&home, || {
            let statuses = status(scratch.path());
            link_slot(scratch.path(), &statuses[0]).expect("link");
            unlink_slot(scratch.path(), &statuses[0].slot).expect("unlink");

            let host = home.join("Foo/Saves");
            assert!(host.is_dir());
            assert!(!std::fs::symlink_metadata(&host)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(
                std::fs::read_to_string(host.join("slot1.sav")).unwrap(),
                "chapter one"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn unlinking_leaves_a_directory_that_was_never_linked_alone() {
        let scratch = Scratch::new("saves-unlink-none");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(&scratch, "executable=x://1\nsave=Saves|{home}/Foo/Saves\n");
        scratch.write("home/Foo/Saves/slot1.sav", b"untouched");

        with_home(&home, || {
            let slot = declared(scratch.path()).remove(0);
            let outcome = unlink_slot(scratch.path(), &slot).expect("unlink");
            assert_eq!(outcome.direction, Direction::InSync);
            assert_eq!(
                std::fs::read_to_string(home.join("Foo/Saves/slot1.sav")).unwrap(),
                "untouched"
            );
        });
    }

    // ---- surviving a rewrite --------------------------------------------

    #[test]
    fn a_rewrite_keeps_a_hand_written_save_line() {
        let old = "title=Old name\nexecutable=steam://rungameid/1\nsave=Saves|{appdata}/Foo\n";
        let new = "title=New name\nexecutable=steam://rungameid/1\ncover=.gamepak/cover.png\n";
        let kept = preserve(old, new);
        assert!(kept.contains("save=Saves|{appdata}/Foo"), "{kept}");
        assert!(kept.contains("title=New name"), "{kept}");
        assert!(!kept.contains("Old name"), "{kept}");
    }

    #[test]
    fn the_mode_is_kept_too() {
        let kept = preserve(
            "savemode=link\nsave={appdata}/A/S\n",
            "title=X\nexecutable=x://1\n",
        );
        assert!(kept.contains("savemode=link"), "{kept}");
    }

    #[test]
    fn a_reordered_bundle_keeps_each_line_with_its_own_game() {
        // The editor exists partly to reorder games. A line that stayed at its
        // old index would point a game at another game's saves.
        let old = "[collection]\ntitle=Two\n\n                   [game]\ntitle=A\nexecutable=steam://rungameid/1\nsave=A|{appdata}/A\n\n                   [game]\ntitle=B\nexecutable=steam://rungameid/2\nsave=B|{appdata}/B\n";
        let new = "[collection]\ntitle=Two\n\n                   [game]\ntitle=B\nexecutable=steam://rungameid/2\n\n                   [game]\ntitle=A\nexecutable=steam://rungameid/1\n";

        let slots = parse_declarations(&preserve(old, new));
        assert_eq!(slots.len(), 2);
        assert_eq!(
            slots[0].label, "B",
            "the rail was reordered, so B comes first"
        );
        assert_eq!(slots[0].game.as_deref(), Some("steam://rungameid/2"));
        assert_eq!(slots[1].label, "A");
        assert_eq!(slots[1].game.as_deref(), Some("steam://rungameid/1"));
    }

    #[test]
    fn a_game_taken_off_the_cartridge_takes_its_line_with_it() {
        let old = "[collection]\ntitle=Two\n\n                   [game]\ntitle=A\nexecutable=steam://rungameid/1\nsave=A|{appdata}/A\n\n                   [game]\ntitle=B\nexecutable=steam://rungameid/2\nsave=B|{appdata}/B\n";
        let new = "[collection]\ntitle=Two\n\n                   [game]\ntitle=A\nexecutable=steam://rungameid/1\n";

        let slots = parse_declarations(&preserve(old, new));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].label, "A");
    }

    #[test]
    fn a_conf_with_nothing_to_carry_is_returned_unchanged() {
        let new = "title=X\nexecutable=x://1\n";
        assert_eq!(preserve("title=Old\nexecutable=x://1\n", new), new);
    }

    #[test]
    fn a_platform_survives_a_rewrite_on_its_own_game() {
        let old = "[collection]\ntitle=T\nplatform=SNES\n\n[game]\nexecutable=x://1\n\n\
                   [game]\nexecutable=x://2\nplatform=GBA\n";
        let new = "[collection]\ntitle=Renamed\n\n[game]\nexecutable=x://1\n\n\
                   [game]\nexecutable=x://2\n";
        let kept = preserve(old, new);
        assert!(
            kept.starts_with("[collection]\ntitle=Renamed\nplatform=SNES\n"),
            "{kept}"
        );
        assert!(kept.contains("executable=x://2\nplatform=GBA"), "{kept}");
    }

    #[test]
    fn the_blank_line_between_sections_survives() {
        let old = "[collection]\ntitle=T\n\n[game]\nexecutable=x://1\nsave=A|{appdata}/A\n";
        let new = "[collection]\ntitle=T\n\n[game]\nexecutable=x://1\n\n[game]\nexecutable=x://2\n";
        let kept = preserve(old, new);
        // Two game sections still, not one run together.
        assert_eq!(kept.matches("[game]").count(), 2, "{kept}");
        assert!(kept.contains("save=A|{appdata}/A\n\n[game]"), "{kept}");
    }

    #[test]
    fn insert_then_eject_carries_a_save_between_two_machines() {
        // The whole feature in one test: play on A, carry the cartridge to B,
        // play there, carry it back.
        let scratch = Scratch::new("saves-round-trip");
        let base = crate::stats::now_unix();
        conf(&scratch, "executable=x://1\nsave=Saves|{home}/Foo/Saves\n");
        let a = scratch.join("machine-a");
        let b = scratch.join("machine-b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();

        // Machine A plays and ejects.
        scratch.write("machine-a/Foo/Saves/slot1.sav", b"chapter one");
        with_home(&a, || {
            for result in detach_all(scratch.path()) {
                result.expect("eject on A");
            }
        });

        // Machine B has never seen this game: insert gives it the save.
        with_home(&b, || {
            for result in attach_all(scratch.path()) {
                result.expect("insert on B");
            }
            assert_eq!(
                std::fs::read_to_string(b.join("Foo/Saves/slot1.sav")).unwrap(),
                "chapter one"
            );
            // B plays on, and ejects.
            std::fs::write(b.join("Foo/Saves/slot1.sav"), b"chapter two").unwrap();
            for result in detach_all(scratch.path()) {
                result.expect("eject on B");
            }
        });

        // B's session was an hour after A's; in this test it was the same
        // millisecond, and the rule is deliberately deaf to changes inside
        // three seconds of a baseline. Stamp the cartridge with the time the
        // session would have happened rather than sleeping through one.
        stamp(&saves_root(scratch.path()).join("saves"), base + 3600);

        // Back on A, the cartridge is the newer of the two.
        with_home(&a, || {
            assert_eq!(status(scratch.path())[0].direction, Direction::Pull);
            for result in attach_all(scratch.path()) {
                result.expect("insert on A");
            }
            assert_eq!(
                std::fs::read_to_string(a.join("Foo/Saves/slot1.sav")).unwrap(),
                "chapter two"
            );
        });
    }

    #[test]
    fn one_unwritable_slot_does_not_stop_the_others() {
        let scratch = Scratch::new("saves-partial");
        let home = scratch.join("home");
        std::fs::create_dir_all(&home).unwrap();
        conf(
            &scratch,
            "executable=x://1\nsave=Good|{home}/Foo/Saves\nsave=Bad|{nonsense}/Bar\n",
        );
        scratch.write(".gamepak/saves/good/slot1.sav", b"arrives anyway");

        with_home(&home, || {
            let results = attach_all(scratch.path());
            assert_eq!(results.len(), 2);
            assert!(home.join("Foo/Saves/slot1.sav").is_file());
            // The unusable one reports itself rather than failing the call.
            assert_eq!(results[1].as_ref().unwrap().direction, Direction::Unusable);
        });
    }

    #[cfg(unix)]
    #[test]
    fn a_link_does_not_report_the_cartridges_own_times_as_the_hosts() {
        // tree_summary must not follow a link, or a linked slot compares the
        // cartridge against itself and every answer after that is noise.
        let scratch = Scratch::new("saves-link-times");
        scratch.write("cart/a.sav", b"x");
        std::os::unix::fs::symlink(scratch.join("cart"), scratch.join("link")).unwrap();
        assert_eq!(tree_summary(&scratch.join("link")), (0, 0));
        assert!(tree_summary(&scratch.join("cart")).1 > 0);
    }
}
