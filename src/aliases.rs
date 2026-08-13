//! Names the person chose for objects they already have.
//!
//! An alias stores the same stable object key Favorites stores and nothing
//! else: no path, no command, no definition. The row it leads to is found in
//! the gathered catalogue at the moment the alias is typed, so an alias whose
//! object is gone shows nothing rather than a row that lies about what is
//! there.
//!
//! That also fixes what can be aliased. `favorites::key` answers for an Agent,
//! a Skill, an MCP server, an application and a saved Quicklink — the objects
//! with an identity that outlives one gather. A session, a file and a history
//! entry have no such identity, so they are not aliasable and saying otherwise
//! would mean storing a path here.
//!
//! Refusal happens at the moment of naming. A name accepted and then silently
//! unreachable is the failure this file exists to avoid, and the moment
//! somebody types it is the only moment a reason can be given.

use crate::item::Item;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(not(test))]
fn file() -> PathBuf {
    crate::paths::config().join("aliases.txt")
}

// No test may read the person's real preference file.
#[cfg(test)]
fn file() -> PathBuf {
    std::env::temp_dir().join(format!("prelude-test-aliases-{}.txt", std::process::id()))
}

/// `alias<TAB>kind<TAB>value`. Everything after the first tab is the object
/// key exactly as `favorites::key` writes it, so the two files speak one
/// vocabulary and neither has to know the other's format.
fn parse(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.trim_end().split_once('\t'))
        .filter(|(alias, target)| {
            !alias.trim().is_empty() && target.contains('\t') && !target.ends_with('\t')
        })
        .map(|(alias, target)| (alias.trim().to_lowercase(), target.to_string()))
        .collect()
}

fn read_at(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path).map(|text| parse(&text)).unwrap_or_default()
}

fn cache() -> &'static std::sync::RwLock<Option<BTreeMap<String, String>>> {
    static ENTRIES: std::sync::OnceLock<std::sync::RwLock<Option<BTreeMap<String, String>>>> =
        std::sync::OnceLock::new();
    ENTRIES.get_or_init(|| std::sync::RwLock::new(None))
}

fn invalidate() {
    if let Ok(mut slot) = cache().write() {
        *slot = None;
    }
}

/// Read once per process. `is_special` asks this on every keystroke, so the
/// open, read and parse happen at most once — the same shape, and for the same
/// reason, as `quicklinks_text`.
fn all() -> BTreeMap<String, String> {
    if let Ok(slot) = cache().read() {
        if let Some(entries) = slot.as_ref() {
            return entries.clone();
        }
    }
    let entries = read_at(&file());
    if let Ok(mut slot) = cache().write() {
        *slot = Some(entries.clone());
    }
    entries
}

/// The object key an exactly-typed alias names. Every lookup folds case.
pub fn target_of(query: &str) -> Option<String> {
    let want = query.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    all().get(&want).cloned()
}

/// Stored pairs, for `alias list` and the settings collection.
pub fn entries() -> Vec<(String, String)> {
    all().into_iter().collect()
}

fn write_all(entries: &BTreeMap<String, String>) -> Result<(), String> {
    let mut text = entries
        .iter()
        .map(|(alias, target)| format!("{alias}\t{target}"))
        .collect::<Vec<_>>()
        .join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    crate::cache::write_state(&file(), text.as_bytes())
        .map_err(|error| format!("could not save aliases: {error}"))?;
    invalidate();
    Ok(())
}

/// Why this name cannot be an alias, said at the moment somebody types it.
///
/// The three grounds are the ones `quicklink_conflict` and `append_quicklink`
/// already refuse on, because the search box resolves in a fixed order and
/// anything earlier in that order wins forever:
///
/// * a scope command — `dynamic_rows_with` settles `f:` before any name;
/// * a built-in Agent's name — an installed Agent leads its own name, so an
///   alias called `claude` would push the row it names down a line;
/// * a keyword a Quicklink already carries, and an alias already stored.
///
/// The key itself is validated by `normalize_quicklink_key`, which is where
/// `:` `/` `@` and `.` are excluded — each already means something in the
/// search box — and which accepts letters and digits of any script.
pub fn conflict(key: &str) -> Option<String> {
    if let Some(why) = crate::compute::quicklink_conflict(key) {
        return Some(why);
    }
    if crate::agent::get(key).is_some() {
        return Some(format!("“{key}” is an Agent's own name — the Agent row would always win"));
    }
    if let Some((existing, _)) =
        crate::compute::quicklink_entry(&crate::compute::quicklinks(), key)
    {
        return Some(format!("“{existing}” is already a quicklink keyword"));
    }
    if all().contains_key(key) {
        return Some(format!("“{key}” is already an alias"));
    }
    None
}

