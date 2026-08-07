//! Sources scoped to the project you are standing in.
//!
//! All of them return empty when there is no project context — which happens
//! when the directory has been deleted out from under the shell.

use crate::exec::{run, shq, which};
use crate::item::{Item, Kind};
use crate::paths;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MARKERS: &[&str] = &[
    ".git", "package.json", "Cargo.toml", "Makefile", "justfile", "Justfile",
    "pyproject.toml", "go.mod",
];

pub fn root() -> Option<PathBuf> {
    let cur = paths::cwd()?.canonicalize().ok()?;
    let mut dir: Option<&Path> = Some(&cur);
    while let Some(d) = dir {
        if MARKERS.iter().any(|m| d.join(m).exists()) {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    Some(cur)
}

fn read(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// The highest-value source: things this specific project can do.
pub fn scripts() -> Vec<Item> {
    let Some(root) = root() else { return Vec::new() };
    let mut items = Vec::new();
    let cwd = root.to_string_lossy().into_owned();

    // package.json — pick the runner the lockfile implies.
    if let Some(text) = read(&root.join("package.json")) {
        let runner = if root.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if root.join("yarn.lock").exists() {
            "yarn"
        } else if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
            "bun run"
        } else {
            "npm run"
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(obj) = v.get("scripts").and_then(|s| s.as_object()) {
                for (name, body) in obj {
                    let body = body.as_str().unwrap_or("").to_string();
                    items.push(
                        Item::new(format!("{runner} {name}"), Kind::Script)
                            .fields(["package.json".to_string(), body])
                            .cwd(&cwd),
                    );
                }
            }
        }
    }

    // Makefile targets: a name at line start followed by ':' but not ':='.
    for name in ["Makefile", "makefile", "GNUmakefile"] {
        let Some(text) = read(&root.join(name)) else { continue };
        for line in text.lines() {
            let Some(target) = make_target(line) else { continue };
            items.push(
                Item::new(format!("make {target}"), Kind::Script).sub(name).cwd(&cwd),
            );
        }
        break;
    }

    // justfile recipes.
    for name in ["justfile", "Justfile", ".justfile"] {
        let Some(text) = read(&root.join(name)) else { continue };
        for line in text.lines() {
            let Some(recipe) = just_recipe(line) else { continue };
            items.push(
                Item::new(format!("just {recipe}"), Kind::Script).sub("justfile").cwd(&cwd),
            );
        }
        break;
    }

    if root.join("Cargo.toml").exists() {
        for c in ["cargo run", "cargo build --release", "cargo test",
                  "cargo clippy --all-targets", "cargo fmt"] {
            items.push(Item::new(c, Kind::Script).sub("Cargo.toml").cwd(&cwd));
        }
    }

    if let Some(text) = read(&root.join("pyproject.toml")) {
        let t = crate::minitoml::parse(&text);
        for key in ["tool.poetry.scripts", "tool.pdm.scripts"] {
            let Some(tbl) = t.get(key) else { continue };
            let runner = if key.contains("poetry") { "poetry" } else { "pdm" };
            for name in tbl.keys() {
                items.push(
                    Item::new(format!("{runner} run {name}"), Kind::Script)
                        .sub("pyproject.toml").cwd(&cwd),
                );
            }
        }
        if root.join("uv.lock").exists() {
            items.push(Item::new("uv run pytest", Kind::Script).sub("pyproject.toml").cwd(&cwd));
        }
    }

    for name in ["docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"] {
        let Some(text) = read(&root.join(name)) else { continue };
        for c in ["docker compose up -d", "docker compose logs -f"] {
            items.push(Item::new(c, Kind::Script).sub(name).cwd(&cwd));
        }
        if let Some((_, body)) = text.split_once("\nservices:") {
            for line in body.lines() {
                let Some(svc) = compose_service(line) else { continue };
                items.push(
                    Item::new(format!("docker compose up -d {svc}"), Kind::Script)
                        .sub(format!("{name} · service")).cwd(&cwd),
                );
            }
        }
        break;
    }
    items
}

fn make_target(line: &str) -> Option<&str> {
    let name = line.split(':').next()?;
    if name.is_empty() || name.len() == line.len() {
        return None;
    }
    if line[name.len()..].starts_with(":=") {
        return None;
    }
    let first = name.chars().next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) {
        return None;
    }
    if matches!(name, "PHONY" | "SUFFIXES" | "DEFAULT") {
        return None;
    }
    Some(name)
}

fn just_recipe(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) || !line.contains(':') {
        return None;
    }
    let head = line.split(':').next()?;
    if line[head.len()..].starts_with(":=") {
        return None;
    }
    let name = head.split_whitespace().next()?;
    let first = name.chars().next()?;
    if !first.is_ascii_alphanumeric()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
    {
        return None;
    }
    Some(name)
}

