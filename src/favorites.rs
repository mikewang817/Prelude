//! Launcher favourites for stable Agent objects.
//!
//! This is a Prelude preference, never metadata written into an Agent's own
//! files. The stored key names only an Agent, Skill or MCP server; paths,
//! commands and definitions do not belong here. Favourites change ordering
//! inside an object's existing kind band and nothing else.

use crate::item::{Item, Kind};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[cfg(not(test))]
fn file() -> PathBuf {
    crate::paths::config().join("favorites.txt")
}

// No test may read the person's real preference file. Tests that exercise
// persistence address `write_at` directly with their own temporary path.
#[cfg(test)]
fn file() -> PathBuf {
    std::env::temp_dir().join(format!("prelude-test-favorites-{}.txt", std::process::id()))
}

pub fn key(item: &Item) -> Option<String> {
    let (kind, value) = match item.kind {
        Kind::Agent => ("agent", item.get("agent")),
        Kind::Skill => ("skill", item.get("name")),
        Kind::Mcp => {
            let id = item.get("capability_id");
            ("mcp", if id.is_empty() { item.get("name") } else { id })
        }
        _ => return None,
    };
    let value = value.trim();
    if value.is_empty() || value.chars().any(|c| matches!(c, '\n' | '\r' | '\t')) {
        return None;
    }
    Some(format!("{kind}\t{value}"))
}

fn parse(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| {
            matches!(line.split_once('\t').map(|(kind, _)| kind), Some("agent" | "skill" | "mcp"))
                && !line.ends_with('\t')
        })
        .map(str::to_string)
        .collect()
}

fn read_at(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path).map(|text| parse(&text)).unwrap_or_default()
}

fn write_at(path: &Path, values: &BTreeSet<String>) -> Result<(), String> {
    let mut text = values.iter().cloned().collect::<Vec<_>>().join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    crate::cache::write_atomic(path, text.as_bytes())
        .map_err(|error| format!("could not save favourites: {error}"))
}

/// Create the empty preference through the module that owns its format. The
/// settings row calls this only when somebody explicitly asks to open it.
pub fn ensure_file() -> Result<PathBuf, String> {
    let path = file();
    if !path.exists() {
        write_at(&path, &BTreeSet::new())?;
    }
    Ok(path)
}

pub fn set(item: &Item, wanted: bool) -> Result<(), String> {
    let key = key(item).ok_or_else(|| "that object cannot be favourited".to_string())?;
    let path = file();
    let mut values = read_at(&path);
    if wanted {
        values.insert(key);
    } else {
        values.remove(&key);
    }
    write_at(&path, &values)
}

/// Mark and promote favourites on the gathered catalogue before its final
/// ranking. `cache::by_rank` compares Kind first, so this bonus can never lift
/// a Skill above an Agent or an MCP server above a Run. This is deliberately
/// not inside `cache::finish`: file scope calls that helper per keystroke and
/// must not read a preference file on every letter.
pub fn decorate(items: &mut [Item]) {
    decorate_with(items, &read_at(&file()));
}

fn decorate_with(items: &mut [Item], values: &BTreeSet<String>) {
    for item in items {
        let favourite = key(item).is_some_and(|key| values.contains(&key));
        if favourite {
            item.data.insert("favorite".into(), "true".into());
            item.score += 1_000.0;
        } else {
            item.data.remove("favorite");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favourites_are_stable_object_keys_and_never_paths_or_definitions() {
        let agent = Item::new("claude", Kind::Agent).put("agent", "claude");
        let skill = Item::new("run skill", Kind::Skill)
            .put("name", "review")
            .put("dir", "/private/home/.claude/skills/review");
        let mcp = Item::new("owner details", Kind::Mcp)
            .put("name", "node repl")
            .put("capability_id", "mcp:node-repl")
            .put("def", "API_KEY=must-not-appear");
        assert_eq!(key(&agent).as_deref(), Some("agent\tclaude"));
        assert_eq!(key(&skill).as_deref(), Some("skill\treview"));
        assert_eq!(key(&mcp).as_deref(), Some("mcp\tmcp:node-repl"));
        let all = [key(&agent), key(&skill), key(&mcp)].into_iter().flatten().collect::<String>();
        assert!(!all.contains("/private") && !all.contains("API_KEY"));
    }

    #[test]
    fn favourite_file_round_trips_and_ignores_unknown_kinds() {
        let root = std::env::temp_dir().join(format!(
            "prelude-favourites-{}-{}",
            std::process::id(),
            crate::frecency::now() as u64
        ));
        let path = root.join("favorites.txt");
        let mut values = BTreeSet::new();
        values.insert("agent\tclaude".into());
        values.insert("skill\treview".into());
        write_at(&path, &values).unwrap();
        std::fs::write(&path, "agent\tclaude\nunknown\tthing\nskill\treview\n").unwrap();
        assert_eq!(read_at(&path), values);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn promotion_stays_inside_the_kind_band() {
        let mut items = vec![
            Item::new("claude", Kind::Agent).put("agent", "claude"),
            Item::new("review", Kind::Skill).put("name", "review"),
        ];
        let values = BTreeSet::from(["skill\treview".to_string()]);
        decorate_with(&mut items, &values);
        items.sort_by(crate::cache::by_rank);
        assert_eq!(items[0].kind, Kind::Agent);
        assert_eq!(items[1].get("favorite"), "true");
    }
}
