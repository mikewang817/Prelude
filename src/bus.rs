//! The message bus: how an agent reaches a human, and how agents reach each
//! other.
//!
//! An agent running in a terminal is a sealed box. It cannot see the other
//! agents on the machine, it cannot talk to them, and the only way it can
//! reach the person who started it is by printing into its own window — which
//! that person may not be looking at. When it needs a decision it stops and
//! waits, and the waiting is invisible. That is the "waiting 12m" the fleet
//! view already detects *from the outside*, and detecting it is only half an
//! answer: the person still has to notice the badge, go there, and read back
//! far enough to see what was asked.
//!
//! This is the other half. Four verbs, all of them things an agent can run
//! from its own shell:
//!
//! ```text
//! prelude ask  "the migration drops legacy_users. proceed?"   ← blocks
//! prelude tell "deploy is green"                              ← one-way
//! prelude say  api-gateway "I changed the auth schema"        ← agent → agent
//! prelude inbox                                               ← what came in
//! ```
//!
//! `ask` is the one that changes what an agent can be. It writes the question
//! where the launcher can see it, notifies the human wherever they are, and
//! *blocks on stdout* until an answer comes back — so the agent can be
//! written as though a person were sitting next to it. The person answers
//! from anywhere: the launcher's top row, `prelude reply` in any terminal,
//! or by hand with `prelude answer <id>`.
//!
//! **Identity is discovered, never declared.** An agent should not have to be
//! told who it is or configured with an address. `$TMUX_PANE` gives a reply
//! address for free in any pane, `$PWD` gives the project, and walking the
//! process tree finds which agent binary we are running under. So the whole
//! interface really is just the four verbs above.
//!
//! **Storage is one JSON file per message**, under the data directory rather
//! than the cache: an unanswered question is not something to be dropped when
//! a cache is cleared. Writes go through `write_atomic`, so a reader never
//! sees half a message, and no daemon owns the bus — every operation is a
//! directory read and a file write, which is what makes it work identically
//! from a launcher, a shell, and an agent's tool call.

use crate::item::{Item, Kind};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How long `ask` waits by default before giving up.
///
/// An agent's own tool call usually has a shorter deadline than this, so the
/// number that matters is the one the caller passes. This is the backstop for
/// when nobody passed one: long enough that stepping away for a coffee does
/// not lose the answer, short enough that a forgotten `ask` in a script does
/// not wedge forever.
pub const DEFAULT_WAIT: u64 = 600;

/// How often a blocked `ask` looks for its answer. Fast enough to feel
/// instant to the person who just typed it, cheap enough to be free.
const POLL: Duration = Duration::from_millis(250);

/// Answered messages are kept this long so `inbox --all` can show what was
/// decided; unanswered ones are never swept.
const KEEP_ANSWERED: u64 = 24 * 3600;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Msg {
    pub id: String,
    /// `ask` (wants an answer), `tell` (a notice), `say` (agent to agent).
    pub kind: String,
    /// Which agent sent it, or `shell` when a person ran it by hand.
    pub from: String,
    #[serde(default)]
    pub from_project: String,
    #[serde(default)]
    pub from_cwd: String,
    /// Where to type a reply back, when the sender was in a pane.
    #[serde(default)]
    pub from_pane: String,
    /// `human`, or the label of the agent this was addressed to.
    pub to: String,
    #[serde(default)]
    pub to_pane: String,
    #[serde(default)]
    pub to_cwd: String,
    pub text: String,
    pub ts: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered: Option<u64>,
}

