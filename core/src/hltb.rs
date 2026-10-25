//! How long a game takes to beat, from HowLongToBeat, written onto the
//! cartridge when it is made.
//!
//! **Off by default**, and the second thing in the project that talks to the
//! network, after SteamGridDB. It asks once per game while a cartridge is
//! being written and never again: the answer goes into `cartridge.conf` beside
//! the game, so the launcher and any skin can read it offline, forever.
//!
//! ```ini
//! hltb_id=10270
//! hltb_main=97200
//! hltb_extra=154800
//! hltb_complete=226800
//! ```
//!
//! Seconds, like `playtime`, so a skin can divide one by the other.
//!
//! # There is no API
//!
//! HowLongToBeat publishes none. What this does is what its own search page
//! does, as worked out by the `howlongtobeatpy` project (1.0.23, which this
//! follows): find the search endpoint in the site's JavaScript, ask it for a
//! short-lived token, and post the search with that token attached. The site
//! changes this every so often, and when it does this stops finding anything
//! until it is updated. A failed lookup is a cartridge without an estimate —
//! a warning, never a failed write.

use std::time::Duration;

use serde_json::{json, Value};

const BASE: &str = "https://howlongtobeat.com/";

/// Where the search lives when the site's script cannot be read for it.
const FALLBACK_SEARCH_PATH: &str = "api/s";

/// The site refuses requests that do not look like a browser.
const BROWSER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/128.0 Safari/537.36";

/// Below this, a result is a different game with a similar name.
const MIN_SIMILARITY: f64 = 0.6;

/// What HowLongToBeat says about one game. Every time is in seconds, and zero
/// means it had no figure.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub id: u64,
    pub name: String,
    pub main: u64,
    pub extra: u64,
    pub complete: u64,
}

impl Estimate {
    /// The `hltb_*` lines for a conf section.
    pub fn conf_lines(&self) -> Vec<String> {
        let mut lines = vec![format!("hltb_id={}", self.id)];
        for (key, seconds) in [
            ("hltb_main", self.main),
            ("hltb_extra", self.extra),
            ("hltb_complete", self.complete),
        ] {
            if seconds > 0 {
                lines.push(format!("{key}={seconds}"));
            }
        }
        lines
    }
}

/// Look one title up, if the user has switched the lookup on.
///
/// `Ok(None)` is "switched off" or "no good match", which are both ordinary.
pub fn lookup(title: &str) -> Result<Option<Estimate>, String> {
    lookup_if(crate::settings::load().hltb_enabled, title)
}

fn lookup_if(enabled: bool, title: &str) -> Result<Option<Estimate>, String> {
    if !enabled {
        return Ok(None);
    }
    let title = title.trim();
    if title.is_empty() {
        return Ok(None);
    }

    let agent = ureq::AgentBuilder::new()
        .user_agent(BROWSER_AGENT)
        .timeout(Duration::from_secs(20))
        .build();

    let path = find_search_path(&agent).unwrap_or_else(|| FALLBACK_SEARCH_PATH.to_string());
    let auth = init(&agent, &path)?;
    let body = search_payload(title, auth.as_ref());

    let mut request = agent
        .post(&format!("{BASE}{path}"))
        .set("Content-Type", "application/json")
        .set("Accept", "*/*")
        .set("Referer", BASE)
        .set("Origin", BASE.trim_end_matches('/'));
    if let Some(auth) = &auth {
        request = request
            .set("x-auth-token", &auth.token)
            .set("x-hp-key", &auth.key)
            .set("x-hp-val", &auth.value);
    }
    let response = request
        .send_string(&body.to_string())
        .map_err(|e| format!("HowLongToBeat search failed: {e}"))?;
    let json: Value = response
        .into_json()
        .map_err(|e| format!("HowLongToBeat answered with something unreadable: {e}"))?;
    Ok(best_match(&json, title))
}

/// The search endpoint, as the site's own script names it.
fn find_search_path(agent: &ureq::Agent) -> Option<String> {
    let html = agent.get(BASE).call().ok()?.into_string().ok()?;
    for src in script_sources(&html) {
        let url = if src.starts_with("http") {
            src
        } else {
            format!("{BASE}{}", src.trim_start_matches('/'))
        };
        let Some(script) = agent
            .get(&url)
            .call()
            .ok()
            .and_then(|r| r.into_string().ok())
        else {
            continue;
        };
        if let Some(path) = search_path_in_script(&script) {
            return Some(path);
        }
    }
    None
}