/// The key, normalized and cleared — or the reason it can never work.
///
/// Every caller goes through this *before* looking at what the name would
/// point at. A name that cannot work is wrong whatever it points at, the
/// reason belongs to the name, and resolving a target costs a whole gather.
/// Ordered the other way round, naming `f:` reported that the target could
/// not be found — which is true, useless, and about the wrong half.
pub fn vet(raw_key: &str) -> Result<String, String> {
    // The refusal names the key. `normalize_quicklink_key` states the rule and
    // is shared with quicklinks, where the key is already on screen in a
    // prompt; here it arrived as an argument among others and a bare "use 1–40
    // letters" leaves the person to work out which word was the problem.
    let key = crate::compute::normalize_quicklink_key(raw_key)
        .map_err(|why| format!("“{}” cannot be a name: {why}", raw_key.trim()))?;
    match conflict(&key) {
        Some(why) => Err(why),
        None => Ok(key),
    }
}

/// Name an object.
pub fn add(raw_key: &str, item: &Item) -> Result<String, String> {
    let key = vet(raw_key)?;
    let target = crate::favorites::key(item)
        .ok_or_else(|| "that row has no identity that outlives a search".to_string())?;
    let path = file();
    // Read, change, write: two windows naming two things at once kept one.
    let _lock = crate::cache::lock_for_write(&path);
    let mut entries = read_at(&path);
    entries.insert(key.clone(), target);
    write_all(&entries)?;
    Ok(key)
}

pub fn remove(raw_key: &str) -> Result<(), String> {
    let key = raw_key.trim().to_lowercase();
    let path = file();
    let _lock = crate::cache::lock_for_write(&path);
    let mut entries = read_at(&path);
    if entries.remove(&key).is_none() {
        return Err(format!("there is no alias called “{key}”"));
    }
    write_all(&entries)
}

/// Objects a name could be given to, as the launcher would show them.
fn aliasable() -> Vec<Item> {
    let mut items = crate::cache::gather();
    items.retain(|item| crate::favorites::key(item).is_some());
    items
}

/// `prelude alias` — the same verbs without fzf, for the reason
/// `settings add-root` exists.
pub fn cli(args: &[&str]) -> i32 {
    let result = match args {
        [] | ["list"] => {
            for (alias, target) in entries() {
                println!("{alias}\t{target}");
            }
            Ok(())
        }
        ["add", key, rest @ ..] if !rest.is_empty() => {
            // The name is vetted first. `aliasable` runs a whole gather, and
            // the answer to "why can I not call it f:" is not about the target.
            let want = match vet(key) {
                Ok(_) => rest.join(" "),
                Err(why) => {
                    eprintln!("{why}");
                    return 2;
                }
            };
            // Exactly one hit or nothing, the rule `bus::say` follows: an
            // alias pointing at the wrong one of two same-named objects is
            // worse than an alias that was refused.
            let hits: Vec<Item> = aliasable()
                .into_iter()
                .filter(|item| item.title.eq_ignore_ascii_case(&want))
                .collect();
            match hits.as_slice() {
                [item] => add(key, item).map(|key| println!("{key} → {}", item.title)),
                [] => Err(format!("nothing here is called “{want}”")),
                many => Err(format!(
                    "“{want}” names {} objects; nothing was saved",
                    many.len()
                )),
            }
        }
        ["remove", key] => remove(key).map(|()| println!("removed {key}")),
        _ => Err("usage: prelude alias [list] | add KEY NAME | remove KEY".into()),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Kind;

    fn app(name: &str) -> Item {
        Item::new(format!("open -a '{name}'"), Kind::App).title(name)
    }

    #[test]
    fn a_name_the_search_box_has_already_spent_is_refused_when_it_is_typed() {
        // The key itself: `:` is excluded because it opens a scope, so the
        // scope would resolve first and the alias would never be reached.
        assert!(crate::compute::normalize_quicklink_key("f:").is_err());
        // And the bare word, which normalizes fine and still cannot work.
        assert!(conflict("f").is_some());
        assert!(conflict("claude").is_some(), "an Agent leads its own name");
        assert!(conflict("zzunusedname").is_none());
    }

    #[test]
    fn an_alias_stores_an_object_key_and_never_a_path_or_a_command() {
        let item = app("Google Chrome").put("path", "/Applications/Google Chrome.app");
        let target = crate::favorites::key(&item).unwrap();
        let line = format!("browser\t{target}");
        let parsed = parse(&line);
        assert_eq!(parsed.get("browser").map(String::as_str), Some("app\tGoogle Chrome"));
        assert!(!line.contains("/Applications"));
        assert!(!line.contains("open -a"));
    }

    #[test]
    fn lookup_folds_case_and_ignores_lines_that_are_not_pairs() {
        let parsed = parse("Browser\tapp\tGoogle Chrome\nbroken\nempty\t\n\tno-alias\tx\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.get("browser").map(String::as_str), Some("app\tGoogle Chrome"));
    }

    #[test]
    fn a_row_with_no_lasting_identity_cannot_be_named() {
        let session = Item::new("claude --resume abc", Kind::Session).title("some conversation");
        assert!(add("chat", &session).is_err());
    }
}