impl Msg {
    pub fn pending(&self) -> bool {
        self.kind == "ask" && self.answer.is_none()
    }
    /// `claude · api-gateway`, or just the agent when there is no project.
    pub fn label(&self) -> String {
        match (self.from.as_str(), self.from_project.as_str()) {
            (a, "") => a.to_string(),
            (a, p) => format!("{a} · {p}"),
        }
    }
    pub fn age(&self) -> u64 {
        now().saturating_sub(self.ts)
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn dir() -> std::path::PathBuf {
    crate::paths::data().join("bus")
}

fn file_of(id: &str) -> std::path::PathBuf {
    dir().join(format!("{id}.json"))
}

/// Every message on the bus, oldest first.
///
/// A file that will not parse is skipped rather than fatal: the bus is read
/// on every gather, and one bad write must never take the launcher down.
pub fn all() -> Vec<Msg> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut out: Vec<Msg> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Msg>(&t).ok())
        .collect();
    out.sort_by_key(|m| m.ts);
    out
}

pub fn get(id: &str) -> Option<Msg> {
    let t = std::fs::read_to_string(file_of(id)).ok()?;
    serde_json::from_str(&t).ok()
}

pub fn save(m: &Msg) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(m)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    crate::cache::write_atomic(&file_of(&m.id), &json)
}

/// Drop answered messages once they are old enough to be history rather than
/// context. An unanswered question is never swept — a question nobody got to
/// is exactly the thing that must not disappear quietly.
fn sweep() {
    let cutoff = now().saturating_sub(KEEP_ANSWERED);
    for m in all() {
        if m.answer.is_some() && m.ts < cutoff {
            let _ = std::fs::remove_file(file_of(&m.id));
        }
    }
}

// ---------------------------------------------------------------------------
// Who is calling
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Who {
    /// The agent binary we are running underneath, or empty at a bare shell.
    pub agent: String,
    pub pane: String,
    pub cwd: String,
    pub project: String,
}

/// Work out who is running this command, without being told.
///
/// The three signals are all free. `$TMUX_PANE` is exported into every pane,
/// so an agent calling us from one hands over a reply address without knowing
/// it did. `$PWD` names the project. And the process tree says which agent we
/// are underneath — an agent's tool call runs `sh -c "prelude …"`, so the
/// agent is our grandparent rather than our parent, and the walk has to climb
/// rather than look once.
pub fn whoami() -> Who {
    let cwd = crate::paths::cwd()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let project = cwd.rsplit('/').next().unwrap_or("").to_string();
    Who {
        agent: enclosing_agent().unwrap_or_default(),
        pane: std::env::var("TMUX_PANE").unwrap_or_default(),
        cwd,
        project,
    }
}

/// Climb the process tree looking for an agent binary.
///
/// One bulk `ps` and an in-memory walk: asking `ps` once per level would be
/// six subprocesses for an answer that is already in the first one's output.
fn enclosing_agent() -> Option<String> {
    let out = crate::exec::run(&["ps", "-Ao", "pid=,ppid=,comm="], Duration::from_secs(5));
    let mut parent: std::collections::HashMap<u32, (u32, String)> = std::collections::HashMap::new();
    for line in out.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (it.next(), it.next()) else { continue };
        let comm = it.collect::<Vec<_>>().join(" ");
        let base = comm.rsplit('/').next().unwrap_or(&comm).to_string();
        if let (Ok(p), Ok(pp)) = (pid.parse(), ppid.parse()) {
            parent.insert(p, (pp, base));
        }
    }
    let names: Vec<&str> = crate::sources::sessions::AGENTS.iter().map(|a| a.name).collect();
    let mut pid = std::process::id();
    // Bounded: a broken tree with a cycle must not spin forever, and no real
    // agent is more than a handful of processes above its own tool call.
    for _ in 0..12 {
        let (pp, comm) = parent.get(&pid)?;
        if let Some(n) = names.iter().find(|n| *n == comm) {
            return Some((*n).to_string());
        }
        if *pp <= 1 {
            return None;
        }
        pid = *pp;
    }
    None
}

// ---------------------------------------------------------------------------
// The verbs
// ---------------------------------------------------------------------------

