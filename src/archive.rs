//! Prelude-only archive state for Skill and MCP inventory objects.
//!
//! Archiving puts a capability away in Prelude. It never moves a Skill copy,
//! edits an Agent configuration or disables an MCP server. The metadata keeps
//! only the same stable object key Favorites uses: no path, command, native
//! definition or credential-bearing value belongs here.

use crate::item::{Item, Kind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const SCHEMA: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
struct Metadata {
    schema: u32,
    #[serde(default)]
    archived: BTreeSet<String>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self { schema: SCHEMA, archived: BTreeSet::new() }
    }
}

#[cfg(not(test))]
fn file() -> PathBuf {
    crate::paths::data().join("capabilities.json")
}

// Tests must never read or write the person's real archive metadata.
#[cfg(test)]
fn file() -> PathBuf {
    std::env::temp_dir().join(format!("prelude-test-capabilities-{}.json", std::process::id()))
}

/// Stable archive identity. A merged Skill is one object by name; every owner
/// variant of one MCP server shares its normalized capability id.
pub fn key(item: &Item) -> Option<String> {
    matches!(item.kind, Kind::Skill | Kind::Mcp)
        .then(|| crate::favorites::key(item))
        .flatten()
}

fn read_at(path: &Path) -> Result<Metadata, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Metadata::default()),
        Err(error) => return Err(format!("could not read capability archive metadata: {error}")),
    };
    let metadata: Metadata = serde_json::from_slice(&bytes)
        .map_err(|error| format!("capability archive metadata is not valid JSON: {error}"))?;
    if metadata.schema != SCHEMA {
        return Err(format!(
            "capability archive metadata schema {} is not supported by this Prelude",
            metadata.schema
        ));
    }
    Ok(metadata)
}

fn write_at(path: &Path, metadata: &Metadata) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|error| format!("could not encode capability archive metadata: {error}"))?;
    // 0600, flushed before the rename, unique temp name — the same rules the
    // bus, favorites and frecency follow, in the one place that owns them.
    crate::cache::write_state(path, &bytes)
        .map_err(|error| format!("could not write capability archive metadata: {error}"))
}

fn set_at(path: &Path, item: &Item, archived: bool) -> Result<(), String> {
    let key = key(item).ok_or_else(|| "that object cannot be archived".to_string())?;
    // Read, change, write: held under the write lock so archiving two things
    // from two windows does not silently keep one of them.
    let _lock = crate::cache::lock_for_write(path);
    // A malformed file is evidence, not an empty preference. Refuse to
    // overwrite it invisibly when an action is applied.
    let mut metadata = read_at(path)?;
    if archived {
        metadata.archived.insert(key);
    } else {
        metadata.archived.remove(&key);
    }
    write_at(path, &metadata)
}

pub fn set(item: &Item, archived: bool) -> Result<(), String> {
    set_at(&file(), item, archived)
}

/// Apply the overlay once to a gathered snapshot. Per-keystroke filtering only
/// reads the resulting `archived` field and never opens this metadata file.
pub fn decorate(items: &mut [Item]) {
    let metadata = read_at(&file()).unwrap_or_default();
    decorate_with(items, &metadata.archived);
}

fn decorate_with(items: &mut [Item], archived: &BTreeSet<String>) {
    for item in items {
        if key(item).is_some_and(|key| archived.contains(&key)) {
            item.data.insert("archived".into(), "true".into());
        } else if matches!(item.kind, Kind::Skill | Kind::Mcp) {
            item.data.remove("archived");
        }
    }
}

/// Default inventory surfaces put archived capabilities away. The complete
/// gathered snapshot retains them so explicit archive scopes can restore them.
pub fn visible(item: &Item) -> bool {
    !matches!(item.kind, Kind::Skill | Kind::Mcp) || item.get("archived") != "true"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_keys_are_stable_and_never_native_data() {
        let skill = Item::new("/review", Kind::Skill)
            .put("name", "review")
            .put("dir", "/private/home/.claude/skills/review");
        let first = Item::new("claude mcp get drive", Kind::Mcp)
            .put("name", "Drive")
            .put("capability_id", "mcp:drive")
            .put("definition_public", "API_KEY=must-not-appear");
        let second = Item::new("codex mcp get drive", Kind::Mcp)
            .put("name", "Drive")
            .put("capability_id", "mcp:drive");
        assert_eq!(key(&skill).as_deref(), Some("skill\treview"));
        assert_eq!(key(&first).as_deref(), Some("mcp\tmcp:drive"));
        assert_eq!(key(&first), key(&second), "owner variants are one capability");
        let stored = format!("{}{}", key(&skill).unwrap(), key(&first).unwrap());
        assert!(!stored.contains("/private") && !stored.contains("API_KEY"));
        assert!(key(&Item::new("claude", Kind::Agent).put("agent", "claude")).is_none());
    }

    #[test]
    fn metadata_is_private_atomic_and_refuses_a_malformed_overwrite() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "prelude-capability-archive-{}-{}",
            std::process::id(),
            crate::frecency::now() as u64
        ));
        let path = root.join("capabilities.json");
        let metadata = Metadata {
            schema: SCHEMA,
            archived: BTreeSet::from(["skill\treview".to_string()]),
        };
        write_at(&path, &metadata).unwrap();
        assert_eq!(read_at(&path).unwrap().archived, metadata.archived);
        #[cfg(unix)]
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);

        let native = root.join("SKILL.md");
        std::fs::write(&native, b"native instructions").unwrap();
        let item = Item::new("/deploy", Kind::Skill)
            .put("name", "deploy")
            .put("file", native.to_string_lossy());
        set_at(&path, &item, true).unwrap();
        assert!(read_at(&path).unwrap().archived.contains("skill\tdeploy"));
        set_at(&path, &item, false).unwrap();
        assert!(!read_at(&path).unwrap().archived.contains("skill\tdeploy"));
        assert_eq!(std::fs::read(&native).unwrap(), b"native instructions");

        std::fs::write(&path, b"not json").unwrap();
        assert!(read_at(&path).unwrap_err().contains("not valid JSON"));
        assert!(set_at(&path, &item, true).unwrap_err().contains("not valid JSON"));
        assert_eq!(std::fs::read(&path).unwrap(), b"not json");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn one_overlay_marks_every_mcp_owner_variant_and_only_that_skill() {
        let mut items = vec![
            Item::new("/review", Kind::Skill).put("name", "review"),
            Item::new("/deploy", Kind::Skill).put("name", "deploy"),
            Item::new("claude mcp get drive", Kind::Mcp)
                .put("name", "Drive")
                .put("capability_id", "mcp:drive"),
            Item::new("codex mcp get drive", Kind::Mcp)
                .put("name", "Drive")
                .put("capability_id", "mcp:drive"),
        ];
        let values = BTreeSet::from(["skill\treview".into(), "mcp\tmcp:drive".into()]);
        decorate_with(&mut items, &values);
        assert_eq!(items.iter().filter(|item| item.get("archived") == "true").count(), 3);
        assert!(visible(&items[1]));
        assert!(!visible(&items[0]) && !visible(&items[2]));
    }
}
