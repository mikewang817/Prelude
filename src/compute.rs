//! Rows computed from what you typed, rather than searched for.

use crate::exec::{run, shq, which};
use crate::item::{Item, Kind};
use crate::paths;
use std::time::Duration;

// ─── unit / currency conversion ──────────────────────────────────────────
// macOS ships an old `units` that doesn't know GB, kph, or any currency, so
// common shorthands are mapped to names it does understand.

const ALIASES: &[(&str, &str)] = &[
    ("gb", "gigabyte"), ("mb", "megabyte"), ("kb", "kilobyte"),
    ("tb", "terabyte"), ("b", "byte"), ("gib", "gibibyte"),
    ("mib", "mebibyte"), ("kib", "kibibyte"),
    ("kph", "km/hour"), ("km/h", "km/hour"), ("mph", "mile/hour"),
    // `units` is case-sensitive on these, but we lowercase the input first.
    ("c", "degC"), ("f", "degF"), ("degc", "degC"), ("degf", "degF"),
    ("k", "K"), ("degk", "K"), ("celsius", "degC"), ("fahrenheit", "degF"),
    ("min", "minute"), ("mins", "minute"), ("hr", "hour"), ("hrs", "hour"),
    ("sec", "second"), ("secs", "second"), ("yr", "year"),
    ("lbs", "pound"), ("lb", "pound"), ("oz", "ounce"),
    ("mi", "mile"), ("ft", "foot"), ("in", "inch"), ("yd", "yard"),
];

const CURRENCIES: &[&str] = &[
    "usd", "cny", "rmb", "eur", "jpy", "gbp", "hkd", "twd", "krw", "aud",
    "cad", "chf", "sgd", "rub", "inr", "brl", "nzd", "sek",
];

fn norm_unit(u: &str) -> String {
    let l = u.trim().to_ascii_lowercase();
    ALIASES
        .iter()
        .find(|(k, _)| *k == l)
        .map(|(_, v)| v.to_string())
        .unwrap_or(l)
}

/// `10kg to lb`, `1gb to mb`, `100 usd to cny`.
pub fn convert(query: &str) -> Option<(String, String)> {
    let (amount, src_raw, dst_raw) = parse_conversion(query)?;
    let (sl, dl) = (src_raw.to_ascii_lowercase(), dst_raw.to_ascii_lowercase());

    if CURRENCIES.contains(&sl.as_str()) && CURRENCIES.contains(&dl.as_str()) {
        let a = if sl == "rmb" { "cny" } else { &sl };
        let b = if dl == "rmb" { "cny" } else { &dl };
        let Some(rates) = fetch_rates() else {
            return Some((
                "exchange rates unavailable (offline?)".into(),
                format!("{amount} {} → {}", sl.to_uppercase(), dl.to_uppercase()),
            ));
        };
        let (ra, rb) = (rates.get(&a.to_uppercase())?, rates.get(&b.to_uppercase())?);
        let val = amount * rb / ra;
        return Some((
            format!("{} {}", crate::calc::fmt_num((val * 100.0).round() / 100.0), dl.to_uppercase()),
            format!("{amount} {} · rate {:.4}", sl.to_uppercase(), rb / ra),
        ));
    }

    which("units")?;
    let (src, dst) = (norm_unit(&sl), norm_unit(&dl));
    let val = if amount > 0.0 {
        units_value(amount, &src, &dst)
    } else {
        None
    };
    let val = match val {
        Some(v) => v,
        None => {
            // `units` refuses zero ("unit reduces to zero") and negatives, and
            // for affine scales the answer isn't simply signed — 0°C is 32°F,
            // −40°C is −40°F. Fit the line through two points it will accept.
            let one = units_value(1.0, &src, &dst)?;
            let two = units_value(2.0, &src, &dst)?;
            let slope = two - one;
            slope * amount + (one - slope)
        }
    };
    Some((
        format!("{} {}", crate::calc::fmt_num(val), dst_raw),
        format!("{amount} {src_raw} → {dst_raw}"),
    ))
}

fn parse_conversion(q: &str) -> Option<(f64, String, String)> {
    let low = q.trim();
    let (lhs, rhs) = ["  to  ", " to ", " in ", " as ", " -> ", " → ", " = "]
        .iter()
        .find_map(|sep| low.split_once(sep))?;
    let rhs = rhs.trim();
    if rhs.is_empty() || !rhs.chars().all(|c| c.is_alphabetic() || "°/".contains(c)) {
        return None;
    }
    let lhs = lhs.trim();
    let split = lhs.find(|c: char| c.is_alphabetic() || c == '°')?;
    let (num, unit) = lhs.split_at(split);
    let num = num.trim().replace(',', "");
    let unit = unit.trim();
    if unit.is_empty() || !unit.chars().all(|c| c.is_alphabetic() || "°/".contains(c)) {
        return None;
    }
    Some((num.parse().ok()?, unit.to_string(), rhs.to_string()))
}

fn units_value(amount: f64, src: &str, dst: &str) -> Option<f64> {
    let from = format!("{amount} {src}");
    let out = run(&["units", "-t", &from, dst], Duration::from_secs(3));
    let s = out.trim();
    if s.is_empty() || !s.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        return None;
    }
    s.parse().ok()
}

type Rates = std::collections::BTreeMap<String, f64>;

/// Exchange rates, cached for a day. Currency conversion is the one feature
/// here that needs the network; everything else is offline.
///
/// Fetched with `curl` rather than an HTTP crate — a TLS stack would dwarf
/// the entire binary for one request made at most once a day.
fn fetch_rates() -> Option<Rates> {
    let path = paths::cache().join("rates.json");
    let fresh = path
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 86_400);
    if fresh {
        if let Ok(t) = std::fs::read_to_string(&path) {
            if let Ok(r) = serde_json::from_str::<Rates>(&t) {
                return Some(r);
            }
        }
    }
    for url in [
        "https://api.frankfurter.dev/v1/latest?base=EUR",
        "https://api.frankfurter.app/latest?from=EUR",
        "https://open.er-api.com/v6/latest/EUR",
    ] {
        let body = run(&["curl", "-sS", "--max-time", "6", url], Duration::from_secs(8));
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else { continue };
        let Some(obj) = v.get("rates").and_then(|r| r.as_object()) else { continue };
        let mut rates: Rates = obj
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
            .collect();
        if rates.is_empty() {
            continue;
        }
        rates.insert("EUR".into(), 1.0);
        if let Ok(j) = serde_json::to_vec(&rates) {
            let _ = crate::cache::write_atomic(&path, &j);
        }
        return Some(rates);
    }
    None
}

// ─── translation ─────────────────────────────────────────────────────────

pub fn translate_app() -> std::path::PathBuf {
    paths::data().join("PreludeTranslate.app/Contents/MacOS/PreludeTranslate")
}

fn lang_alias(l: &str) -> String {
    match l.to_ascii_lowercase().as_str() {
        "zh" | "cn" | "chinese" => "zh-Hans".into(),
        "zht" | "tw" => "zh-Hant".into(),
        "english" => "en".into(),
        "jp" => "ja".into(),
        "kr" => "ko".into(),
        other => other.into(),
    }
}

/// Pin the source language by script.
///
/// Apple's auto-detect hangs indefinitely on very short input — "hello" alone
/// never returns — so guessing beats asking.
fn guess_source(text: &str, target: &str) -> String {
    let has = |lo: char, hi: char| text.chars().any(|c| c >= lo && c <= hi);
    if has('\u{3040}', '\u{30ff}') {
        return "ja".into();
    }
    if has('\u{ac00}', '\u{d7af}') {
        return "ko".into();
    }
    if has('\u{3400}', '\u{9fff}') || has('\u{f900}', '\u{faff}') {
        return if target.starts_with("zh-Hant") { "zh-Hant".into() } else { "zh-Hans".into() };
    }
    if !target.starts_with("en") {
        return "en".into();
    }
    // Auto-detect is right here but only survives on enough text.
    if text.chars().count() >= 25 { "auto".into() } else { "en".into() }
}

