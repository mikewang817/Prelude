//! Caching, background refresh, and the tiered gather.

use crate::item::Item;
use crate::paths;
use std::time::Duration;

pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Re-invoke ourselves as a detached background helper.
pub fn spawn_self(args: &[&str]) {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = std::process::Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn cache_file(name: &str) -> std::path::PathBuf {
    paths::cache().join(format!("{name}.json"))
}

pub fn read_cached(name: &str) -> Vec<Item> {
    std::fs::read_to_string(cache_file(name))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn write_cached(name: &str, items: &[Item]) {
    if let Ok(json) = serde_json::to_vec(items) {
        let _ = write_atomic(&cache_file(name), &json);
    }
}

const REFRESH_TTL: u64 = 5;

fn stale(name: &str) -> bool {
    cache_file(name)
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .map(|age| age.as_secs() >= REFRESH_TTL)
        .unwrap_or(true)
}

/// Sources that shell out but are fast enough to wait for.
const FAST: &[(&str, fn() -> Vec<Item>)] = &[
    ("dirs", crate::sources::user::dirs),
    ("containers", crate::sources::machine::containers),
    ("files", crate::sources::project::files),
    ("procs", crate::sources::machine::procs),
];

/// Too slow to ever block on: lsof costs ~65ms and cannot be made faster.
/// Served from cache and refreshed detached. Safe because the kill command
/// re-resolves the pid at run time rather than trusting the cached one.
const SLOW: &[(&str, fn() -> Vec<Item>)] = &[
    ("ports", crate::sources::machine::ports),
    // Hundreds of session files, each needing its head parsed.
    ("sessions", crate::sources::sessions::all),
    // `claude mcp list` runs a network health check on every server.
    ("mcp", crate::sources::agents::mcp),
];

pub fn refresh_named(name: &str) -> bool {
    for (n, f) in FAST.iter().chain(SLOW.iter()) {
        if *n == name {
            write_cached(n, &f());
            return true;
        }
    }
    false
}

const EXTERNAL_DEADLINE: Duration = Duration::from_millis(50);

pub fn gather() -> Vec<Item> {
    // Kick the subprocess-backed sources off first so they overlap with the
    // cheap local ones instead of adding to them.
    let handles: Vec<_> = FAST
        .iter()
        .map(|(name, f)| {
            let name = *name;
            let f = *f;
            (name, std::thread::spawn(move || {
                let items = f();
                write_cached(name, &items);
                items
            }))
        })
        .collect();

    let mut items = Vec::with_capacity(2600);
    for (name, _) in SLOW {
        // Sessions are numerous enough to swamp the list; only the newest
        // few go in, and `s:` searches the rest.
        if *name == "sessions" {
            items.extend(crate::sources::sessions::recent());
        } else {
            items.extend(read_cached(name));
        }
        if stale(name) {
            spawn_self(&["_refresh", name]);
        }
    }

    // Pure file/CPU work — microseconds each, just run them.
    items.extend(crate::sources::project::scripts());
    items.extend(crate::sources::project::git());
    items.extend(crate::sources::history::source());
    items.extend(crate::sources::user::path_commands());
    items.extend(crate::sources::user::ssh());
    items.extend(crate::sources::user::snippets());
    items.extend(crate::sources::user::clips());
    items.extend(crate::sources::agents::skills());

    items.extend(crate::sources::machine::apps());
    items.extend(crate::sources::machine::system());
    items.extend(crate::sources::agents::configs());

    let deadline = std::time::Instant::now() + EXTERNAL_DEADLINE;
    for (name, h) in handles {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() && !h.is_finished() {
            // Missed the deadline — show the last known result rather than
            // stalling. The straggler keeps running and refreshes the cache
            // while you read the list, so the next launch is current.
            items.extend(read_cached(name));
            continue;
        }
        match join_before(h, left) {
            Some(v) => items.extend(v),
            None => items.extend(read_cached(name)),
        }
    }
    finish(items)
}

fn join_before(h: std::thread::JoinHandle<Vec<Item>>, budget: Duration) -> Option<Vec<Item>> {
    let start = std::time::Instant::now();
    while start.elapsed() < budget {
        if h.is_finished() {
            return h.join().ok();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if h.is_finished() { h.join().ok() } else { None }
}

/// Just the agent-owned rows, for the `a:` overview.
pub fn gather_agents() -> Vec<Item> {
    let mut items = read_cached("mcp");
    items.extend(crate::sources::agents::skills());
    items.extend(crate::sources::agents::configs());
    items.extend(read_cached("sessions"));
    finish(items)
}

/// Dedupe, apply learned ranking, sort.
pub fn finish(items: Vec<Item>) -> Vec<Item> {
    let freq = crate::frecency::load();
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    let mut out = Vec::with_capacity(items.len());
    for mut it in items {
        // Keyed on kind too: the same text can legitimately appear as a file
        // and as a history entry, and they have different actions.
        if !seen.insert((it.kind, it.cmd.clone())) {
            continue;
        }
        if let Some((n, last)) = freq.get(&it.cmd) {
            it.score += crate::frecency::score(*n, *last) * 12.0;
        }
        out.push(it);
    }
    out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    out
}
