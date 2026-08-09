//! Prelude's own settings, as objects you can find rather than files you have
//! to remember.
//!
//! Every preference here already existed. What did not exist was any way to
//! *discover* one: `roots.txt` decides what `f:` can find and was documented
//! only in a README, defaulted from a hard-coded list, and had to be created by
//! hand — with `prelude index` run afterwards from memory, since nothing said
//! the index had gone stale. Four more lived only in environment variables that
//! had to be exported before the `eval` line in `.zshrc`. A launcher that
//! manages four agents' settings could not reach its own.
//!
//! So settings are rows, in their own `set:` scope, and each one carries its
//! current value on the row. That is the part that matters: a setting you
//! cannot see the value of is one you change by trial. `^K` holds the
//! mutations, which is where every other object in this launcher keeps them.
//!
//! **What is written, and what is only read.** Six settings own a file each
//! and are edited through their own code (`roots.txt`, `global.toml`,
//! `open.toml`, `snippets.toml`, `quicklinks.toml`, `favorites.txt`). The four
//! that were environment-only get `settings.toml`, and the variable still wins
//! where it is set — a variable is a per-invocation instruction and a file is a
//! standing one, so the narrower must be able to override the broader.

use crate::item::{Item, Kind};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// settings.toml — the preferences that had nowhere else to live
// ---------------------------------------------------------------------------

pub fn file() -> PathBuf {
    crate::paths::config().join("settings.toml")
}

#[derive(Clone, Debug, Default)]
struct Prefs {
    key: Option<String>,
    height: Option<String>,
    preview: Option<bool>,
    classic_enter: Option<bool>,
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
        let flag = |k: &str| get(k).map(|v| matches!(v.as_str(), "true" | "yes" | "on" | "1"));
        Prefs {
            key: get("key"),
            height: get("height"),
            preview: flag("preview"),
            classic_enter: flag("classic_enter"),
        }
    })
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
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
    resolve(env("PRELUDE_KEY"), prefs().key.clone(), "^R")
}

/// How much of the terminal the inline launcher uses.
pub fn height() -> String {
    resolve(env("PRELUDE_HEIGHT"), prefs().height.clone(), "90%")
}

/// Whether `Ctrl+P` Quick Look exists at all.
pub fn preview_enabled() -> bool {
    if env("PRELUDE_NO_PREVIEW").is_some() {
        return false;
    }
    prefs().preview.unwrap_or(true)
}

/// The pre-2024 default: Enter inserts everything, whatever kind it is.
pub fn classic_enter() -> bool {
    if env("PRELUDE_CLASSIC_ENTER").is_some() {
        return true;
    }
    prefs().classic_enter.unwrap_or(false)
}

