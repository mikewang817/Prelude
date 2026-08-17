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

pub fn fxhash(s: &str) -> u64 {
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

pub const QUICKLINKS_DEFAULT: &str = r#"# Prelude quicklinks — keywords you type to reach one thing.
#
# Two shapes. A template takes a search term:
#
#     [jira]
#     name = "Jira"
#     url  = "https://jira.example.com/issues?jql={q}"     # j api timeout
#
# and a fixed entry points at one file, folder, application or URL:
#
#     [notes]
#     name   = "Notes"
#     kind   = "folder"                # file · folder · app · url · template
#     target = "~/Documents/notes"     # type  notes
#
# {q} is replaced with what you typed, URL-encoded. Keywords are matched
# case-insensitively and may be written in any language.
#
#   ql:                        browse and manage every keyword
#   ^K on any row              create one from the thing you are looking at
#   prelude quicklink --help   the same, from a script
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

# prelude:defaults dev-agent-v3

[so]
name = "Stack Overflow"
url  = "https://stackoverflow.com/search?q={q}"

[crates]
name = "crates.io"
url  = "https://crates.io/search?q={q}"

[docsrs]
name = "docs.rs"
url  = "https://docs.rs/releases/search?query={q}"

[pypi]
name = "PyPI"
url  = "https://pypi.org/search/?q={q}"

[pkg]
name = "pkg.go.dev"
url  = "https://pkg.go.dev/search?q={q}"

[caniuse]
name = "Can I use"
url  = "https://caniuse.com/?search={q}"

[explain]
name = "explainshell"
url  = "https://explainshell.com/explain?cmd={q}"

[hn]
name = "Hacker News"
url  = "https://hn.algolia.com/?q={q}"

[hf]
name = "Hugging Face"
url  = "https://huggingface.co/models?search={q}"

[arxiv]
name = "arXiv"
url  = "https://arxiv.org/search/?query={q}&searchtype=all"

[ccdocs]
name = "Claude Code docs"
url  = "https://docs.claude.com/en/docs/claude-code/overview"

[mcpdocs]
name = "Model Context Protocol"
url  = "https://modelcontextprotocol.io"
"#;

/// Versioned blocks of built-in Quicklinks.
///
/// Each block is added once, to a file that has not seen its marker. An entry
/// whose keyword the person already uses is skipped rather than overwritten,
/// and a deleted default is not restored — the marker records that the block
/// was offered, not that its contents are still present. Adding a block later
/// means appending to this table; nothing else changes.
///
/// Built-in keywords are chosen against the scope prefixes and against the
/// built-in Agent names, because both would be shadowed or shadowing: a
/// keyword the search box has already spent is refused outright, and an exact
/// keyword equal to `claude` would push the Agent row it names down one line
/// for everybody, by default, forever. A test walks this table for both.
/// `(keyword, name, url)`. A url with `{q}` is a template, one without is a
/// fixed link.
pub(crate) type BuiltIn = (&'static str, &'static str, &'static str);

pub(crate) const DEFAULT_BLOCKS: &[(&str, &[BuiltIn])] = &[
    (
        "# prelude:defaults web-search-v2",
        &[
            ("b", "Baidu", "https://www.baidu.com/s?wd={q}"),
            ("bing", "Bing", "https://www.bing.com/search?q={q}"),
            ("ddg", "DuckDuckGo", "https://duckduckgo.com/?q={q}"),
        ],
    ),
    // What this launcher's two audiences look up all day: the package
    // registry and the error message for one, the model hub and the agent's
    // own documentation for the other. `ccdocs` and `mcpdocs` carry no `{q}`,
    // which also makes the shipped file show both shapes — every default used
    // to be a template, so nothing taught that a fixed entry existed.
    (
        "# prelude:defaults dev-agent-v3",
        &[
            ("so", "Stack Overflow", "https://stackoverflow.com/search?q={q}"),
            ("crates", "crates.io", "https://crates.io/search?q={q}"),
            ("docsrs", "docs.rs", "https://docs.rs/releases/search?query={q}"),
            ("pypi", "PyPI", "https://pypi.org/search/?q={q}"),
            ("pkg", "pkg.go.dev", "https://pkg.go.dev/search?q={q}"),
            ("caniuse", "Can I use", "https://caniuse.com/?search={q}"),
            ("explain", "explainshell", "https://explainshell.com/explain?cmd={q}"),
            ("hn", "Hacker News", "https://hn.algolia.com/?q={q}"),
            ("hf", "Hugging Face", "https://huggingface.co/models?search={q}"),
            ("arxiv", "arXiv", "https://arxiv.org/search/?query={q}&searchtype=all"),
            ("ccdocs", "Claude Code docs", "https://docs.claude.com/en/docs/claude-code/overview"),
            ("mcpdocs", "Model Context Protocol", "https://modelcontextprotocol.io"),
        ],
    ),
];

pub fn quicklinks_file() -> std::path::PathBuf {
    paths::config().join("quicklinks.toml")
}

pub(crate) fn add_web_search_defaults(mut text: String) -> (String, bool) {
    let mut changed = false;
    for (marker, entries) in DEFAULT_BLOCKS {
        if text.lines().any(|line| line == *marker) {
            continue;
        }
        let existing = crate::minitoml::parse(&text);
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
        text.push_str(marker);
        text.push('\n');
        for (key, name, url) in *entries {
            if quicklink_entry(&existing, key).is_some() || quicklink_conflict(key).is_some() {
                continue;
            }
            text.push_str(&format!(
                "\n[{key}]\nname = {}\nurl = {}\n",
                toml_string(name),
                toml_string(url),
            ));
        }
        changed = true;
    }
    (text, changed)
}

/// Create the file if it is missing and add any versioned default block it
/// has not seen yet.
///
/// This is the *write* half of reading quicklinks, and it is deliberately not
/// on the per-keystroke path. It used to be: `quicklinks_text` created the
/// file and could rewrite it, and `is_special` plus `dynamic_rows_with`
/// between them called that up to four times per keystroke — a read path with
/// a write in it, on the one path in this program that must not do file work
/// it can avoid. Callers that are already doing a launch's worth of work
/// (`quicklink_items` from `gather`, the settings editor, the CLI, `doctor`)
/// call this; the keystroke path reads and never writes.
pub fn ensure_quicklinks_file() -> Result<std::path::PathBuf, String> {
    let path = quicklinks_file();
    // The versioned default-block migration is read, change, write, and it
    // runs once per gather from `quicklink_items` — so every shell and the
    // panel reach it at once on the launch after an upgrade. Two of them
    // appending the same block concurrently is exactly the lost update this
    // guards, and the loss would be a keyword the person then never sees.
    let _lock = crate::cache::lock_for_write(&path);
    let Some(current) = read_quicklinks_file() else {
        // Absent is a fresh install; unreadable is a file with contents in it
        // that we could not see. Only the first may be written to.
        if path.exists() {
            return Err(format!("{} is there but cannot be read", path.display()));
        }
        crate::cache::write_state(&path, QUICKLINKS_DEFAULT.as_bytes())
            .map_err(|e| e.to_string())?;
        invalidate_quicklinks();
        return Ok(path);
    };
    let (text, changed) = add_web_search_defaults(current);
    if changed {
        crate::cache::write_state(&path, text.as_bytes()).map_err(|e| e.to_string())?;
        invalidate_quicklinks();
    }
    Ok(path)
}

/// The file as it is on disk — `None` when it could not be read at all, which
/// is never the same answer as "empty" and never the same as "the defaults".
///
/// Every write derives from this. It used to substitute `QUICKLINKS_DEFAULT`
/// on any error, which reads harmlessly and is not: `ensure_quicklinks_file`
/// would then write those defaults *over* a file it had just failed to read,
/// and `remove_quicklink` would edit a copy of the built-ins and save it as
/// the person's config. A read that fails must stop the write, not invent its
/// input.
fn read_quicklinks_file() -> Option<String> {
    std::fs::read_to_string(quicklinks_file()).ok()
}

/// The same read, for display: a file that is genuinely absent answers with
/// the built-in defaults, so a fresh install can type `g rust` before anything
/// has been written anywhere. Never used as the basis of a write.
fn read_quicklinks_text() -> String {
    read_quicklinks_file().unwrap_or_else(|| QUICKLINKS_DEFAULT.to_string())
}

/// What a mutation is allowed to start from. A file that is there and
/// unreadable stops the edit; an absent one is a fresh install and starts from
/// the built-ins, which is the only case where substituting them is honest.
fn read_for_write() -> Result<String, String> {
    match read_quicklinks_file() {
        Some(text) => Ok(text),
        None if !quicklinks_file().exists() => Ok(QUICKLINKS_DEFAULT.to_string()),
        None => Err(format!("{} cannot be read; nothing was changed", quicklinks_file().display())),
    }
}

fn quicklinks_cache() -> &'static std::sync::RwLock<Option<String>> {
    static TEXT: std::sync::OnceLock<std::sync::RwLock<Option<String>>> =
        std::sync::OnceLock::new();
    TEXT.get_or_init(|| std::sync::RwLock::new(None))
}

fn invalidate_quicklinks() {
    if let Ok(mut slot) = quicklinks_cache().write() {
        *slot = None;
    }
}

/// The quicklinks file, read once per process.
///
/// One keystroke asks four separate questions of this file — is the query an
/// exact key, is its first word a template, and both again while the rows are
/// built — and each used to be a fresh open, read and parse.
fn quicklinks_text() -> String {
    if let Ok(slot) = quicklinks_cache().read() {
        if let Some(text) = slot.as_ref() {
            return text.clone();
        }
    }
    let text = read_quicklinks_text();
    if let Ok(mut slot) = quicklinks_cache().write() {
        *slot = Some(text.clone());
    }
    text
}

pub fn quicklinks() -> crate::minitoml::Table {
    crate::minitoml::parse(&quicklinks_text())
}

/// Look an entry up the way a person types it.
///
/// `minitoml` keeps a section name exactly as written, and every lookup here
/// lowercases what it was given — so a hand-written `[Design]` or `[GH]` was
/// matched by nothing, produced no row, and reported no error. It was not
/// broken so much as invisible, which is the worse of the two.
pub(crate) fn quicklink_entry<'a>(
    links: &'a crate::minitoml::Table,
    key: &str,
) -> Option<(String, &'a std::collections::BTreeMap<String, String>)> {
    let want = fold_key(key);
    links.iter().find(|(k, _)| fold_key(k) == want).map(|(k, v)| (k.clone(), v))
}

fn fold_key(key: &str) -> String {
    key.trim().trim_matches('"').to_lowercase()
}

#[derive(Clone, Debug)]
pub struct QuicklinkDraft {
    pub name: String,
    pub kind: &'static str,
    pub target: String,
}

impl QuicklinkDraft {
    pub fn is_template(&self) -> bool {
        self.target.contains("{q}")
    }
}

pub fn quicklinkable(kind: Kind) -> bool {
    matches!(kind, Kind::File | Kind::Find | Kind::Config | Kind::Dir | Kind::Link | Kind::App)
}

/// Turn a URL that already has a search term in it into a template to edit.
///
/// The most useful half of this feature — the `{q}` templates — had no way in
/// at all: `quicklink_draft` accepts only fixed objects, so anyone wanting
/// their own issue tracker or wiki search had to hand-write TOML, which is
/// exactly the population least likely to. The way in is the URL you are
/// looking at, and the guess that makes it one keystroke is that the term sits
/// in the last non-empty query parameter — `?q=`, `?wd=`, `?search=`,
/// `?query=` all land on it. The person sees the result before it is saved and
/// can move the `{q}` anywhere.
pub fn template_suggestion(url: &str) -> String {
    let Some((head, tail)) = url.split_once('?') else {
        return format!("{url}{}", if url.ends_with('/') { "{q}" } else { "/{q}" });
    };
    let (query, fragment) = match tail.split_once('#') {
        Some((q, f)) => (q, format!("#{f}")),
        None => (tail, String::new()),
    };
    let mut parts: Vec<String> = query.split('&').map(str::to_string).collect();
    let last = parts.iter().rposition(|p| {
        p.split_once('=').is_some_and(|(k, v)| !k.is_empty() && !v.is_empty())
    });
    match last {
        Some(i) => {
            let key = parts[i].split_once('=').map(|(k, _)| k).unwrap_or_default().to_string();
            parts[i] = format!("{key}={{q}}");
            format!("{head}?{}{fragment}", parts.join("&"))
        }
        None => format!("{url}{}{{q}}", if query.is_empty() { "" } else { "&" }),
    }
}

/// Validate a hand-composed `{q}` template and turn it into a draft.
pub fn template_draft(name: &str, template: &str) -> Result<QuicklinkDraft, String> {
    let template = template.trim();
    if !template.contains("{q}") {
        return Err("a search quicklink needs {q} where the search term goes".into());
    }
    if crate::secrets::looks_secret(template) || url_has_secret(template) {
        return Err("that URL appears to contain a credential and will not be indexed".into());
    }
    // `web_url` judges a real address, so check the shape the template will
    // actually take rather than the one with braces in it.
    if web_url(&template.replace("{q}", "prelude")).is_none() {
        return Err("that is not a safe HTTP or HTTPS URL".into());
    }
    let name = crate::width::flatten(name.trim());
    if name.is_empty() {
        return Err("give it a name to show in the list".into());
    }
    Ok(QuicklinkDraft { name, kind: "template", target: template.to_string() })
}

