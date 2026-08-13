//! Prelude's own settings, as objects you can find rather than files you have
//! to remember.
//!
//! Every preference here already existed. What did not exist was any way to
//! *discover* one: `roots.txt` decides what `f:` can find and was documented
//! only in a README, defaulted from a hard-coded list, and had to be created by
//! hand — with `prelude index` run afterwards from memory, since nothing said
//! the index had gone stale. Other preferences lived only in environment
//! variables that had to be exported before the `eval` line in `.zshrc`.
//! A launcher that manages four agents' settings could not reach its own.
//!
//! So settings are rows, in their own `set:` scope, and each one carries its
//! current value on the row. That is the part that matters: a setting you
//! cannot see the value of is one you change by trial. `^K` holds the
//! mutations, which is where every other object in this launcher keeps them.
//!
//! **What is written, and what is only read.** Six settings own a file each
//! and are edited through their own code (`roots.txt`, `global.toml`,
//! `open.toml`, `snippets.toml`, `quicklinks.toml`, `favorites.txt`). The
//! environment-backed scalar preferences use `settings.toml`, and the variable
//! still wins where it is set — a variable is a per-invocation instruction
//! and a file is a standing one, so the narrower must override the broader.

use crate::item::{Item, Kind};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DEFAULT_KEY: &str = "^R";
const DEFAULT_PREVIEW: bool = true;
const DEFAULT_CLASSIC_ENTER: bool = false;
const DEFAULT_UPDATE: &str = "notify";
const DEFAULT_FALLBACKS: &str = crate::compute::WEB_SEARCH_KEY;
const PREF_KEYS: &[&str] =
    &["key", "preview", "classic_enter", "update", "fallbacks"];

// ---------------------------------------------------------------------------
// settings.toml — the preferences that had nowhere else to live
// ---------------------------------------------------------------------------

pub fn file() -> PathBuf {
    crate::paths::config().join("settings.toml")
}

#[derive(Clone, Debug, Default)]
struct Prefs {
    key: Option<String>,
    preview: Option<bool>,
    classic_enter: Option<bool>,
    update: Option<String>,
    fallbacks: Option<String>,
    present: std::collections::BTreeSet<String>,
}

/// Read once. `on_enter` runs in the per-keystroke footer helper, so a file
/// read per call would be a file read per keystroke.
#[cfg(not(test))]
fn prefs() -> &'static Prefs {
    static PREFS: std::sync::OnceLock<Prefs> = std::sync::OnceLock::new();
    PREFS.get_or_init(|| {
        let Ok(text) = std::fs::read_to_string(file()) else {
            return Prefs::default();
        };
        let parsed = crate::minitoml::parse(&text);
        let root = parsed.get("");
        let get = |k: &str| {
            root.and_then(|t| t.get(k))
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let flag = |k: &str| get(k).and_then(|v| parse_bool(&v));
        Prefs {
            key: get("key").and_then(|v| validate_pref("key", &v).ok()),
            preview: flag("preview"),
            classic_enter: flag("classic_enter"),
            update: get("update").and_then(|v| validate_pref("update", &v).ok()),
            fallbacks: get("fallbacks").and_then(|v| validate_pref("fallbacks", &v).ok()),
            present: root
                .into_iter()
                .flat_map(|table| table.keys().cloned())
                .collect(),
        }
    })
}

/// Unit tests exercise preference rules, not the developer's live machine.
/// Reading ~/.config here made an unrelated personal `classic_enter = true`
/// flip the expected behavior of every File, Run and Question test depending
/// on which parallel test initialized the cache first.
#[cfg(test)]
fn prefs() -> &'static Prefs {
    static PREFS: std::sync::OnceLock<Prefs> = std::sync::OnceLock::new();
    PREFS.get_or_init(Prefs::default)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// Canonicalise the friendly names accepted by the CLI. The launcher rows use
/// the short names; the aliases make the typed surface forgiving without
/// creating a second preference vocabulary in the file.
fn canonical_key(key: &str) -> Option<&'static str> {
    match key.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "key" | "launcher_key" => Some("key"),
        "preview" | "quicklook" | "quick_look" => Some("preview"),
        "fallbacks" | "fallback" => Some("fallbacks"),
        "enter" | "classic_enter" => Some("classic_enter"),
        "update" | "updates" | "auto_update" => Some("update"),
        "hotkey" | "global_hotkey" => Some("hotkey"),
        "paneldir" | "panel_directory" | "directory" => Some("paneldir"),
        "roots" | "search_roots" => Some("roots"),
        "index" | "file_index" => Some("index"),
        "openwith" | "open_with" => Some("openwith"),
        "snippets" => Some("snippets"),
        "quicklinks" => Some("quicklinks"),
        "favorites" | "favourites" => Some("favorites"),
        "aliases" | "alias" => Some("aliases"),
        "all" => Some("all"),
        _ => None,
    }
}

/// Validate values before they can make it into an fzf argv or a zsh
/// `bindkey` line, so a typo is explained at the command that introduced it.
fn validate_pref(key: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    match key {
        "key" => {
            if value.is_empty()
                || value.len() > 32
                || value.chars().any(|c| c.is_control() || c == '\0')
            {
                return Err("key must be a short zsh bindkey sequence such as ^R or ^T".into());
            }
            Ok(value.to_string())
        }
        "update" => match value.trim().to_ascii_lowercase().as_str() {
            v @ ("off" | "notify" | "download" | "apply") => Ok(v.to_string()),
            _ => Err("update is off, notify, download or apply".into()),
        },
        "preview" | "classic_enter" => parse_bool(value)
            .map(|v| v.to_string())
            .ok_or_else(|| format!("{key} is on or off (also true/false, yes/no or 1/0)")),
        // Syntax only. Whether a keyword names something that can take a query
        // is answered against `quicklinks.toml` at the moment it is used, and
        // reported by `checks`: this file must not decide it, because a
        // quicklink removed later would make a valid setting retroactively
        // unwritable.
        "fallbacks" => {
            let keys: Vec<&str> =
                value.split([',', ' ']).map(str::trim).filter(|k| !k.is_empty()).collect();
            if keys.is_empty() {
                return Err("fallbacks is one or more quicklink keywords, in order".into());
            }
            for key in &keys {
                crate::compute::normalize_quicklink_key(key)
                    .map_err(|why| format!("“{key}” cannot be a keyword: {why}"))?;
            }
            Ok(keys.join(", "))
        }
        _ => Err(format!("{key} is not stored in settings.toml")),
    }
}

/// Which of the two sources of a setting wins.
///
/// A variable is a per-invocation instruction and a file is a standing one, so
/// the narrower overrides the broader. Pure, and tested as such: the
/// alternative is a test that exports a variable into a process every other
/// test in the binary is running in.
fn resolve(from_env: Option<String>, from_file: Option<String>, fallback: &str) -> String {
    from_env.or(from_file).unwrap_or_else(|| fallback.to_string())
}

/// The key the zsh widget binds. Consumed by `prelude init zsh`, so a change
/// reaches a shell that starts afterwards rather than this one.
pub fn launcher_key() -> String {
    let from_env = env("PRELUDE_KEY").and_then(|v| validate_pref("key", &v).ok());
    resolve(from_env, prefs().key.clone(), DEFAULT_KEY)
}

/// Whether `Ctrl+P` Quick Look exists at all.
pub fn preview_enabled() -> bool {
    if env("PRELUDE_NO_PREVIEW").is_some() {
        return false;
    }
    prefs().preview.unwrap_or(DEFAULT_PREVIEW)
}

/// What to do about a newer release. `off` is the only value that makes no
/// network request; see `update::Mode`.
pub fn update_mode() -> String {
    if let Some(v) = env("PRELUDE_UPDATE").and_then(|v| validate_pref("update", &v).ok()) {
        return v;
    }
    prefs().update.clone().unwrap_or_else(|| DEFAULT_UPDATE.to_string())
}

/// Exposed so the decision can be tested without a settings file.
pub fn parse_update_mode(value: &str) -> crate::update::Mode {
    match validate_pref("update", value).unwrap_or_default().as_str() {
        "off" => crate::update::Mode::Off,
        "download" => crate::update::Mode::Download,
        "apply" => crate::update::Mode::Apply,
        _ => crate::update::Mode::Notify,
    }
}

/// The pre-2024 default: Enter hands over everything, whatever kind it is.
/// The ordered quicklink keywords a query with no answer falls back to.
///
/// No environment variable: the four that have one had one before
/// `settings.toml` existed, and inventing a fifth would widen the surface this
/// module exists to narrow.
pub fn fallbacks() -> String {
    prefs().fallbacks.clone().unwrap_or_else(|| DEFAULT_FALLBACKS.to_string())
}

/// The keywords that resolve, and those that do not, for the row and `check`.
pub fn fallback_state() -> (Vec<String>, Vec<String>) {
    let links = crate::compute::quicklinks();
    let spec = fallbacks();
    let mut good = Vec::new();
    let mut bad = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for key in spec.split([',', ' ']).map(str::trim).filter(|key| !key.is_empty()) {
        if !seen.insert(key.to_lowercase()) {
            continue;
        }
        match crate::compute::template_provider(&links, key) {
            Some((name, _)) => good.push(name),
            None => bad.push(key.to_string()),
        }
    }
    (good, bad)
}

pub fn classic_enter() -> bool {
    if env("PRELUDE_CLASSIC_ENTER").is_some() {
        return true;
    }
    prefs().classic_enter.unwrap_or(DEFAULT_CLASSIC_ENTER)
}

fn pref_literal(value: &str) -> String {
    if matches!(value, "true" | "false") { value.to_string() } else { format!("{value:?}") }
}