/// `prelude ask "…"` — put a question to the human and wait for the answer.
///
/// stdout carries the answer and nothing else, so `ans=$(prelude ask …)` is
/// the whole integration. Everything else goes to stderr. Exit 0 when
/// answered, 3 when it timed out — distinct, so a script can tell "they said
/// no" from "nobody was there".
pub fn ask(text: &str, wait: u64, no_wait: bool) -> i32 {
    let me = whoami();
    let m = Msg {
        id: format!("{}-{}", now(), std::process::id()),
        kind: "ask".into(),
        from: if me.agent.is_empty() { "shell".into() } else { me.agent.clone() },
        from_project: me.project.clone(),
        from_cwd: me.cwd.clone(),
        from_pane: me.pane.clone(),
        to: "human".into(),
        text: text.into(),
        ts: now(),
        ..Default::default()
    };
    if let Err(e) = save(&m) {
        eprintln!("prelude: could not post the question: {e}");
        return 2;
    }
    sweep();
    post(&format!("{} asks", m.label()), text);
    // Nudge every attached tmux client so a status bar showing the count
    // updates now rather than at its next poll.
    crate::exec::run(&["tmux", "refresh-client", "-S"], Duration::from_secs(1));

    if no_wait {
        eprintln!("prelude: asked — collect the answer with  prelude answer-of {}", m.id);
        println!("{}", m.id);
        return 0;
    }

    eprintln!("prelude: waiting for an answer ({}s)…", wait);
    let deadline = std::time::Instant::now() + Duration::from_secs(wait);
    while std::time::Instant::now() < deadline {
        if let Some(cur) = get(&m.id) {
            if let Some(a) = cur.answer {
                println!("{a}");
                return 0;
            }
        }
        std::thread::sleep(POLL);
    }
    eprintln!(
        "prelude: no answer within {wait}s — it is still in the inbox as {}",
        m.id
    );
    3
}

/// `prelude answer-of <id>` — collect a `--no-wait` answer later.
pub fn answer_of(id: &str) -> i32 {
    match get(id) {
        None => {
            eprintln!("prelude: no message {id}");
            2
        }
        Some(m) => match m.answer {
            Some(a) => {
                println!("{a}");
                0
            }
            None => 3,
        },
    }
}

/// `prelude tell "…"` — a notice, with nothing to wait for.
pub fn tell(text: &str) -> i32 {
    let me = whoami();
    let m = Msg {
        id: format!("{}-{}", now(), std::process::id()),
        kind: "tell".into(),
        from: if me.agent.is_empty() { "shell".into() } else { me.agent.clone() },
        from_project: me.project.clone(),
        from_cwd: me.cwd,
        from_pane: me.pane,
        to: "human".into(),
        text: text.into(),
        ts: now(),
        // A notice needs no reply, so it is born answered: it shows in the
        // inbox as history rather than sitting there demanding attention.
        answer: Some(String::new()),
        answered: Some(now()),
        ..Default::default()
    };
    let _ = save(&m);
    post(&m.label(), text);
    0
}