/// The token the search wants, from `<search path>/init`.
fn init(agent: &ureq::Agent, path: &str) -> Result<Option<Auth>, String> {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let url = format!("{BASE}{path}/init?t={millis}");
    match agent.get(&url).set("Referer", BASE).call() {
        Ok(response) => {
            let json: Value = response
                .into_json()
                .map_err(|e| format!("HowLongToBeat's token was unreadable: {e}"))?;
            Ok(parse_auth(&json))
        }
        // An older site with no token step still answers the search.
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(format!("HowLongToBeat did not hand out a token: {e}")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Auth {
    token: String,
    key: String,
    value: String,
}

/// `{ "token": …, "hpKey": …, "hpVal": … }`, whatever the last two are called
/// this month: the one with "key" in its name, and the one with "val".
fn parse_auth(json: &Value) -> Option<Auth> {
    let object = json.as_object()?;
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let token = object.get("token").map(text).unwrap_or_default();
    let mut key = String::new();
    let mut value = String::new();
    for (name, v) in object {
        let lower = name.to_lowercase();
        if lower.contains("key") {
            key = text(v);
        } else if lower.contains("val") {
            value = text(v);
        }
    }
    (!token.is_empty()).then_some(Auth { token, key, value })
}

/// The `src` of every Next.js chunk script on the page.
fn script_sources(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        rest = &rest[start + 7..];
        let tag_end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..tag_end];
        if let Some(src) = attribute(tag, "src") {
            if src.contains("/_next/static/chunks/") {
                out.push(src);
            }
        }
        rest = &rest[tag_end..];
    }
    out
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}="))?;
    let rest = &tag[at + name.len() + 1..];
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let rest = &rest[1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// Find `fetch("/api/<path>…", { … method: "POST" … })` in a script, and
/// return `api/<path>`.
///
/// The POST is what marks the search: the same script fetches other `/api/`
/// routes with GET.
fn search_path_in_script(script: &str) -> Option<String> {
    let mut rest = script;
    while let Some(at) = rest.find("fetch(") {
        rest = &rest[at + 6..];
        let call = rest.trim_start();
        let Some(quote) = call
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\'' | '`'))
        else {
            continue;
        };
        let body = &call[1..];
        let Some(end) = body.find(quote) else {
            continue;
        };
        let url = &body[..end];
        let Some(path) = url.strip_prefix("/api/") else {
            continue;
        };
        // The options object: up to its first closing brace.
        let after = &body[end + 1..];
        let options = after.split('}').next().unwrap_or("");
        let compact: String = options.chars().filter(|c| !c.is_whitespace()).collect();
        let is_post = compact.contains("method:\"POST\"") || compact.contains("method:'POST'");
        let path: String = path
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/'))
            .collect();
        let path = path.trim_end_matches('/');
        if is_post && !path.is_empty() {
            return Some(format!("api/{path}"));
        }
    }
    None
}

/// The search request, in the shape the site's own page sends.
fn search_payload(title: &str, auth: Option<&Auth>) -> Value {
    let mut body = json!({
        "searchType": "games",
        "searchTerms": title.split_whitespace().collect::<Vec<_>>(),
        "searchPage": 1,
        "size": 20,
        "searchOptions": {
            "games": {
                "userId": 0,
                "platform": "",
                "sortCategory": "popular",
                "rangeCategory": "main",
                "rangeTime": { "min": 0, "max": 0 },
                "gameplay": { "perspective": "", "flow": "", "genre": "", "difficulty": "" },
                "rangeYear": { "max": "", "min": "" },
                "modifier": "hide_dlc"
            },
            "users": { "sortCategory": "postcount" },
            "lists": { "sortCategory": "follows" },
            "filter": "",
            "sort": 0,
            "randomizer": 0
        },
        "useCache": true
    });
    if let (Some(auth), Some(object)) = (auth, body.as_object_mut()) {
        if !auth.key.is_empty() {
            object.insert(auth.key.clone(), Value::String(auth.value.clone()));
        }
    }
    body
}

/// The result that is this game, or `None` when nothing is close enough.
///
/// The site sorts by popularity, so among equally good names the first wins.
fn best_match(json: &Value, title: &str) -> Option<Estimate> {
    let mut best: Option<(f64, Estimate)> = None;
    for entry in json.get("data")?.as_array()? {
        let name = entry.get("game_name").and_then(Value::as_str).unwrap_or("");
        let alias = entry
            .get("game_alias")
            .and_then(Value::as_str)
            .unwrap_or("");
        let score = similarity(title, name).max(similarity(title, alias));
        if score < MIN_SIMILARITY || best.as_ref().is_some_and(|(b, _)| score <= *b) {
            continue;
        }
        let seconds = |key: &str| entry.get(key).and_then(Value::as_u64).unwrap_or(0);
        best = Some((
            score,
            Estimate {
                id: seconds("game_id"),
                name: name.to_string(),
                main: seconds("comp_main"),
                extra: seconds("comp_plus"),
                complete: seconds("comp_100"),
            },
        ));
    }
    best.map(|(_, estimate)| estimate)
}

