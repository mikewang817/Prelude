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

const DEFAULT_KEY: &str = "^R";
const DEFAULT_PREVIEW: bool = true;
const DEFAULT_CLASSIC_ENTER: bool = false;
const DEFAULT_UPDATE: &str = "notify";
const PREF_KEYS: &[&str] = &["key", "preview", "classic_enter", "update"];

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
    present: std::collections::BTreeSet<String>,
}

/// Read once. `on_enter` runs in the per-keystroke footer helper, so a file
/// read per call would be a file read per keystroke.
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
            present: root
                .into_iter()
                .flat_map(|table| table.keys().cloned())
                .collect(),
        }
    })
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

fn source_detail(base: &str, source: &str, environment: &str) -> String {
    match source {
        "environment" => format!("${environment} override · {base}"),
        "invalid environment" => {
            format!("invalid ${environment} ignored · using saved/default · {base}")
        }
        "saved" => format!("saved · {base}"),
        "invalid saved value" => format!("invalid saved value ignored · default · {base}"),
        "default" => format!("default · {base}"),
        _ => base.to_string(),
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

/// Record how many files the last index run found.
///
/// Written beside the index so the settings row can state the number without
/// reading a megabyte of paths on every gather.
pub fn record_index_count(n: usize) {
    let _ = crate::cache::write_atomic(&index_count_file(), n.to_string().as_bytes());
}

/// How many files the index holds.
///
/// Recorded when the index is built. An index built by an older version has
/// no record, so the first reader counts it and writes the number down —
/// once, rather than reading a megabyte of paths on every gather forever, and
/// rather than the row claiming "never indexed" over a working index.
pub fn index_count() -> Option<usize> {
    if let Some(n) = std::fs::read_to_string(index_count_file())
        .ok()
        .and_then(|t| t.trim().parse().ok())
    {
        return Some(n);
    }
    let text = std::fs::read_to_string(crate::compute::fileindex_path()).ok()?;
    let n = text.lines().filter(|l| !l.trim().is_empty()).count();
    record_index_count(n);
    Some(n)
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

/// Has the roots list been edited since the index was built?
///
/// The failure this answers is silent and was the whole problem: you add a
/// folder, `f:` keeps returning the old set, and nothing anywhere says the
/// index is the reason.
pub fn index_stale() -> bool {
    let built = mtime(&crate::compute::fileindex_path());
    match (built, mtime(&roots_file())) {
        (None, _) => true,
        (Some(built), Some(edited)) => edited > built,
        (Some(_), None) => false,
    }
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
        "# Folders `prelude index` walks, one per line; ~/ is expanded.\n\
         # Written by the set: panel. Run `prelude index` after editing.\n\n",
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
pub const EDIT_OPEN: &str = "open";
pub const EDIT_ADD_ROOT: &str = "add-root";
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

fn row(id: &str, title: &str, value: String, detail: &str, edit: &str) -> Item {
    Item::new(format!("set:{id}"), Kind::Setting)
        .title(title)
        .fields([value, detail.to_string()])
        .put("setting", id)
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
    row(
        id,
        title,
        value,
        &source_detail(base_detail, source, environment),
        edit,
    )
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
    let count = index_count();
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
            "Search roots",
            value,
            &format!("{roots_source} · what f: can find"),
            EDIT_ADD_ROOT,
        )
        .put("source", roots_source)
        .put("path", roots_file().to_string_lossy().into_owned()),
    );

    // Indexing can take a minute, so it is its own visible operation rather
    // than a side effect hidden behind adding a root.
    let stale = index_stale();
    let index_value = match (count, &age) {
        (Some(n), Some(a)) => format!("{n} files · built {a}"),
        (None, Some(a)) => format!("built {a}"),
        _ => "never built".into(),
    };
    let index_detail = if stale {
        "stale · rebuild before relying on f:"
    } else {
        "current · paths and Finder tags for every f: search"
    };
    v.push(
        row("index", "File index", index_value, index_detail, EDIT_REBUILD)
            .put("path", indexed.to_string_lossy().into_owned())
            .put("source", if stale { "stale" } else { "current" }),
    );

    let global = crate::global::configured_summary();
    // Installed is a file check. Whether the helper is running needs `pgrep`
    // and belongs in explicit `global status`, never on the gather path.
    let panel_state = if global.installed { "panel installed" } else { "panel not installed" };
    v.push(
        row(
            "hotkey",
            "Global hotkey",
            global.hotkey,
            &format!("{} · {panel_state}", global.hotkey_source),
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
            &format!("{} · where both launchers stand", global.directory_source),
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
        "the zsh widget · needs a new shell",
        EDIT_PROMPT,
        ("key", DEFAULT_KEY, "PRELUDE_KEY"),
    ));
    v.push(preference_row(
        "preview",
        "Quick Look",
        if preview_enabled() { "on".into() } else { "off".into() },
        "Ctrl+P, hidden until asked for",
        EDIT_TOGGLE,
        ("preview", "on", "PRELUDE_NO_PREVIEW"),
    ));
    v.push(preference_row(
        "enter",
        "What Enter does",
        if classic_enter() { "copy everything".into() } else { "per kind".into() },
        "commands are handed over, objects act",
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
        &version_note,
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
            "which application opens what",
            EDIT_OPEN,
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
            "snip: · {{placeholder}} blanks",
            EDIT_OPEN,
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
            "ql: to browse · ^K on any row to add",
            EDIT_OPEN,
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
            "agents, skills and MCP servers",
            EDIT_OPEN,
        )
        .put("source", if favorites.is_file() { "saved" } else { "none" })
        .put("path", favorites.to_string_lossy().into_owned()),
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
            match index_count() {
                Some(n) => out.push(format!("{n} files in the shared index")),
                None => out.push("the index has never been built".into()),
            }
            if index_stale() {
                out.push("The roots have changed since it was built.".into());
            }
            out.push("Rebuilding walks every search root and records Finder tags; it may take a minute.".into());
        }
        "key" => {
            out.push("Bound by `prelude init zsh`, which runs when a shell starts,".into());
            out.push("so a change reaches the next shell rather than this one.".into());
            setting_origin(it, &mut out);
        }
        "preview" | "enter" => {
            setting_origin(it, &mut out);
        }
        _ => {
            let path = it.get("path");
            if !path.is_empty() {
                if let Ok(text) = std::fs::read_to_string(path) {
                    out.extend(text.lines().take(40).map(str::to_string));
                }
            }
        }
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
        "key" | "preview" | "enter" => {
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
        EDIT_ADD_ROOT => add_root_interactively(),
        EDIT_REBUILD => crate::runhere::run_cmd("prelude index"),
        EDIT_TOGGLE => toggle(it),
        EDIT_PROMPT => prompt_for(it),
        _ => open_file(it)
    }
}

pub fn add_root_interactively() -> i32 {
    let Some(raw) = crate::ui::prompt_line_initial(" add a search root ", "~/") else {
        return 130;
    };
    match add_root(&raw) {
        Ok(added) => {
            crate::ui::note(&format!("{added} added — run Rebuild the index to include it"));
            0
        }
        Err(e) => {
            crate::ui::note(&e);
            2
        }
    }
}

pub fn remove_root_interactively() -> i32 {
    let rows = root_rows();
    if rows.is_empty() {
        crate::ui::note("there are no search roots to remove");
        return 2;
    }
    let choices: Vec<(String, String, String)> = rows
        .iter()
        .map(|(entry, state)| (entry.clone(), entry.clone(), state.clone()))
        .collect();
    let Some(entry) = crate::actions::pick_one(" remove a search root ", &choices) else {
        return 130;
    };
    match remove_root(&entry) {
        Ok(()) => {
            crate::ui::note(&format!("{entry} removed — run Rebuild the index to forget it"));
            0
        }
        Err(e) => {
            crate::ui::note(&e);
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
    let (key, now) = match it.get("setting") {
        "preview" => ("preview", !preview_enabled()),
        "enter" => ("classic_enter", !classic_enter()),
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
        key @ ("key" | "preview" | "classic_enter" | "update") => {
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
        key @ ("key" | "preview" | "classic_enter" | "update") => {
            remove_pref(key)?;
            let default = match key {
                "key" => DEFAULT_KEY,
                "preview" => "on",
                "update" => DEFAULT_UPDATE,
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
    if index_stale() {
        out.push(Check {
            severity: "warning",
            setting: "index".into(),
            message: "the file index has not been built for the current roots".into(),
        });
    }
    out
}

fn check(json: bool) -> i32 {
    let checks = checks();
    if json {
        println!("{}", serde_json::to_string_pretty(&checks).unwrap_or_else(|_| "[]".into()));
    } else if checks.is_empty() {
        println!("Prelude settings are valid and the file index is current.");
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
        ["add-root", path] => add_root(path)
            .map(|added| format!("{added} added — run `prelude index` to include it")),
        ["remove-root", path] => remove_root(path)
            .map(|()| format!("{path} removed — run `prelude index` to forget it")),
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
            "{:<26} {:<38} {}",
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
        let built = mtime(&crate::compute::fileindex_path());
        let edited = mtime(&roots_file());
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
                !it.fields.first().map(String::is_empty).unwrap_or(true),
                "{} shows no value",
                it.title
            );
            assert!(
                matches!(
                    it.get("edit"),
                    EDIT_PROMPT | EDIT_TOGGLE | EDIT_OPEN | EDIT_ADD_ROOT | EDIT_REBUILD
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
        assert_eq!(&keys[..4], &["roots", "index", "hotkey", "paneldir"]);
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