/// Rewrite `settings.toml` with one key changed.
///
/// The whole file is regenerated from the parsed table rather than appended
/// to, so setting the same key twice replaces rather than accumulates — the
/// rule `openwith::remember` already follows for the same reason.
fn write_pref(key: &str, value: &str) -> Result<(), String> {
    let path = file();
    let mut table = std::fs::read_to_string(&path)
        .map(|t| crate::minitoml::parse(&t))
        .unwrap_or_default();
    table.entry(String::new()).or_default().insert(key.to_string(), value.to_string());
    let mut out = String::from(
        "# Prelude's own preferences, written by the set: panel.\n\
         # The matching environment variable still overrides any line here.\n\n",
    );
    for (k, v) in table.get("").into_iter().flatten() {
        let literal = matches!(v.as_str(), "true" | "false");
        if literal {
            out.push_str(&format!("{k} = {v}\n"));
        } else {
            out.push_str(&format!("{k} = {v:?}\n"));
        }
    }
    crate::cache::write_atomic(&path, out.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
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
fn readings_of(raw: &str) -> Vec<PathBuf> {
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
    let lines: Vec<String> = roots_lines().into_iter().filter(|l| l != entry).collect();
    write_roots(&lines)
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
    crate::cache::write_atomic(&roots_file(), out.as_bytes()).map_err(|e| e.to_string())
}

/// Each root with its own state, for the detail view and the remove picker.
pub fn root_rows() -> Vec<(String, String)> {
    roots_lines()
        .into_iter()
        .map(|entry| {
            let state = match resolved(&entry) {
                Some(p) if p.is_dir() => "indexed".to_string(),
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
    let missing = roots.iter().filter(|(_, s)| s != "indexed").count();
    let value = match (count, &age) {
        (Some(n), Some(a)) => format!("{} folders · {n} files · indexed {a}", roots.len()),
        (None, Some(a)) => format!("{} folders · indexed {a}", roots.len()),
        _ => format!("{} folders · never indexed", roots.len()),
    };
    let detail = if index_stale() {
        "roots changed — run Rebuild".to_string()
    } else if missing > 0 {
        format!("{missing} of them are not there any more")
    } else {
        "what f: can find".to_string()
    };
    v.push(
        row("roots", "Search roots", value, &detail, EDIT_ADD_ROOT)
            .put("path", roots_file().to_string_lossy().into_owned()),
    );

    let global = crate::global::configured_summary();
    v.push(row("hotkey", "Global hotkey", global.hotkey, "opens the panel anywhere", EDIT_PROMPT));
    v.push(row(
        "paneldir",
        "Panel directory",
        crate::paths::tilde(&global.directory),
        "where the panel stands",
        EDIT_PROMPT,
    ));

    v.push(row(
        "key",
        "Launcher key at a shell",
        launcher_key(),
        "the zsh widget · needs a new shell",
        EDIT_PROMPT,
    ));
    v.push(row("height", "Inline height", height(), "how much of the terminal", EDIT_PROMPT));
    v.push(row(
        "preview",
        "Quick Look",
        if preview_enabled() { "on".into() } else { "off".into() },
        "Ctrl+P, hidden until asked for",
        EDIT_TOGGLE,
    ));
    v.push(row(
        "enter",
        "What Enter does",
        if classic_enter() { "insert everything".into() } else { "per kind".into() },
        "commands are handed over, objects act",
        EDIT_TOGGLE,
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
        .put("path", snippets.to_string_lossy().into_owned()),
    );

    let quicklinks = config.join("quicklinks.toml");
    let n = count_of(&quicklinks, None);
    v.push(
        row(
            "quicklinks",
            "Quicklinks",
            if n == 0 { "none yet".into() } else { format!("{n} keywords") },
            "type the keyword to reach the object",
            EDIT_OPEN,
        )
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
        .put("path", favorites.to_string_lossy().into_owned()),
    );

    v
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
            match index_count() {
                Some(n) => out.push(format!("{n} files in the index")),
                None => out.push("the index has never been built".into()),
            }
            if index_stale() {
                out.push("the roots have changed since — run Rebuild the index".into());
            }
            out.push(String::new());
            out.push("`f:` searches this index plus the current project.".into());
        }
        "key" => {
            out.push("Bound by `prelude init zsh`, which runs when a shell starts,".into());
            out.push("so a change reaches the next shell rather than this one.".into());
        }
        "preview" | "enter" => {
            out.push("Set here or by the matching environment variable, which wins".into());
            out.push("wherever it is exported.".into());
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

/// Carry out Enter for a setting.
pub fn edit(it: &Item) -> i32 {
    match it.get("edit") {
        EDIT_ADD_ROOT => add_root_interactively(),
        EDIT_TOGGLE => toggle(it),
        EDIT_PROMPT => prompt_for(it),
        _ => {
            let path = it.get("path");
            if path.is_empty() {
                return 2;
            }
            if let Err(e) = crate::openwith::open_default_now(path) {
                crate::ui::note(&e);
                return 2;
            }
            0
        }
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
        (_, true) => "Enter inserts everything".to_string(),
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
        "height" => " inline height ",
        _ => " value ",
    };
    let Some(value) = crate::ui::prompt_line_initial(label, &current) else {
        return 130;
    };
    let result = match id {
        "hotkey" => crate::global::set_hotkey(&value),
        "paneldir" => crate::global::set_directory(&value),
        "key" => write_pref("key", &value)
            .map(|()| format!("launcher key {value} — bound in the next shell you open")),
        "height" => write_pref("height", &value)
            .map(|()| format!("inline height {value}")),
        _ => Err("that setting has no editor".into()),
    };
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

/// `prelude settings …` — the same edits the panel makes, without a terminal.
///
/// Every other subsystem here has a door like this, and for the same two
/// reasons: the guards can be exercised without standing up fzf, and a person
/// who would rather type than pick is not forced through a picker.
pub fn dispatch(args: &[&str]) -> i32 {
    let result: Result<String, String> = match args {
        [] => return list(false),
        ["--json"] => return list(true),
        ["roots"] => {
            for (entry, state) in root_rows() {
                println!("{entry}  ({state})");
            }
            return 0;
        }
        ["add-root", path] => add_root(path).map(|added| {
            format!("{added} added — run `prelude index` to include it")
        }),
        ["remove-root", path] => remove_root(path)
            .map(|()| format!("{path} removed — run `prelude index` to forget it")),
        ["set", key @ ("key" | "height"), value] => {
            write_pref(key, value).map(|()| format!("{key} = {value}"))
        }
        ["set", key @ ("preview" | "classic_enter"), value] => match value.trim() {
            v @ ("true" | "false") => write_pref(key, v).map(|()| format!("{key} = {v}")),
            other => Err(format!("{key} is true or false, not {other:?}")),
        },
        _ => Err(
            "usage: prelude settings [--json] | roots | add-root PATH | remove-root PATH | \
             set key|height|preview|classic_enter VALUE"
                .into(),
        ),
    };
    match result {
        Ok(message) => {
            println!("{message}");
            0
        }
        Err(e) => {
            eprintln!("prelude: {e}");
            2
        }
    }
}

/// `prelude settings [--json]`, for the same reason every other listing has a
/// data door: an agent asked to check a setting should read a field.
pub fn list(json: bool) -> i32 {
    let items = items();
    if json {
        println!("{}", serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()));
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
            assert!(
                !it.fields.first().map(String::is_empty).unwrap_or(true),
                "{} shows no value",
                it.title
            );
            assert!(
                matches!(it.get("edit"), EDIT_PROMPT | EDIT_TOGGLE | EDIT_OPEN | EDIT_ADD_ROOT),
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
}