/// `prelude say <target> <text>` — one agent to another.
///
/// Delivery is direct when the target has a pane: the text is typed into it
/// and submitted, because unlike a person at a keyboard the sender here is
/// deliberately sending a message and there is nobody to press Enter for it.
/// An agent with no pane cannot be typed into at all, so the message waits in
/// its inbox for the next time it looks.
pub fn say(target: &str, text: &str) -> i32 {
    if target == "human" {
        return tell(text);
    }
    let runs = crate::sources::running::live();
    let hits = resolve(&runs, target);
    let hit = match hits.len() {
        0 => {
            eprintln!("prelude: nothing running matches {target:?}");
            eprintln!("prelude: try one of:");
            for r in runs.iter().take(12) {
                eprintln!("  {} · {}  ({})", r.get("agent"), r.get("project"), r.get("addr"));
            }
            return 2;
        }
        1 => hits[0].clone(),
        _ => {
            // Guessing between two agents is worse than refusing: the message
            // would land in the wrong conversation and read as the human's.
            eprintln!("prelude: {target:?} matches more than one — be specific:");
            for r in &hits {
                eprintln!("  {} · {}  ({})", r.get("agent"), r.get("project"), r.get("addr"));
            }
            return 2;
        }
    };

    let me = whoami();
    let from = if me.agent.is_empty() { "you".to_string() } else { me.label_of() };
    let mut m = Msg {
        id: format!("{}-{}", now(), std::process::id()),
        kind: "say".into(),
        from: if me.agent.is_empty() { "shell".into() } else { me.agent.clone() },
        from_project: me.project.clone(),
        from_cwd: me.cwd.clone(),
        from_pane: me.pane.clone(),
        to: format!("{} · {}", hit.get("agent"), hit.get("project")),
        to_pane: hit.get("pane").to_string(),
        to_cwd: hit.get("cwd").to_string(),
        text: text.into(),
        ts: now(),
        ..Default::default()
    };

    let pane = hit.get("pane");
    if pane.is_empty() {
        let _ = save(&m);
        eprintln!(
            "prelude: {} is not in a tmux pane — left in its inbox instead",
            m.to
        );
        return 0;
    }
    // Attributed, so the receiving agent knows this came from a peer rather
    // than from the person it is talking to. Without that it will answer the
    // human's own conversation with something nobody asked for.
    let line = format!("[via prelude, from {from}] {text}");
    let d = Duration::from_secs(5);
    crate::exec::run(&["tmux", "send-keys", "-t", pane, "-l", &line], d);
    crate::exec::run(&["tmux", "send-keys", "-t", pane, "Enter"], d);
    m.answer = Some(String::new());
    m.answered = Some(now());
    let _ = save(&m);
    eprintln!("prelude: delivered to {} ({})", m.to, hit.get("addr"));
    0
}

impl Who {
    fn label_of(&self) -> String {
        match (self.agent.as_str(), self.project.as_str()) {
            ("", p) => p.to_string(),
            (a, "") => a.to_string(),
            (a, p) => format!("{a} · {p}"),
        }
    }
}

/// Match a free-text target against the running fleet.
///
/// Deliberately several ways at once — an agent writing `prelude say
/// api-gateway …` should not have to know whether that is a project, a pane
/// or a session name. Exact matches win outright so that a project literally
/// called `claude` cannot be shadowed by every claude on the machine.
pub fn resolve(runs: &[Item], target: &str) -> Vec<Item> {
    let t = target.trim().to_lowercase();
    let exact: Vec<Item> = runs
        .iter()
        .filter(|r| {
            r.get("addr").eq_ignore_ascii_case(&t)
                || r.get("pane").eq_ignore_ascii_case(&t)
                || r.get("project").eq_ignore_ascii_case(&t)
                || r.get("pid") == t
        })
        .cloned()
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    runs.iter()
        .filter(|r| {
            r.get("agent").eq_ignore_ascii_case(&t)
                || r.get("project").to_lowercase().contains(&t)
                || r.get("cwd").to_lowercase().contains(&t)
        })
        .cloned()
        .collect()
}

/// `prelude answer <id> <text>` — the return path.
pub fn answer(id: &str, text: &str) -> i32 {
    let Some(mut m) = get(id) else {
        eprintln!("prelude: no message {id}");
        return 2;
    };
    if m.answer.is_some() {
        eprintln!("prelude: {id} was already answered");
        return 2;
    }
    m.answer = Some(text.to_string());
    m.answered = Some(now());
    if let Err(e) = save(&m) {
        eprintln!("prelude: could not save the answer: {e}");
        return 2;
    }
    crate::exec::run(&["tmux", "refresh-client", "-S"], Duration::from_secs(1));
    eprintln!("prelude: answered {} — {}", m.label(), m.id);
    0
}

/// Questions still waiting on a human, oldest first.
pub fn pending() -> Vec<Msg> {
    all().into_iter().filter(Msg::pending).collect()
}

/// `prelude reply` — answer the oldest waiting question from any terminal.
pub fn reply() -> i32 {
    let p = pending();
    let Some(m) = p.first() else {
        eprintln!("prelude: nothing is waiting on you");
        return 0;
    };
    eprintln!("{} asks:", m.label());
    eprintln!("  {}", m.text);
    let Some(line) = crate::ui::prompt_line(&format!(" answer {} ", m.label())) else {
        return 130;
    };
    answer(&m.id, &line)
}