pub fn translate(text: &str, target: &str) -> Result<String, String> {
    let target = lang_alias(target);
    let app = translate_app();
    if !app.exists() {
        return Err("not built — run:  prelude build-translate".into());
    }
    let src = guess_source(text, &target);
    if src != "auto" && src.split('-').next() == target.split('-').next() {
        return Err(format!("already {target}"));
    }
    let key = format!("{:016x}", fxhash(&format!("{target}\u{0}{text}")));
    let cached = paths::cache().join("translate").join(&key);
    if let Ok(v) = std::fs::read_to_string(&cached) {
        return Ok(v);
    }
    let out = run(&[&app.to_string_lossy(), &target, &src, text], Duration::from_secs(25));
    let out = out.trim().to_string();
    if out.is_empty() {
        // rc 1 = the model refused, almost always an undownloaded language.
        return Err(format!(
            "{target} not available — download it in System Settings › General › Language & Region"
        ));
    }
    let _ = crate::cache::write_atomic(&cached, out.as_bytes());
    Ok(out)
}

fn fxhash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

/// `en: 你好` or `zh: hello` -> (target, text).
pub fn translate_query(q: &str) -> Option<(String, String)> {
    let (lang, text) = q.trim().split_once(':')?;
    let lang = lang.trim();
    let text = text.trim();
    // Language codes are at least two letters. Without this, the `s:` and
    // `f:` prefixes get swallowed as if `s` and `f` were languages.
    if text.is_empty() || lang.len() < 2 || lang.len() > 7 {
        return None;
    }
    if !lang.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
        return None;
    }
    Some((lang.to_string(), text.to_string()))
}

// ─── web addresses and quicklinks ────────────────────────────────────────

/// Turn what looks unambiguously like a web address into one macOS can open.
///
/// No URL crate: this runs on every keystroke, and a new dependency would be
/// paid on every Prelude startup. This is deliberately narrower than a full
/// RFC parser — web pages only, no `file:`, `data:` or `javascript:` schemes,
/// no credentials, and no guessing when a local filename is more plausible.
pub fn web_url(query: &str) -> Option<String> {
    let t = query.trim();
    if t.is_empty()
        || t.len() > 4096
        || t.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }

    let explicit = ["https://", "http://"]
        .iter()
        .find(|scheme| t.get(..scheme.len()).is_some_and(|p| p.eq_ignore_ascii_case(scheme)));
    let (scheme, rest) = match explicit {
        Some(scheme) => (*scheme, &t[scheme.len()..]),
        None => ("", t),
    };

    if scheme.is_empty() {
        if t.starts_with(['/', '~']) || t.starts_with("./") || t.starts_with("../") {
            return None;
        }
        // `Cargo.toml` and `notes.md` are objects before they are speculative
        // domains. The extension guard covers not-yet-created files too;
        // an explicit scheme always wins when someone really owns `foo.app`.
        if std::path::Path::new(t).exists() || looks_like_filename(t) {
            return None;
        }
    }

    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = split_host_port(authority)?;
    let local = host.eq_ignore_ascii_case("localhost")
        || host.parse::<std::net::IpAddr>().is_ok()
        || host.to_ascii_lowercase().ends_with(".local")
        || host.to_ascii_lowercase().ends_with(".test");
    if !valid_web_host(host) {
        return None;
    }

    if !scheme.is_empty() {
        return Some(format!("{scheme}{rest}"));
    }
    let scheme = if local || port.is_some_and(|p| p != 443) { "http://" } else { "https://" };
    Some(format!("{scheme}{t}"))
}

fn split_host_port(authority: &str) -> Option<(&str, Option<u16>)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest.find(']')?;
        let host = &rest[..end];
        host.parse::<std::net::Ipv6Addr>().ok()?;
        let tail = &rest[end + 1..];
        let port = if tail.is_empty() {
            None
        } else {
            Some(tail.strip_prefix(':')?.parse::<u16>().ok().filter(|p| *p > 0)?)
        };
        return Some((host, port));
    }
    if authority.matches(':').count() > 1 {
        return None;
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host, Some(port.parse::<u16>().ok().filter(|p| *p > 0)?))),
        None => Some((authority, None)),
    }
}

fn valid_web_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    if !host.is_ascii() || host.len() > 253 || !host.contains('.') {
        return false;
    }
    let labels = host.trim_end_matches('.').split('.');
    let mut last = "";
    for label in labels {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return false;
        }
        last = label;
    }
    (last.len() >= 2 && last.bytes().all(|b| b.is_ascii_alphabetic()))
        || (last.len() > 4 && last.to_ascii_lowercase().starts_with("xn--"))
}

fn looks_like_filename(t: &str) -> bool {
    if t.contains(['/', '?', '#', ':']) || t.to_ascii_lowercase().starts_with("www.") {
        return false;
    }
    let ext = t.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    matches!(
        ext.as_str(),
        "app" | "c" | "cfg" | "conf" | "cpp" | "css" | "csv" | "doc" | "docx"
            | "dmg" | "gif" | "go" | "gz" | "h" | "hpp" | "html" | "ini" | "java"
            | "jpeg" | "jpg" | "js" | "json" | "jsx" | "kt" | "lock" | "log" | "md"
            | "pdf" | "pkg" | "png" | "ppt" | "pptx" | "py" | "rb" | "rs" | "scss"
            | "sh" | "svg" | "swift" | "tar" | "toml" | "ts" | "tsx" | "txt" | "webp"
            | "xls" | "xlsx" | "xml" | "yaml" | "yml" | "zip" | "zsh"
    )
}

pub const QUICKLINKS_DEFAULT: &str = r#"# Prelude quicklinks
# Type the keyword followed by your search terms, e.g.  g rust async.
# {q} is replaced with what you typed (URL-encoded).
# Fixed file, folder, URL and application entries can be created from ^K.
# prelude:defaults web-search-v2

[g]
name = "Google"
url  = "https://www.google.com/search?q={q}"

[gh]
name = "GitHub"
url  = "https://github.com/search?q={q}"

[npm]
name = "npm"
url  = "https://www.npmjs.com/search?q={q}"

[mdn]
name = "MDN"
url  = "https://developer.mozilla.org/en-US/search?q={q}"

[gs]
name = "Google Scholar"
url  = "https://scholar.google.com/scholar?q={q}"

[b]
name = "Baidu"
url  = "https://www.baidu.com/s?wd={q}"

[bing]
name = "Bing"
url  = "https://www.bing.com/search?q={q}"

[ddg]
name = "DuckDuckGo"
url  = "https://duckduckgo.com/?q={q}"
"#;

const WEB_SEARCH_V2_MARKER: &str = "# prelude:defaults web-search-v2";
const WEB_SEARCH_V2: [(&str, &str, &str); 3] = [
    ("b", "Baidu", "https://www.baidu.com/s?wd={q}"),
    ("bing", "Bing", "https://www.bing.com/search?q={q}"),
    ("ddg", "DuckDuckGo", "https://duckduckgo.com/?q={q}"),
];

pub fn quicklinks_file() -> std::path::PathBuf {
    paths::config().join("quicklinks.toml")
}

pub(crate) fn add_web_search_defaults(mut text: String) -> (String, bool) {
    if text.lines().any(|line| line == WEB_SEARCH_V2_MARKER) {
        return (text, false);
    }
    let existing = crate::minitoml::parse(&text);
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push('\n');
    text.push_str(WEB_SEARCH_V2_MARKER);
    text.push('\n');
    for (key, name, url) in WEB_SEARCH_V2 {
        if existing.contains_key(key) {
            continue;
        }
        text.push_str(&format!(
            "\n[{key}]\nname = {}\nurl = {}\n",
            toml_string(name),
            toml_string(url),
        ));
    }
    (text, true)
}

