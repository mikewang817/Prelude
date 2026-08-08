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

/// One-time removal of derived rows written by builds that retained complete
/// MCP definitions. The private `borrow/` staging area is intentional and is
/// not touched; list/search caches must never contain those values.
pub fn privacy_migrations() {
    if std::env::var_os("PRELUDE_PRIVACY_MIGRATED").is_some() {
        return;
    }
    let marker = paths::cache().join("capability-privacy-v1");
    if !marker.exists() {
        let _ = read_cached("mcp"); // rewrites the MCP cache without `def`
        for name in ["search-items.json", "list.txt", "home.txt"] {
            let _ = std::fs::remove_file(paths::cache().join(name));
        }
        let _ = write_atomic(&marker, b"1\n");
    }
    std::env::set_var("PRELUDE_PRIVACY_MIGRATED", "1");
}

/// `score` is `#[serde(skip)]`, so anything read back from disk arrives at
/// zero and sorts below all two thousand live rows — which is why an MCP
/// server with priority 985 was never once seen near the top. The band is
/// restored from the kind, and a source that ranked its own items says so
/// in `rank`.
pub fn read_cached(name: &str) -> Vec<Item> {
    let mut items: Vec<Item> = std::fs::read_to_string(cache_file(name))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut scrubbed = false;
    for it in &mut items {
        // Older MCP caches retained complete definitions for fast lending.
        // Purge every one while reading so an upgrade stops exposing private
        // arguments before the background refresh lands.
        if name == "mcp" && !it.get("def").is_empty() {
            it.data.remove("def");
            // Old caches did not distinguish why a definition was private.
            // Conservatively mark it until the authoritative CLI refreshes.
            it.data.insert("sensitive".into(), "true".into());
            scrubbed = true;
        }
        it.score = it.kind.priority() as f64 + it.get("rank").parse::<f64>().unwrap_or(0.0);
    }
    if scrubbed {
        if let Ok(json) = serde_json::to_vec(&items) {
            let _ = write_atomic(&cache_file(name), &json);
        }
        for derived in ["search-items.json", "list.txt", "home.txt"] {
            let _ = std::fs::remove_file(paths::cache().join(derived));
        }
    }
    items
}

pub fn write_cached(name: &str, items: &[Item]) {
    if let Ok(json) = serde_json::to_vec(items) {
        let _ = write_atomic(&cache_file(name), &json);
    }
}

/// A derived cache often has identical bytes from one launch to the next.
/// Avoid an atomic rewrite in that common case while still paying only one
/// linear serialization over data the caller already holds.
pub fn write_cached_if_changed(name: &str, items: &[Item]) {
    let Ok(json) = serde_json::to_vec(items) else { return };
    if std::fs::read(cache_file(name)).is_ok_and(|old| old == json) {
        return;
    }
    let _ = write_atomic(&cache_file(name), &json);
}

fn refresh_ttl(name: &str) -> u64 {
    match name {
        "mcp-tools" => 300,
        "mcp" => 60,
        "skill-hashes" => 30,
        _ => 5,
    }
}

pub fn stale(name: &str) -> bool {
    cache_file(name)
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .map(|age| age.as_secs() >= refresh_ttl(name))
        .unwrap_or(true)
}

