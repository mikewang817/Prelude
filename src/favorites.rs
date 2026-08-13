//! Launcher favourites for stable objects.
//!
//! This is a Prelude preference, never metadata written into an Agent's own
//! files. The stored key names an Agent, Skill, MCP server, application or
//! saved Quicklink; paths, commands and definitions do not belong here.
//! Favourites change ordering inside an object's existing band and nothing
//! else.

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

/// A key is one tab-separated line, so a value carrying a tab or a newline
/// would read back as a different key — or as none at all.
fn stable(kind: &str, value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().any(|c| matches!(c, '\n' | '\r' | '\t')) {
        return None;
    }
    Some(format!("{kind}\t{value}"))
}

pub fn key(item: &Item) -> Option<String> {
    // A Quicklink is keyed by the name the person gave it, and that is settled
    // before the target's kind is consulted — the same order `Item::band` uses
    // and for the same reason. A quicklink pointing at an application would
    // otherwise be stored under the application's name and stop being the
    // thing that was named. The *result* of a template carries its provider's
    // key but is not a saved Quicklink, and `is_quicklink` is the line.
    if item.is_quicklink() {
        return stable("quicklink", item.get("quicklink"));
    }
    let (kind, value) = match item.kind {
        Kind::Agent => ("agent", item.get("agent")),
        Kind::Skill => ("skill", item.get("name")),
        Kind::Mcp => {
            let id = item.get("capability_id");
            ("mcp", if id.is_empty() { item.get("name") } else { id })
        }
        // An application is named by its name. The bundle path is the only
        // other thing on the row and is the one thing that may not be stored,
        // and a bundle identifier would mean reading an `Info.plist` for every
        // application on the machine on every gather.
        Kind::App => ("app", item.title.as_str()),
        _ => return None,
    };
    stable(kind, value)
}

fn parse(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| {
            matches!(
                line.split_once('\t').map(|(kind, _)| kind),
                Some("agent" | "skill" | "mcp" | "app" | "quicklink")
            ) && !line.ends_with('\t')
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
    crate::cache::write_state(path, text.as_bytes())
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

/// Stored stable keys, for the Settings collection manager.
pub fn entries() -> Vec<String> {
    read_at(&file()).into_iter().collect()
}

/// Remove a key selected in Settings without manufacturing an Agent item.
pub fn remove_key(key: &str) -> Result<(), String> {
    let path = file();
    let _lock = crate::cache::lock_for_write(&path);
    let mut values = read_at(&path);
    if !values.remove(key) {
        return Err("that Favorite is no longer present".into());
    }
    write_at(&path, &values)
}

pub fn set(item: &Item, wanted: bool) -> Result<(), String> {
    let key = key(item).ok_or_else(|| "that object cannot be favourited".to_string())?;
    let path = file();
    // Read, change, write. Two windows favouriting two different things at
    // the same moment kept one of them without the lock.
    let _lock = crate::cache::lock_for_write(&path);
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
        let app = Item::new("open -a 'Google Chrome'", Kind::App)
            .title("Google Chrome")
            .put("path", "/Applications/Google Chrome.app");
        assert_eq!(key(&agent).as_deref(), Some("agent\tclaude"));
        assert_eq!(key(&skill).as_deref(), Some("skill\treview"));
        assert_eq!(key(&mcp).as_deref(), Some("mcp\tmcp:node-repl"));
        assert_eq!(key(&app).as_deref(), Some("app\tGoogle Chrome"));
        let all = [key(&agent), key(&skill), key(&mcp), key(&app)]
            .into_iter()
            .flatten()
            .collect::<String>();
        assert!(!all.contains("/private") && !all.contains("/Applications"));
        assert!(!all.contains("API_KEY"));
    }

    // A Quicklink is keyed by the name the person gave it, before its target's
    // kind is consulted. Keyed the other way round, a quicklink pointing at an
    // application would be stored under the application and stop being the
    // thing that was named — and it would collide with favouriting that
    // application directly.
    #[test]
    fn a_quicklink_is_keyed_by_its_own_name_whatever_it_points_at() {
        let to_app = Item::new("open -a 'Google Chrome'", Kind::App)
            .title("Google Chrome")
            .quicklink("browser", "fixed");
        let plain_app = Item::new("open -a 'Google Chrome'", Kind::App).title("Google Chrome");
        assert_eq!(key(&to_app).as_deref(), Some("quicklink\tbrowser"));
        assert_eq!(key(&plain_app).as_deref(), Some("app\tGoogle Chrome"));
        assert_ne!(key(&to_app), key(&plain_app));
    }

    // The row `g rust async` produces carries its provider's key so the
    // provider can be edited, but it is a search result rather than a thing
    // anybody saved. Keying it would let one search pin the whole provider.
    #[test]
    fn a_template_result_is_not_a_favouritable_object() {
        let result = Item::new("https://www.google.com/search?q=rust", Kind::Link)
            .put("ql", "result")
            .put("quicklink", "g");
        assert_eq!(key(&result), None);
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
        // Every prefix `key` can produce must survive `parse`. A key written
        // and then dropped on the way back in is the quiet half of this bug:
        // the toggle reports success and the row is never promoted.
        values.insert("app\tGoogle Chrome".into());
        values.insert("quicklink\tbrowser".into());
        std::fs::write(
            &path,
            "agent\tclaude\nunknown\tthing\nskill\treview\napp\tGoogle Chrome\n\
             quicklink\tbrowser\nmcp\t\n",
        )
        .unwrap();
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

    // The same rule for the kinds that just gained a key. An application is a
    // low band and its bonus is larger than the whole frecency range, which is
    // exactly the arithmetic that must not be allowed to reach across bands.
    #[test]
    fn a_favourite_application_never_outranks_an_agent() {
        let mut items = vec![
            Item::new("claude", Kind::Agent).put("agent", "claude"),
            Item::new("open -a 'Google Chrome'", Kind::App).title("Google Chrome"),
            Item::new("https://example.test", Kind::Link).quicklink("browser", "fixed"),
        ];
        let values =
            BTreeSet::from(["app\tGoogle Chrome".to_string(), "quicklink\tbrowser".to_string()]);
        decorate_with(&mut items, &values);
        items.sort_by(crate::cache::by_rank);
        assert_eq!(items[0].kind, Kind::Agent);
        assert_eq!(items[0].get("favorite"), "");
        assert!(items[1].is_quicklink(), "the Quicklink band sits above App");
        assert_eq!(items[2].kind, Kind::App);
    }
}