/// `prelude inbox` — what has come in for whoever is asking.
///
/// From an agent this is how it collects messages left by peers that could
/// not be typed into it. From a person it is the list of questions waiting on
/// them. Which of the two you get is decided by `whoami` rather than by a
/// flag, because an agent should not have to know which it is.
///
/// `--human` overrides that. A person working *inside* an agent's terminal
/// is still a person, and the heuristic — correctly — reads their shell as
/// the agent's. Without the override the questions would be unreachable from
/// exactly the window they are most likely to be sitting in.
pub fn inbox(json: bool, all_of_them: bool, as_human: bool) -> i32 {
    let me = whoami();
    let mine: Vec<Msg> = all()
        .into_iter()
        .filter(|m| {
            if as_human || me.agent.is_empty() {
                // A person: everything addressed to a human.
                m.to == "human" && (all_of_them || m.pending())
            } else {
                // An agent: what was left for this pane or this project, and
                // not yet picked up.
                let for_me = (!me.pane.is_empty() && m.to_pane == me.pane)
                    || (!me.cwd.is_empty() && m.to_cwd == me.cwd);
                for_me && m.kind == "say" && (all_of_them || m.answer.is_none())
            }
        })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&mine).unwrap_or_else(|_| "[]".into()));
        return 0;
    }
    if mine.is_empty() {
        println!("nothing waiting");
        return 0;
    }
    for m in &mine {
        let mark = if m.pending() { "?" } else { "·" };
        println!("{mark} {}  {}  [{}]", m.label(), m.text, m.id);
        if let Some(a) = m.answer.as_ref().filter(|a| !a.is_empty()) {
            println!("  → {a}");
        }
    }
    0
}

/// Mark agent-directed messages as collected, so an agent polling its inbox
/// does not act on the same instruction twice.
pub fn drain() -> i32 {
    let me = whoami();
    let mut n = 0;
    for mut m in all() {
        let for_me = (!me.pane.is_empty() && m.to_pane == me.pane)
            || (!me.cwd.is_empty() && m.to_cwd == me.cwd);
        if for_me && m.kind == "say" && m.answer.is_none() {
            m.answer = Some(String::new());
            m.answered = Some(now());
            let _ = save(&m);
            n += 1;
        }
    }
    eprintln!("prelude: collected {n}");
    0
}

// ---------------------------------------------------------------------------
// Reaching the human
// ---------------------------------------------------------------------------

/// One line through the notification centre.
///
/// `display notification` is fire-and-forget and needs no permission prompt,
/// which is why it wins over anything with a reply button: a notification
/// that requires the user to have installed something is a notification that
/// does not arrive.
pub fn post(title: &str, body: &str) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape(&crate::width::dtrunc(body, 200)),
        escape(title)
    );
    crate::exec::run(&["osascript", "-e", &script], Duration::from_secs(5));
}

/// Into an AppleScript string literal. Quotes and backslashes are the whole
/// grammar; everything else passes through.
pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------------------
// In the launcher
// ---------------------------------------------------------------------------

/// Waiting questions, as rows.
///
/// These outrank everything else on the machine, including a stuck agent: a
/// run that has gone quiet *might* want you, while one of these has said so.
pub fn items() -> Vec<Item> {
    pending()
        .into_iter()
        .map(|m| {
            let age = crate::sources::running::short_dur(m.age());
            Item::new(m.text.clone(), Kind::Msg)
                .title(format!("{} asks", m.label()))
                .fields([
                    m.from_project.clone(),
                    format!("asked {age} ago"),
                    crate::width::flatten(&m.text),
                ])
                .put("id", m.id.clone())
                .put("agent", m.from.clone())
                .put("text", m.text.clone())
                .put("project", m.from_project.clone())
                .put("pane", m.from_pane.clone())
                .put("cwd", m.from_cwd.clone())
                .put("path", m.from_cwd.clone())
        })
        .collect()
}