/// The stable identity behind a selected object. Local targets are resolved
/// before storage so a quicklink works from every directory and `..` cannot
/// make it silently point somewhere else later.
pub fn quicklink_draft(it: &Item) -> Result<Option<QuicklinkDraft>, String> {
    if !quicklinkable(it.kind) {
        return Ok(None);
    }
    let (kind, raw) = match it.kind {
        Kind::Find if it.get("index_kind") == "folder" => ("folder", it.get("path")),
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
    let name = if kind == "folder" {
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

/// A keyword to offer when creating a quicklink for this row.
///
/// Non-ASCII characters used to be dropped one at a time, so every CJK-named
/// file suggested the empty string and the prompt opened blank — for a whole
/// class of user the feature arrived with nothing filled in and a keyword they
/// were then not allowed to type either. Letters of any script are kept now;
/// only the punctuation between them becomes a hyphen.
pub fn quicklink_suggestion(it: &Item) -> String {
    let base = quicklink_draft(it).ok().flatten().map(|d| d.name).unwrap_or_else(|| it.title.clone());
    let base = std::path::Path::new(&base)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or(base);
    let mut out = String::new();
    let mut dash = false;
    for c in base.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
        if out.chars().count() >= KEY_MAX {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

const KEY_MAX: usize = 40;

/// What may be a keyword.
///
/// Letters and digits of any script, plus `-` and `_`. It was ASCII-only,
/// which meant a Chinese user could not name a quicklink in the language the
/// thing they were naming was written in. Everything else is excluded because
/// it already means something in the search box: `:` opens a scope, `/` a
/// skill, `@` an agent, `.` reads as a hostname, and quotes and brackets are
/// the config file's own syntax.
fn valid_quicklink_key(key: &str) -> bool {
    let n = key.chars().count();
    n > 0 && n <= KEY_MAX && key.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
}

pub fn normalize_quicklink_key(raw: &str) -> Result<String, String> {
    let key = raw.trim().to_lowercase();
    if valid_quicklink_key(&key) {
        Ok(key)
    } else {
        Err(format!("use 1–{KEY_MAX} letters, numbers, hyphens or underscores"))
    }
}

/// Words the search box has already spent, and which a quicklink therefore
/// cannot have.
///
/// `dynamic_rows_with` resolves a scope command before it resolves a
/// quicklink, so a quicklink called `f` or `s` was accepted, written to the
/// file, listed by `doctor` — and unreachable forever, with nothing anywhere
/// saying why. Refusing at the point of naming is the only moment this can be
/// explained.
pub fn quicklink_conflict(key: &str) -> Option<String> {
    if SCOPES.iter().any(|d| d.prefix.trim_end_matches(':') == key) {
        return Some(format!("“{key}” is the {key}: scope command — the scope would always win"));
    }
    None
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
    let key = normalize_quicklink_key(key)?;
    if let Some(why) = quicklink_conflict(&key) {
        return Err(why);
    }
    if let Some((existing, _)) = quicklink_entry(&crate::minitoml::parse(&text), &key) {
        return Err(format!("a quicklink called {existing} already exists"));
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "\n{}\n[{key}]\nname = {}\nkind = {}\ntarget = {}\n{}\n",
        quicklink_marker(&key, false),
        toml_string(&draft.name),
        toml_string(draft.kind),
        toml_string(&draft.target),
        quicklink_marker(&key, true),
    ));
    Ok(text)
}

/// The one door every creation path goes through — the launcher, the CLI and
/// the template flow — so the reserved-word check and the duplicate check
/// cannot be true in one of them and not another.
pub fn create_quicklink_from(key: &str, draft: &QuicklinkDraft) -> Result<String, String> {
    let key = normalize_quicklink_key(key)?;
    // Before the lock, not inside it. `flock` is held per open file
    // description, so a second request from *this* process conflicts with the
    // first exactly as another process would — it does not deadlock, because
    // the wait is bounded, but every creation would pay the whole 250ms bound
    // and then proceed unlocked, which is the worst of both.
    ensure_quicklinks_file()?;
    // Read, change, write — held across all three, so two keywords created at
    // the same moment do not keep one.
    let _lock = crate::cache::lock_for_write(&quicklinks_file());
    let text = append_quicklink(read_for_write()?, &key, draft)?;
    write_quicklinks(&text)?;
    Ok(key)
}

fn write_quicklinks(text: &str) -> Result<(), String> {
    crate::cache::write_state(&quicklinks_file(), text.as_bytes()).map_err(|e| e.to_string())?;
    invalidate_quicklinks();
    Ok(())
}

/// The byte range one entry occupies, whether Prelude wrote it or a person did.
///
/// Removal used to work only on marked blocks, and a hand-written entry
/// answered "that quicklink is managed in the config file" — a sentence whose
/// plain reading is the opposite of what it meant, offered in place of the
/// thing the person asked for. Prelude's own entries still use the markers,
/// so removing one leaves every hand-written line byte-for-byte; an unmarked
/// entry is bounded by its `[section]` header and the next one.
fn quicklink_span(text: &str, key: &str) -> Option<(usize, usize)> {
    let begin = format!("{}\n", quicklink_marker(key, false));
    let end = format!("\n{}", quicklink_marker(key, true));
    if let Some(start) = text.find(&begin) {
        if let Some(tail) = text[start + begin.len()..].find(&end) {
            let mut finish = start + begin.len() + tail + end.len();
            if text[finish..].starts_with('\n') {
                finish += 1;
            }
            return Some((if start > 0 && text[..start].ends_with('\n') { start - 1 } else { start }, finish));
        }
    }
    // Unmarked: from its header line to the line before the next section, with
    // the blank lines that separated them left behind rather than collapsed.
    let mut start = None;
    let mut at = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let is_header = trimmed.starts_with('[') && trimmed.ends_with(']');
        if is_header {
            let name = trimmed[1..trimmed.len() - 1].trim();
            match start {
                None if fold_key(name) == fold_key(key) => start = Some(at),
                None => {}
                Some(s) => return Some((s, at)),
            }
        }
        // A marker line belongs to the block it opens, so stop before it.
        if let Some(s) = start {
            if !is_header && trimmed.starts_with("# prelude:quicklink") {
                return Some((s, at));
            }
        }
        at += line.len();
    }
    start.map(|s| (s, text.len()))
}

pub(crate) fn remove_quicklink_block(mut text: String, key: &str) -> Result<String, String> {
    if !valid_quicklink_key(key) {
        return Err("invalid quicklink name".into());
    }
    let (start, finish) =
        quicklink_span(&text, key).ok_or_else(|| format!("no quicklink called {key}"))?;
    text.replace_range(start..finish, "");
    Ok(text)
}

pub fn remove_quicklink(key: &str) -> Result<(), String> {
    let _lock = crate::cache::lock_for_write(&quicklinks_file());
    let key = normalize_quicklink_key(key)?;
    let text = remove_quicklink_block(read_for_write()?, &key)?;
    write_quicklinks(&text)
}

/// Replace one `field = value` line inside an entry, or add it if the entry
/// never had one.
///
/// Renaming, re-pointing and re-labelling were the three things the launcher
/// could not do: creation was a product and every subsequent edit was "open
/// the TOML in `$EDITOR` and find it yourself", in a file that by then has
/// dozens of entries and no ordering.
pub(crate) fn set_quicklink_field(
    text: &str,
    key: &str,
    field: &str,
    value: &str,
) -> Result<String, String> {
    let (start, finish) =
        quicklink_span(text, key).ok_or_else(|| format!("no quicklink called {key}"))?;
    let block = &text[start..finish];
    let mut out = String::with_capacity(block.len() + value.len());
    let mut replaced = false;
    let mut header_end = None;
    for line in block.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            out.push_str(line);
            header_end = Some(out.len());
            continue;
        }
        if !replaced && trimmed.split_once('=').is_some_and(|(k, _)| k.trim().trim_matches('"') == field) {
            out.push_str(&format!("{field} = {}\n", toml_string(value)));
            replaced = true;
            continue;
        }
        out.push_str(line);
    }
    if !replaced {
        let at = header_end.ok_or_else(|| format!("no quicklink called {key}"))?;
        out.insert_str(at, &format!("{field} = {}\n", toml_string(value)));
    }
    let mut whole = text.to_string();
    whole.replace_range(start..finish, &out);
    Ok(whole)
}

/// Rename the keyword, leaving the target and everything else alone.
pub(crate) fn rename_quicklink_key(text: &str, old: &str, new: &str) -> Result<String, String> {
    let new = normalize_quicklink_key(new)?;
    if let Some(why) = quicklink_conflict(&new) {
        return Err(why);
    }
    let links = crate::minitoml::parse(text);
    if fold_key(old) != fold_key(&new) {
        if let Some((existing, _)) = quicklink_entry(&links, &new) {
            return Err(format!("a quicklink called {existing} already exists"));
        }
    }
    let (start, finish) =
        quicklink_span(text, old).ok_or_else(|| format!("no quicklink called {old}"))?;
    let block = text[start..finish]
        .replace(&quicklink_marker(old, false), &quicklink_marker(&new, false))
        .replace(&quicklink_marker(old, true), &quicklink_marker(&new, true));
    let mut out = String::with_capacity(block.len());
    let mut done = false;
    for line in block.split_inclusive('\n') {
        let trimmed = line.trim();
        if !done && trimmed.starts_with('[') && trimmed.ends_with(']') {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(&format!("{indent}[{new}]\n"));
            done = true;
            continue;
        }
        out.push_str(line);
    }
    if !done {
        return Err(format!("no quicklink called {old}"));
    }
    let mut whole = text.to_string();
    whole.replace_range(start..finish, &out);
    Ok(whole)
}

pub fn rename_quicklink(old: &str, new: &str) -> Result<String, String> {
    let _lock = crate::cache::lock_for_write(&quicklinks_file());
    let old = normalize_quicklink_key(old)?;
    let new = normalize_quicklink_key(new)?;
    let text = rename_quicklink_key(&read_for_write()?, &old, &new)?;
    write_quicklinks(&text)?;
    Ok(new)
}

/// Point an existing quicklink somewhere else. A `{q}` value re-points a
/// template; anything else is resolved and stored the way creation would.
pub fn retarget_quicklink(key: &str, target: &str) -> Result<String, String> {
    let key = normalize_quicklink_key(key)?;
    let (kind, stored) = resolve_quicklink_target(target)?;
    // Read, change, write, like every other mutator here — and it reads twice,
    // since the `url = ` cleanup below re-parses what it just changed.
    let _lock = crate::cache::lock_for_write(&quicklinks_file());
    let text = read_for_write()?;
    let text = set_quicklink_field(&text, &key, "target", &stored)?;
    // The old entry may still carry a `url = ` line from a hand-written file;
    // leaving both would let the two disagree about where the keyword goes.
    let text = match quicklink_entry(&crate::minitoml::parse(&text), &key) {
        Some((_, body)) if body.contains_key("url") => {
            set_quicklink_field(&text, &key, "url", &stored)?
        }
        _ => text,
    };
    let text = set_quicklink_field(&text, &key, "kind", kind)?;
    write_quicklinks(&text)?;
    Ok(stored)
}

pub fn rename_quicklink_label(key: &str, name: &str) -> Result<(), String> {
    let _lock = crate::cache::lock_for_write(&quicklinks_file());
    let key = normalize_quicklink_key(key)?;
    let name = crate::width::flatten(name.trim());
    if name.is_empty() {
        return Err("give it a name to show in the list".into());
    }
    let text = set_quicklink_field(&read_for_write()?, &key, "name", &name)?;
    write_quicklinks(&text)
}

/// What a typed target is, and the form it should be stored in.
pub fn resolve_quicklink_target(raw: &str) -> Result<(&'static str, String), String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("give it something to point at".into());
    }
    if raw.contains("{q}") {
        return template_draft("x", raw).map(|d| (d.kind, d.target));
    }
    if crate::secrets::looks_secret(raw) || url_has_secret(raw) {
        return Err("that target appears to contain a credential and will not be indexed".into());
    }
    if let Some(url) = web_url(raw) {
        return Ok(("url", url));
    }
    let real = crate::settings::readings_of(raw)
        .into_iter()
        .find_map(|candidate| candidate.canonicalize().ok())
        .ok_or_else(|| "that target is not there".to_string())?;
    let kind = if real.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")) {
        "app"
    } else if real.is_dir() {
        "folder"
    } else {
        "file"
    };
    Ok((kind, crate::paths::tilde(&real.to_string_lossy())))
}

