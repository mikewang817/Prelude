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

pub fn tilde(p: &str) -> String {
    let h = home();
    let h = h.to_string_lossy();
    if !h.is_empty() && p.starts_with(h.as_ref()) {
        format!("~{}", &p[h.len()..])
    } else {
        p.to_string()
    }
}
