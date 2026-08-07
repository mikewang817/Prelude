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

// ─── quicklinks ──────────────────────────────────────────────────────────

pub const QUICKLINKS_DEFAULT: &str = r#"# Prelude quicklinks
# Type the keyword followed by your search terms, e.g.  g rust async
# {q} is replaced with what you typed (URL-encoded).

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
"#;

pub fn quicklinks() -> crate::minitoml::Table {
    let p = paths::config().join("quicklinks.toml");
    if !p.exists() {
        let _ = std::fs::create_dir_all(paths::config());
        let _ = std::fs::write(&p, QUICKLINKS_DEFAULT);
    }
    std::fs::read_to_string(&p)
        .map(|t| crate::minitoml::parse(&t))
        .unwrap_or_default()
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

/// `g rust async` -> (url, name, term).
pub fn quicklink(q: &str) -> Option<(String, String, String)> {
    let (key, term) = q.trim().split_once(char::is_whitespace)?;
    let term = term.trim();
    if term.is_empty() {
        return None;
    }
    let links = quicklinks();
    let body = links.get(&key.to_ascii_lowercase())?;
    let url = body.get("url")?.replace("{q}", &percent_encode(term));
    let name = body.get("name").cloned().unwrap_or_else(|| key.to_string());
    Some((url, name, term.to_string()))
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

/// `s:query` — search every past agent session, not just the recent ones.
pub fn session_query(q: &str) -> Option<&str> {
    let t = q.trim();
    for p in ["s:", "S:"] {
        if let Some(rest) = t.strip_prefix(p) {
            return Some(rest.trim());
        }
    }
    None
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
    let installed = crate::sources::sessions::installed();
    // Exact name wins over a prefix, so `@pi` never resolves to something else.
    let agent = installed.iter().find(|k| **k == want)
        .or_else(|| installed.iter().find(|k| k.starts_with(&want)))?;
    Some((agent.to_string(), prompt.to_string()))
}

/// Does this query produce a computed row rather than a search?
///
/// Pattern-matching only — this runs on *every* keystroke, so it must not
/// actually compute anything. Evaluating here would shell out to `units`
/// (and once, to the network for exchange rates) on each character typed.
pub fn is_special(q: &str) -> bool {
    let t = q.trim();
    if t.is_empty() {
        return false;
    }
    if t.len() > 2 && (t.starts_with("f:") || t.starts_with("F:")) {
        return true;
    }
    if session_query(t).is_some() || agent_query(t).is_some() {
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
        Some((k, rest)) if !rest.trim().is_empty() => {
            quicklinks().contains_key(&k.to_ascii_lowercase())
        }
        _ => false,
    }
}

/// The rows a query computes, in the order they should appear.
pub fn dynamic_rows(q: &str) -> Vec<Item> {
    let mut rows = Vec::new();
    if let Some(v) = crate::calc::calc(q) {
        rows.push(Item::new(v.clone(), Kind::Calc).title(v).sub(q.trim()));
    }
    if let Some((v, note)) = convert(q) {
        rows.push(Item::new(v.clone(), Kind::Calc).title(v).sub(note));
    }
    if let Some((v, note)) = crate::calc::timecalc(q) {
        rows.push(Item::new(v.clone(), Kind::Calc).title(v).sub(note));
    }
    if let Some((url, name, term)) = quicklink(q) {
        rows.push(
            Item::new(format!("open {}", shq(&url)), Kind::Link)
                .title(format!("{name}: {term}"))
                .sub(format!("{name} · {term}"))
                .put("url", url),
        );
    }
    if let Some(rest) = q.trim().strip_prefix("f:").or_else(|| q.trim().strip_prefix("F:")) {
        match search_fileindex(rest.trim()) {
            Some(hits) => rows.extend(hits),
            None => rows.push(
                Item::new("prelude index", Kind::Find)
                    .title("⚠ file index not built")
                    .sub("no index yet — run:  prelude index"),
            ),
        }
    }
    if let Some(term) = session_query(q) {
        rows.extend(crate::sources::sessions::search(term));
    }
    if let Some((agent, prompt)) = agent_query(q) {
        let cwd = paths::cwd().map(|p| p.to_string_lossy().into_owned());
        let cmd = crate::sources::sessions::start_cmd(&agent, cwd.as_deref(), Some(&prompt));
        rows.push(
            Item::new(cmd, Kind::Session)
                .title(format!("{agent}: {prompt}"))
                .fields([agent.clone(), "new session here".to_string()])
                .put("agent", agent),
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