/// `prelude quicklink …` — the same guards, without a terminal.
///
/// Creation was reachable only through an fzf prompt, which meant quicklinks
/// could not be scripted, synced, backed up or exercised by a test, and every
/// guard in this file had to be verified by pressing keys. `settings
/// add-root` exists for exactly this reason; this is the door that was
/// missing beside it.
pub fn quicklink_cli(args: &[&str]) -> i32 {
    fn fail(e: impl std::fmt::Display) -> i32 {
        eprintln!("prelude: {e}");
        2
    }
    match args {
        [] | ["list"] | ["ls"] => {
            // A file that is there and unreadable must say so rather than
            // let the built-in fallback pass for the person's own list.
            if let Err(e) = ensure_quicklinks_file() {
                return fail(e);
            }
            let rows = quicklink_scope_rows();
            if rows.is_empty() {
                println!("no quicklinks yet — ^K on any file, folder, app or URL creates one");
                return 0;
            }
            let w = rows.iter().map(|it| crate::width::dwidth(it.get("quicklink"))).max().unwrap_or(4);
            for it in rows {
                let target = if it.get("quicklink_target").is_empty() {
                    it.fields.get(1).cloned().unwrap_or_default()
                } else {
                    it.get("quicklink_target").to_string()
                };
                // `dwidth`, not character count: a CJK keyword is two columns
                // wide and `{:<w$}` would put the next column somewhere else.
                println!(
                    "  {}  {}  {}",
                    crate::width::pad_to(it.get("quicklink"), w, false),
                    crate::width::pad_to(it.get("quicklink_kind"), 8, false),
                    target,
                );
            }
            0
        }
        ["list", "--json"] | ["--json"] => {
            // A file that is there and unreadable must say so rather than
            // let the built-in fallback pass for the person's own list.
            if let Err(e) = ensure_quicklinks_file() {
                return fail(e);
            }
            match serde_json::to_string_pretty(&quicklink_scope_rows()) {
                Ok(json) => {
                    println!("{json}");
                    0
                }
                Err(e) => fail(e),
            }
        }
        ["add", key, target, name @ ..] => {
            let (kind, stored) = match resolve_quicklink_target(target) {
                Ok(v) => v,
                Err(e) => return fail(e),
            };
            let label = if name.is_empty() {
                std::path::Path::new(&stored)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| (*key).to_string())
            } else {
                name.join(" ")
            };
            let draft = QuicklinkDraft { name: label, kind, target: stored };
            match create_quicklink_from(key, &draft) {
                Ok(key) => {
                    let how = if draft.is_template() {
                        format!("type “{key} something”")
                    } else {
                        format!("type “{key}”")
                    };
                    println!("added {key} -> {} · {how}", draft.target);
                    0
                }
                Err(e) => fail(e),
            }
        }
        ["rm", key] | ["remove", key] => match remove_quicklink(key) {
            Ok(()) => {
                println!("removed {key}");
                0
            }
            Err(e) => fail(e),
        },
        ["rename", old, new] => match rename_quicklink(old, new) {
            Ok(new) => {
                println!("{old} -> {new}");
                0
            }
            Err(e) => fail(e),
        },
        ["set-name", key, name @ ..] if !name.is_empty() => {
            match rename_quicklink_label(key, &name.join(" ")) {
                Ok(()) => {
                    println!("{key} now reads “{}”", name.join(" "));
                    0
                }
                Err(e) => fail(e),
            }
        }
        ["set-target", key, target] => match retarget_quicklink(key, target) {
            Ok(stored) => {
                println!("{key} -> {stored}");
                0
            }
            Err(e) => fail(e),
        },
        ["check"] => {
            if let Err(e) = ensure_quicklinks_file() {
                return fail(e);
            }
            let problems = quicklink_problems();
            if problems.is_empty() {
                println!("all quicklinks resolve");
                return 0;
            }
            for (key, why) in &problems {
                println!("  {key}: {why}");
            }
            1
        }
        _ => {
            eprintln!("prelude quicklink list [--json]");
            eprintln!("prelude quicklink add KEY TARGET [NAME...]   TARGET is a path, a URL, or a URL with {{q}}");
            eprintln!("prelude quicklink rm KEY");
            eprintln!("prelude quicklink rename OLD NEW");
            eprintln!("prelude quicklink set-name KEY NAME...");
            eprintln!("prelude quicklink set-target KEY TARGET");
            eprintln!("prelude quicklink check");
            2
        }
    }
}

/// Every entry that will not do what its author meant, with the reason.
///
/// `doctor` used to print the bare list of keys, which says a quicklink exists
/// and nothing about whether it works — a missing target, an unusable keyword
/// and a keyword the search box has already spent all looked identical to a
/// healthy one.
pub fn quicklink_problems() -> Vec<(String, String)> {
    quicklink_problems_in(&quicklinks_text())
}

pub(crate) fn quicklink_problems_in(text: &str) -> Vec<(String, String)> {
    let links = crate::minitoml::parse(text);
    let mut out = Vec::new();
    for (stored, body) in &links {
        let key = fold_key(stored);
        if !valid_quicklink_key(&key) {
            out.push((stored.clone(), "not typeable as a keyword".to_string()));
            continue;
        }
        if let Some(why) = quicklink_conflict(&key) {
            out.push((key, why));
            continue;
        }
        let Some(target) = body.get("target").or_else(|| body.get("url")) else {
            out.push((key, "no target or url".to_string()));
            continue;
        };
        if target.contains("{q}") {
            if web_url(&target.replace("{q}", "prelude")).is_none() {
                out.push((key, format!("not a usable URL: {target}")));
            }
            continue;
        }
        match fixed_quicklink_from(text, &key) {
            Some(item) if item.get("quicklink_missing") == "true" => {
                out.push((key, format!("target is gone: {}", item.get("quicklink_target"))))
            }
            Some(_) => {}
            None => out.push((key, format!("unrecognised kind or target: {target}"))),
        }
    }
    out
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
///
/// The row keeps the target's Kind, because that is what Enter and `^K` must
/// act on — a quicklink to a file is opened by the application that owns it,
/// exactly as the file itself would be. Only the band and the label come from
/// its being a quicklink; see `Item::quicklink`.
pub(crate) fn fixed_quicklink_from(text: &str, q: &str) -> Option<Item> {
    let links = crate::minitoml::parse(text);
    let (stored, body) = quicklink_entry(&links, q)?;
    let key = fold_key(&stored);
    if !valid_quicklink_key(&key) {
        return None;
    }
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
        .quicklink(&key, "fixed")
        .put("quicklink_managed", managed)
        .put("quicklink_kind", kind)
        .put("quicklink_missing", if missing { "true" } else { "" })
        .put("quicklink_target", target))
}

/// Quicklinks with arguments are commands before they are links. Keeping
/// them in the searchable catalogue makes `g` and `google` reveal what is
/// available instead of looking like failed fuzzy searches.
pub(crate) fn quicklink_items_from(text: &str) -> Vec<Item> {
    let links = crate::minitoml::parse(text);
    let mut out = Vec::with_capacity(links.len());
    for (stored, body) in &links {
        let key = fold_key(stored);
        if !valid_quicklink_key(&key) {
            continue;
        }
        let Some(target) = body.get("target").or_else(|| body.get("url")) else { continue };
        if target.contains("{q}") {
            let name = body.get("name").cloned().unwrap_or_else(|| key.clone());
            let title = if name.to_lowercase().starts_with("search ") {
                name
            } else {
                format!("Search {name}")
            };
            out.push(
                Item::new(key.clone(), Kind::Search)
                    .title(title)
                    .fields([format!("{key} <query>"), dtrunc_template(target)])
                    .quicklink(&key, "template")
                    .put("mode", "complete-query")
                    .put("completion", format!("{key} "))
                    .put("provider", key.clone())
                    .put("quicklink_managed", managed_quicklink(text, &key).to_string())
                    .put("quicklink_kind", "template")
                    .put("quicklink_target", target.clone())
                    .put("desc", "type a search term"),
            );
        } else if let Some(item) = fixed_quicklink_from(text, &key) {
            out.push(item);
        }
    }
    out
}

/// The host of a template, which is what identifies it — the rest is
/// boilerplate that would push the keyword out of the column.
fn dtrunc_template(target: &str) -> String {
    let rest = target
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(target);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if host.is_empty() { "search".to_string() } else { host.to_string() }
}

pub fn quicklink_items() -> Vec<Item> {
    // The one caller already doing a launch's worth of work, and therefore
    // where the file gets created and migrated. See `ensure_quicklinks_file`.
    let _ = ensure_quicklinks_file();
    quicklink_items_from(&quicklinks_text())
}

pub(crate) fn exact_quicklink_key_from(text: &str, q: &str) -> bool {
    if !valid_quicklink_key(&fold_key(q)) {
        return false;
    }
    quicklink_entry(&crate::minitoml::parse(text), q)
        .is_some_and(|(_, body)| body.contains_key("target") || body.contains_key("url"))
}

pub(crate) fn search_provider_from(text: &str, q: &str) -> Option<Item> {
    let want = q.trim().to_lowercase();
    let by = |name_match: bool| {
        quicklink_items_from(text).into_iter().find(|it| {
            if it.kind != Kind::Search || it.get("provider").is_empty() {
                return false;
            }
            if name_match {
                let name = it.title.strip_prefix("Search ").unwrap_or(&it.title);
                name.to_lowercase() == want
            } else {
                it.get("provider") == want
            }
        })
    };
    by(false).or_else(|| by(true))
}

/// Every entry in the file, including the ones that do not work.
///
/// The ordinary catalogue silently drops an entry whose kind it does not
/// recognise or whose key is not usable, which is correct for a search result
/// and wrong for the one screen whose job is to let you repair it. A broken
/// entry appears here, says what is wrong with it, and carries the same
/// rename, re-point and remove actions as a working one.
pub fn quicklink_scope_rows() -> Vec<Item> {
    let text = quicklinks_text();
    let links = crate::minitoml::parse(&text);
    let working = quicklink_items_from(&text);
    let mut out = Vec::with_capacity(links.len());
    for (stored, body) in &links {
        let key = fold_key(stored);
        if let Some(item) = working.iter().find(|it| it.get("quicklink") == key) {
            out.push(item.clone());
            continue;
        }
        let why = if !valid_quicklink_key(&key) {
            format!("⚠ “{stored}” cannot be typed as a keyword")
        } else if body.get("target").or_else(|| body.get("url")).is_none() {
            "⚠ no target or url".to_string()
        } else if let Some(reason) = quicklink_conflict(&key) {
            format!("⚠ {reason}")
        } else {
            format!("⚠ unknown kind “{}”", body.get("kind").map(String::as_str).unwrap_or(""))
        };
        let path = quicklinks_file().to_string_lossy().into_owned();
        out.push(
            Item::new(path.clone(), Kind::Config)
                .title(body.get("name").cloned().unwrap_or_else(|| stored.clone()))
                .fields([key.clone(), why.clone()])
                .quicklink(&key, "fixed")
                .put("path", path)
                .put("quicklink_managed", managed_quicklink(&text, &key).to_string())
                .put("quicklink_broken", why),
        );
    }
    out
}

/// An exact keyword, in the order the person would expect.
///
/// A key the person typed beats a *name* another entry happens to carry.
/// It used to be the other way round, and the effect was silent: a fixed
/// `[google]` pointing at somebody's own profile was never reachable, because
/// `[g]`'s display name is "Google" and the name match was tried first. The
/// row existed, `doctor` listed it, and typing its keyword opened a different
/// thing entirely.
/// An exact alias leads the list; it does not clear the room.
///
/// It used to be the only row: `is_special` is true for an exact key, so the
/// catalogue underneath was suppressed entirely. One keystroke turned two
/// sensible candidates into one — typing `github` made the `Search GitHub`
/// that was on screen at `githu` disappear, with nothing saying it had been
/// decided against. The keyword still wins, because it is the one thing on
/// screen the person definitely meant; everything else that matches simply
/// follows it.
///
/// Leaving it to fzf instead is not an option, and this was measured rather
/// than assumed: with the whole root list and `--tiebreak=index`, `github`
/// does rank the quicklink first, but `google` loses to an MCP server, and
/// `g` and `b` lose to skills. An alias that wins only when its name is long
/// and unusual is not an alias.
pub(crate) fn quicklink_with_neighbours(exact: Item, q: &str, static_items: &[Item]) -> Vec<Item> {
    let key = exact.get("quicklink").to_string();
    // The same quicklink is in the catalogue too; it belongs on screen once,
    // at the top, as the row the exact match resolved to. Identity is checked
    // both ways `finish` checks it — by key, and by `(kind, cmd)` — because a
    // catalogue cached by an older build carries neither the key nor anything
    // else new, and a duplicated row is exactly what an upgrade would show.
    let same = (exact.kind, exact.cmd.clone());
    let mut rows = vec![exact];
    rows.extend(
        root_items(static_items)
            .into_iter()
            .filter(|it| it.get("quicklink") != key && (it.kind, it.cmd.clone()) != same)
            .filter(|it| matches_terms(it, q))
            .take(50),
    );
    rows
}

/// The object an exactly-typed alias names, leading the rows that still match.
///
/// The same two rules as `quicklink_with_neighbours`, for the same reason: an
/// exact name leads the list, and it does not clear the room — the catalogue
/// underneath keeps whatever else the letters match, so completing a name
/// never deletes a candidate that was on screen a keystroke ago.
///
/// An alias whose object is not in this gather resolves to nothing and the
/// query falls through to ordinary search. That is deliberate: the alternative
/// is a row standing in for an application that has been uninstalled, and a
/// launcher that shows what is not there is worse than one that shows less.
/// `alias:` is where a name with nothing behind it should be explained, and
/// that screen does not exist yet.
fn alias_rows(q: &str, static_items: &[Item]) -> Option<Vec<Item>> {
    let target = crate::aliases::target_of(q)?;
    let exact = static_items
        .iter()
        .find(|it| crate::favorites::key(it).as_deref() == Some(target.as_str()))?
        .clone();
    // Identity both ways `finish` checks it, as `quicklink_with_neighbours`
    // does: by the name and by `(kind, cmd)`. The named row belongs on screen
    // once, at the top, as the row the name resolved to.
    let alias = exact.get("alias").to_string();
    let same = (exact.kind, exact.cmd.clone());
    let mut rows = vec![exact];
    rows.extend(
        root_items(static_items)
            .into_iter()
            .filter(|it| (it.kind, it.cmd.clone()) != same)
            .filter(|it| alias.is_empty() || it.get("alias") != alias)
            .filter(|it| matches_terms(it, q))
            .take(50),
    );
    Some(rows)
}

