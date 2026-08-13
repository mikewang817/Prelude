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
    let Some(text) = paths::read_bounded(&paths::home().join(".ssh/config"), paths::SMALL_FILE)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
    else {
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

pub fn ensure_snippets_file() -> Result<std::path::PathBuf, String> {
    let path = paths::config().join("snippets.toml");
    if !path.exists() {
        crate::cache::write_state(&path, SNIPPETS_DEFAULT.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

pub fn snippets() -> Vec<Item> {
    snippet_entries()
        .into_iter()
        .map(|(name, cmd)| Item::new(cmd, Kind::Snippet).sub(&name).put("name", name))
        .collect()
}

/// Parsed snippet names and commands, shared by the scope and Settings.
pub fn snippet_entries() -> Vec<(String, String)> {
    let path = ensure_snippets_file().unwrap_or_else(|_| paths::config().join("snippets.toml"));
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    crate::minitoml::parse(&text)
        .into_iter()
        .filter_map(|(name, body)| {
            let cmd = body.get("cmd")?.trim().to_string();
            (!cmd.is_empty() && !name.is_empty()).then_some((name, cmd))
        })
        .collect()
}

fn toml_string(value: &str) -> String {
    format!("{value:?}")
}

fn section_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.len() >= 2
        && ((inner.starts_with('"') && inner.ends_with('"'))
            || (inner.starts_with('\'') && inner.ends_with('\'')))
    {
        Some(inner[1..inner.len() - 1].to_string())
    } else {
        Some(inner.to_string())
    }
}

/// Append one snippet without rewriting hand-written comments or ordering.
pub fn add_snippet(name: &str, command: &str) -> Result<(), String> {
    let name = crate::width::flatten(name.trim());
    let command = command.trim();
    if name.is_empty() || command.is_empty() {
        return Err("a snippet needs both a name and a command".into());
    }
    if crate::secrets::looks_secret(command) {
        return Err("that command appears to contain a credential".into());
    }
    let path = ensure_snippets_file()?;
    let _lock = crate::cache::lock_for_write(&path);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    if crate::minitoml::parse(&text).contains_key(&name) {
        return Err(format!("a snippet called {name:?} already exists"));
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("\n[{}]\ncmd = {}\n", toml_string(&name), toml_string(command)));
    crate::cache::write_state(&path, text.as_bytes()).map_err(|error| error.to_string())
}

/// Remove one complete snippet section, leaving every other byte intact.
pub fn remove_snippet(name: &str) -> Result<(), String> {
    let path = ensure_snippets_file()?;
    let _lock = crate::cache::lock_for_write(&path);
    let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut start = None;
    let mut finish = text.len();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if let Some(section) = section_name(line) {
            if start.is_some() {
                finish = offset;
                break;
            }
            if section == name {
                start = Some(offset);
            }
        }
        offset += line.len();
    }
    let start = start.ok_or_else(|| format!("no snippet called {name:?}"))?;
    let mut out = text;
    out.replace_range(start..finish, "");
    crate::cache::write_state(&path, out.as_bytes()).map_err(|error| error.to_string())
}

/// Clipboard history, newest first. Populated by `prelude clipd`.
///
/// The timestamp is recorded as the source rank at microsecond scale so the
/// largest possible frecency bonus cannot lift an old clipping above a newer
/// one. Clipboard history answers "what did I just copy?", not "what have I
/// selected most often?".
pub fn clips() -> Vec<Item> {
    let Ok(text) = std::fs::read_to_string(paths::data().join("clipboard.jsonl")) else {
        return Vec::new();
    };
    clips_from(&text)
}

fn image_identity(value: &serde_json::Value, path: &str) -> String {
    value
        .get("fingerprint")
        .and_then(|fingerprint| fingerprint.as_str())
        .filter(|fingerprint| !fingerprint.is_empty())
        .map(|fingerprint| format!("image:{fingerprint}"))
        .unwrap_or_else(|| format!("image-path:{path}"))
}

pub(crate) fn clips_from(text: &str) -> Vec<Item> {
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let ts = v.get("ts").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("text");
        let base = match kind {
            "files" => {
                let Some(raw) = v.get("paths").and_then(|x| x.as_array()) else { continue };
                let paths: Vec<String> = raw
                    .iter()
                    .filter_map(|x| x.as_str())
                    .filter(|p| std::path::Path::new(p).exists() && !crate::secrets::looks_secret(p))
                    .map(str::to_string)
                    .collect();
                if paths.is_empty() {
                    continue;
                }
                let key = format!("files:{}", paths.join("\0"));
                if !seen.insert(key) {
                    continue;
                }
                let title = if paths.len() == 1 {
                    std::path::Path::new(&paths[0])
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&paths[0])
                        .to_string()
                } else {
                    format!("{} files", paths.len())
                };
                let full = paths.join("\n");
                let cmd = paths.iter().map(|p| shq(p)).collect::<Vec<_>>().join(" ");
                Item::new(cmd, Kind::Clip)
                    .title(crate::width::flatten(&title))
                    .sub(ago(ts))
                    .put("clip_kind", "files")
                    .put("paths", serde_json::to_string(&paths).unwrap_or_default())
                    .put("path", paths[0].clone())
                    .put("full", full)
            }
            "image" => {
                let Some(path) = v.get("path").and_then(|x| x.as_str()) else { continue };
                if crate::clipd::private_asset(path).is_none()
                    || !seen.insert(image_identity(&v, path))
                {
                    continue;
                }
                let width = v.get("width").and_then(|x| x.as_u64()).unwrap_or(0);
                let height = v.get("height").and_then(|x| x.as_u64()).unwrap_or(0);
                let dimensions = if width > 0 && height > 0 {
                    format!("{width}×{height}")
                } else {
                    String::new()
                };
                Item::new(shq(path), Kind::Clip)
                    .title("Image")
                    .fields([dimensions, ago(ts)])
                    .put("clip_kind", "image")
                    .put("path", path)
                    .put("full", path)
            }
            _ => {
                let Some(t) = v.get("t").and_then(|x| x.as_str()) else { continue };
                if t.is_empty() || !seen.insert(format!("text:{t}")) {
                    continue;
                }
                Item::new(t, Kind::Clip)
                    .title(crate::width::flatten(t))
                    .sub(ago(ts))
                    .put("clip_kind", "text")
                    .put("full", t)
            }
        };
        items.push(base.put("ts", format!("{ts:.3}")).rank(ts * 1_000_000.0));
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

#[cfg(test)]
mod tests {
    use super::image_identity;

    #[test]
    fn clipboard_images_are_identified_by_pixels_not_private_filename() {
        let first = serde_json::json!({"fingerprint":"fnv1a64-v1:same"});
        let second = serde_json::json!({"fingerprint":"fnv1a64-v1:same"});
        assert_eq!(image_identity(&first, "/clipboard/1.png"), image_identity(&second, "/clipboard/2.png"));
        let old = serde_json::json!({});
        assert_ne!(image_identity(&old, "/clipboard/1.png"), image_identity(&old, "/clipboard/2.png"));
    }
}