fn inline_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            escaped = true;
            continue;
        }
        match (quote, ch) {
            (None, '\'' | '"') => quote = Some(ch),
            (Some(open), close) if open == close => quote = None,
            (None, '#') => return &line[index..],
            _ => {}
        }
    }
    ""
}

/// Change one root key while preserving comments, unknown keys and future
/// sections byte-for-byte. A settings panel should not punish somebody for
/// opening its file and documenting why they chose a value.
fn update_pref_text(text: &str, key: &str, value: Option<&str>) -> String {
    let replacement = value.map(|v| format!("{key} = {}", pref_literal(v)));
    let mut out = Vec::new();
    let mut in_root = true;
    let mut found = false;
    let mut inserted = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if in_root && trimmed.starts_with('[') {
            if !found && !inserted {
                if let Some(line) = &replacement {
                    out.push(line.clone());
                    out.push(String::new());
                }
                inserted = true;
            }
            in_root = false;
        }
        let is_key = in_root
            && trimmed
                .split_once('=')
                .is_some_and(|(left, _)| left.trim().trim_matches(['\'', '"']) == key);
        if is_key {
            if !found {
                if let Some(replacement) = &replacement {
                    let comment = inline_comment(line);
                    out.push(if comment.is_empty() {
                        replacement.clone()
                    } else {
                        format!("{replacement}  {comment}")
                    });
                }
                found = true;
            }
            continue; // duplicate definitions are removed as part of the edit
        }
        out.push(line.to_string());
    }
    if !found && !inserted {
        if !out.is_empty() && !out.last().is_some_and(String::is_empty) {
            out.push(String::new());
        }
        if let Some(line) = replacement {
            out.push(line);
        }
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    if out.is_empty() {
        String::new()
    } else {
        out.join("\n") + "\n"
    }
}

fn write_pref(key: &str, value: &str) -> Result<(), String> {
    let value = validate_pref(key, value)?;
    let path = file();
    // Read, change, write: two settings changed at the same moment from two
    // windows kept one of them without this.
    let _lock = crate::cache::lock_for_write(&path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        "# Prelude's own preferences, written by the set: panel.\n\
         # The matching environment variable still overrides any line here.\n"
            .into()
    });
    let out = update_pref_text(&text, key, Some(&value));
    crate::cache::write_state(&path, out.as_bytes()).map_err(|e| e.to_string())
}

fn remove_pref(key: &str) -> Result<(), String> {
    let path = file();
    let _lock = crate::cache::lock_for_write(&path);
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(()) };
    let out = update_pref_text(&text, key, None);
    crate::cache::write_state(&path, out.as_bytes()).map_err(|e| e.to_string())
}

fn pref_source(key: &str, environment: &str) -> &'static str {
    let environment_value = env(environment);
    let valid_environment = environment_value.as_ref().is_some_and(|value| match key {
        "key" => validate_pref(key, value).is_ok(),
        // These variables are flags: any non-empty value intentionally means on.
        _ => true,
    });
    if valid_environment {
        "environment"
    } else if environment_value.is_some() {
        "invalid environment"
    } else {
        let valid = match key {
            "key" => prefs().key.is_some(),
            "preview" => prefs().preview.is_some(),
            "classic_enter" => prefs().classic_enter.is_some(),
            _ => false,
        };
        if valid {
            "saved"
        } else if prefs().present.contains(key) {
            "invalid saved value"
        } else {
            "default"
        }
    }
}

// ---------------------------------------------------------------------------
// The search roots, and whether the index still reflects them
// ---------------------------------------------------------------------------

pub fn roots_file() -> PathBuf {
    crate::paths::config().join("roots.txt")
}

fn index_count_file() -> PathBuf {
    crate::paths::cache().join("fileindex.count")
}

/// Record how many files and folders the last index run found. Kept beside
/// the index so drawing Settings never reads megabytes of paths.
const INDEX_SCHEMA: &str = "2";

pub fn record_index_counts(counts: crate::compute::IndexCounts) {
    let text = format!("{INDEX_SCHEMA}\t{}\t{}", counts.files, counts.folders);
    let _ = crate::cache::write_atomic(&index_count_file(), text.as_bytes());
}

fn saved_index_counts() -> Option<(bool, crate::compute::IndexCounts)> {
    let text = std::fs::read_to_string(index_count_file()).ok()?;
    let fields: Vec<&str> = text.trim().split('\t').collect();
    match fields.as_slice() {
        [schema, files, folders] if *schema == INDEX_SCHEMA => Some((
            true,
            crate::compute::IndexCounts {
                files: files.parse().ok()?,
                folders: folders.parse().ok()?,
            },
        )),
        // A short-lived development build wrote two unversioned counts. Read
        // it accurately but rebuild it just like the original one-count form.
        [files, folders] => Some((
            false,
            crate::compute::IndexCounts {
                files: files.parse().ok()?,
                folders: folders.parse().ok()?,
            },
        )),
        [files] => Some((
            false,
            crate::compute::IndexCounts { files: files.parse().ok()?, folders: 0 },
        )),
        _ => None,
    }
}

pub fn index_counts() -> Option<crate::compute::IndexCounts> {
    if let Some((_, counts)) = saved_index_counts() {
        return Some(counts);
    }
    let text = std::fs::read_to_string(crate::compute::fileindex_path()).ok()?;
    let mut counts = crate::compute::IndexCounts::default();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if line.starts_with("D\t") {
            counts.folders += 1;
        } else {
            counts.files += 1;
        }
    }
    // Reading an older index must not bless it as schema 2: doing so would
    // cancel the automatic rebuild that adds folders. The count is cheap once
    // per upgrade and the builder writes the versioned sidecar when complete.
    Some(counts)
}

fn mtime(p: &Path) -> Option<u64> {
    std::fs::metadata(p)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn mtime_nanos(p: &Path) -> Option<u128> {
    std::fs::metadata(p)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos())
}

/// Has the roots list been edited since the index was built?
///
/// The failure this answers is silent and was the whole problem: you add a
/// folder, `f:` keeps returning the old set, and nothing anywhere says the
/// index is the reason.
pub fn index_stale() -> bool {
    let built = mtime_nanos(&crate::compute::fileindex_path());
    match (built, mtime_nanos(&roots_file())) {
        (None, _) => true,
        (Some(built), Some(edited)) => edited > built,
        (Some(_), None) => false,
    }
}

/// A current v1 index is still missing every folder. Treat that as an upgrade
/// to prepare in the background without misreporting the person's roots as
/// stale while the old file search remains usable.
pub fn index_needs_rebuild() -> bool {
    index_stale() || !saved_index_counts().is_some_and(|(current, _)| current)
}

/// Add a folder to the search roots.
///
/// The guard is `paths::is_protected`, the same one the Trash uses and the
/// same one `project::root` uses to decide what is not a project. `~` is the
/// case it exists for: seven levels of `fd` over a home directory walks into
/// `~/Library`, which macOS protects as other applications' data, and the only
/// symptom is a system dialog naming the terminal rather than Prelude.
pub fn add_root(raw: &str) -> Result<String, String> {
    let readings = readings_of(raw);
    let Some(real) = readings.iter().find_map(|p| p.canonicalize().ok()) else {
        let shown = readings.first().map(|p| p.display().to_string()).unwrap_or_default();
        return Err(format!("{shown} is not there"));
    };
    if !real.is_dir() {
        return Err(format!("{} is not a folder", real.display()));
    }
    if crate::paths::is_protected(&real) {
        return Err(format!(
            "{} holds projects rather than being one; indexing it would walk every application's data",
            crate::paths::tilde(&real.to_string_lossy())
        ));
    }
    let tilde = crate::paths::tilde(&real.to_string_lossy());
    // Held across the read and the write, so adding two roots at once keeps
    // both — and so the duplicate check below cannot be raced past.
    let _lock = crate::cache::lock_for_write(&roots_file());
    let mut lines = roots_lines();
    if lines.iter().any(|l| resolved(l).as_deref() == Some(&real)) {
        return Err(format!("{tilde} is already a search root"));
    }
    lines.push(tilde.clone());
    write_roots(&lines)?;
    Ok(tilde)
}