fn exact_quicklink_item(q: &str) -> Option<Item> {
    let text = quicklinks_text();
    if exact_quicklink_key_from(&text, q) {
        if let Some(item) = fixed_quicklink_from(&text, q) {
            return Some(item);
        }
    }
    search_provider_from(&text, q)
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
    Config,
    Settings,
    /// The keywords the person saved. Everything else in this list is a source
    /// Prelude discovered; this is the one the person wrote, and it was the
    /// only one with nowhere to be seen as a whole — the sole way to answer
    /// "what quicklinks do I have" was to open the TOML.
    Quicklinks,
    /// Text on its way to being a clear English prompt.
    Prompt,
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
    ScopeDef { scope: Scope::Files, prefix: "f:", title: "Files & Folders", desc: "current project and indexed roots" },
    ScopeDef { scope: Scope::Clipboard, prefix: "c:", title: "Clipboard History", desc: "recent copied text" },
    ScopeDef { scope: Scope::History, prefix: "h:", title: "Shell History", desc: "recent commands" },
    ScopeDef { scope: Scope::Applications, prefix: "app:", title: "Applications", desc: "installed macOS apps" },
    ScopeDef { scope: Scope::Commands, prefix: "cmd:", title: "Commands", desc: "$PATH and system commands" },
    ScopeDef { scope: Scope::Directories, prefix: "dir:", title: "Folders", desc: "indexed, frequent and recent folders" },
    ScopeDef { scope: Scope::Project, prefix: "proj:", title: "Current Project", desc: "scripts, files and git" },
    ScopeDef { scope: Scope::Ssh, prefix: "ssh:", title: "SSH Hosts", desc: "~/.ssh/config" },
    ScopeDef { scope: Scope::Snippets, prefix: "snip:", title: "Snippets", desc: "saved command templates" },
    ScopeDef { scope: Scope::Ports, prefix: "port:", title: "Listening Ports", desc: "local TCP listeners" },
    ScopeDef { scope: Scope::Processes, prefix: "proc:", title: "Processes", desc: "CPU and memory consumers" },
    ScopeDef { scope: Scope::Containers, prefix: "docker:", title: "Containers", desc: "running Docker containers" },
    ScopeDef { scope: Scope::Config, prefix: "cfg:", title: "Agent Config", desc: "settings and instruction files" },
    ScopeDef { scope: Scope::Quicklinks, prefix: "ql:", title: "Quicklinks", desc: "keywords you saved yourself" },
    // With nothing after it, the clipboard is the subject: copy, press the
    // chord, `p:`, Enter. That is the whole of what PromptConverter's menu-bar
    // "Convert Clipboard" was, minus the menu bar.
    ScopeDef { scope: Scope::Prompt, prefix: "p:", title: "Rewrite as a Prompt", desc: "clipboard or typed text, into clear English" },
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

pub(crate) fn exact_scope_command(q: &str) -> Option<Item> {
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

/// What Ctrl+R does to the query it finds: into `h:`, and back out.
///
/// The key that opened the launcher spent decades meaning "search my shell
/// history", and the fingers that press it have not been told otherwise —
/// they press it, see a list, and press it again, which at a shell walked to
/// the next match. So a second press moves the typed text into the history
/// scope: `git commit` becomes `h:git commit`, and the search the person
/// actually meant runs over the three thousand commands root search
/// deliberately excludes. A third press carries the text back out.
///
/// A query already in some *other* scope switches scope rather than nesting:
/// `f:serve` becomes `h:serve`, because "search this in history instead" is
/// the whole meaning of the key, and `h:f:serve` is a question with an empty
/// answer.
pub fn history_toggle(q: &str) -> String {
    let t = q.trim_start();
    if let Some((scope, term)) = scope_query(t) {
        if scope == Scope::History {
            return term.to_string();
        }
        return format!("h:{term}");
    }
    format!("h:{t}")
}

pub fn needs_static_items(q: &str) -> bool {
    let t = q.trim();
    scope_query(t).is_some()
        // Ordinary name search also includes the current project's live file
        // rows, so a file created a moment ago does not wait for the shared
        // index. Parsing the cached snapshot costs no filesystem walk.
        || (t.chars().count() >= 2 && !is_special(t))
        || t.starts_with('/')
        || (t.starts_with('@') && !t.chars().any(char::is_whitespace))
        // An exact alias needs the catalogue so the rows it leads can come
        // with it. This is a config read on the handful of keystrokes that
        // complete a keyword, not on every keystroke.
        || exact_quicklink_item(t).is_some()
        // A named object needs it for a stronger reason: a quicklink stores
        // its target beside the key, but an alias stores only an identity and
        // the row itself lives in the catalogue.
        || crate::aliases::target_of(t).is_some()
}

fn skill_prefix_rows(q: &str, static_items: &[Item]) -> Option<Vec<Item>> {
    let rest = q.trim().strip_prefix('/')?;
    let name = rest.split_whitespace().next().unwrap_or(rest);
    if static_items.iter().any(|it| {
        it.kind == Kind::Skill
            && crate::archive::visible(it)
            && it.title.eq_ignore_ascii_case(name)
    }) {
        return None;
    }
    let want = rest.to_lowercase();
    Some(static_items.iter()
        .filter(|it| {
            it.kind == Kind::Skill
                && crate::archive::visible(it)
                && it.title.to_lowercase().contains(&want)
        })
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

/// The empty query: the two things you open this to reach.
///
/// **Skills and Sessions**, and nothing else. Both are things you *act on* —
/// a Skill's Enter hands you the sentence that runs it in whatever Agent you
/// have open, and a Session's resumes a conversation. Everything that used to
/// sit beside them was a *readout*: how many Skills `claude` has, that a Run is
/// working, that an MCP server is connected. All true, none of it a reason to
/// have opened a launcher.
///
/// The screen this replaces held five kinds and around thirty rows, of which
/// thirteen were inventory counts and health states. Reading them was the whole
/// interaction; there was nothing to press Enter on. So they are not deleted —
/// `a:`, `r:` and `mcp:` still hold them and root search still matches an Agent
/// or a server by name — they are just not what an empty query is for.
///
/// A blocked question stays, and it is the one deliberate addition to that
/// pair. `Kind::Msg` appears *only* while an Agent is sitting on `prelude ask`
/// waiting for a person, so it costs nothing on an ordinary open and its
/// absence would mean nobody ever sees it: the Agent then blocks for its whole
/// timeout while the answer sits behind a scope prefix nobody had a reason to
/// type. That is not an inventory row; it is an interruption, and it is the
/// one thing here the person did not go looking for.
///
/// This screen was also briefly the opposite mistake — an attention list, where
/// healthy Skills and servers were hidden so only exceptions reached it. That
/// was wrong because it went quiet exactly when nothing was broken. The rule
/// now is neither "everything" nor "only problems": it is *what has an action
/// behind it*.
///
/// Ordering is `cache::by_rank` like everywhere else, so the bands do the work
/// and this screen has no second ordering rule of its own.
///
/// Sessions are the exception that has to be counted rather than filtered:
/// there are hundreds of them, `gather` puts only the newest
/// `sessions::IN_MAIN_LIST` in the list at all, and `s:` owns the rest.
pub fn home_items(items: &[Item]) -> Vec<Item> {
    let mine: Vec<Item> = items
        .iter()
        .filter(|it| {
            (matches!(it.kind, Kind::Msg | Kind::Skill | Kind::Session)
                && crate::archive::visible(it))
                // A newer release, on the rare day there is one. It is one row,
                // it appears only while there is something to do about it, and
                // it goes away when the update is taken — which is a different
                // thing from the attention-list mistake this screen already
                // made once, where healthy objects were *hidden* to leave only
                // exceptions.
                || it.get("update") == "available"
        })
        .cloned()
        .collect();
    // `items.is_empty()` is the difference between a person who has nothing
    // yet and a gather that failed, and they must not render the same. A real
    // first run still returns hundreds of rows — the applications, the `$PATH`
    // commands, the scope commands — so an empty catalogue means the sources
    // did not answer, and greeting somebody who has fifty Skills with "start
    // here" because one gather fell over is the `Vec<Item>` mistake this
    // codebase already made once, on a screen where it is unmissable.
    if mine.is_empty() && !items.is_empty() { first_run_rows() } else { mine }
}

/// Where the guide lives, for the one row that has nothing local to point at.
const GUIDE_URL: &str = "https://github.com/mikewang817/Prelude#readme";

/// What the home shows before the person has anything of their own on it.
///
/// The two kinds this screen is made of are earned rather than installed: a
/// Skill arrives when you write one, a Session when you have talked to an
/// agent, and a Msg when one asks you something. So a correct `home_items`
/// returns nothing on a machine where none of that has happened yet, and fzf
/// draws an empty rectangle — which is the `搜索用不了` failure exactly, and
/// worse placed than the one `fallback_rows` already fixes. That one needs an
/// unlucky query; this one is guaranteed, it happens to everybody once, and it
/// happens at the only moment when nobody yet has a reason to believe the
/// launcher works at all. First impression is the whole of it: an empty box
/// says the thing is broken, and the person who dismisses it has no second
/// question to ask.
///
/// These are ordinary rows and deliberately not a new kind of one. Two are the
/// scope commands `scope_commands` already builds, so Enter completes them,
/// the footer describes them and `^K` acts on them exactly as it does
/// everywhere else — an onboarding row that behaved specially would be
/// teaching a launcher that stops being true the moment it has taught it.
/// Which two, and what they say rather than showing, is `FIRST_RUN`. Leading
/// somebody's first press into `skill:` or `s:` would answer an empty screen
/// with a second empty screen, and those are the two the person is here to be
/// told they do not have yet.
fn first_run_rows() -> Vec<Item> {
    let mut rows = vec![
        Item::new(GUIDE_URL, Kind::Link)
            .title("Start here — how Prelude works")
            .fields(["skills and conversations appear on this screen".to_string()]),
    ];
    rows.extend(FIRST_RUN.iter().filter_map(|(prefix, says)| {
        SCOPES
            .iter()
            .find(|d| d.prefix == *prefix)
            .map(|d| scope_item(d).fields([says.to_string()]))
    }));
    rows
}

/// The two destinations offered before anything has been earned, and what
/// each row says instead of naming its scope.
///
/// A scope command's own detail column is its prefix — `c:`, `set:` — which is
/// exactly right in root search, where the person typed something and is being
/// shown how to narrow it. On this screen it is the first thing anybody sees of
/// Prelude, and `c:` is not language. The row keeps its command, its
/// `complete-query` mode and its behaviour; only what it *reads* changes, so
/// the prefix is still what Enter puts in the query and still what teaches the
/// shorthand — one keystroke later, at the moment it becomes useful.
///
/// Two rather than four. `f:` and `app:` were here and are the same mistake as
/// `skill:` in slower motion: the file index is built in the background after
/// a first install, so `f:` on a brand-new machine is a scope that is briefly
/// empty for a reason nobody can see, and `app:` answers a question the person
/// already has Spotlight for. What is left is the pair worth knowing on day
/// one — the clipboard, which is the thing this launcher has that the built-in
/// one does not, and settings, which is where the folders it searches are
/// chosen.
const FIRST_RUN: &[(&str, &str)] = &[
    ("c:", "everything you have copied, newest first"),
    ("set:", "the folders searched, the hotkey, the keys"),
];

/// What an ordinary root query may fuzzy-match. Large and private sources
/// are commands here, not thousands of eager rows: `f` finds Files & Folders,
/// and `f:` opens that scope. A separate per-query helper mixes in only the
/// best ten filesystem matches. Fixed Quicklinks remain root commands even when
/// their target happens to be a file or application.
///
/// Sessions are here too, and that is the change this paragraph exists for.
/// They were held back because there are hundreds of them — which kept the
/// catalogue small and made the launcher unable to answer its most ordinary
/// question about them. Typing a project name found agents, skills and files
/// and none of the conversations you had *in that project*; the fifteen on the
/// home disappeared at the first keystroke, and `s:` was a prefix you had to
/// know rather than a way to narrow. `Item::band` keeps them below Skill and
/// Agent, so they extend the answer rather than displacing it.
pub fn root_items(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|it| {
            (matches!(
                it.kind,
                Kind::Msg | Kind::Agent | Kind::Run | Kind::Skill | Kind::Session | Kind::Search
            ) && crate::archive::visible(it))
                || it.is_quicklink()
                || it.get("update") == "available"
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CapabilityArchiveView {
    Visible,
    Archived,
    All,
}

/// `skill:` and `mcp:` keep archived objects out until explicitly asked for.
/// Unknown `is:` words remain search needles, so a typo collapses the list
/// instead of silently widening it to every capability.
fn capability_archive_filter(term: &str) -> (CapabilityArchiveView, String) {
    let mut view = CapabilityArchiveView::Visible;
    let mut needles = Vec::new();
    for word in term.split_whitespace() {
        match word.to_ascii_lowercase().as_str() {
            "is:archived" => view = CapabilityArchiveView::Archived,
            "is:all" => view = CapabilityArchiveView::All,
            _ => needles.push(word),
        }
    }
    (view, needles.join(" "))
}

/// `p:` — one row, describing work that has not happened yet.
///
/// Deliberately one and not one per profile. Three rows differing only by a
/// name in the middle column is the "two rows for one intent" that `/` already
/// refuses; the other profiles are in `^K`, where every other per-row choice in
/// this launcher lives. It also keeps `cmd` unique without encoding a profile
/// into it, which `finish`'s `(kind, cmd)` dedupe would otherwise collapse.
///
/// **Nothing here reaches the network.** The row states what would be sent and
/// where; Enter is what sends it. `scoped_rows` runs on every keystroke.
fn prompt_rows(term: &str, static_items: &[Item]) -> Vec<Item> {
    prompt_rows_with(&crate::rewrite::config(), term, static_items)
}

/// The rule, with the configuration handed in — for the reason `surface()`
/// gives: a rule that reads preferences to decide cannot be walked in a test,
/// and every branch here is one a person actually meets.
fn prompt_rows_with(cfg: &crate::rewrite::Config, term: &str, static_items: &[Item]) -> Vec<Item> {
    // Not configured yet, so hand over the controls rather than an explanation
    // of them. These are the real setting rows: `←`, `→` and Enter all work
    // here exactly as they do in `set:`.
    if cfg.provider == crate::rewrite::Provider::Off || cfg.model.trim().is_empty() {
        return crate::settings::items()
            .into_iter()
            .filter(|item| item.get("group") == "Rewrite")
            .collect();
    }

    let typed = term.trim();
    // With nothing typed the subject is the clipboard, taken from the snapshot
    // the launcher already gathered rather than from `pbpaste` — a subprocess
    // per keystroke to read a value that is already in memory.
    let (source, whence) = if typed.is_empty() {
        match static_items
            .iter()
            .find(|it| it.kind == Kind::Clip && it.get("clip_kind") == "text")
        {
            Some(clip) => {
                let full = clip.get("full");
                let text = if full.trim().is_empty() { clip.cmd.clone() } else { full.to_string() };
                (text, "from the clipboard")
            }
            None => {
                return vec![Item::new("p:", Kind::Search)
                    .title("Type the text to rewrite")
                    .sub("nothing on the clipboard to use instead")
                    .put("mode", "complete-query")
                    .put("completion", "p:")];
            }
        }
    } else {
        (typed.to_string(), "typed here")
    };

    if source.trim().is_empty() {
        return Vec::new();
    }
    let profile_name = crate::rewrite::profile(&cfg.profile_id)
        .map(|p| p.name.to_string())
        .unwrap_or_else(|| "Custom".to_string());
    // Where the text is about to go, said on the row rather than in Details.
    // On the default it is this machine; on any other endpoint it is somebody
    // else's, and that is not a fact to make a person open a submenu for.
    let where_to = if cfg.provider == crate::rewrite::Provider::Ollama
        && (cfg.url.contains("localhost") || cfg.url.contains("127.0.0.1"))
    {
        format!("{} · on this machine", cfg.model)
    } else {
        format!("{} · {}", cfg.model, host_of(&cfg.url))
    };
    vec![Item::new(source.clone(), Kind::Rewrite)
        .title(crate::width::flatten(&source))
        .fields([profile_name.clone(), where_to.clone(), whence.to_string()])
        .put("rw_source", source)
        .put("rw_profile", cfg.profile_id.clone())
        .put("rw_profile_name", profile_name)
        .put("rw_model", cfg.model.clone())
        .put("rw_where", where_to)]
}

/// The host of a URL, for saying where text is about to be sent.
pub fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
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
        ensure_fileindex();
        let mut rows: Vec<Item> = static_items
            .iter()
            .filter(|item| item.kind == File)
            .filter_map(|item| scored_filesystem_item(item.clone(), term, 40))
            .collect();
        match search_fileindex(term) {
            Some(hits) => rows.extend(hits),
            None if rows.is_empty() => rows.push(
                Item::new("prelude index", Find)
                    .title("⚠ file index not built")
                    .sub("file search is being prepared"),
            ),
            None => {}
        }
        return finish_filesystem_rows(rows, SCOPED_FILE_RESULT_LIMIT);
    }
    if scope == Scope::Directories {
        ensure_fileindex();
        // zoxide and recent `cd` targets are a ranking signal, not a separate
        // definition of what a folder search is. Indexed folders fill the
        // catalogue; a folder the person actually uses wins otherwise equal
        // name matches.
        let mut rows: Vec<Item> = static_items
            .iter()
            .filter(|item| item.kind == Dir)
            .filter_map(|item| scored_filesystem_item(item.clone(), term, 400))
            .collect();
        if let Some(hits) = search_fileindex_kind(term, Some(IndexedKind::Folder), SCOPED_FILE_RESULT_LIMIT) {
            rows.extend(hits);
        }
        return finish_filesystem_rows(rows, SCOPED_FILE_RESULT_LIMIT);
    }
    if scope == Scope::Agent {
        // The control scope, and the only one with a filter vocabulary of its
        // own. Session is deliberately absent: `s:` owns the hundreds of old
        // conversations, and putting them here made the agent overview a
        // session browser.
        let (filters, needles) = parse_agent_filters(term);
        return static_items
            .iter()
            .filter(|it| matches!(it.kind, Msg | Agent | Run | Skill | Config))
            .filter(|it| crate::archive::visible(it))
            .filter(|it| matches_agent_filters(it, &filters, &needles))
            .take(200)
            .cloned()
            .collect();
    }
    if scope == Scope::Quicklinks {
        let mut rows = quicklink_scope_rows();
        rows.retain(|it| matches_terms(it, term));
        rows.sort_by(crate::cache::by_rank);
        rows.truncate(200);
        return rows;
    }
    if scope == Scope::Settings {
        // Settings are Prelude's own tiny live form, not an inventory source.
        // Reading them from the launch snapshot left old values and even old
        // row structure on screen until the whole panel refreshed.
        let group = term.trim().to_ascii_lowercase();
        let exact_group = match group.as_str() {
            "search" => Some("Search"),
            "launcher" => Some("Launcher"),
            "behavior" | "behaviour" => Some("Behavior"),
            "rewrite" => Some("Rewrite"),
            "library" => Some("Library"),
            _ => None,
        };
        return crate::settings::items()
            .into_iter()
            .filter(|item| {
                exact_group
                    .map(|group| item.get("group") == group)
                    .unwrap_or_else(|| matches_terms(item, term))
            })
            .collect();
    }
    if scope == Scope::Skills {
        let kind = Skill;
        let (archive_view, needles) = capability_archive_filter(term);
        return static_items
            .iter()
            .filter(|item| item.kind == kind)
            .filter(|item| match archive_view {
                CapabilityArchiveView::Visible => crate::archive::visible(item),
                CapabilityArchiveView::Archived => item.get("archived") == "true",
                CapabilityArchiveView::All => true,
            })
            .filter(|item| matches_terms(item, &needles))
            .take(200)
            .cloned()
            .collect();
    }
    if scope == Scope::Prompt {
        return prompt_rows(term, static_items);
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
        Scope::Skills => false,
        Scope::Config => kind == Config,
        Scope::Settings => kind == Setting,
        Scope::Quicklinks | Scope::Prompt => false,
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
    let (_, body) = quicklink_entry(&links, key)?;
    let template = body.get("target").or_else(|| body.get("url"))?;
    if !template.contains("{q}") {
        return None;
    }
    let url = template.replace("{q}", &percent_encode(term));
    let name = body.get("name").cloned().unwrap_or_else(|| key.to_string());
    Some((url, name, term.to_string(), fold_key(key)))
}

pub fn quicklink(q: &str) -> Option<(String, String, String, String)> {
    quicklink_from(&quicklinks_text(), q)
}

fn is_quicklink_template(key: &str) -> bool {
    quicklink_entry(&quicklinks(), key)
        .and_then(|(_, body)| body.get("target").or_else(|| body.get("url")).cloned())
        .is_some_and(|target| target.contains("{q}"))
}

// ─── the web search that is always there ─────────────────────────────────

/// The keyword the always-on web search follows, and what it falls back to.
///
/// Following `[g]` rather than hard-coding Google is the point: somebody who
/// re-points that keyword at Baidu, Kagi or an intranet search has said where
/// their web searches should go, and a second search that ignored them would
/// be a launcher arguing with its own configuration. Deleting the keyword does
/// not delete the fallback, which is why the built-in template stays here.
pub const WEB_SEARCH_KEY: &str = "g";
pub const WEB_SEARCH_NAME: &str = "Google";
pub const WEB_SEARCH_TEMPLATE: &str = "https://www.google.com/search?q={q}";

/// A quicklink that can take a query, or nothing.
///
/// The `{q}` test is what makes a fallback list a list of *providers* rather
/// than of arbitrary rows: every fallback has to display the query, and a
/// fixed target has nowhere to put it.
pub(crate) fn template_provider(
    links: &crate::minitoml::Table,
    key: &str,
) -> Option<(String, String)> {
    quicklink_entry(links, key).and_then(|(key, body)| {
        let template = body.get("target").or_else(|| body.get("url"))?.clone();
        if !template.contains("{q}") {
            return None;
        }
        Some((body.get("name").cloned().unwrap_or(key), template))
    })
}

/// The row that turns any query into a web search, appended to every list.
///
/// A launcher whose search box answers *nothing at all* to `git commit` reads
/// as broken rather than as principled, and that is exactly what happened: the
/// root list is the agent inventory plus search commands and Quicklinks, so an
/// ordinary sentence matched none of it and fzf drew an empty box. Every other
/// row here has to be found; this one is computed from what was typed, so it
/// cannot fail to exist.
///
/// Three things keep it from being in the way. Its display text *is* the
/// query, so it fuzzy-matches by construction and needs no help to survive
/// filtering. It is emitted last, after the catalogue, so `--tiebreak=index`
/// leaves it below anything that scored the same. And it is absent inside a
/// scope — `f:`, `c:`, `h:` and the rest are a person saying where to look,
/// and answering "or the web" to that is not an answer.
pub fn fallback_rows(q: &str) -> Vec<Item> {
    fallback_rows_from(&quicklinks_text(), &crate::settings::fallbacks(), q)
}

/// `spec` is the person's ordered list of quicklink keywords.
///
/// Order is theirs and is preserved; a repeated keyword is taken once. A
/// keyword that names nothing, or names something with no `{q}` in it, is
/// dropped here and reported by `prelude settings check` — a fallback that
/// cannot carry the query would be a row that lies about what pressing it
/// does.
///
/// If nothing at all resolves, the built-in provider is emitted anyway. That
/// is the one property this row exists for: it must be impossible for a query
/// to dead-end, so an empty or broken list degrades to a working search rather
/// than to silence.
pub(crate) fn fallback_rows_from(text: &str, spec: &str, q: &str) -> Vec<Item> {
    let term = q.trim();
    // Nothing typed is the home screen, and the bare browsers (`:` for the
    // scope list, `/` for skills) are a keystroke on the way somewhere, not a
    // thing anybody wants looked up.
    if term.is_empty() || term == ":" || term == "/" || term == "@" {
        return Vec::new();
    }
    // A scope is a statement about where to look, so no web row inside one.
    // A query that merely *spells* a scope's name (`app`, `docker`) is an
    // ordinary root query and keeps its fallback like any other.
    if scope_query(term).is_some() {
        return Vec::new();
    }
    let links = crate::minitoml::parse(text);
    let mut seen = std::collections::BTreeSet::new();
    let mut providers: Vec<(String, String)> = Vec::new();
    for key in spec.split([',', ' ']).map(str::trim).filter(|key| !key.is_empty()) {
        if !seen.insert(key.to_lowercase()) {
            continue;
        }
        if let Some(provider) = template_provider(&links, key) {
            providers.push(provider);
        }
    }
    if providers.is_empty() {
        providers.push((WEB_SEARCH_NAME.to_string(), WEB_SEARCH_TEMPLATE.to_string()));
    }
    let encoded = percent_encode(term);
    providers
        .into_iter()
        .map(|(name, template)| {
            let url = template.replace("{q}", &encoded);
            Item::new(url.clone(), Kind::Link)
                // The query itself, not "Search Google for …": fzf matches
                // displayed text, and a row that has to be matched against a
                // prefix nobody typed is a row that disappears when it is
                // needed. Every provider in the list obeys this, not just the
                // first — otherwise the second fallback is invisible to the
                // query that produced it.
                .title(term)
                .fields([format!("Search {name}"), dtrunc_template(&template)])
                .put("url", url)
                .put("web_search", "true")
        })
        .collect()
}

// ─── indexed file and folder search ─────────────────────────────────────

/// Ordinary search gets enough local objects to answer a remembered name
/// without turning the launcher into a file browser. `f:` and `dir:` remain
/// available when the person explicitly asks for a longer list.
pub const ROOT_FILE_RESULT_LIMIT: usize = 10;
/// Applications are fewer and more sharply named than files, so a shorter
/// block says everything a query of this kind can mean.
pub const ROOT_APP_RESULT_LIMIT: usize = 5;
const SCOPED_FILE_RESULT_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IndexedKind {
    File,
    Folder,
}

impl IndexedKind {
    fn marker(self) -> &'static str {
        match self {
            Self::File => "F",
            Self::Folder => "D",
        }
    }

    fn data(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexCounts {
    pub files: usize,
    pub folders: usize,
}

struct IndexRecord<'a> {
    kind: IndexedKind,
    path: &'a str,
    tags: Vec<String>,
}

/// v2 records carry an explicit type. A line from the old, file-only index is
/// still read as a file so an upgrade never turns a working search blank
/// before the automatic rebuild has finished.
fn parse_index_record(record: &str) -> IndexRecord<'_> {
    let mut fields = record.split('\t');
    let first = fields.next().unwrap_or_default();
    let (kind, path, encoded) = match first {
        "F" => (IndexedKind::File, fields.next().unwrap_or_default(), fields.next()),
        "D" => (IndexedKind::Folder, fields.next().unwrap_or_default(), fields.next()),
        _ => (IndexedKind::File, first, fields.next()),
    };
    IndexRecord {
        kind,
        path,
        tags: encoded.and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default(),
    }
}

pub fn fileindex_path() -> std::path::PathBuf {
    paths::cache().join("fileindex.txt")
}

fn index_lock_path() -> std::path::PathBuf {
    paths::cache().join("fileindex.build")
}

pub fn index_building() -> bool {
    crate::cache::lock_is_held(&index_lock_path())
}

/// Search can ask for an index without waiting for it. The old index remains
/// available until the new one is atomically complete, and the held kernel
/// lock collapses every simultaneous request to one builder.
pub fn ensure_fileindex() {
    if !crate::settings::index_needs_rebuild() || index_building() {
        return;
    }
    crate::cache::spawn_self(&["_index"]);
}

fn index_roots_from(text: Option<&str>) -> Vec<String> {
    if let Some(text) = text {
        // Once the person has a roots file, it is authoritative even when it
        // contains no folders. Falling back on an empty file made deleting the
        // final row silently restore three folders that Settings no longer
        // showed. Defaults are onboarding, not an inescapable minimum.
        return text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                line.strip_prefix("~/")
                    .map(|rest| paths::home().join(rest).to_string_lossy().into_owned())
                    .unwrap_or_else(|| line.to_string())
            })
            .collect();
    }
    default_roots_in(&paths::home())
}

/// The folders a first search looks in: the person's own, read off `$HOME`.
///
/// Three names were hard-coded here — `App`, `Documents`, `Desktop` — and two
/// of them are Apple's while the third is one developer's habit. Measured
/// against the machine this was written on, that list reached four of
/// twenty-five top-level folders: `Learn`, `services`, `skills`, `test`,
/// `kimicli` and fifteen more were unreachable by name, and nothing said so.
/// A launcher that cannot find your own checkout is not one you keep opening
/// long enough to discover that `set:` would have fixed it.
///
/// Guessing harder is not the repair. A longer table of conventions —
/// `Developer`, `Projects`, `Code`, `src`, `repos` — was written first and
/// measured at 60,263 entries against the old 60,104: nine names, and on this
/// machine it found one folder the three did not. The names people use are not
/// drawn from a list anybody can write down, which is the same trap
/// `skill_dirs` documents, except that here the answer is already on disk. So
/// this reads `$HOME` instead of predicting it.
///
/// **Each folder is its own root, and that is load-bearing rather than
/// incidental.** The obvious form of this change is to index `$HOME` itself,
/// and it costs a level of depth inside every project, because `max_depth`
/// counts from the root: `~/App` walked from home reaches 33,752 entries where
/// walked directly it reaches 59,411. Rooting one level down keeps every
/// existing path at exactly the depth it has today, needs no compensating
/// `max_depth` bump for user-chosen roots that never asked for one, and leaves
/// `paths::is_protected` — which refuses `$HOME` for the Trash and for
/// `project::root` — untouched and still true.
///
/// Some names are held back. `Library` is other applications' data, 256,016
/// entries here, and asking for it summons a TCC panel that names the terminal
/// rather than Prelude; that is `project::root`'s lesson and the reason this
/// whole function is a filter rather than a `read_dir`. `Applications` holds
/// bundles whose insides are nobody's search result, and `app:` already
/// answers for them by name. The three media folders go on the measurement in
/// `excluded_home_folder` rather than on an opinion about what people search
/// for. A person who wants any of them adds one row in `set:`, which is what
/// defaults are for, since the file becomes authoritative the moment it is
/// edited.
///
/// The name test is doing more work than it looks like, and this is the part
/// to keep. **A symlinked root is descended even under `follow_links(false)`**
/// — measured, because the flag governs links met *during* a walk and the root
/// is not one of them — so `Library` has to be refused by name here rather
/// than left for the walker to decline. What that does not catch is a
/// `Documents` which iCloud has made a symlink into `Library/Mobile
/// Documents`, and refusing *that* is the one thing deliberately not done:
/// `Desktop` and `Documents` are the two least surprising search folders there
/// are, they are already defaults today, and switching iCloud on must not
/// quietly delete them from the list.
fn default_roots_in(home: &std::path::Path) -> Vec<String> {
    let mut roots: Vec<String> = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // `is_dir` rather than the entry's own file type, so a folder that
            // is a symlink — which is what iCloud makes of `Documents` — is
            // still offered rather than silently dropped.
            //
            // `is_protected` because these are folders the person did not
            // choose, which is exactly the check that rule names. It is the
            // second guard rather than the first: it canonicalizes, at 2µs a
            // root, and the name test above has already discarded the ones
            // that would fail.
            (!excluded_home_folder(&name)
                && entry.path().is_dir()
                && !crate::paths::is_protected(&entry.path()))
            .then(|| entry.path().to_string_lossy().into_owned())
        })
        .collect();
    if roots.is_empty() {
        // An unreadable `$HOME` is not an empty one, and the difference has to
        // be visible: onboarding with nothing in it looks identical to a
        // person who deliberately cleared the list.
        return ["Documents", "Desktop", "Downloads"]
            .iter()
            .map(|d| home.join(d).to_string_lossy().into_owned())
            .collect();
    }
    // Whatever `read_dir` hands back is the filesystem's order, which is not
    // an order anybody can read. Settings prints this list.
    roots.sort();
    roots
}