pub fn ensure_quicklinks_file() -> Result<std::path::PathBuf, String> {
    let path = quicklinks_file();
    if !path.exists() {
        crate::cache::write_atomic(&path, QUICKLINKS_DEFAULT.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn quicklinks_text() -> String {
    let p = ensure_quicklinks_file().unwrap_or_else(|_| quicklinks_file());
    let text = std::fs::read_to_string(&p).unwrap_or_default();
    let (text, changed) = add_web_search_defaults(text);
    if changed {
        let _ = crate::cache::write_atomic(&p, text.as_bytes());
    }
    text
}

pub fn quicklinks() -> crate::minitoml::Table {
    crate::minitoml::parse(&quicklinks_text())
}

#[derive(Clone, Debug)]
pub struct QuicklinkDraft {
    pub name: String,
    pub kind: &'static str,
    pub target: String,
}

pub fn quicklinkable(kind: Kind) -> bool {
    matches!(kind, Kind::File | Kind::Find | Kind::Config | Kind::Dir | Kind::Link | Kind::App)
}

/// The stable identity behind a selected object. Local targets are resolved
/// before storage so a quicklink works from every directory and `..` cannot
/// make it silently point somewhere else later.
pub fn quicklink_draft(it: &Item) -> Result<Option<QuicklinkDraft>, String> {
    if !quicklinkable(it.kind) {
        return Ok(None);
    }
    let (kind, raw) = match it.kind {
        Kind::File | Kind::Find => ("file", it.get("path")),
        Kind::Config => ("config", it.get("path")),
        Kind::Dir => ("folder", it.get("path")),
        Kind::Link => ("url", it.get("url")),
        Kind::App => ("app", it.get("path")),
        _ => unreachable!(),
    };
    if raw.is_empty() {
        return Err("that row has no stable target".into());
    }
    let target = if kind == "url" {
        if crate::secrets::looks_secret(raw) || url_has_secret(raw) {
            return Err("that URL appears to contain a credential and will not be indexed".into());
        }
        web_url(raw).ok_or_else(|| "that is not a safe HTTP or HTTPS URL".to_string())?
    } else {
        let real = std::path::Path::new(raw)
            .canonicalize()
            .map_err(|_| "that target is not there any more".to_string())?;
        crate::paths::tilde(&real.to_string_lossy())
    };
    let name = if matches!(it.kind, Kind::Dir) {
        raw.trim_end_matches('/').rsplit('/').next().unwrap_or(&it.title).to_string()
    } else {
        crate::width::flatten(&it.title)
    };
    Ok(Some(QuicklinkDraft { name, kind, target }))
}

fn url_has_secret(url: &str) -> bool {
    let Some(query) = url.split_once('?').map(|(_, q)| q.split('#').next().unwrap_or(q)) else {
        return false;
    };
    query.split('&').filter_map(|part| part.split_once('=').map(|(k, _)| k))
        .any(|key| matches!(
            key.to_ascii_lowercase().as_str(),
            "api_key" | "apikey" | "key" | "token" | "access_token" | "auth"
                | "authorization" | "password" | "secret" | "signature" | "sig"
                | "x-amz-signature" | "code"
        ))
}

pub fn quicklink_suggestion(it: &Item) -> String {
    let base = quicklink_draft(it).ok().flatten().map(|d| d.name).unwrap_or_else(|| it.title.clone());
    let base = std::path::Path::new(&base)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(base);
    let mut out = String::new();
    let mut dash = false;
    for c in base.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
        if out.len() >= 32 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn valid_quicklink_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 40
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub fn normalize_quicklink_key(raw: &str) -> Result<String, String> {
    let key = raw.trim().to_ascii_lowercase();
    if valid_quicklink_key(&key) {
        Ok(key)
    } else {
        Err("use 1–40 letters, numbers, hyphens or underscores".into())
    }
}

fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn quicklink_marker(key: &str, end: bool) -> String {
    format!("# prelude:quicklink {key} {}", if end { "end" } else { "begin" })
}

pub(crate) fn append_quicklink(
    mut text: String,
    key: &str,
    draft: &QuicklinkDraft,
) -> Result<String, String> {
    if !valid_quicklink_key(key) {
        return Err("use 1–40 letters, numbers, hyphens or underscores".into());
    }
    if crate::minitoml::parse(&text).contains_key(key) {
        return Err(format!("a quicklink called {key} already exists"));
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "\n{}\n[{key}]\nname = {}\nkind = {}\ntarget = {}\n{}\n",
        quicklink_marker(key, false),
        toml_string(&draft.name),
        toml_string(draft.kind),
        toml_string(&draft.target),
        quicklink_marker(key, true),
    ));
    Ok(text)
}

pub fn create_quicklink(key: &str, it: &Item) -> Result<QuicklinkDraft, String> {
    let key = normalize_quicklink_key(key)?;
    let draft = quicklink_draft(it)?.ok_or_else(|| "that kind cannot be a quicklink".to_string())?;
    let text = append_quicklink(quicklinks_text(), &key, &draft)?;
    crate::cache::write_atomic(&quicklinks_file(), text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(draft)
}

pub(crate) fn remove_quicklink_block(mut text: String, key: &str) -> Result<String, String> {
    if !valid_quicklink_key(key) {
        return Err("invalid quicklink name".into());
    }
    let begin = format!("{}\n", quicklink_marker(key, false));
    let end = format!("\n{}", quicklink_marker(key, true));
    let start = text.find(&begin).ok_or_else(|| "that quicklink is managed in the config file".to_string())?;
    let tail = text[start + begin.len()..]
        .find(&end)
        .ok_or_else(|| "quicklink block is incomplete".to_string())?;
    let mut finish = start + begin.len() + tail + end.len();
    if text[finish..].starts_with('\n') {
        finish += 1;
    }
    if start > 0 && text[..start].ends_with('\n') {
        text.replace_range(start - 1..finish, "");
    } else {
        text.replace_range(start..finish, "");
    }
    Ok(text)
}

pub fn remove_quicklink(key: &str) -> Result<(), String> {
    let key = normalize_quicklink_key(key)?;
    let text = remove_quicklink_block(quicklinks_text(), &key)?;
    crate::cache::write_atomic(&quicklinks_file(), text.as_bytes()).map_err(|e| e.to_string())
}

fn expand_quicklink_target(target: &str) -> String {
    target.strip_prefix("~/")
        .map(|rest| paths::home().join(rest).to_string_lossy().into_owned())
        .unwrap_or_else(|| target.to_string())
}

fn managed_quicklink(text: &str, key: &str) -> bool {
    let begin = quicklink_marker(key, false);
    let end = quicklink_marker(key, true);
    text.lines().any(|line| line == begin) && text.lines().any(|line| line == end)
}

/// Resolve an exact fixed quicklink back into the kind of object it targets.
pub(crate) fn fixed_quicklink_from(text: &str, q: &str) -> Option<Item> {
    let key = q.trim().to_ascii_lowercase();
    if !valid_quicklink_key(&key) {
        return None;
    }
    let links = crate::minitoml::parse(text);
    let body = links.get(&key)?;
    let raw = body.get("target").or_else(|| body.get("url"))?;
    if raw.contains("{q}") {
        return None;
    }
    let kind = body.get("kind").map(String::as_str)
        .unwrap_or_else(|| if body.contains_key("url") { "url" } else { "" });
    let name = body.get("name").cloned().unwrap_or_else(|| key.clone());
    let target = expand_quicklink_target(raw);
    let managed = managed_quicklink(text, &key).to_string();
    let item = match kind {
        "url" => {
            let url = web_url(&target)?;
            Item::new(url.clone(), Kind::Link).title(name).put("url", url)
        }
        "file" => Item::new(target.clone(), Kind::File).title(name).put("path", target.clone()),
        "config" => Item::new(target.clone(), Kind::Config).title(name).put("path", target.clone()),
        "folder" | "dir" => Item::new(format!("cd {}", shq(&target)), Kind::Dir)
            .title(name).put("path", target.clone()),
        "app" => Item::new(format!("open {}", shq(&target)), Kind::App)
            .title(name).put("path", target.clone()),
        _ => return None,
    };
    let missing = kind != "url" && !std::path::Path::new(&target).exists();
    Some(item
        .fields([key.clone(), if missing { "⚠ target missing".to_string() } else { crate::paths::tilde(&target) }])
        .put("quicklink", key)
        .put("quicklink_managed", managed)
        .put("quicklink_target", target))
}

/// Quicklinks with arguments are commands before they are links. Keeping
/// them in the searchable catalogue makes `g` and `google` reveal what is
/// available instead of looking like failed fuzzy searches.
pub(crate) fn quicklink_items_from(text: &str) -> Vec<Item> {
    let links = crate::minitoml::parse(text);
    let mut out = Vec::with_capacity(links.len());
    for (key, body) in links {
        if !valid_quicklink_key(&key) {
            continue;
        }
        let Some(target) = body.get("target").or_else(|| body.get("url")) else { continue };
        if target.contains("{q}") {
            let name = body.get("name").cloned().unwrap_or_else(|| key.clone());
            let title = if name.to_ascii_lowercase().starts_with("search ") {
                name
            } else {
                format!("Search {name}")
            };
            out.push(
                Item::new(key.clone(), Kind::Search)
                    .title(title)
                    .fields([format!("{key} <query>"), "web search".to_string()])
                    .put("mode", "complete-query")
                    .put("completion", format!("{key} "))
                    .put("provider", key)
                    .put("desc", "type a search term"),
            );
        } else if let Some(item) = fixed_quicklink_from(text, &key) {
            out.push(item);
        }
    }
    out
}

pub fn quicklink_items() -> Vec<Item> {
    quicklink_items_from(&quicklinks_text())
}

pub(crate) fn exact_quicklink_key_from(text: &str, q: &str) -> bool {
    let key = q.trim().to_ascii_lowercase();
    if !valid_quicklink_key(&key) {
        return false;
    }
    crate::minitoml::parse(text)
        .get(&key)
        .is_some_and(|body| body.contains_key("target") || body.contains_key("url"))
}

pub(crate) fn search_provider_from(text: &str, q: &str) -> Option<Item> {
    let want = q.trim().to_ascii_lowercase();
    quicklink_items_from(text).into_iter().find(|it| {
        if it.kind != Kind::Search || it.get("provider").is_empty() {
            return false;
        }
        let name = it.title.strip_prefix("Search ").unwrap_or(&it.title);
        it.get("provider") == want || name.to_ascii_lowercase() == want
    })
}

fn exact_quicklink_item(q: &str) -> Option<Item> {
    let text = quicklinks_text();
    search_provider_from(&text, q).or_else(|| {
        exact_quicklink_key_from(&text, q).then(|| fixed_quicklink_from(&text, q)).flatten()
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Agent,
    Running,
    Sessions,
    Files,
    Clipboard,
    History,
    Applications,
    Commands,
    Directories,
    Project,
    Ssh,
    Snippets,
    Ports,
    Processes,
    Containers,
    Skills,
    Mcp,
    Config,
    Settings,
}

struct ScopeDef {
    scope: Scope,
    prefix: &'static str,
    title: &'static str,
    desc: &'static str,
}

const SCOPES: &[ScopeDef] = &[
    ScopeDef { scope: Scope::Agent, prefix: "a:", title: "Agent Control Center", desc: "agents, runs, skills, MCP and config" },
    ScopeDef { scope: Scope::Running, prefix: "r:", title: "Running Agents", desc: "working and waiting now" },
    ScopeDef { scope: Scope::Sessions, prefix: "s:", title: "Past Conversations", desc: "all agent sessions" },
    ScopeDef { scope: Scope::Skills, prefix: "skill:", title: "Skills", desc: "all installed agent capabilities" },
    ScopeDef { scope: Scope::Files, prefix: "f:", title: "Search Files", desc: "project and indexed roots" },
    ScopeDef { scope: Scope::Clipboard, prefix: "c:", title: "Clipboard History", desc: "recent copied text" },
    ScopeDef { scope: Scope::History, prefix: "h:", title: "Shell History", desc: "recent commands" },
    ScopeDef { scope: Scope::Applications, prefix: "app:", title: "Applications", desc: "installed macOS apps" },
    ScopeDef { scope: Scope::Commands, prefix: "cmd:", title: "Commands", desc: "$PATH and system commands" },
    ScopeDef { scope: Scope::Directories, prefix: "dir:", title: "Folders", desc: "zoxide and recent cd targets" },
    ScopeDef { scope: Scope::Project, prefix: "proj:", title: "Current Project", desc: "scripts, files and git" },
    ScopeDef { scope: Scope::Ssh, prefix: "ssh:", title: "SSH Hosts", desc: "~/.ssh/config" },
    ScopeDef { scope: Scope::Snippets, prefix: "snip:", title: "Snippets", desc: "saved command templates" },
    ScopeDef { scope: Scope::Ports, prefix: "port:", title: "Listening Ports", desc: "local TCP listeners" },
    ScopeDef { scope: Scope::Processes, prefix: "proc:", title: "Processes", desc: "CPU and memory consumers" },
    ScopeDef { scope: Scope::Containers, prefix: "docker:", title: "Containers", desc: "running Docker containers" },
    ScopeDef { scope: Scope::Mcp, prefix: "mcp:", title: "MCP Servers", desc: "all agent integrations" },
    ScopeDef { scope: Scope::Config, prefix: "cfg:", title: "Agent Config", desc: "settings and instruction files" },
    // Prelude's own, as opposed to the four agents' above it. It was the only
    // thing in this list the launcher could not reach.
    ScopeDef { scope: Scope::Settings, prefix: "set:", title: "Prelude Settings", desc: "search roots, hotkey, keys and rules" },
];

fn scope_item(d: &ScopeDef) -> Item {
    Item::new(d.prefix, Kind::Search)
        .title(d.title)
        .fields([d.prefix.to_string(), d.desc.to_string()])
        .put("mode", "complete-query")
        .put("completion", d.prefix)
        .put("desc", d.desc)
}

pub fn scope_commands() -> Vec<Item> {
    SCOPES.iter().map(scope_item).collect()
}

fn exact_scope_command(q: &str) -> Option<Item> {
    let want = q.trim().to_ascii_lowercase();
    SCOPES.iter()
        .find(|d| d.prefix.trim_end_matches(':') == want)
        .map(scope_item)
}

pub fn scope_query(q: &str) -> Option<(Scope, &str)> {
    let t = q.trim();
    let (head, rest) = t.split_once(':')?;
    let head = head.to_ascii_lowercase();
    let def = SCOPES.iter().find(|d| d.prefix.trim_end_matches(':') == head)?;
    Some((def.scope, rest.trim()))
}

pub fn needs_static_items(q: &str) -> bool {
    let t = q.trim();
    scope_query(t).is_some()
        || t.starts_with('/')
        || (t.starts_with('@') && !t.chars().any(char::is_whitespace))
}

fn skill_prefix_rows(q: &str, static_items: &[Item]) -> Option<Vec<Item>> {
    let rest = q.trim().strip_prefix('/')?;
    let name = rest.split_whitespace().next().unwrap_or(rest);
    if static_items.iter().any(|it| it.kind == Kind::Skill && it.title.eq_ignore_ascii_case(name)) {
        return None;
    }
    let want = rest.to_lowercase();
    Some(static_items.iter()
        .filter(|it| it.kind == Kind::Skill && it.title.to_lowercase().contains(&want))
        .take(100)
        .cloned()
        .collect())
}

fn agent_prompt_rows(q: &str, static_items: &[Item]) -> Option<Vec<Item>> {
    let rest = q.trim().strip_prefix('@')?;
    if rest.chars().any(char::is_whitespace) {
        return None;
    }
    let want = rest.to_ascii_lowercase();
    Some(static_items.iter()
        .filter(|it| it.kind == Kind::Agent && it.title.to_ascii_lowercase().contains(&want))
        .map(|it| {
            let agent = it.get("agent");
            Item::new(format!("@{agent}"), Kind::Search)
                .title(format!("Ask {agent}"))
                .fields([format!("@{agent} <question>"), "answer inside Prelude".to_string()])
                .put("mode", "complete-query")
                .put("completion", format!("@{agent} "))
                .put("ask", agent)
                .put("desc", "type a question")
        })
        .collect())
}

// ─── the agent home ──────────────────────────────────────────────────────

/// The empty query: the things this launcher exists to manage.
///
/// Agents, what they are running, their Skills, their MCP servers and the
/// conversations you have had with them — the inventory, on one screen,
/// because looking at it *is* how you manage it.
///
/// This was briefly an attention list instead: healthy Skills and servers were
/// pushed into `/name` and `mcp:` so that only exceptions — a server that had
/// stopped answering, a skill whose copies had drifted — reached the home. It
/// reads well as a principle and was wrong in practice. A launcher you open to
/// see what you have is not improved by hiding what you have; the panel went
/// quiet exactly when nothing was broken, which is most of the time.
///
/// Ordering is `cache::by_rank` like everywhere else, so the bands do the work:
/// a question that is blocking somebody, then agents, what they are running,
/// Skills, MCP, and the recent conversations underneath. There is deliberately
/// no second ordering rule for this one screen — one of those was enough.
///
/// Sessions are the exception that has to be counted rather than filtered:
/// there are hundreds of them, `gather` puts only the newest
/// `sessions::IN_MAIN_LIST` in the list at all, and `s:` owns the rest.
pub fn home_items(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|it| {
            matches!(
                it.kind,
                Kind::Msg | Kind::Agent | Kind::Run | Kind::Skill | Kind::Mcp | Kind::Session
            )
        })
        .cloned()
        .collect()
}

/// What an ordinary root query may fuzzy-match. Large and private sources
/// are commands here, not thousands of eager rows: `f` finds Search Files,
/// and `f:` opens that scope. Fixed Quicklinks remain root commands even when
/// their target happens to be a file or application.
///
/// Sessions are the one kind held back: there are hundreds of them and `s:`
/// owns them, on the same rule the home follows.
pub fn root_items(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|it| {
            matches!(
                it.kind,
                Kind::Msg | Kind::Agent | Kind::Run | Kind::Skill | Kind::Mcp | Kind::Search
            ) || !it.get("quicklink").is_empty()
        })
        .cloned()
        .collect()
}

// ─── control queries inside `a:` ─────────────────────────────────────────

/// What an `a:` query filters by, as opposed to what it searches for.
///
/// The fields are *and*ed against each other. Within a field the answer
/// depends on what the field asks, and the two shapes here are not the same
/// question:
///
/// * `states`, `agents` and `projects` are *or*s, on `sessions::Filters`'
///   reasoning — one run cannot be in two projects, so anding two `project:`
///   words would always produce nothing.
/// * `using` is an *and*, because two capabilities are two independent facts
///   about one run: `using deploy using node_repl` asks for the run that
///   loaded both, and oring them would answer a question nobody typed.
/// * `without` is the negation of that or — a run is kept only when it loaded
///   *none* of the named capabilities.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentFilters {
    /// `waiting`, `working`, `dead` — a Run's own vocabulary, which is the
    /// only kind on this scope that has a state at all.
    pub states: Vec<String>,
    /// `agent:claude`. Exact, never widened.
    pub agents: Vec<String>,
    /// `project:Prelude`. The directory's own name or the whole path.
    pub projects: Vec<String>,
    /// `using <capability>` — a Skill or MCP server this run explicitly
    /// loaded. Confirmed capability, never installed inventory.
    pub using: Vec<String>,
    /// `without <capability>`.
    pub without: Vec<String>,
    /// Filter-shaped words that meant nothing: `agent:` with no name, a
    /// trailing `using` with nothing after it. Searched for literally, so the
    /// list visibly collapses — `sessions::Filters` records why at length:
    /// a filter that quietly matches everything looks exactly like one that
    /// worked.
    pub unknown: Vec<String>,
}

/// Split an `a:` query into what it filters by and what it searches for.
///
/// Quote-aware, through the same splitter `s:` uses, and for the same reason.
/// A Skill name really is an identifier — `/name` is how one is invoked — but
/// an MCP server's name is whatever its owner called it, and three of the ones
/// on this machine are `claude.ai Google Drive`-shaped. Split on whitespace,
/// `a:using claude.ai Google Drive` becomes `using=["claude.ai"]` plus two
/// stray needles and answers a different question with an empty list, which
/// is exactly the silent-filter failure the `unknown` field exists to avoid.
/// So `a:using "claude.ai Google Drive"` holds together, in both the
/// `using X` and the `using:X` form. A multi-word *project* still needs no
/// quotes: its words work as needles, which is the `a:claude Prelude` form
/// the plan writes.
///
/// Pure, and it must stay pure: `scoped_rows` runs from the per-keystroke
/// helper, where a subprocess, a directory read or a relationship join would
/// be paid on every letter typed.
pub fn parse_agent_filters(term: &str) -> (AgentFilters, Vec<String>) {
    let mut filters = AgentFilters::default();
    let mut needles: Vec<String> = Vec::new();
    let words: Vec<String> = crate::sources::sessions::split_words(term)
        .iter()
        .map(|word| word.to_lowercase())
        .collect();
    let mut i = 0;
    while i < words.len() {
        let word = words[i].clone();
        i += 1;
        // `using deploy` takes the next word, because that is how the plan
        // writes it. `using:deploy` is accepted below for symmetry with the
        // other keys; a bare `using` at the end of a query named nothing.
        if word == "using" || word == "without" {
            match words.get(i) {
                Some(capability) => {
                    if word == "using" {
                        filters.using.push(capability.clone());
                    } else {
                        filters.without.push(capability.clone());
                    }
                    i += 1;
                }
                None => filters.unknown.push(word),
            }
            continue;
        }
        let Some((key, value)) = word.split_once(':') else {
            // A state word is a state filter. The vocabulary is small, fixed
            // and owned by `running::State`, so `a:waiting` cannot drift from
            // what a Run can actually be.
            match is_run_state(&word) {
                true => filters.states.push(word),
                false => needles.push(word),
            }
            continue;
        };
        match key {
            "agent" if !value.is_empty() => filters.agents.push(value.to_string()),
            "project" if !value.is_empty() => filters.projects.push(value.to_string()),
            "using" if !value.is_empty() => filters.using.push(value.to_string()),
            "without" if !value.is_empty() => filters.without.push(value.to_string()),
            "state" if is_run_state(value) => filters.states.push(value.to_string()),
            "agent" | "project" | "using" | "without" | "state" => filters.unknown.push(word),
            // `a:` shares its box with paths and other prefixes, so anything
            // else containing a colon is an ordinary search word.
            _ => needles.push(word),
        }
    }
    (filters, needles)
}

/// The words a `a:` state filter knows, which are a Run's own three.
///
/// Small, fixed, and stated once: a filter that accepts a word nothing can
/// ever be answers every query with an empty list and looks like a bug in the
/// data rather than in the query.
fn is_run_state(word: &str) -> bool {
    matches!(word, "working" | "waiting" | "dead")
}

/// The one state word for a row, across the kinds that have one.
///
/// A Run says `working`, `waiting` or `dead`. A question carries no state
/// field because being one *is* its state — it is explicitly blocked on a
/// person, which is the plainest waiting in the building, and `a:waiting`
/// that omitted it would be answering the wrong question.
fn control_state(it: &Item) -> &str {
    match it.kind {
        Kind::Msg => "waiting",
        _ => it.get("state"),
    }
}

/// Did this run explicitly load that Skill or MCP server?
///
/// Reads only `run_skills` and `run_mcp`, which Milestone 5 fills in from the
/// run's own flags. That is a *confirmed* capability and deliberately not the
/// installed inventory: "claude has forty skills" and "this run loaded one"
/// are different facts, and a filter that conflated them would answer
/// `using` with every run of an agent that merely has the skill installed.
fn run_loaded(it: &Item, want: &str) -> bool {
    ["run_skills", "run_mcp"].iter().any(|key| {
        it.get(key).split(',').map(str::trim).any(|name| name.eq_ignore_ascii_case(want))
    })
}

/// Exact project match, never widened into a substring — `sessions::in_project`'s
/// rule, for the same reason: `project:app` quietly including every project
/// whose path contains "app" is a filter pretending to be an answer.
///
/// Three fields, because one row shape has no `project` of its own. A Run
/// sits in exactly one directory; an Agent is in as many as it has
/// runs, and `agents.rs` writes them as the JSON array `projects`. Reading
/// only the singular fields made `a:project:Prelude` hide the very agent
/// working in Prelude while listing its run — a filter that answers half the
/// question is the same lie as one that widens.
///
/// The array is small (one entry per live run of that agent), parsed only when
/// a `project:` filter was actually typed, and touches nothing outside the
/// row — `scoped_rows` stays pure and subprocess-free.
fn control_project(it: &Item, want: &str) -> bool {
    if it.get("project").to_lowercase() == want {
        return true;
    }
    if project_path_is(it.get("cwd"), want) {
        return true;
    }
    let projects = it.get("projects");
    !projects.is_empty()
        && serde_json::from_str::<Vec<String>>(projects).is_ok_and(|projects| {
            projects
                .iter()
                .any(|project| project.to_lowercase() == want || project_path_is(project, want))
        })
}

/// A directory answering to that name: the whole path, or its own last
/// component, and nothing in between.
fn project_path_is(path: &str, want: &str) -> bool {
    let path = path.trim_end_matches('/');
    !path.is_empty()
        && (path.to_lowercase() == want
            || path.rsplit('/').next().unwrap_or_default().to_lowercase() == want)
}

fn matches_agent_filters(it: &Item, filters: &AgentFilters, needles: &[String]) -> bool {
    if !filters.states.is_empty() && !filters.states.iter().any(|s| s == control_state(it)) {
        return false;
    }
    if !filters.agents.is_empty() {
        // A merged Skill row carries every owner in one comma-joined field,
        // so membership rather than equality — still exact per name.
        let mine: Vec<String> =
            it.get("agent").split(',').map(|a| a.trim().to_lowercase()).collect();
        if !filters.agents.iter().any(|want| mine.iter().any(|a| a == want)) {
            return false;
        }
    }
    if !filters.projects.is_empty()
        && !filters.projects.iter().any(|want| control_project(it, want))
    {
        return false;
    }
    if !filters.using.is_empty() || !filters.without.is_empty() {
        // Only a Run can answer this. Letting `without` fall through to
        // skills, servers and agents would return the entire scope minus one
        // row and call it an answer — the same lie as an unrecognised filter
        // that matches everything.
        if it.kind != Kind::Run
            || !filters.using.iter().all(|want| run_loaded(it, want))
            || filters.without.iter().any(|want| run_loaded(it, want))
        {
            return false;
        }
    }
    // A word that looked like a filter and was not one is searched for
    // literally, which empties the list visibly instead of silently widening
    // it.
    let term = needles.iter().chain(filters.unknown.iter()).cloned().collect::<Vec<_>>().join(" ");
    matches_terms(it, &term)
}

fn matches_terms(it: &Item, term: &str) -> bool {
    let needles: Vec<String> = term.split_whitespace().map(str::to_lowercase).collect();
    if needles.is_empty() {
        return true;
    }
    let hay = format!("{} {} {} {}", it.title, it.subtitle, it.fields.join(" "), it.cmd).to_lowercase();
    needles.iter().all(|n| hay.contains(n))
}

pub fn scoped_rows(scope: Scope, term: &str, static_items: &[Item]) -> Vec<Item> {
    use Kind::*;
    if scope == Scope::Running {
        return crate::sources::running::live()
            .into_iter().filter(|it| matches_terms(it, term)).take(100).collect();
    }
    if scope == Scope::Sessions {
        return crate::sources::sessions::search(term);
    }
    if scope == Scope::Files {
        let mut rows: Vec<Item> = static_items.iter()
            .filter(|it| it.kind == File && matches_terms(it, term))
            .cloned().collect();
        match search_fileindex(term) {
            Some(hits) => rows.extend(hits),
            None if rows.is_empty() => rows.push(
                Item::new("prelude index", Find)
                    .title("⚠ file index not built")
                    .sub("no index yet — run:  prelude index"),
            ),
            None => {}
        }
        let mut rows = crate::cache::finish(rows);
        rows.truncate(100);
        return rows;
    }
    if scope == Scope::Agent {
        // The control scope, and the only one with a filter vocabulary of its
        // own. Session is deliberately absent: `s:` owns the hundreds of old
        // conversations, and putting them here made the agent overview a
        // session browser.
        let (filters, needles) = parse_agent_filters(term);
        return static_items
            .iter()
            .filter(|it| matches!(it.kind, Msg | Agent | Run | Skill | Mcp | Config))
            .filter(|it| matches_agent_filters(it, &filters, &needles))
            .take(200)
            .cloned()
            .collect();
    }
    let wanted = |kind| match scope {
        Scope::Clipboard => kind == Clip,
        Scope::History => kind == History,
        Scope::Applications => kind == App,
        Scope::Commands => matches!(kind, Path | Sys),
        Scope::Directories => kind == Dir,
        Scope::Project => matches!(kind, Script | File | Git),
        Scope::Ssh => kind == Ssh,
        Scope::Snippets => kind == Snippet,
        Scope::Ports => kind == Port,
        Scope::Processes => kind == Proc,
        Scope::Containers => kind == Container,
        Scope::Skills => kind == Skill,
        Scope::Mcp => kind == Mcp,
        Scope::Config => kind == Config,
        Scope::Settings => kind == Setting,
        Scope::Agent | Scope::Running | Scope::Sessions | Scope::Files => false,
    };
    static_items.iter()
        .filter(|it| wanted(it.kind) && matches_terms(it, term))
        .take(200)
        .cloned()
        .collect()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `g rust async` -> (url, name, term, key).
pub(crate) fn quicklink_from(text: &str, q: &str) -> Option<(String, String, String, String)> {
    let (key, term) = q.trim().split_once(char::is_whitespace)?;
    let term = term.trim();
    if term.is_empty() {
        return None;
    }
    let links = crate::minitoml::parse(text);
    let body = links.get(&key.to_ascii_lowercase())?;
    let template = body.get("target").or_else(|| body.get("url"))?;
    if !template.contains("{q}") {
        return None;
    }
    let url = template.replace("{q}", &percent_encode(term));
    let name = body.get("name").cloned().unwrap_or_else(|| key.to_string());
    Some((url, name, term.to_string(), key.to_ascii_lowercase()))
}

pub fn quicklink(q: &str) -> Option<(String, String, String, String)> {
    quicklink_from(&quicklinks_text(), q)
}

fn is_quicklink_template(key: &str) -> bool {
    quicklinks()
        .get(&key.to_ascii_lowercase())
        .and_then(|body| body.get("target").or_else(|| body.get("url")))
        .is_some_and(|target| target.contains("{q}"))
}

// ─── whole-disk file search ──────────────────────────────────────────────

pub fn fileindex_path() -> std::path::PathBuf {
    paths::cache().join("fileindex.txt")
}

pub fn index_roots() -> Vec<String> {
    let cfg = paths::config().join("roots.txt");
    if let Ok(t) = std::fs::read_to_string(&cfg) {
        let v: Vec<String> = t
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                l.strip_prefix("~/")
                    .map(|r| paths::home().join(r).to_string_lossy().into_owned())
                    .unwrap_or_else(|| l.to_string())
            })
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    ["App", "Documents", "Desktop"]
        .iter()
        .map(|d| paths::home().join(d).to_string_lossy().into_owned())
        .collect()
}

pub fn build_fileindex() -> usize {
    let finder = which("fd").or_else(|| which("fdfind"));
    let mut lines = Vec::new();
    for root in index_roots() {
        if !std::path::Path::new(&root).is_dir() {
            continue;
        }
        let out = match &finder {
            Some(fd) => run(
                &[&fd.to_string_lossy(), "--type", "f", "--max-depth", "7",
                  "--color", "never", ".", &root],
                Duration::from_secs(90),
            ),
            None => run(
                &["find", &root, "-maxdepth", "7", "-type", "f"],
                Duration::from_secs(90),
            ),
        };
        lines.extend(out.lines().map(str::to_string));
    }
    let _ = crate::cache::write_atomic(&fileindex_path(), lines.join("\n").as_bytes());
    // Recorded so the settings row can state the size without reading a
    // megabyte of paths on every gather.
    crate::settings::record_index_count(lines.len());
    lines.len()
}

/// `f:name` searches a prebuilt index. Spotlight indexing may be disabled and
/// a live `fd` over $HOME takes over 30 seconds, so neither works live.
pub fn search_fileindex(term: &str) -> Option<Vec<Item>> {
    let text = std::fs::read_to_string(fileindex_path()).ok()?;
    let needles: Vec<String> = term
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let mut out = Vec::new();
    for line in text.lines() {
        let low = line.to_ascii_lowercase();
        if needles.iter().all(|n| low.contains(n.as_str())) {
            let name = line.rsplit('/').next().unwrap_or(line).to_string();
            let dir = line.rsplit_once('/').map(|(d, _)| paths::tilde(d)).unwrap_or_default();
            out.push(Item::new(line, Kind::Find).title(name).sub(dir).put("path", line));
            if out.len() >= 60 {
                break;
            }
        }
    }
    Some(out)
}

/// `@claude refactor this` — start an agent in the current directory with a
/// prompt, turning the launcher into the agent's front door.
pub fn agent_query(q: &str) -> Option<(String, String)> {
    let t = q.trim().strip_prefix('@')?;
    let (agent, prompt) = t.split_once(char::is_whitespace)?;
    let prompt = prompt.trim();
    if prompt.is_empty() || agent.is_empty() {
        return None;
    }
    let want = agent.to_lowercase();
    let installed = crate::agent::installed();
    // Exact name wins over a prefix, so `@pi` never resolves to something else.
    let agent = installed.iter().find(|k| **k == want)
        .or_else(|| installed.iter().find(|k| k.starts_with(&want)))?;
    Some((agent.to_string(), prompt.to_string()))
}

/// `/skill-name args` — invoke a skill and answer in the panel.
pub fn skill_query(q: &str) -> Option<(Item, String)> {
    let t = q.trim();
    let rest = t.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (rest, ""),
    };
    let skills = crate::sources::agents::skills();
    let hit = skills.into_iter().find(|s| s.title.eq_ignore_ascii_case(name))?;
    Some((hit, args.to_string()))
}

/// Does this query produce a computed row rather than a search?
///
/// Intent recognition only — this runs on *every* keystroke, so it must not
/// evaluate a result. The one config lookup is a tiny local file needed for
/// exact Quicklink aliases; calculations, subprocesses and network work stay
/// in `dynamic_rows_with`.
fn looks_like_local_path(q: &str) -> bool {
    let trimmed = q.trim();
    let unquoted = match (trimmed.chars().next(), trimmed.chars().last()) {
        (Some(a), Some(b)) if a == b && matches!(a, '\'' | '"') && trimmed.len() > 1 => {
            &trimmed[1..trimmed.len() - 1]
        }
        _ => trimmed,
    };
    unquoted.starts_with('/')
        || unquoted.starts_with("~/")
        || unquoted.starts_with("./")
        || unquoted.starts_with("../")
        || unquoted.starts_with("file:///")
        || (!unquoted.contains("://") && unquoted.contains('/'))
}

/// Turn an explicitly typed or pasted local path into the same object row the
/// file index would have produced. Existence is checked here, never in
/// `is_special`, so the per-keystroke intent test remains lexical and cheap.
fn local_path_item(q: &str) -> Option<Item> {
    if !looks_like_local_path(q) {
        return None;
    }
    let raw = q.trim();
    let raw = raw.strip_prefix("file://").unwrap_or(raw);
    let path = crate::settings::readings_of(raw)
        .into_iter()
        .find_map(|candidate| candidate.canonicalize().ok())?;
    let path_text = path.to_string_lossy().into_owned();
    let title = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("/")
        .to_string();
    if path.is_dir() {
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        {
            return Some(
                Item::new(format!("open {}", shq(&path_text)), Kind::App)
                    .title(title)
                    .sub(paths::tilde(&path_text))
                    .put("path", path_text),
            );
        }
        Some(
            Item::new(format!("cd {}", shq(&path_text)), Kind::Dir)
                .title(title)
                .sub(paths::tilde(&path_text))
                .put("path", path_text),
        )
    } else if path.is_file() {
        let parent = path
            .parent()
            .map(|parent| paths::tilde(&parent.to_string_lossy()))
            .unwrap_or_default();
        Some(
            Item::new(path_text.clone(), Kind::File)
                .title(title)
                .sub(parent)
                .put("path", path_text),
        )
    } else {
        None
    }
}

pub fn is_special(q: &str) -> bool {
    let t = q.trim();
    if t.is_empty() {
        return false;
    }
    if t == ":"
        || exact_scope_command(t).is_some()
        || scope_query(t).is_some()
        || t.starts_with(['/', '@'])
        || looks_like_local_path(t)
    {
        return true;
    }
    if exact_quicklink_item(t).is_some() {
        return true;
    }
    if translate_query(t).is_some() {
        return true;
    }
    if crate::calc::calc(t).is_some() || crate::calc::timecalc(t).is_some() {
        return true;
    }
    if parse_conversion(t).is_some() {
        return true;
    }
    match t.split_once(char::is_whitespace) {
        Some((k, rest)) if !rest.trim().is_empty() => is_quicklink_template(k),
        _ => false,
    }
}

/// The rows a query computes, in the order they should appear.
pub fn dynamic_rows_with(q: &str, static_items: &[Item]) -> Vec<Item> {
    let mut rows = Vec::new();
    if q.trim() == ":" {
        return scope_commands();
    }
    if let Some(item) = exact_scope_command(q) {
        return vec![item];
    }
    if let Some((scope, term)) = scope_query(q) {
        return scoped_rows(scope, term, static_items);
    }
    // `/` by itself is the Skill browser. Any more specific path that really
    // exists wins over slash-command browsing; a dragged `/Users/…` path must
    // not become an empty Skill search.
    if q.trim() != "/" {
        if let Some(item) = local_path_item(q) {
            return vec![item];
        }
    }
    if let Some(rows) = skill_prefix_rows(q, static_items) {
        return rows;
    }
    if let Some(rows) = agent_prompt_rows(q, static_items) {
        return rows;
    }
    if let Some(item) = exact_quicklink_item(q) {
        return vec![item];
    }
    if let Some(url) = web_url(q) {
        rows.push(
            Item::new(url.clone(), Kind::Link)
                .title(q.trim())
                .sub(&url)
                .put("url", url),
        );
    }
    if let Some(v) = crate::calc::calc(q) {
        rows.push(Item::new(v.clone(), Kind::Calc).title(v).sub(q.trim()));
    }
    if let Some((v, note)) = convert(q) {
        rows.push(Item::new(v.clone(), Kind::Calc).title(v).sub(note));
    }
    if let Some((v, note)) = crate::calc::timecalc(q) {
        rows.push(Item::new(v.clone(), Kind::Calc).title(v).sub(note));
    }
    if let Some((url, name, term, key)) = quicklink(q) {
        rows.push(
            Item::new(url.clone(), Kind::Link)
                .title(format!("{name}: {term}"))
                .sub(format!("{name} · {term}"))
                .put("url", url)
                .put("quicklink", key)
                .put("quicklink_managed", "false"),
        );
    }
    if let Some((skill, args)) = skill_query(q) {
        // Pick an agent that actually has it; "shared" is a directory, not
        // something you can run.
        let agent = skill.get("agent").split(',').map(str::trim)
            .find(|a| !a.is_empty() && *a != "shared").unwrap_or("claude").to_string();
        let prompt = if args.is_empty() {
            skill.cmd.clone()
        } else {
            format!("{} {args}", skill.cmd)
        };
        rows.push(
            Item::new(crate::sources::sessions::start_cmd(&agent, None, Some(&prompt)), Kind::Session)
                .title(format!("{}{}", skill.title, if args.is_empty() { String::new() } else { format!(" {args}") }))
                .fields([agent.clone(), "⏎ answers in the panel".to_string(), skill.get("desc").to_string()])
                .put("agent", agent)
                .put("prompt", prompt)
                .put("mode", "start"),
        );
    }
    if let Some((agent, prompt)) = agent_query(q) {
        let cwd = paths::cwd().map(|p| p.to_string_lossy().into_owned());
        let cmd = crate::sources::sessions::start_cmd(&agent, cwd.as_deref(), Some(&prompt));
        rows.push(
            Item::new(cmd, Kind::Session)
                .title(format!("{agent}: {prompt}"))
                .fields([agent.clone(), "⏎ answers in the panel".to_string()])
                .put("agent", agent)
                .put("prompt", prompt)
                .put("mode", "start"),
        );
    }
    if let Some((lang, text)) = translate_query(q) {
        match translate(&text, &lang) {
            Ok(v) => rows.push(
                Item::new(v.clone(), Kind::Translate)
                    .title(v)
                    .sub(format!("{lang} · {text}"))
                    .put("source", text)
                    .put("target", lang),
            ),
            Err(e) => rows.push(Item::new(text, Kind::Translate).title(format!("⚠ {e}")).sub(e)),
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(state: &str, project: &str) -> Item {
        Item::new(format!("kill {state}{project}"), Kind::Run)
            .title("claude")
            .fields([project.to_string(), state.to_string()])
            .put("state", state)
            .put("agent", "claude")
            .put("project", project)
    }

    /// The empty query is the inventory: the agents, what they are running,
    /// their capabilities and the conversations you have had — plus a question
    /// that is blocking somebody. Everything else on the machine is behind a
    /// query, which is what stops the home being a list of two thousand files.
    #[test]
    fn the_home_is_the_agent_inventory_and_nothing_else() {
        let rows = vec![
            Item::new("agent", Kind::Agent).title("claude"),
            run("working", "api"),
            Item::new("/deploy", Kind::Skill).title("deploy"),
            Item::new("codex mcp get n", Kind::Mcp).title("node_repl"),
            Item::new("old", Kind::Session).title("old"),
            Item::new("ask", Kind::Msg).title("claude asks"),
            Item::new("cargo", Kind::Path).title("cargo"),
            Item::new("copied", Kind::Clip).title("copied"),
            Item::new("git status", Kind::History).title("git status"),
        ];
        let shown: Vec<String> = home_items(&rows).into_iter().map(|it| it.title).collect();
        assert_eq!(shown, ["claude", "claude", "deploy", "node_repl", "old", "claude asks"]);
        // Healthy or not is not the question any more: a server that answers
        // and a skill that agrees with its copies are things you own, and the
        // home is where you look at what you own.
        let unhealthy =
            vec![Item::new("codex mcp get n", Kind::Mcp).title("gmail").put("health", "failed")];
        assert_eq!(home_items(&unhealthy).len(), 1);

        // Ordering is `cache::by_rank`'s job, so this filter must leave the
        // order it was handed alone.
        let pair = vec![run("waiting", "first"), run("waiting", "second")];
        let projects: Vec<String> =
            home_items(&pair).iter().map(|it| it.get("project").to_string()).collect();
        assert_eq!(projects, ["first", "second"]);

        // A typed query searches the same inventory, minus the hundreds of
        // old conversations that `s:` owns.
        let root: Vec<String> = root_items(&rows).into_iter().map(|it| it.title).collect();
        assert!(root.contains(&"node_repl".to_string()) && root.contains(&"deploy".to_string()));
        assert!(!root.contains(&"old".to_string()), "sessions have their own s: scope");
    }

    #[test]
    fn skills_are_a_visible_named_scope_and_slash_remains_an_accelerator() {
        let commands = scope_commands();
        let skills = commands.iter().find(|item| item.title == "Skills").expect("Skills command");
        assert_eq!(skills.get("completion"), "skill:");
        let (scope, term) = scope_query("skill: review").expect("skill scope");
        assert_eq!(scope, Scope::Skills);
        assert_eq!(term, "review");
        let rows = vec![
            Item::new("one", Kind::Skill).title("review"),
            Item::new("two", Kind::Mcp).title("review server"),
        ];
        assert_eq!(scoped_rows(scope, term, &rows).len(), 1);
    }

    #[test]
    fn agent_filters_are_a_vocabulary_not_a_guess() {
        let (f, needles) = parse_agent_filters("waiting agent:Claude project:Prelude using deploy");
        assert_eq!(f.states, ["waiting"]);
        assert_eq!(f.agents, ["claude"]);
        assert_eq!(f.projects, ["prelude"]);
        assert_eq!(f.using, ["deploy"]);
        assert!(needles.is_empty() && f.unknown.is_empty());

        let (f, needles) = parse_agent_filters("claude Prelude");
        assert!(f.states.is_empty(), "an agent name is not a state word");
        assert_eq!(needles, ["claude", "prelude"]);

        // A filter word that named nothing is searched for literally rather
        // than dropped, so the list collapses where it can be seen.
        let (f, _) = parse_agent_filters("using");
        assert_eq!(f.unknown, ["using"]);
        assert!(f.using.is_empty(), "a capability nobody named must not match every run");
        assert_eq!(parse_agent_filters("state:banana").0.unknown, ["state:banana"]);
        assert_eq!(parse_agent_filters("agent:").0.unknown, ["agent:"]);
        assert_eq!(parse_agent_filters("./notes.md").1, ["./notes.md"]);
    }

    /// An MCP server's name is whatever its owner called it, and three of the
    /// ones on this machine have spaces in them. Unquoted, the query silently
    /// became a different question with an empty answer.
    #[test]
    fn a_capability_name_may_contain_spaces_when_it_is_quoted() {
        let (f, needles) = parse_agent_filters("using \"claude.ai Google Drive\"");
        assert_eq!(f.using, ["claude.ai google drive"]);
        assert!(needles.is_empty() && f.unknown.is_empty());

        // The `key:value` form holds together the same way.
        let (f, needles) = parse_agent_filters("using:'my skill' without:\"other skill\"");
        assert_eq!(f.using, ["my skill"]);
        assert_eq!(f.without, ["other skill"]);
        assert!(needles.is_empty());

        // Unquoted stays exactly as it was: one word to the keyword, the rest
        // as needles, so the list collapses visibly rather than widening.
        let (f, needles) = parse_agent_filters("using my skill");
        assert_eq!(f.using, ["my"]);
        assert_eq!(needles, ["skill"]);

        // And an apostrophe in an ordinary English search is not a quote —
        // `split_words` only quotes when the mark is closed later.
        assert_eq!(parse_agent_filters("don't panic").1, ["don't", "panic"]);
    }

    #[test]
    fn the_agent_scope_answers_state_project_and_capability() {
        let with = |it: Item, skills: &str, mcp: &str| it.put("run_skills", skills).put("run_mcp", mcp);
        let items = vec![
            Item::new("ask", Kind::Msg)
                .title("claude asks")
                .fields(["Prelude".to_string(), "asked 2m ago".to_string()])
                .put("agent", "claude")
                .put("project", "Prelude"),
            with(run("waiting", "Prelude"), "deploy", ""),
            with(run("working", "api"), "", "node_repl"),
            // An Agent is in as many projects as it has runs, and carries
            // them as the JSON array `agents.rs` writes.
            Item::new("claude", Kind::Agent)
                .title("claude")
                .put("agent", "claude")
                .put("projects", r#"["Prelude","/Users/mike/App/api"]"#),
            Item::new("/deploy", Kind::Skill).title("deploy").put("agent", "claude, codex"),
            Item::new("old", Kind::Session).title("old").put("agent", "claude"),
        ];
        let titles = |term: &str| -> Vec<String> {
            scoped_rows(Scope::Agent, term, &items).into_iter().map(|it| it.title).collect()
        };

        assert_eq!(titles("waiting"), ["claude asks", "claude"], "a question is waiting on you");
        assert!(titles("dead").is_empty(), "nothing here has stopped");
        assert!(!titles("").iter().any(|t| t == "old"), "sessions have their own s: scope");
        assert_eq!(titles("claude Prelude"), ["claude asks", "claude"]);

        // Confirmed capability, and only a Run can answer it.
        assert_eq!(titles("using deploy"), ["claude"], "the run that loaded it, not the skill row");
        assert_eq!(titles("using node_repl").len(), 1);
        assert_eq!(titles("without deploy").len(), 1, "runs that did not load it, and nothing else");
        assert!(titles("using nothing-loaded").is_empty());
        // An exact project is never widened into a substring.
        assert!(titles("project:prel").is_empty());
        assert!(titles("project:us").is_empty(), "nor is a path component");

        // `project:` reads all three places a project is written down: the
        // singular field a Run carries, the working directory, and
        // the array an Agent carries one entry of per live run. Reading only
        // the first two hid the agent working in the very project asked about
        // while happily listing its run.
        assert_eq!(titles("project:api").len(), 2, "the run in api, and the agent that has it");
        assert_eq!(
            titles("project:prelude"),
            ["claude asks", "claude", "claude"],
            "the question, the run and the agent"
        );
        assert_eq!(
            titles("project:/Users/mike/App/api"),
            ["claude"],
            "the whole path answers too, and only the agent records that one"
        );
    }
}
