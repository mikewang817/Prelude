//! Which application opens a file, and remembering the answer.
//!
//! `$EDITOR` is the wrong default for a launcher. It is right for the file
//! you are about to edit in the terminal you are already in, and wrong for
//! every other reason you might pick a file out of a list — you wanted Zed,
//! or Preview, or Finder. macOS already knows which app owns a file type, so
//! the default here is simply to ask it, and the ^K panel is where you
//! override it: once, or from now on.
//!
//! Choices live in `open.toml` beside `snippets.toml`, keyed by extension:
//!
//! ```toml
//! [apps]
//! json = "Zed"
//! "*"  = "Visual Studio Code"     # everything not named above
//! ```

use std::path::Path;
use std::process::{Command, Stdio};

const SECTION: &str = "apps";
/// The key standing for "anything not named explicitly".
const ANY: &str = "*";

fn file() -> std::path::PathBuf {
    crate::paths::config().join("open.toml")
}

/// Materialise the editable rules file only when the settings row explicitly
/// asks to open it. File actions remain the authority for writing rules.
pub fn ensure_file() -> Result<std::path::PathBuf, String> {
    let path = file();
    if !path.exists() {
        let text = "# Which application opens what, written by Prelude's ^K panel.\n\
                    # Keys are extensions without the dot; \"*\" is everything else.\n\
                    # Add rules from a file's Actions panel.\n\n[apps]\n";
        crate::cache::write_state(&path, text.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn table() -> crate::minitoml::Table {
    std::fs::read_to_string(file())
        .map(|t| crate::minitoml::parse(&t))
        .unwrap_or_default()
}

/// Saved extension/application pairs, for the Settings collection manager.
pub fn rules() -> Vec<(String, String)> {
    table()
        .remove(SECTION)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// Lowercased extension, without the dot. Empty for extensionless files.
pub fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The application the user has chosen for this path, if any.
///
/// A named extension wins over the catch-all, so setting a blanket editor
/// does not stop `.png` going to Preview.
pub fn chosen_for(path: &str) -> Option<String> {
    let t = table();
    let apps = t.get(SECTION)?;
    let ext = ext_of(path);
    apps.get(&ext)
        .or_else(|| apps.get(ANY))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Remember an application for an extension, or for everything.
///
/// Rewrites the file from the parsed table rather than appending, so choosing
/// twice for the same extension replaces rather than accumulates.
pub fn remember(ext: &str, app: &str) -> Result<(), String> {
    // Read, change, write: two windows setting an open-with rule at the same
    // moment kept one of them.
    let _lock = crate::cache::lock_for_write(&file());
    let mut t = table();
    let key = if ext.is_empty() { ANY.to_string() } else { ext.to_ascii_lowercase() };
    t.entry(SECTION.to_string()).or_default().insert(key, app.to_string());
    write_rules(&t)
}

/// Return one extension to the macOS default without touching other rules.
pub fn forget(ext: &str) -> Result<(), String> {
    let _lock = crate::cache::lock_for_write(&file());
    let mut table = table();
    let key = if ext.trim().is_empty() { ANY } else { ext.trim() };
    let removed = table
        .entry(SECTION.to_string())
        .or_default()
        .remove(key)
        .is_some();
    if !removed {
        return Err(format!("no open-with rule exists for {key}"));
    }
    write_rules(&table)
}

fn write_rules(table: &crate::minitoml::Table) -> Result<(), String> {
    let mut out = String::from(
        "# Which application opens what, written by Prelude's Settings and file Actions.\n\
         # Keys are extensions without the dot; \"*\" is everything else.\n\
         # Delete a line to go back to the system default.\n\n[apps]\n",
    );
    for (key, app) in table.get(SECTION).into_iter().flatten() {
        out.push_str(&format!("{} = {:?}\n", quoted_key(key), app));
    }
    crate::cache::write_state(&file(), out.as_bytes()).map_err(|e| e.to_string())
}

fn quoted_key(k: &str) -> String {
    if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') && !k.is_empty() {
        k.to_string()
    } else {
        format!("{k:?}")
    }
}

/// Arguments passed to `/usr/bin/open`, kept as separate strings rather than
/// shell syntax. Exposed so tests can pin spaces and application names without
/// actually launching anything.
pub fn open_args(target: &str, app: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(app) = app.map(str::trim).filter(|a| !a.is_empty()) {
        args.push("-a".to_string());
        args.push(app.to_string());
    }
    args.push(target.to_string());
    args
}

fn is_ghostty_app(target: &str) -> bool {
    Path::new(target)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("Ghostty.app"))
}

/// The command text carried by an Application row.
///
/// Prelude's global panel is itself a hidden Ghostty process. A plain
/// `open -a Ghostty` can therefore ask that process for a normal window. The
/// surface gate repairs such a misroute, but `-n` avoids it entirely and gives
/// the selected app its own ordinary instance. Other applications retain
/// normal macOS reuse semantics.
pub fn app_command(name: &str) -> String {
    let flag = if name.eq_ignore_ascii_case("Ghostty") { "-na" } else { "-a" };
    format!("open {flag} {}", crate::exec::shq(name))
}

/// Arguments for launching an Application row. The exact path is retained so
/// multiple app installations are not confused; `open -n PATH` is the
/// path-specific equivalent of `open -na Ghostty`.
pub fn launch_args(target: &str) -> Vec<String> {
    let mut args = Vec::new();
    if is_ghostty_app(target) {
        args.push("-n".to_string());
    }
    args.push(target.to_string());
    args
}

/// Hand a URL, file, folder or application directly to macOS Launch Services.
///
/// Arguments go straight to `/usr/bin/open`, never through `sh -c`, so an
/// object's default action does not touch the terminal buffer or shell
/// history. `app` is used only for an explicit/remembered file association;
/// folders and URLs pass `None` and keep the system owner.
pub fn open_now(target: &str, app: Option<&str>) -> Result<(), String> {
    if target.is_empty() {
        return Err("nothing to open".into());
    }
    let status = Command::new("/usr/bin/open")
        .args(open_args(target, app))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("could not ask macOS to open it: {e}"))?;
    if status.success() { Ok(()) } else { Err("macOS could not open it".into()) }
}

pub fn open_default_now(path: &str) -> Result<(), String> {
    open_now(path, chosen_for(path).as_deref())
}

/// Arguments for a folder that explicitly belongs to Finder rather than to
/// whichever application Launch Services happens to associate with it.
pub fn finder_open_args(target: &str) -> Vec<String> {
    vec!["-b".into(), "com.apple.finder".into(), target.into()]
}

/// Finder can become active and immediately lose focus again when Prelude's
/// quick-terminal closes and macOS restores the previously active app. Run the
/// activation just after that handoff instead of racing it synchronously.
fn activate_finder_after_panel() -> Result<(), String> {
    // `open -a Finder` uses Launch Services and needs no Automation consent.
    // The shell carries only this fixed command; no path or user text reaches
    // it, and it outlives the launcher long enough to win the focus handoff.
    Command::new("/bin/sh")
        .args(["-c", "sleep 0.15; exec /usr/bin/open -a Finder"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not bring Finder to the front: {e}"))
}

/// Open a folder in Finder and make Finder the foreground application.
pub fn open_finder_now(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("nothing to open in Finder".into());
    }
    let status = Command::new("/usr/bin/open")
        .args(finder_open_args(path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("could not ask Finder to open it: {e}"))?;
    if !status.success() {
        return Err("Finder could not open it".into());
    }
    activate_finder_after_panel()
}

/// Launch an application. Ghostty is deliberately a new instance so Prelude's
/// hidden quick-terminal process cannot receive the ordinary window request.
pub fn launch_now(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("nothing to launch".into());
    }
    let status = Command::new("/usr/bin/open")
        .args(launch_args(path))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("could not ask macOS to launch it: {e}"))?;
    if status.success() { Ok(()) } else { Err("macOS could not launch it".into()) }
}

pub fn reveal_now(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("nothing to reveal".into());
    }
    let status = Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("could not ask Finder to reveal it: {e}"))?;
    if !status.success() {
        return Err("Finder could not reveal it".into());
    }
    activate_finder_after_panel()
}

/// Show the installed applications and return the one picked.
pub fn pick_app(subject: &str) -> Option<String> {
    let apps = crate::sources::machine::apps();
    if apps.is_empty() {
        return None;
    }
    use crate::ansi::{DIM, RESET};
    use crate::render::SEP;
    let feed: String = apps
        .iter()
        .map(|a| {
            let where_ = crate::paths::tilde(a.get("path"));
            format!("{:<34}{DIM}{where_}{RESET}{SEP}{}\n", crate::width::dtrunc(&a.title, 32), a.title)
        })
        .collect();
    crate::ui::pick_raw(
        feed.trim_end(),
        &format!(" open {subject} with "),
        "⌕ ",
        "Open  Enter   ·   Back  Esc",
        "",
    )
}