/// A package that belongs to one application and is a database rather than a
/// folder of the person's files.
///
/// `~/Pictures` becomes a default search folder the moment the defaults are
/// read off `$HOME`, and a Photos library is the `~/Library` problem in
/// miniature: tens of thousands of derivative files nobody would ever search
/// for by name, behind a TCC boundary whose panel names the terminal rather
/// than Prelude. Measured here, the walk already comes back empty because
/// macOS refuses it — so what this buys is not the rows, which were never
/// arriving. It is not asking.
///
/// The check is by extension because that is what makes these packages what
/// they are, and it applies to every root rather than only the defaults: a
/// person who adds `~/Pictures` by hand meant the pictures too.
fn app_managed_library(path: &std::path::Path) -> bool {
    const LIBRARIES: &[&str] =
        &["photoslibrary", "photolibrary", "musiclibrary", "tvlibrary", "imovielibrary", "theater"];
    path.extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|ext| LIBRARIES.contains(&ext.as_str()))
        || path.file_name().is_some_and(|name| name == "Photo Booth Library")
}

fn excluded_home_folder(name: &str) -> bool {
    name.starts_with('.')
        || name == "Library"
        // "Applications" and "Applications (Parallels)" alike.
        || name.starts_with("Applications")
        // Media, and this one is a latency decision rather than a taste one.
        // `f:` scans the whole index on every keystroke and already costs
        // 18.3 ms p95 against 60,104 entries. Enumerating `$HOME` with these
        // three included measured 97,488 — a 62% index for a keystroke path
        // that is felt — and without them 61,460, which is the size it is
        // today. `Pictures` alone was 36,028 of the difference. So every work
        // folder the old three names missed arrives for +2%, and the folders
        // whose contents are looked at rather than typed stay out where the
        // person can add them from `set:` in one keystroke if they disagree.
        || matches!(name, "Pictures" | "Movies" | "Music")
}

