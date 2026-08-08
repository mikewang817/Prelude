//! Every agent alive on this machine, wherever it is running.
//!
//! `sessions.rs` reads conversation files: what you talked about, whenever
//! that was. This reads the machine: what is alive at this second, and — the
//! only question that matters once there are eighty of them — which ones have
//! stopped and are waiting for you.
//!
//! **The backbone is the process list, not tmux.** An agent in a terminal tab
//! is no less running than one in a pane, and a fleet view that sees half the
//! fleet is worse than none. tmux is an enhancement: a run that has a pane
//! also gains an address you can jump to and type into.
//!
//! The signal for "is it stuck" is silence, and every agent emits it twice
//! over. A pane's `#{window_activity}` moves whenever the TUI redraws. And
//! every agent appends to its session file as it works — a tool call, a
//! message — while writing nothing at all as it waits for you. The second
//! clock needs no terminal of any kind, which is why it is the one that
//! generalises.
//!
//! Cost splits the work in two. *Finding* the fleet is ~95ms — `ps` with full
//! command lines, one bulk `lsof` for their directories, tmux — and lives in
//! the cache tier. Deciding what each one is *doing* is then a `stat` and a
//! `kill(pid, 0)` per row: syscalls, not subprocesses. So that half runs live
//! on every gather, and the state you read is the state now rather than the
//! state when the cache was written.

use crate::item::{Item, Kind};
use std::time::Duration;

/// How long an agent has to go quiet before it is probably waiting on you.
///
/// Long enough to sit out a slow tool call, short enough that a blocked run
/// surfaces while you still care.
const QUIET: u64 = 30;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Working,
    Waiting,
    Dead,
}

/// What the last entry in a conversation was, which is the difference between
/// "quiet because it is thinking" and "quiet because it asked you something".
///
/// Silence alone cannot tell them apart, and getting it wrong is expensive in
/// one direction: a run reported as waiting that is really three minutes into
/// a build teaches you to distrust the badge, and a badge you distrust is
/// worth less than no badge. Every agent's file says which it is —
/// an assistant turn that ends in a tool call is *acting*; one that ends in
/// prose has finished and handed back.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Turn {
    /// The last thing it did was speak, and then stop.
    Spoke,
    /// It is mid-turn: running a tool, or working on what it was just told.
    Acting,
}

/// Dead, mid-turn, quiet, or talking. The whole state machine.
///
/// `turn` is what the conversation file says; `None` when there is no file to
/// read, which is when silence is all there is to go on.
pub fn classify(dead: bool, silent: u64, turn: Option<Turn>) -> State {
    if dead {
        return State::Dead;
    }
    // Mid-turn beats any clock. A tool call that takes ten minutes is not a
    // question, however long it stays quiet.
    if turn == Some(Turn::Acting) {
        return State::Working;
    }
    if silent >= QUIET {
        State::Waiting
    } else {
        State::Working
    }
}

/// How much of the tail of a conversation to read.
///
/// One tool result holding a large file can be tens of kilobytes on its own,
/// so a small window would miss the last complete line entirely. This is
/// generous enough to hold the last few entries of any real conversation and
/// still cheap: a seek and one read per agent, no parsing of the rest.
const TAIL_BYTES: u64 = 64 * 1024;

/// Read the end of a conversation and say whether the agent is mid-turn.
///
/// Only the last *message* matters, so entries that carry no message —
/// attachments, summaries, meta rows — are skipped rather than counted.
pub fn last_turn(path: &str) -> Option<Turn> {
    use std::io::{Read, Seek, SeekFrom};
    if path.is_empty() {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::with_capacity(TAIL_BYTES as usize);
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);

    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(msg) = v.get("message") else { continue };
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role == "user" {
            // Either a tool result coming back or a person having just typed.
            // Both mean the agent has something to get on with.
            return Some(Turn::Acting);
        }
        if role != "assistant" {
            continue;
        }
        // An assistant turn that ends in a tool call is still going.
        let acting = msg
            .get("content")
            .and_then(|c| c.as_array())
            .is_some_and(|parts| {
                parts.iter().any(|p| p.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
            });
        return Some(if acting { Turn::Acting } else { Turn::Spoke });
    }
    None
}

