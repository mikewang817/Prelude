//! The fleet, outside the launcher.
//!
//! Everything `running.rs` knows — which agents are alive, which have
//! stopped and are waiting for you — was only visible after pressing the
//! hotkey. But the whole point of the waiting signal is that you do *not*
//! know to look: an agent asks a question and sits there, and the cost is
//! every minute until you notice.
//!
//! Three ways out, all built on the same `running::live()` the launcher
//! uses, so they cannot disagree with it:
//!
//!   * `prelude fleet` — the list as text, for a human or a script.
//!   * `prelude fleet --status` — one short line for a status bar or a
//!     prompt, empty when there is nothing to say.
//!   * `prelude watch` — a daemon that posts a notification the moment a
//!     run goes quiet. Silence is what a question looks like from outside
//!     the process; this is what makes the silence audible.

use crate::item::Item;
use std::time::Duration;

/// `prelude fleet`: every agent alive right now, waiting first.
///
/// An explicit invocation wants the truth now, so the identity half is
/// re-found inline (~95ms) rather than read from however old a cache.
///
/// `--json` is not a convenience — it is the form an *agent* reads. An agent
/// asked to "check on the others and take over anything stuck" needs the pid,
/// the project and the session file, not a table aligned for human eyes.
pub fn list(json: bool) -> i32 {
    crate::cache::refresh_named("fleet");
    let items = crate::sources::running::live();
    if json {
        println!("{}", serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()));
        return 0;
    }
    if items.is_empty() {
        println!("no agents running");
        return 0;
    }
    print!("{}", table(&items));
    0
}

/// `prelude fleet --status`: written to be called every few seconds by a
/// status bar, so it must never pay for subprocesses itself — cached
/// identities, live states, and a detached refresh when the cache has aged.
/// Exactly the launcher's deal.
pub fn status() -> i32 {
    if crate::cache::stale("fleet") {
        crate::cache::spawn_self(&["_refresh", "fleet"]);
    }
    let items = crate::sources::running::live();
    let line = status_line(
        crate::bus::pending().len(),
        count(&items, "waiting"),
        count(&items, "working"),
    );
    if !line.is_empty() {
        println!("{line}");
    }
    0
}

fn count(items: &[Item], state: &str) -> usize {
    items.iter().filter(|i| i.get("state") == state).count()
}

/// The status-bar text. Words rather than glyphs — a `⚑` is only legible to
/// someone who already knows what it means — and nothing at all when there
/// is nothing to say, so an idle machine has an empty segment instead of a
/// permanent `0 waiting`.
///
/// The three counts are in descending order of how much they are your
/// problem. An agent that has *asked* something is blocked on you by name;
/// one that has merely gone quiet might be; one that is working is neither.
pub fn status_line(asking: usize, waiting: usize, working: usize) -> String {
    let mut parts = Vec::new();
    if asking > 0 {
        parts.push(format!("{asking} asking"));
    }
    if waiting > 0 {
        parts.push(format!("{waiting} waiting"));
    }
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    parts.join(" · ")
}

/// Should a change of state produce a notification?
///
/// Only the edge into waiting: a run first seen already quiet counts,
/// because starting the watcher late is not a reason to miss the one agent
/// that has been stuck all along. A run that stays waiting has already been
/// announced, and a run going back to work needs no announcement at all.
pub fn should_notify(prev: Option<&str>, cur: &str) -> bool {
    cur == "waiting" && prev != Some("waiting")
}

const TICK: Duration = Duration::from_secs(5);
/// Re-find the fleet every third tick: a new agent shows up within 15s,
/// while the expensive ps+lsof pass stays at a fraction of a percent of a
/// core. State, the cheap half, is re-read every tick.
const REFIND_EVERY: u32 = 3;

/// `prelude watch`: the daemon. Loops forever; end it with ^C or kill.
pub fn watch() -> i32 {
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut tick: u32 = 0;
    eprintln!("prelude: watching — you will be notified when an agent stops and waits");
    loop {
        if tick.is_multiple_of(REFIND_EVERY) {
            crate::cache::refresh_named("fleet");
        }
        tick = tick.wrapping_add(1);
        let items = crate::sources::running::live();
        let mut now: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for it in &items {
            let pid = it.get("pid").to_string();
            let state = it.get("state").to_string();
            if should_notify(seen.get(&pid).map(String::as_str), &state) {
                notify(it);
            }
            now.insert(pid, state);
        }
        // Wholesale replacement: a pid that has gone is forgotten, so the
        // same pid number reused later is a fresh run, not a suppressed one.
        seen = now;
        std::thread::sleep(TICK);
    }
}

/// Tell the human that a run has gone quiet, and say what it last said.
///
/// The subject line alone ("claude · api-gateway stopped") makes you go and
/// look before you can decide whether it matters — which is most of the cost
/// this is supposed to remove. The answer is already on disk: the last thing
/// the agent said is the last message in its conversation file, so the
/// notification carries the actual question and the decision can be made from
/// the banner.
fn notify(it: &Item) {
    let head = format!("{} · {}", it.get("agent"), it.get("project"));
    let body = last_said(it);
    crate::bus::post(&head, &body);
    println!("{head} — {body}");
}

/// The last thing this run said, falling back to the conversation's subject
/// and then to a bare statement of fact.
fn last_said(it: &Item) -> String {
    let tail = crate::sources::running::transcript_tail(it.get("session"), 1);
    let said = tail
        .first()
        .map(|l| l.trim_start_matches(['⏺', '›', ' ']).trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    match said.or_else(|| Some(it.get("subject").to_string()).filter(|s| !s.is_empty())) {
        Some(s) => crate::width::dtrunc(&crate::width::flatten(&s), 140),
        None => "stopped and is waiting for you".into(),
    }
}

/// The fleet as a plain table, one run per line, waiting sorted first —
/// `live()` already ranks them, so the order here is the launcher's order.
fn table(items: &[Item]) -> String {
    let mut rows: Vec<[String; 5]> = items
        .iter()
        .map(|it| {
            [
                it.get("agent").to_string(),
                it.fields.get(1).cloned().unwrap_or_default(),
                it.get("project").to_string(),
                it.get("addr").to_string(),
                crate::width::dtrunc(it.get("subject"), 60),
            ]
        })
        .collect();
    rows.sort_by_key(|r| r[1] != *"waiting" && !r[1].starts_with("waiting"));
    let mut w = [0usize; 4];
    for r in &rows {
        for (i, wi) in w.iter_mut().enumerate() {
            *wi = (*wi).max(crate::width::dwidth(&r[i]));
        }
    }
    let mut out = String::new();
    for r in &rows {
        for (i, wi) in w.iter().enumerate() {
            out.push_str(&r[i]);
            out.push_str(&" ".repeat(wi.saturating_sub(crate::width::dwidth(&r[i])) + 2));
        }
        out.push_str(r[4].trim());
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    out
}