/// The search folders, read once per process.
///
/// Memoised because `index_stale` now stats each of them and runs on the
/// per-keystroke path, so without this the roots file would be opened and
/// parsed several times per keystroke to answer a question whose answer cannot
/// change inside one process — `_dynamic` is a fresh process every time, which
/// is what makes that true. Same reasoning as `quicklinks_text`.
///
/// A mutation in *this* process — `add_root` — is safe against it for a reason
/// worth stating rather than relying on: `index_stale` compares the roots
/// file's own mtime first, so a just-edited list is reported stale before the
/// memoised contents are ever consulted, and the rebuild happens in a new
/// process that reads the file again.
pub fn index_roots() -> Vec<String> {
    static ROOTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let cfg = paths::config().join("roots.txt");
            let text = std::fs::read_to_string(cfg).ok();
            index_roots_from(text.as_deref())
        })
        .clone()
}

#[cfg(target_os = "macos")]
const FILE_TAGS_JXA: &str = r#"
ObjC.import('Foundation')
function run(argv) {
  const text = $.NSString.stringWithContentsOfFileEncodingError(
    argv[0], $.NSUTF8StringEncoding, null)
  if (!text) return ''
  const out = []
  for (const path of text.js.split('\n')) {
    if (!path) continue
    try {
      const value = Ref()
      const url = $.NSURL.fileURLWithPath(path)
      if (!url.getResourceValueForKeyError(value, $.NSURLTagNamesKey, null) || !value[0]) continue
      const tags = value[0].js.map(tag => tag.js)
      if (tags.length) out.push(JSON.stringify({path: path, tags: tags}))
    } catch (_) {}
  }
  return out.join('\n')
}
"#;

pub(crate) fn sanitized_file_tags(
    output: &str,
    allowed: &std::collections::HashSet<&str>,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut found = std::collections::HashMap::new();
    for line in output.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(path) = value.get("path").and_then(|value| value.as_str()) else { continue };
        if !allowed.contains(path) {
            continue;
        }
        let mut tags: Vec<String> = value
            .get("tags")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .map(crate::width::flatten)
            .filter(|tag| {
                !tag.is_empty()
                    && crate::width::dwidth(tag) <= 80
                    && !crate::secrets::looks_secret(tag)
            })
            .take(16)
            .collect();
        tags.sort_by_key(|tag| tag.to_ascii_lowercase());
        tags.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        if !tags.is_empty() {
            found.insert(path.to_string(), tags);
        }
    }
    found
}

#[cfg(target_os = "macos")]
fn finder_tags(index: &std::path::Path, lines: &[String]) -> std::collections::HashMap<String, Vec<String>> {
    let Some(index) = index.to_str() else { return std::collections::HashMap::new() };
    let output = run(
        &["/usr/bin/osascript", "-l", "JavaScript", "-e", FILE_TAGS_JXA, index],
        Duration::from_secs(120),
    );
    let allowed: std::collections::HashSet<&str> = lines.iter().map(String::as_str).collect();
    sanitized_file_tags(&output, &allowed)
}

#[cfg(not(target_os = "macos"))]
fn finder_tags(_: &std::path::Path, _: &[String]) -> std::collections::HashMap<String, Vec<String>> {
    std::collections::HashMap::new()
}

fn indexed_paths() -> Vec<(IndexedKind, String)> {
    indexed_paths_from(&index_roots())
}

fn indexed_paths_from(roots: &[String]) -> Vec<(IndexedKind, String)> {
    let mut paths = Vec::new();
    for root in roots {
        let root = std::path::Path::new(&root);
        if !root.is_dir() {
            continue;
        }
        // The same in-process walker used by the live project source, rather
        // than collecting a subprocess's bounded stdout. A large selected
        // root must not silently lose everything beyond the output cap.
        let walker = ignore::WalkBuilder::new(root)
            .max_depth(Some(7))
            .follow_links(false)
            .filter_entry(|entry| !app_managed_library(entry.path()))
            .build();
        // Depth 0 is the root itself, and it goes in. It was skipped, which
        // made every search folder the one folder its own name could never
        // find: `~/App` was a root, so `f:app` answered with `…/src/app`
        // seven levels down and never with the folder the person meant.
        paths.extend(walker.flatten().filter_map(|entry| {
            let kind = match entry.file_type() {
                Some(file_type) if file_type.is_file() => IndexedKind::File,
                Some(file_type) if file_type.is_dir() => IndexedKind::Folder,
                _ => return None,
            };
            Some((kind, entry.path().to_string_lossy().into_owned()))
        }));
    }
    paths.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.marker().cmp(right.0.marker())));
    paths.dedup_by(|left, right| left.1 == right.1);
    paths
}

pub fn build_fileindex() -> IndexCounts {
    let Some(_lock) = crate::cache::try_lock(&index_lock_path(), Duration::ZERO) else {
        return crate::settings::index_counts().unwrap_or_default();
    };
    let paths = indexed_paths();
    let tag_paths: Vec<String> = paths.iter().map(|(_, path)| path.clone()).collect();
    let staging = paths::cache().join(format!("fileindex.tags.{}", std::process::id()));
    // Finder's tag reader accepts one path-list file. This staging file is not
    // the live index: search continues to use the old generation throughout
    // the walk and metadata pass.
    let _ = crate::cache::write_atomic(&staging, tag_paths.join("\n").as_bytes());
    let tags = finder_tags(&staging, &tag_paths);
    let _ = std::fs::remove_file(&staging);
    let records: Vec<String> = paths
        .iter()
        .map(|(kind, path)| {
            let prefix = format!("{}\t{path}", kind.marker());
            match tags.get(path) {
                Some(tags) => format!("{prefix}\t{}", serde_json::to_string(tags).unwrap_or_default()),
                None => prefix,
            }
        })
        .collect();
    let _ = crate::cache::write_atomic(&fileindex_path(), records.join("\n").as_bytes());
    let counts = IndexCounts {
        files: paths.iter().filter(|(kind, _)| *kind == IndexedKind::File).count(),
        folders: paths.iter().filter(|(kind, _)| *kind == IndexedKind::Folder).count(),
    };
    crate::settings::record_index_counts(counts);
    counts
}

fn name_score(name: &str, needles: &[String]) -> Option<i64> {
    if needles.is_empty() {
        return Some(0);
    }
    let name = name.to_lowercase();
    let joined = needles.join(" ");
    if name == joined {
        return Some(10_000);
    }
    if name.starts_with(&joined) {
        return Some(8_000);
    }
    if name.contains(&joined) {
        return Some(6_000);
    }
    if needles.iter().all(|needle| name.contains(needle)) {
        return Some(5_000);
    }
    // A forgiving subsequence belongs below every real substring match. It
    // tolerates remembered abbreviations without making parent paths search
    // terms and without requiring fzf to rank fifty thousand records itself.
    let compact: String = needles.concat();
    let mut chars = name.chars();
    compact
        .chars()
        .all(|wanted| chars.by_ref().any(|candidate| candidate == wanted))
        .then_some(3_000)
}

fn filesystem_score(
    name: &str,
    path: &str,
    tags: &[String],
    term: &str,
    kind: IndexedKind,
) -> Option<i64> {
    let words: Vec<String> = crate::sources::sessions::split_words(term)
        .into_iter()
        .map(|word| word.to_lowercase())
        .collect();
    let tags_low: Vec<String> = tags.iter().map(|tag| tag.to_lowercase()).collect();
    let mut ordinary = Vec::new();
    for word in words {
        if let Some(tag) = word.strip_prefix("tag:") {
            if tag.is_empty() || !tags_low.iter().any(|candidate| candidate.contains(tag)) {
                return None;
            }
        } else {
            ordinary.push(word);
        }
    }
    let path_query = ordinary.iter().any(|word| word.contains('/'));
    let score = if ordinary.is_empty() {
        4_000
    } else if let Some(score) = name_score(name, &ordinary) {
        score
    } else if path_query && path_matches(path, &ordinary) {
        4_500
    } else if ordinary
        .iter()
        .all(|needle| tags_low.iter().any(|candidate| candidate.contains(needle)))
    {
        4_000
    } else {
        return None;
    };
    Some(score + if kind == IndexedKind::Folder && !term.trim().is_empty() { 250 } else { 0 })
}

fn path_matches(path: &str, words: &[String]) -> bool {
    let query = words.join(" ");
    let parts: Vec<&str> = query.split('/').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return false;
    }
    let components: Vec<String> = path
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut at = 0;
    for part in parts {
        let Some(found) = components[at..].iter().position(|component| component.contains(part)) else {
            return false;
        };
        at += found + 1;
    }
    true
}