impl State {
    fn label(self, silent: u64) -> String {
        match self {
            State::Working => "working".into(),
            State::Waiting => format!("waiting {}", short_dur(silent)),
            State::Dead => "exited".into(),
        }
    }
    /// Waiting sorts first: those are the ones holding you up.
    fn rank(self) -> i64 {
        match self {
            State::Waiting => 2,
            State::Working => 1,
            State::Dead => 0,
        }
    }
    fn key(self) -> &'static str {
        match self {
            State::Working => "working",
            State::Waiting => "waiting",
            State::Dead => "dead",
        }
    }
}

pub fn short_dur(secs: u64) -> String {
    match secs {
        s if s < 90 => format!("{s}s"),
        s if s < 5400 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

fn is_agent(cmd: &str) -> Option<&'static str> {
    let base = cmd.rsplit('/').next().unwrap_or(cmd);
    super::sessions::AGENTS.iter().map(|a| a.name).find(|n| *n == base)
}

/// Subcommands that are tooling rather than a conversation.
///
/// Every agent binary is also its own admin CLI, and those invocations are
/// not runs: nobody is talking to `claude mcp list`, it has no session and no
/// pane worth jumping to, and it exits in under a second.
///
/// This is not hypothetical — Prelude asks each agent for its MCP status on
/// every refresh, so without this filter the launcher listed *its own probes*
/// as a dozen phantom agents in the project it was launched from. Anything
/// that reports the fleet has to not be part of it.
const NOT_A_CONVERSATION: &[&str] = &[
    "mcp", "config", "doctor", "update", "install", "uninstall", "migrate-installer",
    "setup-token", "login", "logout", "auth", "completion", "upgrade", "help", "version",
];

/// Is this argument list a conversation, or the binary being used as a tool?
///
/// Judged on the first bare word only. Flags are not subcommands, so
/// `claude --resume abc` is a conversation, and `codex exec …` is a batch run
/// that is reported (and marked) rather than dropped.
pub fn is_conversation(args: &[&str]) -> bool {
    match args.iter().find(|w| !w.starts_with('-')) {
        None => true,
        Some(first) => !NOT_A_CONVERSATION.contains(&first.to_lowercase().as_str()),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Is this process still there? Signal 0 checks permission to signal and
/// delivers nothing, which is the standard way to ask.
fn alive(pid: &str) -> bool {
    unsafe extern "C" {
        unsafe fn kill(pid: i32, sig: i32) -> i32;
    }
    match pid.parse::<i32>() {
        Ok(p) if p > 0 => (unsafe { kill(p, 0) }) == 0,
        _ => false,
    }
}

fn mtime_of(path: &str) -> Option<u64> {
    if path.is_empty() {
        return None;
    }
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// (pane id, pid of the process rooted in it, address, last activity).
///
/// One subprocess for every pane on the machine — the fields are asked for by
/// name, so a hundred panes cost what one does.
fn panes() -> Vec<(String, String, String, u64)> {
    const F: &str = "#{pane_id}\u{1f}#{pane_pid}\u{1f}#{window_activity}\u{1f}\
                     #{session_name}:#{window_index}.#{pane_index}";
    let out = crate::exec::run(&["tmux", "list-panes", "-a", "-F", F], Duration::from_secs(2));
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\u{1f}').collect();
            if f.len() < 4 {
                return None;
            }
            Some((f[0].into(), f[1].into(), f[3].into(), f[2].parse().unwrap_or(0)))
        })
        .collect()
}

/// Find the fleet. Expensive, and therefore cached.
///
/// Records identity only — which agent, which pid, which directory, which
/// pane if any, which conversation file. Nothing here says what a run is
/// *doing*: that is decided live, because a state read out of a cache is a
/// state that was true some minutes ago.
pub fn fleet() -> Vec<Item> {
    let ps = crate::exec::run(
        &["ps", "-Ao", "pid=,ppid=,etime=,command="],
        Duration::from_secs(5),
    );
    let mut found: Vec<(String, String, &'static str, String, bool)> = Vec::new();
    for line in ps.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid), Some(etime)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let rest: Vec<&str> = it.collect();
        let Some(agent) = rest.first().and_then(|c| is_agent(c)) else { continue };
        // `claude mcp list` is not a run, and Prelude starts one of those
        // itself on every refresh.
        if !is_conversation(&rest[1..]) {
            continue;
        }
        // `claude -p …` and `codex exec …` print an answer and exit. Nothing
        // to jump to and an exit code to care about, so they are marked
        // rather than mixed in.
        let batch = rest.iter().any(|w| *w == "-p" || *w == "--print" || *w == "exec");
        found.push((pid.into(), ppid.into(), agent, etime.into(), batch));
    }
    if found.is_empty() {
        return Vec::new();
    }
    let cwds = cwd_of_agents();
    // A pane reports the pid of its *root* process — the shell you typed
    // `claude` into — while ps reports claude itself. So a run is matched to
    // a pane by its own pid or its parent's; matching pids alone finds none
    // of the agents anybody actually starts.
    let panes = panes();
    let sessions = crate::cache::read_cached("sessions");

    found
        .into_iter()
        .map(|(pid, ppid, agent, etime, batch)| {
            let cwd = cwds.get(&pid).cloned().unwrap_or_default();
            let pane = panes
                .iter()
                .find(|(_, root, ..)| *root == pid || *root == ppid);
            // The conversation this run is most likely writing: same agent,
            // same directory, most recently touched. Two agents in one
            // directory cannot be told apart this way, and are not.
            let session = sessions
                .iter()
                .filter(|s| !cwd.is_empty() && s.get("agent") == agent && s.get("cwd") == cwd)
                .max_by(|a, b| {
                    let x = a.get("ts").parse::<f64>().unwrap_or(0.0);
                    let y = b.get("ts").parse::<f64>().unwrap_or(0.0);
                    x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
                });
            let mut it = Item::new(format!("kill {pid}"), Kind::Run)
                .title(agent)
                .put("agent", agent)
                .put("cwd", cwd.clone())
                .put("path", cwd)
                .put("pid", pid)
                .put("etime", etime);
            if batch {
                it = it.put("batch", "1");
            }
            if let Some((pane_id, _, addr, _)) = pane {
                it = it.put("pane", pane_id).put("addr", addr);
            }
            if let Some(s) = session {
                it = it.put("session", s.get("file")).put("subject", s.title.clone());
            }
            it
        })
        .collect()
}

/// Working directories of every agent process, in one call.
///
/// `lsof` is the only way to read another process's cwd on macOS, and costs
/// ~60ms however it is asked — so it is asked once, for every agent name at
/// once, rather than once per pid.
fn cwd_of_agents() -> std::collections::HashMap<String, String> {
    let mut argv: Vec<&str> = vec!["lsof", "-a", "-d", "cwd", "-Fpn"];
    for a in super::sessions::AGENTS {
        argv.push("-c");
        argv.push(a.name);
    }
    let out = crate::exec::run(&argv, Duration::from_secs(5));
    let mut map = std::collections::HashMap::new();
    let mut pid = String::new();
    // -F output is one field per line, tagged by its first character.
    for line in out.lines() {
        match line.as_bytes().first() {
            Some(b'p') => pid = line[1..].to_string(),
            Some(b'n') if !pid.is_empty() => {
                map.insert(std::mem::take(&mut pid), line[1..].to_string());
            }
            _ => {}
        }
    }
    map
}

/// The fleet as it stands *now*: cached identities, live state.
///
/// Every read here is a syscall, so a hundred runs cost well under a
/// millisecond. A run whose process has gone is dropped rather than shown.
pub fn live() -> Vec<Item> {
    let now = now();
    // Which runs still exist is a `kill(pid, 0)` each and settles the common
    // case — nothing running — before tmux is asked anything. `list-panes` is
    // a subprocess, and spawning one to decorate an empty list is the whole
    // cost of this source on a machine with no agents on it.
    let runs: Vec<Item> = crate::cache::read_cached("fleet")
        .into_iter()
        .filter(|it| alive(it.get("pid")))
        .collect();
    if runs.is_empty() {
        return Vec::new();
    }
    let panes = panes();
    runs.into_iter()
        .map(|mut it| {
            // Two clocks, one meaning. The pane's is the more direct when
            // there is one; the session file is the one that exists
            // everywhere. Whichever moved last is what this run last did.
            let by_pane = panes
                .iter()
                .find(|(id, ..)| id == it.get("pane"))
                .map(|(.., act)| *act);
            let by_file = mtime_of(it.get("session"));
            let last = by_pane.into_iter().chain(by_file).max();
            let silent = last.map(|t| now.saturating_sub(t)).unwrap_or(0);
            // A batch run writes to a pipe and keeps no conversation file, so
            // silence tells you nothing about it. It is reported as working:
            // it is doing something, there is simply no way to watch.
            let state = if last.is_none() || it.get("batch") == "1" {
                State::Working
            } else {
                // The conversation says what the clock cannot: whether this
                // run is quiet because it is thinking or quiet because it
                // asked you something and stopped.
                //
                // Only asked when the clock is about to say "waiting", which
                // is the only answer it can change — a run that moved a
                // second ago is working whatever its last entry was. That
                // keeps the cost proportional to the number of *quiet* runs
                // rather than to the size of the fleet: eighty busy agents
                // cost nothing, and reading 64KB per row on every keystroke
                // would not have fit the budget.
                let turn = (silent >= QUIET).then(|| last_turn(it.get("session"))).flatten();
                classify(false, silent, turn)
            };
            let cwd = it.get("cwd").to_string();
            let project = match cwd.rsplit('/').next().unwrap_or("") {
                "" => crate::paths::tilde(&cwd),
                p => p.to_string(),
            };
            let addr = match it.get("addr") {
                "" => format!("pid {}", it.get("pid")),
                a => a.to_string(),
            };
            let subject = it.get("subject").to_string();
            it.fields = vec![project.clone(), state.label(silent), addr.clone(), subject];
            it.cmd = if it.get("pane").is_empty() {
                format!("kill {}", it.get("pid"))
            } else {
                format!("tmux switch-client -t {addr}")
            };
            it.data.insert("project".into(), project);
            it.data.insert("addr".into(), addr);
            it.data.insert("state".into(), state.key().into());
            it.score = Kind::Run.priority() as f64 + state.rank() as f64;
            it
        })
        .collect()
}

/// What a run last said, read from its conversation file rather than its
/// screen — so it works for an agent in a terminal tab, over ssh, or with no
/// terminal at all.
pub fn transcript_tail(path: &str, want: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let Some(msg) = v.get("message") else { continue };
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let mut said = String::new();
        match msg.get("content") {
            Some(serde_json::Value::String(s)) => said = s.clone(),
            Some(serde_json::Value::Array(parts)) => {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                        said.push_str(t);
                    }
                }
            }
            _ => {}
        }
        let said = said.trim();
        if said.is_empty() {
            continue;
        }
        out.push(format!("{}  {said}", if role == "user" { "›" } else { "⏺" }));
        if out.len() >= want {
            break;
        }
    }
    out.reverse();
    out
}
