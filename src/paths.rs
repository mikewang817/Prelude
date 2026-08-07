//! Where things live. No hard-coded personal paths — everything honours the
//! XDG variables so the binary is portable to any machine.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
        .join("prelude")
}

pub fn cache() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache")
}
pub fn data() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}
pub fn config() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// The current directory, or `None` when it has been deleted out from under
/// the shell. Deliberately not falling back to `$HOME`: that would make every
/// project-scoped source treat the whole home directory as "the project" and
/// scan it, which measured 93ms against a 40ms budget.
pub fn cwd() -> Option<PathBuf> {
    std::env::current_dir().ok()
}

/// Paths nothing in this launcher may move, whatever the row said.
///
/// Trashing is recoverable, so this is not the last line of defence — the
/// confirmation is. It is the line against the *catastrophic* rather than
/// the merely wrong: a fuzzy list, a mis-aimed Enter, and a row whose path
/// happened to be `$HOME`. Everything here is either irreplaceable or
/// system-owned, and none of it is something a person meant to select from
/// a launcher.
pub fn is_protected(p: &std::path::Path) -> bool {
    let Ok(real) = p.canonicalize() else { return true };
    if real.parent().is_none() {
        return true; // the root itself
    }
    if real == home() {
        return true;
    }
    let s = real.to_string_lossy();
    // Whole trees, contents included. Compared *after* canonicalize, which
    // on macOS is the only way to catch these at all: `/etc` and `/var` are
    // symlinks into `/private`, so a list of the names people type protects
    // nothing.
    const TREES: &[&str] =
        &["/System", "/usr", "/bin", "/sbin", "/Library", "/private/etc", "/private/var"];
    if TREES.iter().any(|d| s == *d || s.starts_with(&format!("{d}/"))) {
        return true;
    }
    // …and these directories themselves, but not what is inside them. A file
    // in /tmp is an ordinary file; /tmp is not.
    const THEMSELVES: &[&str] =
        &["/Applications", "/Users", "/private/tmp", "/opt", "/opt/homebrew", "/Volumes"];
    THEMSELVES.contains(&s.as_ref())
}

/// Move something to the Trash, and say where it went.
///
/// Never `remove_file`, never `remove_dir_all`. The Trash costs the same as
/// deleting and leaves the thing sitting there to be dragged back out, which
/// turns "that was the wrong row" from a loss into an inconvenience. A name
/// already in there is never overwritten — you get `notes 2`.
pub fn trash(p: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if is_protected(p) {
        return Err(format!("{} is not something to delete from here", p.display()));
    }
    if !p.exists() {
        return Err(format!("{} is not there any more", p.display()));
    }
    let name = p.file_name().ok_or_else(|| "no name".to_string())?;
    let dir = home().join(".Trash");
    std::fs::create_dir_all(&dir).map_err(|e| format!("no Trash: {e}"))?;
    let mut dest = dir.join(name);
    let mut n = 2;
    while dest.exists() {
        dest = dir.join(format!("{} {n}", name.to_string_lossy()));
        n += 1;
        if n > 999 {
            return Err("too many copies of that name in the Trash".into());
        }
    }
    std::fs::rename(p, &dest).map_err(|e| format!("could not move it to the Trash: {e}"))?;
    Ok(dest)
}

pub fn tilde(p: &str) -> String {
    let h = home();
    let h = h.to_string_lossy();
    if !h.is_empty() && p.starts_with(h.as_ref()) {
        format!("~{}", &p[h.len()..])
    } else {
        p.to_string()
    }
}