fn indexed_item(kind: IndexedKind, path: &str, tags: &[String], score: i64) -> Item {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let cmd = match kind {
        IndexedKind::File => path.to_string(),
        IndexedKind::Folder => format!("cd {}", shq(path)),
    };
    // The context is the object's own complete path. It was the parent, which
    // told the reader least exactly where it mattered most: the row for
    // `~/App` read `App · ~`, and eight rows all titled `app` asked the
    // reader to join two columns in their head.
    Item::new(cmd, Kind::Find)
        .title(name)
        .sub(paths::tilde(path))
        .put("path", path)
        .put("tags", tags.join("\u{1e}"))
        .put("index_kind", kind.data())
        .rank(score as f64)
}

fn indexed_kind(item: &Item) -> IndexedKind {
    if item.kind == Kind::Dir || item.get("index_kind") == "folder" {
        IndexedKind::Folder
    } else {
        IndexedKind::File
    }
}

fn scored_filesystem_item(mut item: Item, term: &str, bonus: i64) -> Option<Item> {
    let path = item.get("path").to_string();
    let name = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&item.title);
    let score = filesystem_score(name, &path, &[], term, indexed_kind(&item))? + bonus;
    item.data.insert("rank".into(), format!("{score:.3}"));
    item.score = item.band() as f64 + score as f64;
    Some(item)
}

/// One folder, one dedup key, however it was written down.
///
/// The index stores clean paths, but the zoxide and recent-`cd` evidence
/// merged into `dir:` stores whatever was typed: `~/App/` with its
/// trailing slash, and `~/APp` from a miscased `cd` that macOS's
/// case-insensitive filesystem was happy to honour. Keyed on the raw string,
/// each spelling was its own row, and the folder a person uses most arrived
/// in triplicate at the top of the very scope that exists to rank it.
///
/// Folding case can in principle merge two genuinely distinct folders on a
/// case-sensitive volume; that is accepted, because the default macOS volume
/// is case-insensitive and the duplicated rows appear there on every real
/// machine, while the sibling pair differing only by case appears on almost
/// none.
fn filesystem_identity(path: &str) -> String {
    let trimmed = if path.len() > 1 { path.trim_end_matches('/') } else { path };
    trimmed.to_lowercase()
}

fn finish_filesystem_rows(rows: Vec<Item>, limit: usize) -> Vec<Item> {
    let freq = crate::frecency::load();
    let mut best: std::collections::HashMap<String, Item> = std::collections::HashMap::new();
    for mut item in rows {
        let path = item.get("path").to_string();
        if path.is_empty() {
            continue;
        }
        if let Some((uses, last)) = freq.get(&item.cmd) {
            item.score += crate::frecency::bonus(*uses, *last);
        }
        let identity = filesystem_identity(&path);
        match best.get(&identity) {
            Some(existing) if existing.score >= item.score => {}
            _ => {
                best.insert(identity, item);
            }
        }
    }
    let mut rows: Vec<Item> = best.into_values().collect();
    rows.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            .then_with(|| left.get("path").cmp(right.get("path")))
    });
    rows.truncate(limit);
    rows
}

struct FileCandidate<'a> {
    kind: IndexedKind,
    path: &'a str,
    tags: Vec<String>,
    score: i64,
    name_lower: String,
}

impl PartialEq for FileCandidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score && self.name_lower == other.name_lower && self.path == other.path
    }
}

impl Eq for FileCandidate<'_> {}

impl PartialOrd for FileCandidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileCandidate<'_> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // BinaryHeap keeps its greatest member on top. Define "greatest" as
        // the worst retained hit: lower score, then a later name/path. A new
        // candidate can replace it in O(log N), so even a query matching every
        // indexed object allocates only the N rows it can actually display.
        other
            .score
            .cmp(&self.score)
            .then_with(|| self.name_lower.cmp(&other.name_lower))
            .then_with(|| self.path.cmp(other.path))
    }
}

fn search_fileindex_from_kind(
    text: &str,
    term: &str,
    wanted: Option<IndexedKind>,
    limit: usize,
) -> Vec<Item> {
    let mut best = std::collections::BinaryHeap::with_capacity(limit.saturating_add(1));
    for line in text.lines() {
        let record = parse_index_record(line);
        if record.path.is_empty() || wanted.is_some_and(|kind| kind != record.kind) {
            continue;
        }
        let name = record.path.rsplit('/').next().unwrap_or(record.path);
        let Some(score) = filesystem_score(name, record.path, &record.tags, term, record.kind) else {
            continue;
        };
        let candidate = FileCandidate {
            kind: record.kind,
            path: record.path,
            tags: record.tags,
            score,
            name_lower: name.to_lowercase(),
        };
        if best.len() < limit {
            best.push(candidate);
        } else if best.peek().is_some_and(|worst| candidate < *worst) {
            let _ = best.pop();
            best.push(candidate);
        }
    }
    let rows = best
        .into_vec()
        .into_iter()
        .map(|candidate| {
            indexed_item(candidate.kind, candidate.path, &candidate.tags, candidate.score)
        })
        .collect();
    finish_filesystem_rows(rows, limit)
}

#[cfg(test)]
pub(crate) fn search_fileindex_from(text: &str, term: &str) -> Vec<Item> {
    search_fileindex_from_kind(text, term, None, SCOPED_FILE_RESULT_LIMIT)
}

fn search_fileindex_kind(
    term: &str,
    wanted: Option<IndexedKind>,
    limit: usize,
) -> Option<Vec<Item>> {
    let text = std::fs::read_to_string(fileindex_path()).ok()?;
    Some(search_fileindex_from_kind(&text, term, wanted, limit))
}

/// Search a prebuilt local index. Finder tags are captured while rebuilding,
/// never by launching metadata tools on each keystroke.
pub fn search_fileindex(term: &str) -> Option<Vec<Item>> {
    search_fileindex_kind(term, None, SCOPED_FILE_RESULT_LIMIT)
}

/// The small local answer mixed into ordinary launcher search. Parent paths
/// are display context only: searching `OpenGhostty` can return the folder
/// named `OpenGhosttyFromAnyFolder`, never every `main.swift` below it.
pub fn root_filesystem_rows(term: &str, static_items: &[Item]) -> Vec<Item> {
    let term = term.trim();
    if term.is_empty() || term.chars().count() < 2 || term.starts_with("tag:") {
        return Vec::new();
    }
    ensure_fileindex();
    let mut rows = search_fileindex_kind(term, None, ROOT_FILE_RESULT_LIMIT).unwrap_or_default();
    rows.extend(
        static_items
            .iter()
            .filter(|item| item.kind == Kind::File)
            .filter_map(|item| scored_filesystem_item(item.clone(), term, 100)),
    );
    finish_filesystem_rows(rows, ROOT_FILE_RESULT_LIMIT)
}