/// How alike two titles are, from 0 to 1.
///
/// Case, punctuation and trademark symbols do not count. A number in the title
/// that the other does not share costs a tenth, as in `howlongtobeatpy`, so
/// "Dark Souls 3" does not settle for "Dark Souls".
fn similarity(wanted: &str, found: &str) -> f64 {
    let a = normalise(wanted);
    let b = normalise(found);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let longest = a.chars().count().max(b.chars().count()) as f64;
    let mut score = 1.0 - levenshtein(&a, &b) as f64 / longest;
    let numbers = |s: &str| -> Vec<String> {
        s.split(' ')
            .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
            .map(str::to_string)
            .collect()
    };
    let theirs = numbers(&b);
    if numbers(&a).iter().any(|n| !theirs.contains(n)) {
        score -= 0.1;
    }
    score
}

fn normalise(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_is_the_fetch_that_posts() {
        let script = r#"a=fetch("/api/user/1",{method:"GET"});b=fetch("/api/finder/v2".concat(x),{method:"POST",headers:{}});"#;
        assert_eq!(
            search_path_in_script(script).as_deref(),
            Some("api/finder/v2")
        );
        let spaced = "fetch( '/api/s/' , { method: 'POST' })";
        assert_eq!(search_path_in_script(spaced).as_deref(), Some("api/s"));
        assert_eq!(
            search_path_in_script(r#"fetch("/api/x",{method:"GET"})"#),
            None
        );
    }

    #[test]
    fn only_next_chunks_are_read_for_the_endpoint() {
        let html = r#"<script src="/_next/static/chunks/pages/_app-1a2b.js" defer></script><script src="https://cdn.example/other.js"></script><script>inline()</script>"#;
        assert_eq!(
            script_sources(html),
            vec!["/_next/static/chunks/pages/_app-1a2b.js"]
        );
    }

    #[test]
    fn the_token_comes_with_a_named_pair() {
        let json = json!({ "token": "t0k", "hpKey": "k9", "hpVal": "v7" });
        assert_eq!(
            parse_auth(&json),
            Some(Auth {
                token: "t0k".into(),
                key: "k9".into(),
                value: "v7".into()
            })
        );
        assert_eq!(parse_auth(&json!({})), None);
    }

    #[test]
    fn the_pair_is_added_to_the_search() {
        let auth = Auth {
            token: "t".into(),
            key: "k9".into(),
            value: "v7".into(),
        };
        let body = search_payload("Hollow Knight", Some(&auth));
        assert_eq!(body["k9"], "v7");
        assert_eq!(body["searchTerms"], json!(["Hollow", "Knight"]));
    }

    fn results() -> Value {
        json!({ "data": [
            { "game_id": 1, "game_name": "Dark Souls", "game_alias": "",
              "comp_main": 150_000, "comp_plus": 220_000, "comp_100": 360_000 },
            { "game_id": 3, "game_name": "Dark Souls III", "game_alias": "Dark Souls 3",
              "comp_main": 115_000, "comp_plus": 170_000, "comp_100": 350_000 },
            { "game_id": 9, "game_name": "Hollow Knight", "game_alias": "",
              "comp_main": 97_200, "comp_plus": 154_800, "comp_100": 226_800 }
        ]})
    }

    #[test]
    fn the_right_game_is_picked_from_similar_names() {
        let hk = best_match(&results(), "Hollow Knight").unwrap();
        assert_eq!(hk.id, 9);
        assert_eq!(hk.main, 97_200);
        let ds3 = best_match(&results(), "Dark Souls 3").unwrap();
        assert_eq!(
            ds3.id, 3,
            "the alias matches, and the number rules out the first game"
        );
        assert_eq!(best_match(&results(), "Stardew Valley"), None);
    }

    #[test]
    fn trademark_symbols_and_case_do_not_count() {
        assert!(similarity("DARK SOULS™ III", "Dark Souls III") > 0.99);
    }

    #[test]
    fn a_missing_figure_is_left_out_of_the_conf() {
        let e = Estimate {
            id: 4,
            name: "X".into(),
            main: 3600,
            extra: 0,
            complete: 7200,
        };
        assert_eq!(
            e.conf_lines(),
            vec!["hltb_id=4", "hltb_main=3600", "hltb_complete=7200"]
        );
    }

    #[test]
    fn switched_off_asks_nobody() {
        assert_eq!(lookup_if(false, "Hollow Knight"), Ok(None));
        assert!(!crate::settings::Settings::default().hltb_enabled);
    }
}
