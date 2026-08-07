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

use crate::exec::shq;
use std::path::Path;

const SECTION: &str = "apps";
/// The key standing for "anything not named explicitly".
const ANY: &str = "*";

fn file() -> std::path::PathBuf {
    crate::paths::config().join("open.toml")
}

fn table() -> crate::minitoml::Table {
    std::fs::read_to_string(file())
        .map(|t| crate::minitoml::parse(&t))
        .unwrap_or_default()
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
    let mut t = table();
    let key = if ext.is_empty() { ANY.to_string() } else { ext.to_ascii_lowercase() };
    t.entry(SECTION.to_string()).or_default().insert(key, app.to_string());
    let mut out = String::from(
        "# Which application opens what, written by Prelude's ^K panel.\n\
         # Keys are extensions without the dot; \"*\" is everything else.\n\
         # Delete a line to go back to the system default.\n\n[apps]\n",
    );
    for (k, v) in t.get(SECTION).into_iter().flatten() {
        out.push_str(&format!("{} = {:?}\n", quoted_key(k), v));
    }
    let p = file();
    crate::cache::write_atomic(&p, out.as_bytes()).map_err(|e| e.to_string())
}

fn quoted_key(k: &str) -> String {
    if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') && !k.is_empty() {
        k.to_string()
    } else {
        format!("{k:?}")
    }
}

/// The command that opens `path` — with `app` if given, otherwise with
/// whatever macOS considers its owner.
///
/// `open` rather than the app's binary: it hands the file to a running
/// instance instead of starting a second copy, and it is the same path the
/// Finder takes, so a file type nobody has claimed still lands somewhere
/// sensible instead of failing.
pub fn open_cmd(path: &str, app: Option<&str>) -> String {
    match app {
        Some(a) if !a.trim().is_empty() => format!("open -a {} {}", shq(a.trim()), shq(path)),
        _ => format!("open {}", shq(path)),
    }
}

/// What Enter should run for this path, honouring any remembered choice.
pub fn open_default(path: &str) -> String {
    open_cmd(path, chosen_for(path).as_deref())
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