fn compose_service(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("  ")?;
    if rest.starts_with(char::is_whitespace) {
        return None;
    }
    let name = rest.strip_suffix(':')?;
    let first = name.chars().next()?;
    if !first.is_ascii_alphanumeric()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    {
        return None;
    }
    Some(name)
}

/// Branches, read straight off disk — no subprocess, no waiting.
pub fn git() -> Vec<Item> {
    let Some(root) = root() else { return Vec::new() };
    let gitdir = root.join(".git");
    if !gitdir.is_dir() {
        return Vec::new();
    }
    let mut names = Vec::new();
    let heads = gitdir.join("refs/heads");
    collect_refs(&heads, &heads, &mut names);
    if let Some(packed) = read(&gitdir.join("packed-refs")) {
        for line in packed.lines() {
            if let Some((_, r)) = line.split_once(" refs/heads/") {
                names.push(r.trim().to_string());
            }
        }
    }
    let current = read(&gitdir.join("HEAD"))
        .and_then(|h| h.trim().strip_prefix("ref: refs/heads/").map(str::to_string));

    names.sort();
    names.dedup();
    let mut items: Vec<Item> = names
        .into_iter()
        .filter(|n| Some(n.as_str()) != current.as_deref())
        .map(|n| Item::new(format!("git switch {}", shq(&n)), Kind::Git).sub("checkout branch"))
        .collect();
    for (c, s) in [
        ("git status --short --branch", "working tree"),
        ("git pull --rebase", "sync"),
        ("git log --oneline -20", "recent commits"),
        ("git diff", "unstaged changes"),
    ] {
        items.push(Item::new(c, Kind::Git).sub(s));
    }
    items
}

fn collect_refs(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_refs(base, &p, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
}

/// Files in the current project, for inserting a path.
pub fn files() -> Vec<Item> {
    let Some(root) = root() else { return Vec::new() };
    let rootstr = root.to_string_lossy().into_owned();
    // No --strip-cwd-prefix: fd rejects it alongside an explicit path.
    let out = match which("fd").or_else(|| which("fdfind")) {
        Some(fd) => run(
            &[&fd.to_string_lossy(), "--type", "f", "--max-depth", "6",
              "--color", "never", ".", &rootstr],
            Duration::from_secs(5),
        ),
        None => run(
            &["find", &rootstr, "-maxdepth", "6", "-type", "f", "-not", "-path", "*/.git/*"],
            Duration::from_secs(5),
        ),
    };
    out.lines()
        .take(2000)
        .filter(|p| !p.trim().is_empty() && !p.contains("/.git/"))
        .filter_map(|p| {
            let full = if p.starts_with('/') {
                p.to_string()
            } else {
                root.join(p).to_string_lossy().into_owned()
            };
            let rel = full.strip_prefix(&rootstr)?.trim_start_matches('/').to_string();
            let dir = rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_else(|| ".".into());
            Some(Item::new(rel.clone(), Kind::File).title(rel).sub(dir).put("path", full))
        })
        .collect()
}
