//! Sources that come from the user's own files and habits.

use crate::exec::{run, shq, which};
use crate::item::{Item, Kind};
use crate::paths;
use std::time::Duration;

/// Every executable on $PATH — the long tail, ranked lowest.
///
/// Scanning costs ~250ms, so a stale cache is always served immediately and
/// refreshed in a detached process. Only the very first run pays full price.
pub fn path_commands() -> Vec<Item> {
    let cache = paths::cache().join("path.txt");
    let fresh = cache
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 6 * 3600);

    if cache.exists() {
        if !fresh {
            crate::cache::spawn_self(&["_refresh-path"]);
        }
        return std::fs::read_to_string(&cache)
            .unwrap_or_default()
            .split_whitespace()
            .map(|n| Item::new(n, Kind::Path))
            .collect();
    }
    scan_path().into_iter().map(|n| Item::new(n, Kind::Path)).collect()
}

pub fn scan_path() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name.len() < 2 {
                    continue;
                }
                if is_executable(&e.path()) {
                    names.insert(name);
                }
            }
        }
    }
    let out: Vec<String> = names.into_iter().collect();
    let _ = crate::cache::write_atomic(&paths::cache().join("path.txt"), out.join("\n").as_bytes());
    out
}

fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Directory frecency — read zoxide's database rather than reinventing it.
/// Falls back to `cd` targets mined from history when zoxide is absent or
/// its database is empty (installed but never initialised).
pub fn dirs() -> Vec<Item> {
    let mut list: Vec<String> = Vec::new();
    if which("zoxide").is_some() {
        list = run(&["zoxide", "query", "-l"], Duration::from_secs(1))
            .lines()
            .map(str::to_string)
            .filter(|l| !l.trim().is_empty())
            .collect();
    }
    if list.is_empty() {
        list = super::history::dirs_from_history();
    }
    let here = paths::cwd().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    list.into_iter()
        .take(300)
        .filter(|d| *d != here && std::path::Path::new(d).is_dir())
        .map(|d| {
            let short = paths::tilde(&d);
            Item::new(format!("cd {}", shq(&d)), Kind::Dir)
                .sub(short)
                .put("path", d)
        })
        .collect()
}

/// Hosts from ~/.ssh/config.
pub fn ssh() -> Vec<Item> {
    let Ok(text) = std::fs::read_to_string(paths::home().join(".ssh/config")) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("Host ").or_else(|| t.strip_prefix("host ")) else {
            continue;
        };
        for host in rest.split_whitespace() {
            if host.contains('*') || host.contains('?') {
                continue;
            }
            items.push(
                Item::new(format!("ssh {}", shq(host)), Kind::Ssh)
                    .title(host)
                    .sub("~/.ssh/config")
                    .put("host", host),
            );
        }
    }
    items
}

pub const SNIPPETS_DEFAULT: &str = r#"# Prelude snippets
# {{placeholder}} markers become <…> for you to fill in after inserting.

["serve this folder"]
cmd = "python3 -m http.server {{port}}"

["what is on a port"]
cmd = "lsof -nP -iTCP:{{port}} -sTCP:LISTEN"

["biggest files here"]
cmd = "du -ah . | sort -rh | head -20"

["git undo last commit, keep changes"]
cmd = "git reset --soft HEAD~1"

["tar a folder"]
cmd = "tar -czf {{name}}.tar.gz {{folder}}"
"#;

pub fn snippets() -> Vec<Item> {
    let p = paths::config().join("snippets.toml");
    if !p.exists() {
        let _ = std::fs::create_dir_all(paths::config());
        let _ = std::fs::write(&p, SNIPPETS_DEFAULT);
    }
    let Ok(text) = std::fs::read_to_string(&p) else { return Vec::new() };
    crate::minitoml::parse(&text)
        .into_iter()
        .filter_map(|(name, body)| {
            let cmd = body.get("cmd")?.clone();
            if cmd.is_empty() || name.is_empty() {
                return None;
            }
            Some(Item::new(cmd, Kind::Snippet).sub(&name).put("name", name))
        })
        .collect()
}

/// Clipboard history, newest first. Populated by `prelude clipd`.
pub fn clips() -> Vec<Item> {
    let Ok(text) = std::fs::read_to_string(paths::data().join("clipboard.jsonl")) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(t) = v.get("t").and_then(|x| x.as_str()) else { continue };
        if t.is_empty() || !seen.insert(t.to_string()) {
            continue;
        }
        let ts = v.get("ts").and_then(|x| x.as_f64()).unwrap_or(0.0);
        items.push(
            Item::new(t, Kind::Clip)
                .title(crate::width::flatten(t))
                .sub(ago(ts))
                .put("full", t),
        );
        if items.len() >= 200 {
            break;
        }
    }
    items
}

pub fn ago(ts: f64) -> String {
    if ts <= 0.0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let s = (now - ts).max(0.0);
    if s < 60.0 {
        "just now".into()
    } else if s < 3600.0 {
        format!("{}m ago", (s / 60.0) as u64)
    } else if s < 86400.0 {
        format!("{}h ago", (s / 3600.0) as u64)
    } else {
        format!("{}d ago", (s / 86400.0) as u64)
    }
}