/// The applications whose own names match, mixed into ordinary search.
///
/// Typing an application's name is the single most common reason anybody
/// opens a launcher, and it was the one population root search could not
/// answer. `Chrome` returned eight `node_modules` icon files and a Google
/// search; `Google Chrome.app` was reachable only by first knowing to type
/// `app:`. That is not a discoverability gap — the person did not fail to
/// find a feature, they asked the launcher its most ordinary question and
/// were told about something else.
///
/// It costs nothing. Every app row is already in the snapshot an ordinary
/// query parses for its own file rows, so this is a filter over a `Vec` that
/// is in memory either way: no index read, no filesystem walk, no subprocess.
///
/// Two rules keep the block honest. It sits *below* the catalogue, so an
/// installed Agent still leads its own name and a quicklink keyword still
/// resolves ahead of an application that happens to contain it. And it takes
/// only substring-or-better matches — `name_score`'s forgiving subsequence
/// tier is refused here, because the block is emitted before the file rows
/// and `--tiebreak=index` would otherwise let a stretched app match outrank a
/// file whose name the query actually spells.
pub fn root_application_rows(term: &str, static_items: &[Item]) -> Vec<Item> {
    const REAL_MATCH: i64 = 5_000;
    let term = term.trim();
    if term.chars().count() < 2 || term.starts_with("tag:") {
        return Vec::new();
    }
    let words: Vec<String> = crate::sources::sessions::split_words(term)
        .into_iter()
        .map(|word| word.to_lowercase())
        .collect();
    // A path or a tag is a question about the filesystem. Answering it with
    // applications would make the block mean something different per query.
    if words.is_empty()
        || words.iter().any(|word| word.contains('/') || word.starts_with("tag:"))
    {
        return Vec::new();
    }
    let freq = crate::frecency::load();
    let mut rows: Vec<Item> = static_items
        .iter()
        .filter(|item| item.kind == Kind::App)
        .filter_map(|item| {
            let score = name_score(&item.title, &words).filter(|s| *s >= REAL_MATCH)?;
            let mut item = item.clone();
            item.data.insert("rank".into(), format!("{score:.3}"));
            item.score = item.band() as f64 + score as f64;
            if let Some((uses, last)) = freq.get(&item.cmd) {
                item.score += crate::frecency::bonus(*uses, *last);
            }
            Some(item)
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });
    rows.truncate(ROOT_APP_RESULT_LIMIT);
    rows
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
    let hit = skills
        .into_iter()
        .find(|skill| crate::archive::visible(skill) && skill.title.eq_ignore_ascii_case(name))?;
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
        Some(
            Item::new(path_text.clone(), Kind::File)
                .title(title)
                .sub(paths::tilde(&path_text))
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
    // A query that exactly spells a scope's name — `app`, `docker`, `f` — is
    // deliberately *not* special. It used to be, and the result was the exact
    // alias bug wearing a different hat: typing `app` collapsed the whole
    // launcher to one `app:` row, so the folder named `App` and the
    // application whose name contains "app" vanished on the keystroke that
    // completed the word. The scope command still leads — `dynamic_rows_with`
    // emits it first — but the room stays furnished.
    if t == ":"
        || scope_query(t).is_some()
        || t.starts_with(['/', '@'])
        || looks_like_local_path(t)
    {
        return true;
    }
    if exact_quicklink_item(t).is_some() {
        return true;
    }
    // Beside the quicklink lookup and not on top of it: the two vocabularies
    // cannot collide, because `aliases::conflict` refuses a name a quicklink
    // already carries at the moment it is typed. One memoised read of a small
    // file, on the same terms the quicklink lookup is admitted here.
    if crate::aliases::target_of(t).is_some() {
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
    // The scope command the query exactly names leads the list; it does not
    // clear the room. `dynamic` prints the catalogue, the local tiers and the
    // web fallback underneath, and drops the catalogue's own copy of this row
    // so the screen holds it once, at the top.
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
        // A slash can also be a deliberate indexed-path query rather than a
        // literal existing path. It is the one opt-in to parent-path matching:
        // `OpenGhostty/main` may find main.swift below that folder, while the
        // ordinary `OpenGhostty` query returns only the folder itself.
        if !q.trim().starts_with('/') && looks_like_local_path(q) {
            ensure_fileindex();
            if let Some(hits) = search_fileindex_kind(q.trim(), None, SCOPED_FILE_RESULT_LIMIT) {
                return hits;
            }
        }
    }
    if let Some(rows) = skill_prefix_rows(q, static_items) {
        return rows;
    }
    if let Some(rows) = agent_prompt_rows(q, static_items) {
        return rows;
    }
    if let Some(item) = exact_quicklink_item(q) {
        return quicklink_with_neighbours(item, q, static_items);
    }
    if let Some(rows) = alias_rows(q, static_items) {
        return rows;
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
                // A result, not a saved quicklink: it keeps the key so the
                // provider behind it can be edited, and stays an ordinary Link
                // row so it can also be *saved* — the row you are looking at
                // after a search is the one you most want to keep, and it was
                // the single row where "Create Quicklink…" was suppressed.
                .put("ql", "result")
                .put("quicklink", key)
                .put("quicklink_term", term)
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

    /// Built literally, never through `rewrite::config()`, so the developer's
    /// own preferences and environment cannot decide what these assert.
    fn ready_rewrite_config() -> crate::rewrite::Config {
        crate::rewrite::Config {
            provider: crate::rewrite::Provider::Ollama,
            url: "http://localhost:11434".into(),
            model: "m".into(),
            timeout: 60,
            profile_id: crate::rewrite::default_profile().id.into(),
            system: crate::rewrite::default_profile().system.into(),
            built_in: true,
            review: false,
        }
    }

    /// The whole point of the row: it describes work, and the request happens
    /// on Enter. `scoped_rows` runs on every keystroke.
    #[test]
    fn a_prompt_row_states_the_work_without_doing_it() {
        let rows = prompt_rows_with(&ready_rewrite_config(), " 帮我检查登录按钮 ", &[]);
        assert_eq!(rows.len(), 1, "one row, not one per style");
        let row = &rows[0];
        assert_eq!(row.kind, Kind::Rewrite);
        assert_eq!(row.get("rw_source"), "帮我检查登录按钮", "trimmed, and the source is carried");
        assert_eq!(row.get("rw_model"), "m");
        // Enter acts, and no preference may turn that into handing back the
        // very text the row exists to replace.
        assert_eq!(
            crate::defaults::enter_with(row, true),
            crate::defaults::Default_::Act(crate::defaults::Verb::Rewrite),
        );
    }

    /// With nothing typed the subject is the newest clipping, taken from the
    /// snapshot rather than from `pbpaste` — a subprocess per keystroke to
    /// read a value that is already in memory.
    #[test]
    fn an_empty_prompt_query_uses_the_clipboard_from_the_snapshot() {
        let clip = Item::new("这个方案…", Kind::Clip)
            .put("clip_kind", "text")
            .put("full", "这个方案我还不确定是否可行");
        let rows = prompt_rows_with(&ready_rewrite_config(), "", &[clip]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("rw_source"), "这个方案我还不确定是否可行", "the full clip, not its label");
        assert_eq!(rows[0].fields[2], "from the clipboard");

        // An image has no text to rewrite, so it is not the subject.
        let image = Item::new("shot.png", Kind::Clip).put("clip_kind", "image");
        let rows = prompt_rows_with(&ready_rewrite_config(), "", &[image]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, Kind::Search, "nothing to use, so say what to do");
    }

    /// Answering "no model yet" with an empty box is the failure
    /// `fallback_rows` exists to prevent, one scope along.
    #[test]
    fn an_unconfigured_prompt_scope_hands_over_the_controls() {
        let mut cfg = ready_rewrite_config();
        cfg.model = String::new();
        let rows = prompt_rows_with(&cfg, "anything", &[]);
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|it| it.kind == Kind::Setting && it.get("group") == "Rewrite"));

        cfg.model = "m".into();
        cfg.provider = crate::rewrite::Provider::Off;
        let rows = prompt_rows_with(&cfg, "anything", &[]);
        assert!(rows.iter().all(|it| it.kind == Kind::Setting), "off says so with its own switch");
    }

    /// The row names where the text is about to go. Local and remote must not
    /// read the same, because that is the only difference that matters.
    #[test]
    fn a_prompt_row_says_where_the_text_goes() {
        let local = prompt_rows_with(&ready_rewrite_config(), "hello there", &[]);
        assert!(local[0].fields[1].contains("on this machine"), "{:?}", local[0].fields);

        let mut remote = ready_rewrite_config();
        remote.provider = crate::rewrite::Provider::OpenAi;
        remote.url = "https://api.example.com/v1".into();
        let rows = prompt_rows_with(&remote, "hello there", &[]);
        assert!(rows[0].fields[1].contains("api.example.com"), "{:?}", rows[0].fields);
        assert!(!rows[0].fields[1].contains("on this machine"));
    }

    #[test]
    fn a_url_host_is_read_without_its_scheme_or_path() {
        assert_eq!(host_of("https://api.example.com/v1/chat"), "api.example.com");
        assert_eq!(host_of("http://localhost:11434"), "localhost:11434");
    }

    #[test]
    fn a_complete_settings_category_is_an_exact_filter() {
        let rows = scoped_rows(Scope::Settings, "search", &[]);
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|item| item.get("group") == "Search"));
        assert!(!rows.iter().any(|item| item.get("setting") == "quicklinks"));
    }

    #[test]
    fn an_explicitly_empty_search_folder_list_stays_empty() {
        assert!(index_roots_from(Some("# intentionally none\n")).is_empty());
        assert!(!index_roots_from(None).is_empty(), "defaults exist before first edit");
        let rows = index_roots_from(Some("~/App\n/tmp/work\n"));
        assert_eq!(rows[0], paths::home().join("App").to_string_lossy());
        assert_eq!(rows[1], "/tmp/work");
    }

    /// A first run has no Skills and no conversations, so the screen this
    /// launcher opens on is the one nobody has written anything for. It must
    /// not be blank.
    #[test]
    fn a_first_run_home_says_something_rather_than_nothing() {
        // A real first run has gathered plenty — applications, `$PATH`, the
        // scope commands — and none of it is the person's own.
        let gathered = vec![Item::new("cargo", Kind::Path).title("cargo")];
        let rows = home_items(&gathered);
        assert!(!rows.is_empty(), "an empty home is the one screen everybody meets first");

        // Whereas nothing at all means the sources did not answer, and that is
        // not a person to be onboarded.
        assert!(home_items(&[]).is_empty(), "a failed gather is not a new install");

        // Every row has to act, which is the rule this screen already keeps.
        // Four are the scope commands, so Enter completes them exactly as it
        // does in root search; one opens the guide.
        assert!(rows.iter().all(|it| {
            it.get("mode") == "complete-query" || matches!(it.kind, Kind::Link)
        }));

        // Never into a scope that is empty for the very reason we are here,
        // and not into the index that is still being built behind this screen.
        let scopes: Vec<&str> = rows.iter().map(|it| it.cmd.as_str()).collect();
        for dead_end in ["skill:", "s:", "r:", "a:", "f:", "app:"] {
            assert!(
                !scopes.contains(&dead_end),
                "{dead_end} answers an empty screen with another empty screen"
            );
        }

        // No row on the first screen a person sees may explain itself with a
        // scope prefix. `c:` is not language; the row says what it holds, and
        // Enter is what teaches the shorthand.
        for row in &rows {
            for field in &row.fields {
                assert!(
                    !SCOPES.iter().any(|d| d.prefix == field),
                    "{:?} explains itself with a scope prefix",
                    row.title
                );
            }
        }

        // And it is onboarding rather than a fixture: one Skill of the
        // person's own displaces the whole thing.
        let mine = vec![Item::new("/deploy", Kind::Skill).title("deploy")];
        let shown: Vec<String> = home_items(&mine).into_iter().map(|it| it.title).collect();
        assert_eq!(shown, ["deploy"]);
    }

    /// The defaults are the person's own folders, so the parts worth pinning
    /// are the two exclusions and the shape — never a count, which is a
    /// property of whoever's machine is running the test.
    #[test]
    fn search_defaults_are_the_home_folders_minus_other_apps_data() {
        // Not `temp_dir()`: on macOS that canonicalizes under `/private/var`,
        // which `is_protected` refuses as a system tree — so the fixture would
        // exercise the fallback instead of the enumeration. `target/` is
        // under `/Users`, like the homes this actually runs against.
        let home = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("prelude-roots-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&home);
        for name in
            ["App", "Learn", "Documents", "Library", "Applications", "Applications (Parallels)"]
        {
            std::fs::create_dir_all(home.join(name)).unwrap();
        }
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::fs::write(home.join("notes.txt"), "not a folder").unwrap();

        let names: Vec<String> = default_roots_in(&home)
            .iter()
            .map(|root| {
                std::path::Path::new(root).file_name().unwrap().to_string_lossy().into_owned()
            })
            .collect();

        // Whatever the person actually has, including the names no table of
        // conventions would have contained.
        assert_eq!(names, vec!["App", "Documents", "Learn"]);
        // 256,016 entries on the machine this was measured on, and a TCC
        // panel naming the terminal rather than Prelude.
        assert!(!names.iter().any(|n| n == "Library"));
        // Bundles, which `app:` answers for by name.
        assert!(!names.iter().any(|n| n.starts_with("Applications")));
        // 36,028 of 97,488 entries, on a path `f:` rescans every keystroke.
        assert!(excluded_home_folder("Pictures") && excluded_home_folder("Movies"));
        assert!(!names.iter().any(|n| n.starts_with('.')), "hidden trees are not search folders");
        assert!(!names.iter().any(|n| n == "notes.txt"), "a file is not a search folder");
        let _ = std::fs::remove_dir_all(&home);

        // `Pictures` is a search folder now, so the one thing inside it that
        // is a database rather than a person's files must not be walked —
        // whether it arrived by default or because somebody chose it.
        let pictures = std::path::Path::new("/Users/x/Pictures");
        assert!(app_managed_library(&pictures.join("Photos Library.photoslibrary")));
        assert!(app_managed_library(&pictures.join("Photo Booth Library")));
        assert!(!app_managed_library(&pictures.join("xhs")));
        assert!(!app_managed_library(&pictures.join("logo/mark.png")));
    }

    /// zoxide and recent-`cd` evidence record whatever was typed, so one
    /// folder arrives spelled three ways — clean, with a trailing slash, and
    /// with a miscasing the case-insensitive filesystem honoured. Keyed on
    /// the raw string, each spelling was its own row, and the folder a
    /// person uses most led `dir:` in triplicate.
    #[test]
    fn one_folder_is_one_row_however_the_evidence_spelled_it() {
        let spellings = ["/Users/x/App", "/Users/x/App/", "/Users/x/APp"];
        let rows: Vec<Item> = spellings
            .iter()
            .map(|path| {
                Item::new(format!("cd {path}"), Kind::Dir).title("App").put("path", *path)
            })
            .collect();
        let out = finish_filesystem_rows(rows, 10);
        assert_eq!(out.len(), 1, "three spellings of one folder are one row");
        assert_eq!(filesystem_identity("/"), "/", "the filesystem root keeps its slash");
    }

    /// A root was the one folder its own name could never find: depth 0 was
    /// skipped, so `~/App` was absent from the very index built out of it,
    /// and `f:app` answered with `…/src/app` seven levels down while the
    /// folder the person actually meant was not a row at all.
    #[test]
    fn the_index_contains_each_root_itself() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("prelude-idx-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("inner")).unwrap();
        std::fs::write(root.join("inner").join("file.txt"), "x").unwrap();
        let root_text = root.to_string_lossy().into_owned();
        let paths = indexed_paths_from(std::slice::from_ref(&root_text));
        assert!(
            paths.iter().any(|(kind, path)| *kind == IndexedKind::Folder && *path == root_text),
            "the root itself is a Folder row"
        );
        assert!(paths.iter().any(|(kind, path)| {
            *kind == IndexedKind::Folder && path.ends_with("inner")
        }));
        assert!(paths.iter().any(|(kind, path)| {
            *kind == IndexedKind::File && path.ends_with("file.txt")
        }));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An unreadable `$HOME` must not be reported as a person who cleared the
    /// list: the first is a failure and the second is a preference.
    #[test]
    fn unreadable_home_still_offers_somewhere_to_look() {
        let missing = std::path::Path::new("/nope/nothing/here");
        assert_eq!(default_roots_in(missing).len(), 3);
    }

    fn run(state: &str, project: &str) -> Item {
        Item::new(format!("kill {state}{project}"), Kind::Run)
            .title("claude")
            .fields([project.to_string(), state.to_string()])
            .put("state", state)
            .put("agent", "claude")
            .put("project", project)
    }

    /// The empty query is Skills and Sessions — the two kinds with an action
    /// behind them — plus a question that is blocking somebody.
    ///
    /// Agents, Runs and MCP servers are readouts. `claude · 0 skills · 3 mcp`
    /// is true and there is nothing to press Enter on; the same for a Run that
    /// is working and a server that is connected. They are still gathered,
    /// still in `a:`/`r:`/`mcp:`, and still matched by name in root search —
    /// they are simply not what an empty query is for. Everything else on the
    /// machine remains behind a query, which is what stops the home being a
    /// list of two thousand files.
    #[test]
    fn the_home_is_what_you_can_act_on_and_nothing_else() {
        let rows = vec![
            Item::new("agent", Kind::Agent).title("claude"),
            run("working", "api"),
            Item::new("/deploy", Kind::Skill).title("deploy"),
            Item::new("old", Kind::Session).title("old"),
            Item::new("ask", Kind::Msg).title("claude asks"),
            Item::new("cargo", Kind::Path).title("cargo"),
            Item::new("copied", Kind::Clip).title("copied"),
            Item::new("git status", Kind::History).title("git status"),
        ];
        let shown: Vec<String> = home_items(&rows).into_iter().map(|it| it.title).collect();
        assert_eq!(shown, ["deploy", "old", "claude asks"]);
        // A blocked question is the deliberate third kind: it appears only
        // while an Agent is sitting on `prelude ask`, so it costs nothing on an
        // ordinary open, and leaving it out would mean the Agent blocks for its
        // whole timeout while the answer sits behind a prefix nobody typed.
        assert!(shown.contains(&"claude asks".to_string()));
        // An Agent row is a readout too, and equally absent: it is still in
        // root search by name and in `a:`.
        let agent = vec![Item::new("agent", Kind::Agent).title("claude")];
        assert!(!home_items(&agent).iter().any(|it| it.kind == Kind::Agent));
        assert_eq!(root_items(&agent).len(), 1);

        // Ordering is `cache::by_rank`'s job, so this filter must leave the
        // order it was handed alone.
        let pair = vec![
            Item::new("/first", Kind::Skill).title("first"),
            Item::new("/second", Kind::Skill).title("second"),
        ];
        let order: Vec<String> = home_items(&pair).into_iter().map(|it| it.title).collect();
        assert_eq!(order, ["first", "second"]);

        // A typed query searches the same inventory *and* the conversations.
        // They used to be held back, which meant typing a project name found
        // agents, skills and files but none of the sessions you had in that
        // project — and the ones on the home vanished at the first keystroke.
        let root: Vec<String> = root_items(&rows).into_iter().map(|it| it.title).collect();
        assert!(root.contains(&"deploy".to_string()));
        assert!(root.contains(&"old".to_string()), "a conversation is findable by typing");
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
        ];
        assert_eq!(scoped_rows(scope, term, &rows).len(), 1);
    }

    #[test]
    fn archived_capabilities_leave_inventory_but_remain_recoverable_in_their_scopes() {
        let rows = vec![
            Item::new("/current", Kind::Skill).title("current").put("name", "current"),
            Item::new("/retired", Kind::Skill)
                .title("retired")
                .put("name", "retired")
                .put("archived", "true"),
        ];
        // The archived Skill leaves every ordinary surface and stays
        // reachable by asking for it.
        assert_eq!(home_items(&rows).len(), 1);
        assert_eq!(root_items(&rows).len(), 1);
        assert_eq!(scoped_rows(Scope::Agent, "", &rows).len(), 1);

        let titles = |scope, term| {
            scoped_rows(scope, term, &rows)
                .into_iter()
                .map(|item| item.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(Scope::Skills, ""), ["current"]);
        assert_eq!(titles(Scope::Skills, "is:archived"), ["retired"]);
        assert_eq!(titles(Scope::Skills, "is:all"), ["current", "retired"]);


        assert!(scoped_rows(Scope::Skills, "is:unknown", &rows).is_empty());

        let slash = dynamic_rows_with("/", &rows);
        assert_eq!(slash.into_iter().map(|item| item.title).collect::<Vec<_>>(), ["current"]);
        assert!(dynamic_rows_with("/retired", &rows).is_empty());
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