/// The ways a typed or pasted path can be meant, most literal first.
///
/// A path with a space in it reaches the clipboard already escaped more often
/// than not — shell completion writes `Mobile\ Documents`, dragging a folder
/// into a terminal writes the same, and `com~apple~CloudDocs` picks up
/// backslashes too. Pasting that into a prompt is the ordinary thing to do,
/// and it produced `is not there` about a folder plainly sitting on screen.
///
/// The literal reading is tried first and always, so a directory whose name
/// genuinely contains a backslash or a quote is never taken away by this. The
/// unescaped readings are what it falls back to.
pub(crate) fn readings_of(raw: &str) -> Vec<PathBuf> {
    let trimmed = raw.trim();
    let unquoted = match (trimmed.chars().next(), trimmed.chars().last()) {
        (Some(a), Some(b)) if a == b && (a == '\'' || a == '"') && trimmed.len() > 1 => {
            &trimmed[1..trimmed.len() - 1]
        }
        _ => trimmed,
    };
    let mut out = Vec::new();
    for text in [trimmed, unquoted, &unescape(trimmed), &unescape(unquoted)] {
        let p = match text.strip_prefix("~/") {
            Some(rest) => crate::paths::home().join(rest),
            None if text == "~" => crate::paths::home(),
            None => PathBuf::from(text),
        };
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Drop one level of shell escaping: `\x` becomes `x`, for any `x`.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn remove_root(entry: &str) -> Result<(), String> {
    let readings = readings_of(entry);
    let wanted = readings.iter().find_map(|path| path.canonicalize().ok());
    let _lock = crate::cache::lock_for_write(&roots_file());
    let mut removed = None;
    let lines: Vec<String> = roots_lines()
        .into_iter()
        .filter(|line| {
            let matches = line == entry
                || wanted.as_ref().is_some_and(|wanted| resolved(line).as_ref() == Some(wanted));
            if matches {
                if removed.is_none() {
                    removed = Some(line.clone());
                }
                false
            } else {
                true
            }
        })
        .collect();
    let removed = removed.ok_or_else(|| format!("{} is not a search root", entry.trim()))?;
    write_roots(&lines)?;
    let _ = removed;
    Ok(())
}

/// The roots file as written, comments and all.
///
/// `compute::index_roots` answers "which directories does the index cover",
/// falling back to three defaults when the file is absent. This answers "what
/// does the file say", which is a different question the moment somebody wants
/// to edit it: removing a line from a fallback that was never written down is
/// not something a person can be shown.
fn roots_lines() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(roots_file()) else {
        // Materialise the defaults on first edit, so what is on screen and
        // what is in the file are the same list from then on.
        return crate::compute::index_roots()
            .iter()
            .map(|r| crate::paths::tilde(r))
            .collect();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn resolved(entry: &str) -> Option<PathBuf> {
    let p = match entry.strip_prefix("~/") {
        Some(rest) => crate::paths::home().join(rest),
        None => PathBuf::from(entry),
    };
    p.canonicalize().ok()
}

fn write_roots(lines: &[String]) -> Result<(), String> {
    let mut out = String::from(
        "# Folders Prelude indexes, one per line; ~/ is expanded.\n\
         # Written by the set: panel. Changes rebuild the index automatically.\n\n",
    );
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    crate::cache::write_state(&roots_file(), out.as_bytes()).map_err(|e| e.to_string())
}

/// Each root with its own state, for the detail view and the remove picker.
pub fn root_rows() -> Vec<(String, String)> {
    roots_lines()
        .into_iter()
        .map(|entry| {
            let state = match resolved(&entry) {
                Some(p) if p.is_dir() => "available".to_string(),
                Some(p) => format!("not a folder: {}", p.display()),
                None => "missing".to_string(),
            };
            (entry, state)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The settings themselves
// ---------------------------------------------------------------------------

/// How Enter edits a setting, which is not the same question for all of them.
///
/// `Open` is the honest default for the four that are a *list* — their file is
/// the editor, and Prelude offers only the mutations it can make safely.
pub const EDIT_PROMPT: &str = "prompt";
pub const EDIT_TOGGLE: &str = "toggle";
pub const EDIT_COLLECTION: &str = "collection";
pub const EDIT_MANAGE_ROOTS: &str = "manage-roots";
pub const EDIT_REBUILD: &str = "rebuild";

fn ago(secs: u64) -> String {
    match secs {
        s if s < 90 => "just now".into(),
        s if s < 5400 => format!("{}m ago", s / 60),
        s if s < 172_800 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86400),
    }
}

fn count_of(path: &Path, section: Option<&str>) -> usize {
    let Ok(text) = std::fs::read_to_string(path) else { return 0 };
    match section {
        Some(s) => crate::minitoml::parse(&text).get(s).map(|t| t.len()).unwrap_or(0),
        None => crate::minitoml::parse(&text)
            .keys()
            .filter(|k| !k.is_empty())
            .count(),
    }
}

fn setting_group(id: &str) -> &'static str {
    match id {
        "roots" | "index" | "fallbacks" => "Search",
        "hotkey" | "paneldir" | "key" => "Launcher",
        "preview" | "enter" | "update" => "Behavior",
        _ => "Library",
    }
}

fn row(id: &str, title: &str, value: String, detail: &str, edit: &str) -> Item {
    Item::new(format!("set:{id}"), Kind::Setting)
        .title(title)
        .sub(setting_group(id))
        .fields([value, detail.to_string()])
        .put("setting", id)
        .put("group", setting_group(id))
        .put("edit", edit)
}

fn preference_row(
    id: &str,
    title: &str,
    value: String,
    base_detail: &str,
    edit: &str,
    meta: (&str, &str, &str),
) -> Item {
    let (pref_key, default, environment) = meta;
    let source = pref_source(pref_key, environment);
    let visible_detail = if source == "environment" {
        format!("{base_detail} · overridden by ${environment}")
    } else {
        base_detail.to_string()
    };
    row(id, title, value, &visible_detail, edit)
    .put("source", source)
    .put("default", default)
    .put("environment", environment)
    .put("path", file().to_string_lossy().into_owned())
}

/// Every setting, with its current value on the row.
///
/// Cheap by construction: a handful of files under two kilobytes each, plus
/// two `stat`s. Nothing here reads the file index, which is a megabyte of
/// paths — its size is recorded when it is built.
pub fn items() -> Vec<Item> {
    let config = crate::paths::config();
    let mut v = Vec::new();

    // Search roots first: it is the one that decides what another scope can
    // find at all, and the one nothing else in the launcher mentions.
    let roots = root_rows();
    let indexed = crate::compute::fileindex_path();
    let counts = index_counts();
    let age = mtime(&indexed).map(|t| ago(crate::bus::now().saturating_sub(t)));
    let missing = roots.iter().filter(|(_, s)| s != "available").count();
    let value = if missing == 0 {
        format!("{} folders · all available", roots.len())
    } else {
        format!("{} folders · {missing} missing", roots.len())
    };
    let roots_source = if roots_file().is_file() { "saved" } else { "built-in" };
    v.push(
        row(
            "roots",
            "Search folders",
            value,
            "choose where file and folder search looks",
            EDIT_MANAGE_ROOTS,
        )
        .put("source", roots_source)
        .put("path", roots_file().to_string_lossy().into_owned()),
    );

    // Rebuilding is automatic after root edits; this row remains an explicit
    // repair door and an honest status surface.
    let stale = index_needs_rebuild();
    let index_value = match (counts, &age) {
        (Some(counts), Some(a)) => {
            format!("{} files · {} folders · updated {a}", counts.files, counts.folders)
        }
        (None, Some(a)) => format!("updated {a}"),
        _ if crate::compute::index_building() => "building…".into(),
        _ => "preparing…".into(),
    };
    let index_detail = if crate::compute::index_building() {
        "updating in the background · the previous index stays available"
    } else if stale {
        "update scheduled · the previous index stays available"
    } else {
        "current · names and Finder tags for direct, f: and dir: search"
    };
    v.push(
        row("index", "Search index", index_value, index_detail, EDIT_REBUILD)
            .put("path", indexed.to_string_lossy().into_owned())
            .put("source", if stale { "stale" } else { "current" }),
    );

    // The Search group, because it is what an unanswerable query answers with.
    //
    // The value column is the list exactly as it is stored, because `get`
    // prints it and `set` has to accept what `get` printed. What the keywords
    // resolve to is named in Details, and anything unusable is called out in
    // the Effect column, the way an environment override already is.
    let (good, bad) = fallback_state();
    let effect = if good.is_empty() {
        format!(
            "none of these work · the built-in {} search is used",
            crate::compute::WEB_SEARCH_NAME
        )
    } else if bad.is_empty() {
        "providers offered in order for a query with no answer".to_string()
    } else {
        format!("in order for a query with no answer · {} unusable", bad.len())
    };
    v.push(preference_row(
        "fallbacks",
        "When nothing matches",
        fallbacks(),
        &effect,
        EDIT_PROMPT,
        ("fallbacks", DEFAULT_FALLBACKS, ""),
    ));

    let global = crate::global::configured_summary();
    // Installed is a file check. Whether the helper is running needs `pgrep`
    // and belongs in explicit `global status`, never on the gather path.
    let panel_state = if global.installed { "panel installed" } else { "panel not installed" };
    v.push(
        row(
            "hotkey",
            "Global hotkey",
            global.hotkey,
            &format!("opens Prelude anywhere · {panel_state}"),
            EDIT_PROMPT,
        )
        .put("source", global.hotkey_source)
        .put("default", "cmd+shift+space")
        .put("path", crate::global::config_file().to_string_lossy().into_owned()),
    );
    v.push(
        row(
            "paneldir",
            "Panel directory",
            crate::paths::tilde(&global.directory),
            "working folder used by both launcher entry points",
            EDIT_PROMPT,
        )
        .put("source", global.directory_source)
        .put("default", crate::paths::tilde(&crate::paths::home().to_string_lossy()))
        .put("path", crate::global::config_file().to_string_lossy().into_owned()),
    );

    v.push(preference_row(
        "key",
        "Launcher key at a shell",
        launcher_key(),
        "opens Prelude from a shell · applies to new shells",
        EDIT_PROMPT,
        ("key", DEFAULT_KEY, "PRELUDE_KEY"),
    ));
    v.push(preference_row(
        "preview",
        "Quick Look",
        if preview_enabled() { "on".into() } else { "off".into() },
        "show selected files and clipboard content with Ctrl+P",
        EDIT_TOGGLE,
        ("preview", "on", "PRELUDE_NO_PREVIEW"),
    ));
    v.push(preference_row(
        "enter",
        "What Enter does",
        if classic_enter() { "copy everything".into() } else { "per kind".into() },
        "open objects directly; keep commands reviewable",
        EDIT_TOGGLE,
        ("classic_enter", "per kind", "PRELUDE_CLASSIC_ENTER"),
    ));
    // The one setting that decides whether Prelude reaches the network without
    // being asked, so its current value is on the row rather than in a manual.
    let version_note = match crate::update::panel_is_stale() {
        Some(running) => format!("running {running} · binary is {}", crate::update::VERSION),
        None => format!("you have {}", crate::update::VERSION),
    };
    v.push(preference_row(
        "update",
        "Updates",
        update_mode(),
        &format!("update policy · {version_note}"),
        EDIT_TOGGLE,
        ("update", DEFAULT_UPDATE, "PRELUDE_UPDATE"),
    ));

    let openwith = config.join("open.toml");
    let n = count_of(&openwith, Some("apps"));
    v.push(
        row(
            "openwith",
            "Open-with rules",
            if n == 0 { "none yet".into() } else { format!("{n} extensions") },
            "remember which apps open each file type",
            EDIT_COLLECTION,
        )
        .put("source", if openwith.is_file() { "saved" } else { "none" })
        .put("path", openwith.to_string_lossy().into_owned()),
    );

    let snippets = config.join("snippets.toml");
    let n = count_of(&snippets, None);
    v.push(
        row(
            "snippets",
            "Snippets",
            if n == 0 { "none yet".into() } else { format!("{n} saved") },
            "reusable commands with fill-in blanks",
            EDIT_COLLECTION,
        )
        .put("source", if snippets.is_file() { "saved" } else { "built-in" })
        .put("path", snippets.to_string_lossy().into_owned()),
    );

    let quicklinks = config.join("quicklinks.toml");
    let n = count_of(&quicklinks, None);
    v.push(
        row(
            "quicklinks",
            "Quicklinks",
            if n == 0 { "none yet".into() } else { format!("{n} keywords") },
            "named shortcuts to files, folders and web searches",
            EDIT_COLLECTION,
        )
        .put("source", if quicklinks.is_file() { "saved" } else { "built-in" })
        .put("path", quicklinks.to_string_lossy().into_owned()),
    );

    let favorites = config.join("favorites.txt");
    let n = std::fs::read_to_string(&favorites)
        .map(|t| t.lines().filter(|l| l.contains('\t')).count())
        .unwrap_or(0);
    v.push(
        row(
            "favorites",
            "Favorites",
            if n == 0 { "none yet".into() } else { format!("{n} promoted") },
            "promote agents, skills, servers, apps and quicklinks",
            EDIT_COLLECTION,
        )
        .put("source", if favorites.is_file() { "saved" } else { "none" })
        .put("path", favorites.to_string_lossy().into_owned()),
    );

    let aliases = config.join("aliases.txt");
    let n = std::fs::read_to_string(&aliases)
        .map(|t| t.lines().filter(|l| l.contains('\t')).count())
        .unwrap_or(0);
    v.push(
        row(
            "aliases",
            "Aliases",
            if n == 0 { "none yet".into() } else { format!("{n} named") },
            "type a name you chose to go straight to the object",
            EDIT_COLLECTION,
        )
        .put("source", if aliases.is_file() { "saved" } else { "none" })
        .put("path", aliases.to_string_lossy().into_owned()),
    );

    // This is a settings form, not a learned catalogue: keep related controls
    // together. The 100-point gap is wider than the frecency cap, so opening
    // one row often cannot scatter the form on the next launch.
    let count = v.len();
    v.into_iter()
        .enumerate()
        .map(|(index, item)| item.rank(((count - index) * 100) as f64))
        .collect()
}

/// The full current value, for Quick Look and for `Show them`.
pub fn detail(it: &Item) -> Vec<String> {
    let mut out = Vec::new();
    match it.get("setting") {
        "roots" => {
            for (entry, state) in root_rows() {
                out.push(format!("  {entry}  ({state})"));
            }
            out.push(String::new());
            out.push("`f:` searches these roots plus the current project.".into());
        }
        "index" => {
            match index_counts() {
                Some(counts) => out.push(format!(
                    "{} files and {} folders in the shared index",
                    counts.files, counts.folders
                )),
                None => out.push("the index is being prepared".into()),
            }
            if index_needs_rebuild() {
                out.push("Prelude is updating it in the background.".into());
            }
            out.push("Rebuilding walks every search root and records names and Finder tags.".into());
        }
        "hotkey" => {
            out.push(format!("Current global chord: {}", it.fields[0]));
            out.push("Changing it checks known macOS conflicts and re-registers the panel.".into());
            out.push("Left restores Cmd+Shift+Space; right or Enter asks for a new chord.".into());
            setting_origin(it, &mut out);
        }
        "paneldir" => {
            out.push(format!("Current launcher directory: {}", it.fields[0]));
            out.push("Both the global panel and shell launcher gather project rows from here.".into());
            out.push("Left restores $HOME; right or Enter chooses another existing directory.".into());
            setting_origin(it, &mut out);
        }
        "key" => {
            out.push(format!("Current shell key: {}", it.fields[0]));
            out.push("Bound by `prelude init zsh`, which runs when a shell starts,".into());
            out.push("so a change reaches the next shell rather than this one.".into());
            setting_origin(it, &mut out);
        }
        "preview" => {
            out.push("Quick Look controls Ctrl+P and contextual clipboard previews.".into());
            out.push("Left sets it off; right sets it on; Enter toggles it.".into());
            setting_origin(it, &mut out);
        }
        "enter" => {
            out.push("Per kind opens objects and keeps commands reviewable.".into());
            out.push("Copy everything applies only to payload rows; Settings remains operable.".into());
            out.push("Left chooses per kind; right chooses copy everything; Enter toggles.".into());
            setting_origin(it, &mut out);
        }
        "update" => {
            out.push("off: make no automatic request".into());
            out.push("notify: check and tell you (default)".into());
            out.push("download: verify and stage a release".into());
            out.push("apply: install it when the panel next starts".into());
            out.push("Left/right step without wrapping; Enter shows all four choices.".into());
            setting_origin(it, &mut out);
        }
        "fallbacks" => {
            let (good, bad) = fallback_state();
            out.push("Offered in this order when a query matches nothing:".into());
            if good.is_empty() {
                out.push(format!("  {} (built in)", crate::compute::WEB_SEARCH_NAME));
            } else {
                out.extend(good.iter().enumerate().map(|(n, name)| format!("  {}. {name}", n + 1)));
            }
            for key in &bad {
                out.push(format!("  {key} — no quicklink of that name takes a {{q}}"));
            }
            out.push(String::new());
            out.push("Each row shows the query itself, so it survives the search that made it.".into());
            out.push("A query inside a scope gets none of them.".into());
            setting_origin(it, &mut out);
        }
        "openwith" | "snippets" | "quicklinks" | "favorites" | "aliases" => {
            let rows = collection_rows(it.get("setting"));
            if rows.is_empty() {
                out.push(format!("No {}s yet.", collection_singular(it.get("setting"))));
            } else {
                out.extend(rows.into_iter().take(40).map(|(_, name, detail)| {
                    if detail.is_empty() { format!("  {name}") } else { format!("  {name}  ({detail})") }
                }));
            }
            out.push(String::new());
            out.push("Left removes one; right adds one; Enter opens the collection manager.".into());
        }
        _ => {}
    }
    out
}

fn setting_origin(it: &Item, out: &mut Vec<String>) {
    out.push(String::new());
    out.push(format!("source: {}", it.get("source")));
    if !it.get("default").is_empty() {
        out.push(format!("default: {}", it.get("default")));
    }
    if !it.get("environment").is_empty() {
        out.push(format!("environment override: ${}", it.get("environment")));
    }
    if it.get("source") == "environment" {
        out.push("The environment wins until it is unset in that shell.".into());
    }
}

fn ensure_setting_file(it: &Item) -> Result<PathBuf, String> {
    let path = match it.get("setting") {
        "roots" => {
            if !roots_file().exists() {
                write_roots(&roots_lines())?;
            }
            roots_file()
        }
        "openwith" => crate::openwith::ensure_file()?,
        "snippets" => crate::sources::user::ensure_snippets_file()?,
        "quicklinks" => crate::compute::ensure_quicklinks_file()?,
        "favorites" => crate::favorites::ensure_file()?,
        "aliases" => crate::aliases::ensure_file()?,
        "key" | "preview" | "enter" | "fallbacks" => {
            if !file().exists() {
                crate::cache::write_atomic(
                    &file(),
                    b"# Prelude's own preferences. Environment variables override these values.\n",
                )
                .map_err(|e| e.to_string())?;
            }
            file()
        }
        _ => PathBuf::from(it.get("path")),
    };
    if path.as_os_str().is_empty() {
        Err("that setting has no file of its own".into())
    } else if !path.exists() {
        Err(format!("{} does not exist yet", crate::paths::tilde(&path.to_string_lossy())))
    } else {
        Ok(path)
    }
}

pub fn open_file(it: &Item) -> i32 {
    let result = ensure_setting_file(it).and_then(|path| {
        crate::openwith::open_default_now(&path.to_string_lossy())
    });
    match result {
        Ok(()) => 0,
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

/// Carry out Enter for a setting.
pub fn edit(it: &Item) -> i32 {
    match it.get("edit") {
        EDIT_MANAGE_ROOTS => manage_roots_interactively(),
        EDIT_REBUILD => crate::runhere::run_cmd("prelude index"),
        EDIT_TOGGLE if it.get("setting") == "update" => choose_update_mode(),
        EDIT_TOGGLE => toggle(it),
        EDIT_PROMPT => prompt_for(it),
        EDIT_COLLECTION => manage_collection(it),
        _ => open_file(it),
    }
}

fn choose_folder() -> Result<Option<String>, String> {
    let output = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "activate",
            "-e",
            "POSIX path of (choose folder with prompt \"Choose a folder Prelude should search\")",
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not open the folder chooser: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        if error.to_ascii_lowercase().contains("user canceled") {
            return Ok(None);
        }
        return Err(format!("the folder chooser failed: {}", error.trim()));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().trim_end_matches('/').to_string();
    Ok((!path.is_empty()).then_some(path))
}

pub fn add_root_interactively() -> i32 {
    let raw = match choose_folder() {
        Ok(Some(path)) => path,
        Ok(None) => return 130,
        Err(error) => {
            crate::ui::note(&error);
            return 2;
        }
    };
    match add_root(&raw) {
        Ok(added) => {
            crate::compute::ensure_fileindex();
            crate::ui::note(&format!("{added} added — file search is updating"));
            0
        }
        Err(e) => {
            crate::ui::note(&e);
            2
        }
    }
}

pub fn manage_roots_interactively() -> i32 {
    let rows = root_rows();
    let mut choices = vec![(
        "add".to_string(),
        "Add a folder…".to_string(),
        "opens the macOS folder chooser".to_string(),
    )];
    choices.extend(rows.iter().enumerate().map(|(index, (entry, state))| {
        let detail = if state == "available" {
            entry.clone()
        } else {
            format!("{entry} · {state}")
        };
        (format!("root:{index}"), root_name(entry), detail)
    }));
    let Some(choice) = crate::actions::pick_one(" search folders ", &choices) else {
        return 130;
    };
    if choice == "add" {
        return add_root_interactively();
    }
    let Some(index) = choice.strip_prefix("root:").and_then(|index| index.parse::<usize>().ok()) else {
        return 2;
    };
    let Some((entry, state)) = rows.get(index) else { return 2 };
    let mut actions = Vec::new();
    if state == "available" {
        actions.push(("show".to_string(), "Show in Finder".to_string(), entry.clone()));
    }
    actions.push((
        "remove".to_string(),
        "Remove from search…".to_string(),
        "the folder and its contents stay untouched".to_string(),
    ));
    let Some(action) = crate::actions::pick_one(" search folder ", &actions) else {
        return 130;
    };
    if action == "show" {
        let Some(path) = resolved(entry) else {
            crate::ui::note("that folder is no longer available");
            return 2;
        };
        return match crate::openwith::open_finder_now(&path.to_string_lossy()) {
            Ok(()) => 0,
            Err(error) => {
                crate::ui::note(&error);
                2
            }
        };
    }
    remove_root_confirmed(entry)
}

/// Short, item-specific arrow labels shown directly under the settings table.
/// These are deliberately verbs, not generic "Previous"/"Next" everywhere:
/// a collection is added to or removed from, while a boolean is set on/off.
pub fn direction_label(it: &Item, direction: &str) -> &'static str {
    let left = direction == "left";
    match it.get("setting") {
        "roots" => if left { "Remove folder" } else { "Add folder" },
        "index" => if left { "Show status" } else { "Rebuild now" },
        "hotkey" | "paneldir" | "key" | "fallbacks" => if left { "Reset default" } else { "Change…" },
        "preview" => if left { "Set off" } else { "Set on" },
        "enter" => if left { "Per kind" } else { "Copy everything" },
        "update" => if left { "Previous mode" } else { "Next mode" },
        "openwith" | "snippets" | "quicklinks" | "favorites" | "aliases" => {
            if left { "Remove one…" } else { "Add one…" }
        }
        _ => if left { "Previous" } else { "Next" },
    }
}

/// Whether an arrow opens a chooser/prompt or performs an immediate mutation.
/// fzf uses this to choose visible `execute` for interactive flows and quiet
/// `execute-silent` for one-step enum changes.
pub fn direction_is_interactive(it: &Item, direction: &str) -> bool {
    if it.get("source") == "environment" {
        return true;
    }
    !matches!(
        (it.get("setting"), direction),
        ("preview" | "enter" | "update", _)
            | ("hotkey" | "paneldir" | "key" | "fallbacks", "left")
    )
}

/// Apply the semantic left/right action for one settings row.
pub fn adjust(it: &Item, direction: &str) -> i32 {
    if it.get("source") == "environment" {
        let variable = it.get("environment");
        let lines = vec![
            format!("${variable} currently supplies this value."),
            "Prelude did not save a competing value that would appear later.".into(),
            "Unset the variable in that launch environment before changing this setting.".into(),
        ];
        return crate::runhere::show_text("Setting overridden by the environment", &lines);
    }
    let left = direction == "left";
    match it.get("setting") {
        "roots" => if left { remove_root_interactively() } else { add_root_interactively() },
        "index" => if left { show_index_status() } else { crate::runhere::run_cmd("prelude index") },
        "hotkey" | "paneldir" | "key" | "fallbacks" => {
            if left { reset_item(it) } else { prompt_for(it) }
        }
        "preview" => set_boolean("preview", !left),
        "enter" => set_boolean("classic_enter", !left),
        "update" => step_update(if left { -1 } else { 1 }),
        "openwith" | "snippets" | "quicklinks" | "favorites" | "aliases" => {
            if left { remove_collection_item(it) } else { add_collection_item(it) }
        }
        _ => 2,
    }
}

fn show_index_status() -> i32 {
    let item = items().into_iter().find(|item| item.get("setting") == "index");
    let lines = item.as_ref().map(detail).unwrap_or_else(|| vec!["index status unavailable".into()]);
    crate::runhere::show_text("Prelude file and folder index", &lines)
}

fn choose_update_mode() -> i32 {
    let current = update_mode();
    let choices = [
        ("off".to_string(), "Off".to_string(), "never contact the update service".to_string()),
        ("notify".to_string(), "Notify".to_string(), "check and tell you".to_string()),
        ("download".to_string(), "Download".to_string(), "verify and stage the release".to_string()),
        ("apply".to_string(), "Apply".to_string(), "install it when the panel next starts".to_string()),
    ];
    let Some(mode) = crate::actions::pick_one(
        &format!(" update mode · current {current} "),
        &choices,
    ) else {
        return 130;
    };
    set_update(&mode)
}

const UPDATE_MODES: [&str; 4] = ["off", "notify", "download", "apply"];

fn stepped_update(current: &str, delta: isize) -> &'static str {
    let at = UPDATE_MODES.iter().position(|mode| *mode == current).unwrap_or(1) as isize;
    let next = (at + delta).clamp(0, UPDATE_MODES.len() as isize - 1) as usize;
    UPDATE_MODES[next]
}

fn step_update(delta: isize) -> i32 {
    set_update(stepped_update(&update_mode(), delta))
}

fn set_update(mode: &str) -> i32 {
    if let Err(error) = write_pref("update", mode) {
        crate::ui::note(&error);
        return 2;
    }
    if env("PRELUDE_UPDATE").is_some() {
        crate::ui::note("saved, but $PRELUDE_UPDATE remains effective in this environment");
    }
    0
}

fn set_boolean(key: &str, value: bool) -> i32 {
    if let Err(error) = write_pref(key, if value { "true" } else { "false" }) {
        crate::ui::note(&error);
        return 2;
    }
    let overridden = match key {
        "preview" => env("PRELUDE_NO_PREVIEW").is_some(),
        _ => env("PRELUDE_CLASSIC_ENTER").is_some(),
    };
    if overridden {
        crate::ui::note("saved, but the environment variable remains effective");
    }
    0
}

fn remove_root_interactively() -> i32 {
    let rows = root_rows();
    if rows.is_empty() {
        crate::ui::note("there are no search folders to remove");
        return 0;
    }
    let choices: Vec<(String, String, String)> = rows
        .iter()
        .enumerate()
        .map(|(index, (entry, state))| (index.to_string(), root_name(entry), format!("{entry} · {state}")))
        .collect();
    let Some(index) = crate::actions::pick_one(" remove search folder ", &choices)
        .and_then(|index| index.parse::<usize>().ok())
    else {
        return 130;
    };
    let Some((entry, _)) = rows.get(index) else { return 2 };
    remove_root_confirmed(entry)
}

fn root_name(entry: &str) -> String {
    Path::new(entry.trim_end_matches('/'))
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(entry)
        .to_string()
}

fn remove_root_confirmed(entry: &str) -> i32 {
    if !crate::ui::confirm(
        &format!("stop searching {entry}?"),
        "Remove from search",
        "the folder and every file in it stay untouched",
    ) {
        return 130;
    }
    match remove_root(entry) {
        Ok(()) => {
            crate::compute::ensure_fileindex();
            crate::ui::note(&format!("{entry} removed — file search is updating"));
            0
        }
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

fn collection_rows(id: &str) -> Vec<(String, String, String)> {
    match id {
        "openwith" => crate::openwith::rules()
            .into_iter()
            .map(|(extension, app)| {
                let label = if extension == "*" { "all other files".into() } else { format!(".{extension}") };
                (extension, label, app)
            })
            .collect(),
        "snippets" => crate::sources::user::snippet_entries()
            .into_iter()
            .map(|(name, command)| (name.clone(), name, command))
            .collect(),
        "quicklinks" => crate::compute::quicklink_scope_rows()
            .into_iter()
            .map(|item| {
                let key = item.get("quicklink").to_string();
                let target = if item.get("quicklink_target").is_empty() {
                    item.cmd.clone()
                } else {
                    item.get("quicklink_target").to_string()
                };
                (key, item.title, target)
            })
            .collect(),
        "favorites" => crate::favorites::entries()
            .into_iter()
            .map(|key| {
                let (kind, name) = key.split_once('\t').unwrap_or(("object", &key));
                (key.clone(), name.to_string(), kind.to_string())
            })
            .collect(),
        // The key is the alias, because that is what `aliases::remove` takes
        // and what the person typed. The object it names is the detail.
        "aliases" => crate::aliases::entries()
            .into_iter()
            .map(|(alias, target)| {
                let (kind, name) = target.split_once('\t').unwrap_or(("object", &target));
                (alias.clone(), alias, format!("{name} · {kind}"))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn manage_collection(it: &Item) -> i32 {
    let id = it.get("setting");
    loop {
        let rows = collection_rows(id);
        let mut choices = vec![(
            "add".to_string(),
            format!("Add {}…", collection_singular(id)),
            "→ from the Settings row".to_string(),
        )];
        choices.extend(rows.iter().enumerate().map(|(index, (_, name, detail))| {
            (format!("item:{index}"), name.clone(), detail.clone())
        }));
        let Some(choice) = crate::actions::pick_one(&format!(" {} ", it.title.to_lowercase()), &choices) else {
            return 130;
        };
        if choice == "add" {
            return add_collection_item(it);
        }
        let Some(index) = choice.strip_prefix("item:").and_then(|value| value.parse::<usize>().ok()) else {
            return 2;
        };
        let Some((key, name, detail)) = rows.get(index) else { return 2 };
        let actions = vec![
            ("remove".to_string(), format!("Remove {name}…"), detail.clone()),
            ("file".to_string(), "Open the backing file".to_string(), crate::paths::tilde(it.get("path"))),
        ];
        match crate::actions::pick_one(&format!(" {name} "), &actions).as_deref() {
            Some("remove") => return remove_collection_key(id, key, name),
            Some("file") => return open_file(it),
            _ => continue,
        }
    }
}

fn collection_singular(id: &str) -> &'static str {
    match id {
        "openwith" => "rule",
        "snippets" => "snippet",
        "quicklinks" => "Quicklink",
        "favorites" => "Favorite",
        "aliases" => "alias",
        _ => "item",
    }
}

fn add_collection_item(it: &Item) -> i32 {
    match it.get("setting") {
        "openwith" => add_openwith_rule(),
        "snippets" => add_snippet_interactively(),
        "quicklinks" => add_quicklink_interactively(),
        "favorites" => add_favorite_interactively(),
        "aliases" => add_alias_interactively(),
        _ => 2,
    }
}

fn remove_collection_item(it: &Item) -> i32 {
    let rows = collection_rows(it.get("setting"));
    if rows.is_empty() {
        crate::ui::note(&format!("there are no {}s to remove", collection_singular(it.get("setting"))));
        return 0;
    }
    let choices: Vec<(String, String, String)> = rows
        .iter()
        .enumerate()
        .map(|(index, (_, name, detail))| (index.to_string(), name.clone(), detail.clone()))
        .collect();
    let Some(index) = crate::actions::pick_one(
        &format!(" remove {} ", collection_singular(it.get("setting"))),
        &choices,
    ).and_then(|value| value.parse::<usize>().ok()) else {
        return 130;
    };
    let Some((key, name, _)) = rows.get(index) else { return 2 };
    remove_collection_key(it.get("setting"), key, name)
}

fn remove_collection_key(id: &str, key: &str, name: &str) -> i32 {
    let confirm = matches!(id, "snippets" | "quicklinks");
    if confirm && !crate::ui::confirm(
        &format!("remove {name}?"),
        &format!("Remove {name}"),
        if id == "quicklinks" { "the target is untouched" } else { "the command is removed from Prelude" },
    ) {
        return 130;
    }
    let result = match id {
        "openwith" => crate::openwith::forget(key),
        "snippets" => crate::sources::user::remove_snippet(key),
        "quicklinks" => crate::compute::remove_quicklink(key),
        "favorites" => crate::favorites::remove_key(key),
        "aliases" => crate::aliases::remove(key),
        _ => Err("that setting is not a collection".into()),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

fn add_openwith_rule() -> i32 {
    let Some(raw) = crate::ui::prompt_line(" file extension · * means all other files ") else {
        return 130;
    };
    let extension = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    if extension.is_empty()
        || (extension != "*" && !extension.chars().all(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_')))
    {
        crate::ui::note("use an extension without the dot, or * for all other files");
        return 2;
    }
    let Some(app) = crate::openwith::pick_app(&format!(".{extension} files")) else {
        return 130;
    };
    match crate::openwith::remember(&extension, &app) {
        Ok(()) => 0,
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

fn add_snippet_interactively() -> i32 {
    let Some(name) = crate::ui::prompt_line(" snippet name ") else { return 130 };
    let Some(command) = crate::ui::prompt_line(" command · {{name}} marks a blank ") else { return 130 };
    match crate::sources::user::add_snippet(&name, &command) {
        Ok(()) => 0,
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

fn add_quicklink_interactively() -> i32 {
    let Some(target) = crate::ui::prompt_line(" target · path, URL, or URL with {q} ") else {
        return 130;
    };
    let (kind, stored) = match crate::compute::resolve_quicklink_target(&target) {
        Ok(value) => value,
        Err(error) => {
            crate::ui::note(&error);
            return 2;
        }
    };
    let Some(key) = crate::ui::prompt_line(" Quicklink keyword ") else { return 130 };
    let default_name = Path::new(stored.trim_end_matches('/'))
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(key.trim());
    let Some(name) = crate::ui::prompt_line_initial(" name shown in search ", default_name) else {
        return 130;
    };
    let draft = crate::compute::QuicklinkDraft { name, kind, target: stored };
    match crate::compute::create_quicklink_from(&key, &draft) {
        Ok(_) => 0,
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

fn add_favorite_interactively() -> i32 {
    // The whole catalogue, not `gather_agents`: applications and saved
    // Quicklinks can be favourited from their own action panels, and a manager
    // that can remove a kind it cannot add is half a collection. `gather`
    // already decorates, so the retain below reads a marked list.
    let mut items = crate::cache::gather();
    items.retain(|item| crate::favorites::key(item).is_some() && item.get("favorite") != "true");
    items.sort_by(crate::cache::by_rank);
    let choices: Vec<(String, String, String)> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (index.to_string(), item.title.clone(), item.style().1.to_string()))
        .collect();
    if choices.is_empty() {
        crate::ui::note("everything that can be a Favorite already is");
        return 0;
    }
    let Some(index) = crate::actions::pick_one(" add Favorite ", &choices)
        .and_then(|value| value.parse::<usize>().ok())
    else {
        return 130;
    };
    let Some(item) = items.get(index) else { return 2 };
    match crate::favorites::set(item, true) {
        Ok(()) => 0,
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

/// Pick an object, then name it.
///
/// The object comes first because it is the part a person can recognise; the
/// name is theirs to invent and is the only part that can be refused. The
/// refusal comes from `aliases::vet`, the same door `prelude alias` uses, so
/// the two surfaces cannot disagree about what a name may be.
fn add_alias_interactively() -> i32 {
    let mut items = crate::cache::gather();
    items.retain(|item| crate::favorites::key(item).is_some() && item.get("alias").is_empty());
    items.sort_by(crate::cache::by_rank);
    if items.is_empty() {
        crate::ui::note("everything that can be named already has a name");
        return 0;
    }
    let choices: Vec<(String, String, String)> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (index.to_string(), item.title.clone(), item.style().1.to_string()))
        .collect();
    let Some(item) = crate::actions::pick_one(" name an object ", &choices)
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(|index| items.get(index))
    else {
        return 130;
    };
    let Some(raw) = crate::ui::prompt_line(&format!(" what to call {} ", item.title)) else {
        return 130;
    };
    match crate::aliases::add(&raw, item) {
        Ok(key) => {
            crate::ui::note(&format!("{key} now goes to {}", item.title));
            0
        }
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

fn toggle(it: &Item) -> i32 {
    // Four values rather than two, so it cycles. `off` is deliberately last:
    // reaching it takes an explicit walk rather than one stray keypress.
    if it.get("setting") == "update" {
        let next = match update_mode().as_str() {
            "notify" => "download",
            "download" => "apply",
            "apply" => "off",
            _ => "notify",
        };
        if let Err(e) = write_pref("update", next) {
            crate::ui::note(&e);
            return 2;
        }
        let said = match next {
            "off" => "updates: never checked".to_string(),
            "notify" => "updates: tell me".to_string(),
            "download" => "updates: verify and stage them".to_string(),
            _ => "updates: install as the panel next starts".to_string(),
        };
        crate::ui::note(&if env("PRELUDE_UPDATE").is_some() {
            format!("{said} — but $PRELUDE_UPDATE is set and remains effective")
        } else {
            said
        });
        return 0;
    }
    // The main launcher caches runtime preferences for the duration of one
    // panel, while arrow adjustments reload live rows through a subprocess.
    // Toggle from the row now on screen, not that older process cache, so
    // `→` followed by Enter cannot write the same value twice or flip backward.
    let (key, now) = match it.get("setting") {
        "preview" => ("preview", it.fields.first().map(String::as_str) != Some("on")),
        "enter" => ("classic_enter", it.fields.first().map(String::as_str) != Some("copy everything")),
        _ => return 2,
    };
    // The environment variable, where it is set, is what the launcher will
    // actually obey — so writing the file and reporting success would be a
    // setting that visibly does nothing.
    let overridden = match key {
        "preview" => env("PRELUDE_NO_PREVIEW").is_some(),
        _ => env("PRELUDE_CLASSIC_ENTER").is_some(),
    };
    if let Err(e) = write_pref(key, if now { "true" } else { "false" }) {
        crate::ui::note(&e);
        return 2;
    }
    let said = match (key, now) {
        ("preview", true) => "Quick Look on".to_string(),
        ("preview", false) => "Quick Look off".to_string(),
        (_, true) => "Enter copies everything".to_string(),
        (_, false) => "Enter acts per kind".to_string(),
    };
    crate::ui::note(&if overridden {
        format!("{said} — but the environment variable is set and still wins")
    } else {
        format!("{said} — takes effect the next time you open the launcher")
    });
    0
}

fn prompt_for(it: &Item) -> i32 {
    let id = it.get("setting");
    let current = it.fields.first().cloned().unwrap_or_default();
    let label = match id {
        "hotkey" => " global chord ",
        "paneldir" => " panel directory ",
        "key" => " launcher key ",
        _ => " value ",
    };
    let Some(value) = crate::ui::prompt_line_initial(label, &current) else {
        return 130;
    };
    let result = set_named(id, &value);
    match result {
        Ok(message) => {
            crate::ui::note(&message);
            0
        }
        Err(e) => {
            crate::ui::note(&e);
            2
        }
    }
}

fn override_for(key: &str) -> Option<&'static str> {
    match key {
        "key" if env("PRELUDE_KEY").is_some_and(|v| validate_pref("key", &v).is_ok()) => {
            Some("PRELUDE_KEY")
        }
        "preview" if env("PRELUDE_NO_PREVIEW").is_some() => Some("PRELUDE_NO_PREVIEW"),
        "update" if env("PRELUDE_UPDATE").is_some() => Some("PRELUDE_UPDATE"),
        "classic_enter" if env("PRELUDE_CLASSIC_ENTER").is_some() => {
            Some("PRELUDE_CLASSIC_ENTER")
        }
        _ => None,
    }
}

fn set_named(raw_key: &str, raw_value: &str) -> Result<String, String> {
    let key = canonical_key(raw_key).ok_or_else(|| format!("unknown setting {raw_key:?}"))?;
    match key {
        "hotkey" => crate::global::set_hotkey(raw_value),
        "paneldir" => crate::global::set_directory(raw_value),
        key @ ("key" | "preview" | "classic_enter" | "update" | "fallbacks") => {
            let value = validate_pref(key, raw_value)?;
            write_pref(key, &value)?;
            let shown = match (key, value.as_str()) {
                ("preview", "true") => "preview = on".into(),
                ("preview", "false") => "preview = off".into(),
                ("classic_enter", "true") => "enter = copy everything".into(),
                ("classic_enter", "false") => "enter = per kind".into(),
                ("key", value) => format!("key = {value} — bound in the next shell you open"),
                (key, value) => format!("{key} = {value}"),
            };
            Ok(match override_for(key) {
                Some(variable) => format!(
                    "{shown} — saved, but ${variable} is set and remains effective"
                ),
                None => shown,
            })
        }
        _ => Err(format!("{key} is managed through its own action, not `settings set`")),
    }
}

fn reset_named(raw_key: &str) -> Result<String, String> {
    let key = canonical_key(raw_key).ok_or_else(|| format!("unknown setting {raw_key:?}"))?;
    match key {
        "all" => {
            let path = file();
            let mut text = std::fs::read_to_string(&path).unwrap_or_default();
            for key in PREF_KEYS {
                text = update_pref_text(&text, key, None);
            }
            crate::cache::write_state(&path, text.as_bytes()).map_err(|e| e.to_string())?;
            let variables: Vec<String> = PREF_KEYS
                .iter()
                .filter_map(|key| override_for(key).map(|variable| format!("${variable}")))
                .collect();
            let message = "key, Quick Look, Enter and Updates restored to defaults";
            Ok(if variables.is_empty() {
                message.into()
            } else {
                format!("{message} — {} still override them", variables.join(", "))
            })
        }
        "hotkey" => crate::global::set_hotkey("cmd+shift+space"),
        "paneldir" => crate::global::set_directory_default(),
        key @ ("key" | "preview" | "classic_enter" | "update" | "fallbacks") => {
            remove_pref(key)?;
            let default = match key {
                "key" => DEFAULT_KEY,
                "preview" => "on",
                "update" => DEFAULT_UPDATE,
                "fallbacks" => DEFAULT_FALLBACKS,
                _ => "per kind",
            };
            let shown = format!("{key} restored to {default}");
            Ok(match override_for(key) {
                Some(variable) => format!(
                    "{shown} — but ${variable} is set and remains effective"
                ),
                None => shown,
            })
        }
        _ => Err(format!("{key} has no scalar default to restore")),
    }
}

/// Restore the selected scalar preference from the action panel.
pub fn reset_item(it: &Item) -> i32 {
    match reset_named(it.get("setting")) {
        Ok(message) => {
            crate::ui::note(&message);
            0
        }
        Err(error) => {
            crate::ui::note(&error);
            2
        }
    }
}

#[derive(Serialize)]
struct SettingView {
    key: String,
    group: String,
    title: String,
    value: String,
    detail: String,
    source: String,
    default: Option<String>,
    environment: Option<String>,
    path: Option<String>,
}

fn view(it: &Item) -> SettingView {
    let optional = |value: &str| (!value.is_empty()).then(|| value.to_string());
    SettingView {
        key: it.get("setting").to_string(),
        group: it.get("group").to_string(),
        title: it.title.clone(),
        value: it.fields.first().cloned().unwrap_or_default(),
        detail: it.fields.get(1).cloned().unwrap_or_default(),
        source: it.get("source").to_string(),
        default: optional(it.get("default")),
        environment: optional(it.get("environment")),
        path: optional(it.get("path")).map(|p| crate::paths::tilde(&p)),
    }
}

#[derive(Serialize)]
struct Check {
    severity: &'static str,
    setting: String,
    message: String,
}

fn checks() -> Vec<Check> {
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string(file()) {
        let parsed = crate::minitoml::parse(&text);
        if let Some(root) = parsed.get("") {
            for (key, value) in root {
                if !PREF_KEYS.contains(&key.as_str()) {
                    out.push(Check {
                        severity: "warning",
                        setting: key.clone(),
                        message: "unknown key; Prelude leaves it untouched but does not use it".into(),
                    });
                } else if let Err(message) = validate_pref(key, value) {
                    out.push(Check { severity: "error", setting: key.clone(), message });
                }
            }
        }
    }
    // Syntax passed and the keyword still names nothing that can take a query.
    // Only resolvable here, against `quicklinks.toml`, and worth saying: the
    // setting looks saved and does nothing, which is the shape this file is
    // meant to make impossible.
    let (good, bad) = fallback_state();
    for key in bad {
        out.push(Check {
            severity: "warning",
            setting: "fallbacks".into(),
            message: format!("“{key}” is not a quicklink that takes a {{q}}; it is skipped"),
        });
    }
    if good.is_empty() {
        out.push(Check {
            severity: "warning",
            setting: "fallbacks".into(),
            message: format!(
                "nothing in the list works; the built-in {} search is used instead",
                crate::compute::WEB_SEARCH_NAME
            ),
        });
    }
    for (key, variable) in [("key", "PRELUDE_KEY")] {
        if let Some(value) = env(variable) {
            if let Err(message) = validate_pref(key, &value) {
                out.push(Check {
                    severity: "error",
                    setting: key.into(),
                    message: format!("${variable} is invalid: {message}"),
                });
            }
        }
    }
    for (entry, state) in root_rows() {
        if state != "available" {
            out.push(Check {
                severity: "warning",
                setting: "roots".into(),
                message: format!("{entry}: {state}"),
            });
        }
    }
    if index_needs_rebuild() {
        out.push(Check {
            severity: "warning",
            setting: "index".into(),
            message: if crate::compute::index_building() {
                "the file and folder index is updating for the current roots".into()
            } else {
                "the file and folder index needs an update for the current roots".into()
            },
        });
    }
    out
}

fn check(json: bool) -> i32 {
    let checks = checks();
    if json {
        println!("{}", serde_json::to_string_pretty(&checks).unwrap_or_else(|_| "[]".into()));
    } else if checks.is_empty() {
        println!("Prelude settings are valid and the file and folder index is current.");
    } else {
        for check in &checks {
            println!("{:<8} {:<16} {}", check.severity, check.setting, check.message);
        }
    }
    if checks.iter().any(|check| check.severity == "error") {
        2
    } else if checks.is_empty() {
        0
    } else {
        1
    }
}

fn get(raw_key: &str, json: bool) -> i32 {
    let Some(key) = canonical_key(raw_key) else {
        eprintln!("prelude: unknown setting {raw_key:?}");
        return 2;
    };
    let item_key = if key == "classic_enter" { "enter" } else { key };
    let Some(item) = items().into_iter().find(|item| item.get("setting") == item_key) else {
        eprintln!("prelude: {raw_key:?} is not a setting");
        return 2;
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&view(&item)).unwrap_or_else(|_| "{}".into()));
    } else {
        println!("{}", item.fields.first().cloned().unwrap_or_default());
    }
    0
}

fn setting_path(raw_key: Option<&str>) -> Result<PathBuf, String> {
    let Some(raw_key) = raw_key else { return Ok(file()) };
    let key = canonical_key(raw_key).ok_or_else(|| format!("unknown setting {raw_key:?}"))?;
    let item_key = if key == "classic_enter" { "enter" } else { key };
    items()
        .into_iter()
        .find(|item| item.get("setting") == item_key)
        .map(|item| PathBuf::from(item.get("path")))
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| format!("{raw_key} has no file of its own"))
}

/// `prelude settings …` — the same edits the panel makes, without a terminal.
pub fn dispatch(args: &[&str]) -> i32 {
    let result: Result<String, String> = match args {
        [] | ["list"] => return list(false),
        ["--json"] | ["list", "--json"] => return list(true),
        ["get", key] => return get(key, false),
        ["get", key, "--json"] | ["get", "--json", key] => return get(key, true),
        ["check"] => return check(false),
        ["check", "--json"] | ["--json", "check"] => return check(true),
        ["path"] => setting_path(None).map(|path| path.to_string_lossy().into_owned()),
        ["path", key] => setting_path(Some(key)).map(|path| path.to_string_lossy().into_owned()),
        ["roots"] => {
            for (entry, state) in root_rows() {
                println!("{entry}  ({state})");
            }
            return 0;
        }
        ["add-root", path] => add_root(path).map(|added| {
            crate::compute::ensure_fileindex();
            format!("{added} added — file search is updating")
        }),
        ["remove-root", path] => remove_root(path).map(|()| {
            crate::compute::ensure_fileindex();
            format!("{path} removed — file search is updating")
        }),
        ["set", key, value] => set_named(key, value),
        ["reset", key] => reset_named(key),
        _ => Err(
            "usage: prelude settings [list] [--json] | get KEY [--json] | check [--json] | \
             path [KEY] | roots | add-root PATH | remove-root PATH | set KEY VALUE | reset KEY|all"
                .into(),
        ),
    };
    match result {
        Ok(message) => {
            println!("{message}");
            0
        }
        Err(error) => {
            eprintln!("prelude: {error}");
            2
        }
    }
}

/// Stable settings records rather than launcher implementation fields. This
/// is the data surface an agent can safely inspect.
pub fn list(json: bool) -> i32 {
    let items = items();
    if json {
        let views: Vec<SettingView> = items.iter().map(view).collect();
        println!("{}", serde_json::to_string_pretty(&views).unwrap_or_else(|_| "[]".into()));
        return 0;
    }
    for it in &items {
        println!(
            "{:<10} {:<26} {:<38} {}",
            it.get("group"),
            it.title,
            it.fields.first().cloned().unwrap_or_default(),
            it.fields.get(1).cloned().unwrap_or_default()
        );
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard that stops the settings panel recreating the bug it was
    /// written after. `~` as a search root is seven levels of `fd` over the
    /// home directory, which walks into `~/Library` — other applications'
    /// data, as far as macOS is concerned — and the dialog that results names
    /// the terminal rather than Prelude.
    #[test]
    fn a_container_cannot_be_added_as_a_search_root() {
        for holder in ["~", "/", "/Users", "/Applications", "/System"] {
            let e = add_root(holder).unwrap_err();
            assert!(
                e.contains("holds projects") || e.contains("not there"),
                "{holder} must be refused: {e}"
            );
        }
        assert!(add_root("/definitely/not/here").unwrap_err().contains("not there"));
    }

    /// A path with a space in it reaches the clipboard already escaped, and
    /// pasting that into the prompt is the ordinary thing to do. It used to
    /// answer `is not there` about a folder plainly on screen — the worst
    /// available wording, because it names the one thing that is not wrong.
    ///
    /// The literal reading is still first and still always tried, so a folder
    /// whose name genuinely contains a backslash is not taken away by this.
    #[test]
    fn a_shell_escaped_paste_is_read_as_the_path_it_names() {
        let want = std::path::PathBuf::from("/Users/x/Mobile Documents/com~apple~CloudDocs");
        for pasted in [
            r"/Users/x/Mobile\ Documents/com\~apple\~CloudDocs",
            r"'/Users/x/Mobile Documents/com~apple~CloudDocs'",
            r#""/Users/x/Mobile Documents/com~apple~CloudDocs""#,
            "  /Users/x/Mobile Documents/com~apple~CloudDocs  ",
        ] {
            assert!(readings_of(pasted).contains(&want), "{pasted}");
        }
        // The literal is offered first, so a real backslash still wins where
        // such a directory exists.
        let odd = r"/tmp/a";
        assert_eq!(readings_of(odd).first().unwrap(), std::path::Path::new(odd));
        // `~` is still expanded, and still refused by the guard above.
        assert_eq!(readings_of("~").first().unwrap(), &crate::paths::home());
        assert_eq!(
            readings_of("~/work").first().unwrap(),
            &crate::paths::home().join("work")
        );
    }

    /// A stale index is the failure this surface exists to make visible: you
    /// add a folder, `f:` keeps answering from the old set, and nothing says
    /// the index is why.
    #[test]
    fn editing_the_roots_marks_the_index_stale() {
        // Absent index, whatever the roots say.
        let built = mtime_nanos(&crate::compute::fileindex_path());
        let edited = mtime_nanos(&roots_file());
        let stale = match (built, edited) {
            (None, _) => true,
            (Some(b), Some(e)) => e > b,
            (Some(_), None) => false,
        };
        assert_eq!(stale, index_stale());
    }

    /// Every setting states its value on the row. A setting whose value you
    /// cannot see is one you change by trial.
    #[test]
    fn every_setting_shows_what_it_is_currently_set_to() {
        for it in items() {
            assert_eq!(it.kind, Kind::Setting);
            assert!(!it.get("setting").is_empty(), "{} has no id", it.title);
            assert!(!it.get("source").is_empty(), "{} has no source", it.title);
            assert!(
                matches!(it.get("group"), "Search" | "Launcher" | "Behavior" | "Library"),
                "{} has no user-facing category",
                it.title,
            );
            assert_eq!(it.subtitle, it.get("group"));
            assert!(
                !it.fields.first().map(String::is_empty).unwrap_or(true),
                "{} shows no value",
                it.title
            );
            assert!(
                matches!(
                    it.get("edit"),
                    EDIT_PROMPT
                        | EDIT_TOGGLE
                        | EDIT_COLLECTION
                        | EDIT_MANAGE_ROOTS
                        | EDIT_REBUILD
                ),
                "{} has no editor",
                it.title
            );
        }
    }

    /// An environment variable is a per-invocation instruction and the file is
    /// a standing one, so the variable wins. Reporting otherwise would be a
    /// settings panel that lies about what the launcher will do.
    ///
    /// Checked as a rule rather than by exporting a variable, which would
    /// change the process every other test in this binary is running in.
    #[test]
    fn an_environment_variable_outranks_the_file() {
        let e = || Some("40%".to_string());
        let f = || Some("70%".to_string());
        assert_eq!(resolve(e(), f(), "90%"), "40%");
        assert_eq!(resolve(None, f(), "90%"), "70%");
        assert_eq!(resolve(None, None, "90%"), "90%");
        // An empty variable is not an instruction; `env` filters it out
        // before it reaches here, so the file still speaks.
        assert_eq!(resolve(None, f(), "90%"), "70%");
    }

    #[test]
    fn bad_values_are_refused_before_they_reach_zsh() {
        assert_eq!(validate_pref("preview", "off").as_deref(), Ok("false"));
        assert_eq!(validate_pref("classic_enter", "YES").as_deref(), Ok("true"));
        assert!(validate_pref("preview", "perhaps").is_err());
        assert!(validate_pref("key", "^T\nnext").is_err());
    }

    #[test]
    fn editing_a_preference_preserves_the_rest_of_the_file() {
        let before = "# why this is notify\nupdate = \"notify\" # stay quiet\nunknown = \"future\"\n\n[future]\nvalue = \"kept\"\n";
        let changed = update_pref_text(before, "update", Some("download"));
        assert!(changed.contains("# why this is notify"));
        assert!(changed.contains("update = \"download\"  # stay quiet"));
        assert!(changed.contains("unknown = \"future\""));
        assert!(changed.contains("[future]\nvalue = \"kept\""));
        let reset = update_pref_text(&changed, "update", None);
        assert!(!reset.lines().any(|line| line.trim_start().starts_with("update")));
        assert!(reset.contains("unknown = \"future\""));
    }

    #[test]
    fn the_settings_form_keeps_a_stable_task_order() {
        let mut rows = items();
        rows.sort_by(crate::cache::by_rank);
        let keys: Vec<&str> = rows.iter().map(|item| item.get("setting")).collect();
        // `fallbacks` joins the Search group, after the index and before the
        // Launcher rows: it is what an unanswerable *search* answers with.
        assert_eq!(&keys[..4], &["roots", "index", "fallbacks", "hotkey"]);
        assert_eq!(rows[0].get("edit"), EDIT_MANAGE_ROOTS);
        assert_eq!(rows[0].title, "Search folders");
        assert_eq!(rows[0].fields[1], "choose where file and folder search looks");
        assert_eq!(crate::defaults::describe(&rows[0], crate::defaults::Surface::Clipboard), "Manage folders");
        assert_eq!(
            crate::defaults::on_enter(&rows[0]),
            crate::defaults::Default_::Act(crate::defaults::Verb::EditSetting),
            "settings stay operable even when copy-everything is enabled",
        );
    }

    #[test]
    fn every_setting_has_two_specific_arrow_actions() {
        let expected = [
            ("roots", "Remove folder", "Add folder", true, true),
            ("index", "Show status", "Rebuild now", true, true),
            ("hotkey", "Reset default", "Change…", false, true),
            ("paneldir", "Reset default", "Change…", false, true),
            ("key", "Reset default", "Change…", false, true),
            ("preview", "Set off", "Set on", false, false),
            ("enter", "Per kind", "Copy everything", false, false),
            ("update", "Previous mode", "Next mode", false, false),
            ("fallbacks", "Reset default", "Change…", false, true),
            ("openwith", "Remove one…", "Add one…", true, true),
            ("snippets", "Remove one…", "Add one…", true, true),
            ("quicklinks", "Remove one…", "Add one…", true, true),
            ("favorites", "Remove one…", "Add one…", true, true),
            ("aliases", "Remove one…", "Add one…", true, true),
        ];
        let rows = items();
        assert_eq!(rows.len(), expected.len());
        for (id, left, right, left_interactive, right_interactive) in expected {
            let row = rows.iter().find(|item| item.get("setting") == id).unwrap();
            assert_eq!(direction_label(row, "left"), left, "{id} left label");
            assert_eq!(direction_label(row, "right"), right, "{id} right label");
            assert_eq!(direction_is_interactive(row, "left"), left_interactive, "{id} left mode");
            assert_eq!(direction_is_interactive(row, "right"), right_interactive, "{id} right mode");
        }
    }

    #[test]
    fn update_arrows_stop_at_the_ends_instead_of_wrapping() {
        assert_eq!(stepped_update("off", -1), "off");
        assert_eq!(stepped_update("off", 1), "notify");
        assert_eq!(stepped_update("notify", -1), "off");
        assert_eq!(stepped_update("download", 1), "apply");
        assert_eq!(stepped_update("apply", 1), "apply");
    }

    #[test]
    fn friendly_cli_names_resolve_to_one_preference_vocabulary() {
        assert_eq!(canonical_key("quick-look"), Some("preview"));
        assert_eq!(canonical_key("enter"), Some("classic_enter"));
        assert_eq!(canonical_key("panel-directory"), Some("paneldir"));
        assert_eq!(canonical_key("favourites"), Some("favorites"));
        assert_eq!(canonical_key("not-a-setting"), None);
    }
}