/// A named source: what to call it in the cache, and how to gather it.
type Source = (&'static str, fn() -> Vec<Item>);

/// Sources that shell out but are fast enough to wait for.
const FAST: &[Source] = &[
    ("dirs", crate::sources::user::dirs),
    ("containers", crate::sources::machine::containers),
    ("files", crate::sources::project::files),
    ("procs", crate::sources::machine::procs),
];

/// Too slow to ever block on: lsof costs ~65ms and cannot be made faster.
/// Served from cache and refreshed detached. Safe because the kill command
/// re-resolves the pid at run time rather than trusting the cached one.
const SLOW: &[Source] = &[
    ("ports", crate::sources::machine::ports),
    // Hundreds of session files, each needing its head parsed.
    ("sessions", crate::sources::sessions::all),
    // `claude mcp list` runs a network health check on every server.
    ("mcp", crate::sources::agents::mcp),
    // Full Skill trees can contain scripts and references. Hash them away
    // from the launch path; gather reads only the small fingerprint cache.
    ("skill-hashes", crate::sources::agents::skill_hashes),
    // MCP initialize + tools/list can start local server processes. It is a
    // background inventory, never part of health gather or a keypress.
    ("mcp-tools", crate::mcp_tools::inventory),
    // ps with full command lines plus a bulk lsof for their working
    // directories: ~95ms, and worth having only for completeness.
    ("fleet", crate::sources::running::fleet),
];

pub fn refresh_named(name: &str) -> bool {
    // Not a cache of Items, and deliberately reached through the same door.
    // The task store's open-set index is reconstructed by walking the whole
    // directory — 54 ms over five thousand records, more than a launch's
    // entire budget — so the launcher refuses to do it in the foreground and
    // hands it here instead, exactly as it hands over a stale SLOW source.
    if name == "task-index" {
        crate::task::rebuild_index();
        return true;
    }
    // Same door, same reason: the bus's pending-question index is
    // reconstructed by reading every message file, which is ~60 ms once five
    // thousand have accumulated — and undelivered mail is never swept, so that
    // is a ceiling a busy fleet reaches and stays at.
    if name == "bus-index" {
        crate::bus::rebuild_index();
        return true;
    }
    for (n, f) in FAST.iter().chain(SLOW.iter()) {
        if *n == name {
            write_cached(n, &f());
            return true;
        }
    }
    false
}

/// The whole of `gather`, not the part of it left after the local sources
/// have run.
///
/// It used to be measured from the moment the local work finished, which
/// meant the real bound was that work *plus* this — around sixty
/// milliseconds against a budget of forty. Anchoring it at the start makes
/// the number mean what `bench` asserts: a launch takes this long at worst,
/// whatever any subprocess is doing.
const EXTERNAL_DEADLINE: Duration = Duration::from_millis(40);

pub fn gather() -> Vec<Item> {
    let deadline = std::time::Instant::now() + EXTERNAL_DEADLINE;
    // Kick the subprocess-backed sources off first so they overlap with the
    // cheap local ones instead of adding to them. Each reports through the
    // channel as it finishes, so nothing is polled and no one waits on a
    // source that is already done.
    let (tx, rx) = std::sync::mpsc::channel();
    for (i, (name, f)) in FAST.iter().enumerate() {
        let (name, f, tx) = (*name, *f, tx.clone());
        std::thread::spawn(move || {
            let items = f();
            write_cached(name, &items);
            // The receiver is gone once the deadline passes; the cache write
            // above is what that straggler was still worth.
            let _ = tx.send((i, items));
        });
    }
    drop(tx);

    let mut items = Vec::with_capacity(2600);
    // Search providers, fixed Quicklinks and scope commands belong in global
    // search, but the empty-query home filters them back out.
    items.extend(crate::compute::quicklink_items());
    items.extend(crate::compute::scope_commands());

    // Read once, then hand down. Sessions is six hundred rows of JSON that
    // the recent list, the skill ranking and the agent summary all want, and
    // the MCP list is wanted twice — between them they were most of the
    // local half of a gather, spent parsing the same two files over.
    let sessions = read_cached("sessions");
    let mcp = read_cached("mcp");
    let runs = crate::sources::running::live_with_sessions(&sessions);
    let sessions = crate::sources::running::annotate_sessions(sessions, &runs);
    write_cached_if_changed("sessions-linked", &sessions);
    for (name, _) in SLOW {
        if stale(name) {
            spawn_self(&["_refresh", name]);
        }
    }
    items.extend(read_cached("ports"));
    // Sessions are numerous enough to swamp the list; only the newest
    // few go in, and `s:` searches the rest.
    items.extend(
        sessions.iter()
            .filter(|session| crate::sources::sessions::visible(session))
            .take(crate::sources::sessions::IN_MAIN_LIST)
            .cloned(),
    );
    items.extend(mcp.iter().cloned());
    // The `fleet` cache is deliberately *not* extended here, unlike its three
    // neighbours. It records who is running, not what they are doing, and
    // `running::live` below is what turns it into rows. Both went in, the
    // cached one first — and `finish` keeps the first of a duplicate pair, so
    // every run in the launcher showed a blank row while the live state it
    // had just computed was thrown away.

    // Pure file/CPU work — microseconds each, just run them.
    items.extend(crate::sources::project::scripts());
    items.extend(crate::sources::project::git());
    items.extend(crate::sources::history::source());
    items.extend(crate::sources::user::path_commands());
    items.extend(crate::sources::user::ssh());
    items.extend(crate::sources::user::snippets());
    items.extend(crate::sources::user::clips());
    let skills = crate::sources::agents::skills_with(&sessions);
    items.extend(skills.iter().cloned());

    // Questions agents are blocked on — the most urgent thing the launcher
    // can show, and for a long time the one unbounded reader left on this
    // path. It was described here as a directory read of a handful of small
    // files, which stopped being true the moment a fleet started leaving mail
    // for itself: `bus::all()` costs about 10 µs a message, the store has no
    // ceiling because undelivered mail is never swept, and five thousand
    // messages measured 60 ms against a budget of forty. `items()` reads the
    // bus's pending-question index instead — bounded by how many decisions are
    // outstanding — and never walks the directory, not even when that index
    // has gone missing: it asks a detached process to rebuild it, exactly as
    // `home_tasks` does below.
    items.extend(crate::bus::items());

    // …and the work behind them: what is still outstanding, plus the results
    // and failures nobody has dismissed yet. Bounded by both, rather than by
    // everything ever done, which is the whole reason a Task source can sit on
    // the gather path at all: the same store read whole costs 54 ms once a few
    // thousand finished records have accumulated, more than this entire
    // budget. `home_tasks` never walks the directory, not even when its index
    // has gone missing — it asks a detached process to rebuild it instead.
    let mut tasks = crate::task::items_as(crate::item::Kind::Task);
    attach_runs(&mut tasks, &runs);
    items.extend(tasks);

    // Identities come from the cache; what each one is *doing* is decided
    // here and now, out of syscalls. A fleet view that is a minute stale is
    // worse than none — it tells you an agent is stuck that has since moved
    // on, and vice versa.
    items.extend(runs.iter().cloned());
    items.extend(crate::sources::machine::apps());
    items.extend(crate::sources::machine::system());
    items.extend(crate::sources::agents::configs());
    items.extend(crate::sources::agents::summary(&skills, &mcp, &sessions, &runs));

    // Collected by index rather than in arrival order: `finish` keeps the
    // first of any duplicate pair, so which source got there first must not
    // decide what the list contains.
    let mut fast: Vec<Option<Vec<Item>>> = (0..FAST.len()).map(|_| None).collect();
    while fast.iter().any(Option::is_none) {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        // Missed the deadline — show the last known result rather than
        // stalling. The straggler keeps running and refreshes the cache
        // while you read the list, so the next launch is current.
        let Ok((i, v)) = rx.recv_timeout(left) else { break };
        fast[i] = Some(v);
    }
    for (i, (name, _)) in FAST.iter().enumerate() {
        match fast[i].take() {
            Some(v) => items.extend(v),
            None => items.extend(read_cached(name)),
        }
    }
    finish(items)
}

/// Give a Task the address of the Run doing it.
///
/// The Task store records which Run picked the work up, because an agent said
/// so. It cannot record where that process *lives*: a pane comes and goes
/// while the record does not, so caching one would be caching a fact with a
/// shorter life than the file holding it. The live Run list has it, and by
/// here both are in hand.
///
/// Without this the honest default for a Task is unreachable. `defaults.rs`
/// reads `pane` to decide whether to go to the agent working on something or
/// merely show its record, and a row that never carries one always answers
/// the second — for a task an agent is visibly running right now. `actions.rs`
/// gates its `zoom` row on the same key.
///
/// Built as a join rather than a scan per row: this runs on every gather, and
/// neither the fleet nor the outstanding work is bounded by anything Prelude
/// controls.
fn attach_runs(tasks: &mut [Item], runs: &[Item]) {
    if tasks.is_empty() {
        return;
    }
    let by_id: std::collections::HashMap<&str, &Item> = runs
        .iter()
        .filter(|run| !run.get("run_id").is_empty())
        .map(|run| (run.get("run_id"), run))
        .collect();
    for task in tasks {
        let run_id = task.get("run_id").to_string();
        let Some(run) = by_id.get(run_id.as_str()) else { continue };
        for key in ["pane", "addr"] {
            let value = run.get(key);
            if !value.is_empty() {
                task.data.insert(key.to_string(), value.to_string());
            }
        }
    }
}

/// The agent control centre as data. Sessions deliberately have their own
/// `sessions` command and `s:` scope; including hundreds of them here made
/// both the human overview and `prelude agents --json` session listings in
/// disguise.
pub fn gather_agents() -> Vec<Item> {
    let sessions = read_cached("sessions");
    let mcp = read_cached("mcp");
    let runs = crate::sources::running::live_with_sessions(&sessions);
    let sessions = crate::sources::running::annotate_sessions(sessions, &runs);
    write_cached_if_changed("sessions-linked", &sessions);
    let skills = crate::sources::agents::skills_with(&sessions);
    let mut items = crate::bus::items();
    let mut tasks = crate::task::items_as(crate::item::Kind::Task);
    attach_runs(&mut tasks, &runs);
    items.extend(tasks);
    items.extend(crate::sources::agents::summary(&skills, &mcp, &sessions, &runs));
    items.extend(runs);
    items.extend(skills);
    items.extend(mcp);
    items.extend(crate::sources::agents::configs());
    finish(items)
}

/// The one rule for what comes first: **kind decides the band, and learned
/// ranking only orders things inside it.**
///
/// This used to be a single number — kind priority plus a frecency bonus
/// capped at 60 — and the cap was doing a job it could not do. The whole
/// agent cluster spans 25 points (Agent 1000 down to Config 975) while the
/// bonus reached 60, so the bonus was two and a half times wider than the
/// thing it was supposed to nudge within. The bands stopped being bands: a
/// skill used twice this morning outranked `claude` itself, and a config
/// file outranked a skill. The comment on the cap even said what it was for
/// — rise within your kind, do not vault over a whole category — while the
/// arithmetic guaranteed the opposite.
///
/// Comparing the band first makes that structural instead of arithmetical.
/// No cap can be miscalibrated, no future change to a priority can silently
/// re-open the hole, and the two questions stay separate: *what kind of
/// thing is this* and *how much do you use this one*.
///
/// `score` still carries the band as a constant, which is harmless: it is
/// identical for every item being compared at the second level, so it
/// cancels. What is left there is the source's own ordering (a run's state,
/// a session's recency) plus frecency.
pub fn by_rank(a: &Item, b: &Item) -> std::cmp::Ordering {
    b.kind
        .priority()
        .cmp(&a.kind.priority())
        .then_with(|| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
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
            // Adds to whatever the source already said about where this
            // belongs inside its kind, on the same scale — see `Item::rank`.
            it.score += crate::frecency::bonus(*n, *last);
        }
        out.push(it);
    }
    // Stable, so items the frecency cap has tied keep the order their source
    // produced them in — newest session first, and so on.
    out.sort_by(by_rank);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Kind;

    fn task(run_id: &str) -> Item {
        Item::new("prelude task show t1", Kind::Task).put("run_id", run_id)
    }

    fn run(run_id: &str, pane: &str, addr: &str) -> Item {
        Item::new(format!("kill {run_id}"), Kind::Run)
            .put("run_id", run_id)
            .put("pane", pane)
            .put("addr", addr)
    }

    /// The join is what makes "go to the agent doing it" reachable at all, and
    /// it has to stay conservative: a Task whose Run has since exited must not
    /// inherit an address, or Enter sends you to a pane that is not there.
    #[test]
    fn a_task_takes_the_address_of_the_run_doing_it_and_no_other() {
        let mut tasks = vec![task("claude:42:7"), task("codex:9:1"), task("")];
        let runs = vec![run("claude:42:7", "%3", "work:1.0")];
        attach_runs(&mut tasks, &runs);

        assert_eq!(tasks[0].get("pane"), "%3", "the run doing it lends its pane");
        assert_eq!(tasks[0].get("addr"), "work:1.0");
        assert_eq!(tasks[1].get("pane"), "", "a run that has gone lends nothing");
        assert_eq!(tasks[2].get("pane"), "", "a task nobody picked up has no address");
    }

    /// A run outside tmux is a pid and no more. Copying an empty pane over
    /// would be harmless, but copying an `addr` of "pid 42" would put a row in
    /// the panel offering to jump somewhere that cannot be jumped to.
    #[test]
    fn a_run_with_no_pane_lends_nothing_to_jump_to() {
        let mut tasks = vec![task("pi:5:2")];
        attach_runs(&mut tasks, &[run("pi:5:2", "", "")]);
        assert_eq!(tasks[0].get("pane"), "");
        assert_eq!(tasks[0].get("addr"), "");
    }
}
