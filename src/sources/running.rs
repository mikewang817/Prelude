//! Every agent alive on this machine, wherever it is running.
//!
//! `sessions.rs` reads conversation files: what you talked about, whenever
//! that was. This reads the machine: what is alive at this second, and — the
//! only question that matters once there are eighty of them — which ones have
//! stopped and are waiting for you.
//!
//! **The backbone is the process list.** An agent in a terminal tab is no less
//! running than one anywhere else, and a fleet view that sees half the fleet
//! is worse than none. This once asked tmux as well, which added an address
//! for the subset of runs that had a pane — something to jump to and type
//! into. Every one of those two abilities has gone, and with them the reason
//! to treat one terminal's runs as richer than another's.
//!
//! The signal for "is it stuck" is silence, and an agent appends to its
//! session file as it works — a tool call, a message — while writing nothing
//! at all as it waits for you. That clock needs no terminal of any kind,
//! which is why it is the one that survived.
//!
//! Cost splits the work in two. *Finding* the fleet is `ps` with full command
//! lines plus one bulk `lsof` for their directories, and lives in the cache
//! tier. Deciding what each one is *doing* is then a `stat` and a
//! `kill(pid, 0)` per row: syscalls, not subprocesses. So that half runs live
//! on every gather, and the state you read is the state now rather than the
//! state when the cache was written.
//!
//! **A run's effective context is split the same way, on the same rule: pay
//! where the fact is stable.** Which capabilities a process explicitly
//! borrowed can only be read from its argument vector, which exists once, in
//! the cached half — and the arguments themselves are never kept, only the
//! capability *name*, because `--mcp-config` can be a whole server definition
//! with an API key in it. The branch changes under a run and costs two file
//! reads, so it is taken live. The model costs a bounded file read and is
//! wanted only when somebody looks at one run, so `effective_context` reads it
//! on the explicit path and gather never pays for it at all.

use crate::item::{Item, Kind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

/// How much of the *head* of a conversation is worth reading.
///
/// Only pi needs one, and only for the model — it writes `model_change` when
/// a session opens and then never again unless the model changes, so the one
/// structured record of it sits in the first few lines. Far smaller than the
/// tail because nothing here is looking for a message.
const HEAD_BYTES: u64 = 8 * 1024;

/// A bounded slice of a file that may be megabytes, from either end.
///
/// Session files are not read whole anywhere on a live path: one tool result
/// holding a large file can be tens of kilobytes on its own, and a
/// conversation that has run all afternoon is a hundred megabytes. The first
/// line of a tail window is usually half a record; every reader here parses
/// line by line and skips what will not parse, so that costs nothing.
fn window(path: &str, at_end: bool, bytes: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    if path.is_empty() {
        return None;
    }
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut buf = Vec::new();
    if at_end {
        f.seek(SeekFrom::Start(len.saturating_sub(bytes))).ok()?;
        f.read_to_end(&mut buf).ok()?;
    } else {
        buf.resize(bytes.min(len) as usize, 0);
        f.read_exact(&mut buf).ok()?;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Read the end of a conversation and say whether the agent is mid-turn.
///
/// Only the last *message* matters, so entries that carry no message —
/// attachments, summaries, meta rows — are skipped rather than counted.
pub fn last_turn(path: &str) -> Option<Turn> {
    let text = window(path, true, TAIL_BYTES)?;

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

/// The model a run is using — but only where the agent's own file records it
/// as a structured field.
///
/// There is no fourth source and there must not be one. A `--model` flag says
/// what was *asked for*, a config file says what the default would be, and
/// neither is evidence of what actually answered; a run started against a
/// model that its account cannot serve is exactly when a person looks. Where
/// nothing structured says so there is simply no model line.
///
/// Three formats, three fields. claude writes `message.model` on every
/// assistant turn and codex a `turn_context` per turn, so the tail finds
/// both. pi writes `model_change` once when a session opens, so its record is
/// at the head — a second bounded read rather than an excuse to read a
/// hundred-megabyte file.
pub fn model_of(agent: &str, path: &str) -> Option<String> {
    if let Some(model) = window(path, true, TAIL_BYTES).and_then(|text| model_in(agent, &text)) {
        return Some(model);
    }
    if agent == "pi" {
        return window(path, false, HEAD_BYTES).and_then(|text| model_in(agent, &text));
    }
    None
}

fn model_in(agent: &str, text: &str) -> Option<String> {
    let field = |v: &serde_json::Value, of: &str, key: &str| -> Option<String> {
        v.get(of)?.get(key)?.as_str().map(str::to_string)
    };
    for line in text.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let found = match agent {
            "claude" => field(&v, "message", "model"),
            "codex" if kind == "turn_context" => field(&v, "payload", "model"),
            "pi" if kind == "model_change" => {
                v.get("modelId").and_then(|m| m.as_str()).map(str::to_string)
            }
            _ => None,
        };
        // A name, not a payload: anything unreasonably long is a field that
        // has changed meaning underfoot, and a source degrades to nothing
        // rather than putting it on screen.
        if let Some(model) = found.filter(|m| !m.is_empty() && m.len() <= 80) {
            return Some(model);
        }
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
    crate::agent::SPECS.iter().map(|spec| spec.name).find(|name| *name == base)
}

/// Subcommands that are tooling rather than a conversation.
///
/// Every agent binary is also its own admin CLI, and those invocations are
/// not runs: nobody is talking to `claude mcp list`, it has no session, and it
/// exits in under a second.
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

pub(crate) fn elapsed_seconds(s: &str) -> Option<u64> {
    let (days, clock) = match s.split_once('-') {
        Some((d, rest)) => (d.parse::<u64>().ok()?, rest),
        None => (0, s),
    };
    let parts: Vec<u64> = clock.split(':').map(str::parse).collect::<Result<_, _>>().ok()?;
    let seconds = match parts.as_slice() {
        [minutes, seconds] => minutes * 60 + seconds,
        [hours, minutes, seconds] => hours * 3600 + minutes * 60 + seconds,
        _ => return None,
    };
    Some(days * 86_400 + seconds)
}

/// A native session id explicitly named on this process's command line.
/// Prompts are deliberately not retained: they can contain credentials and
/// are irrelevant once the relation has been extracted.
pub(crate) fn requested_session(agent: &str, args: &[&str]) -> Option<String> {
    let value_after = |flags: &[&str]| {
        args.windows(2)
            .find(|w| flags.contains(&w[0]) && !w[1].starts_with('-'))
            .map(|w| w[1].to_string())
            .or_else(|| {
                args.iter().find_map(|arg| {
                    flags.iter().find_map(|flag| {
                        arg.strip_prefix(&format!("{flag}=")).filter(|v| !v.is_empty()).map(str::to_string)
                    })
                })
            })
    };
    match agent {
        "claude" => value_after(&["--resume", "-r"]),
        "pi" | "opencode" => value_after(&["--session"]),
        "codex" => args
            .iter()
            .position(|a| *a == "resume")
            .and_then(|i| args.get(i + 1))
            .filter(|id| !id.starts_with('-'))
            .map(|id| (*id).to_string()),
        _ => None,
    }
}

/// What a borrowed capability is called when the flag that named it cannot be
/// reduced to a name.
///
/// Never the path and never the value. A staged `--mcp-config` file is under
/// Prelude's own cache and says nothing a person wants to read; an inline one
/// is a server definition with an env block in it. "Something was loaded and
/// we will not say what" is a fact; the argument is not one we are allowed to
/// keep.
const UNNAMED: &str = "an unnamed borrowed capability";

/// Skills and MCP servers this process explicitly took for this one run.
///
/// Read from the argument vector and only from there — a capability appears
/// against a run because that run's own command line named it, never because
/// the agent happens to have it installed. Those are two different facts and
/// the whole of Milestone 5 is keeping them apart.
///
/// The flags are the ones `lend.rs` writes, per agent: claude `--plugin-dir`
/// and `--mcp-config`, pi `--skill`, codex `-c mcp_servers.…`. The three
/// pairings with no flag contribute nothing, exactly as they offer nothing.
///
/// Like `requested_session`, this extracts a relation and keeps nothing else.
pub(crate) fn borrowed_capabilities(agent: &str, args: &[&str]) -> (Vec<String>, Vec<String>) {
    let (mut skills, mut mcp) = (Vec::new(), Vec::new());
    match agent {
        "claude" => {
            for dir in values_for(args, "--plugin-dir") {
                skills.push(name_of_path(dir));
            }
            for value in values_for(args, "--mcp-config") {
                mcp.extend(mcp_config_names(value));
            }
        }
        "pi" => {
            for dir in values_for(args, "--skill") {
                skills.push(name_of_path(dir));
            }
        }
        "codex" => {
            for value in values_for(args, "-c") {
                if let Some(name) = codex_mcp_key(value) {
                    mcp.push(name);
                }
            }
        }
        _ => {}
    }
    // Set-based, not `dedup()`: that removes only *adjacent* repeats, and the
    // one command line that repeats a name never repeats it adjacently —
    // `codex -c mcp_servers.a.command=… -c mcp_servers.b.command=…
    // -c mcp_servers.a.env=…` is two servers, and used to be recorded as
    // three capabilities with one of them named twice.
    for list in [&mut skills, &mut mcp] {
        let mut seen = std::collections::BTreeSet::new();
        list.retain(|name| seen.insert(name.clone()));
    }
    (skills, mcp)
}

/// Every value given to `flag`, in either spelling.
fn values_for<'a>(args: &[&'a str], flag: &str) -> Vec<&'a str> {
    let equals = format!("{flag}=");
    let mut out = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix(equals.as_str()) {
            if !value.is_empty() {
                out.push(value);
            }
        } else if *arg == flag {
            // Exactly one word, however variadic the flag is. `--mcp-config`
            // keeps eating bare words until the next option, so a prompt
            // typed after it would otherwise be read as a second capability
            // — and a prompt is the one thing that reliably contains
            // everything.
            if let Some(value) = args.get(i + 1).filter(|value| !value.starts_with('-')) {
                out.push(value);
            }
        }
    }
    out
}

/// A capability name, from the path a flag pointed at.
///
/// The last component is what every agent uses as the name. The path itself
/// is not kept: it discloses a home directory, and for a Prelude-staged
/// borrow it is a cache location nobody wants to read.
fn name_of_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let base = match parts.last() {
        // `--skill …/name/SKILL.md` names the file, not the skill.
        Some(last) if last.eq_ignore_ascii_case("SKILL.md") => {
            parts.get(parts.len().wrapping_sub(2)).copied().unwrap_or("")
        }
        Some(last) => last,
        None => "",
    };
    plausible_name(base.strip_suffix(".json").unwrap_or(base))
}

/// A name we are willing to record, or the admission that there is none.
///
/// Bounded and conservative: a capability name is a word, so anything long,
/// punctuated or credential-shaped is a value that has arrived where a name
/// was expected and must not be stored as one.
fn plausible_name(name: &str) -> String {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || " ._-".contains(c))
        && !crate::secrets::looks_secret(name);
    if ok { name.to_string() } else { UNNAMED.to_string() }
}

/// claude's `--mcp-config` takes either a path to a config file or the config
/// itself, and the inline form is the dangerous one — a complete server
/// definition, env block included. It is parsed for its server names and then
/// dropped on the floor.
fn mcp_config_names(value: &str) -> Vec<String> {
    if !value.trim_start().starts_with('{') {
        return vec![name_of_path(value)];
    }
    let mut names: Vec<String> = serde_json::from_str::<serde_json::Value>(value.trim())
        .ok()
        .and_then(|v| {
            let servers = v.get("mcpServers")?.as_object()?;
            Some(servers.keys().map(|name| plausible_name(name)).collect())
        })
        .unwrap_or_default();
    if names.is_empty() {
        names = scan_mcp_config_names(value);
    }
    if names.is_empty() { vec![UNNAMED.to_string()] } else { names }
}

/// How far into an inline config the scan below will look for server names.
///
/// A server name is at the front of the object by construction, so this is
/// already far past where one can be; the bound exists so that an argument
/// that is not a config at all costs a fixed amount rather than its own length.
const MCP_SCAN_BYTES: usize = 8 * 1024;

/// Server names out of an inline config that will not parse.
///
/// `ps` joins the argument vector with spaces and `fleet` splits it back on
/// them, so an inline `--mcp-config={…}` holding a single space arrives as a
/// fragment: valid JSON up to the point it was cut, and nothing serde will
/// accept. That is not an exotic case — Prelude's own `_lend-mcp` emits one
/// whenever a server's command lives in an application bundle with a space in
/// its name, and every such run recorded "an unnamed borrowed capability"
/// instead of the server it had just staged.
///
/// So the front of the object is read directly: the `mcpServers` key, the
/// brace after it, and the quoted keys immediately inside it. Only keys —
/// anything at a deeper nesting level is a value, and values are where
/// commands, arguments, headers and env blocks live. Every name still goes
/// through `plausible_name`, so a fragment that cuts mid-string, an escaped
/// character or anything credential-shaped is recorded as unnamed rather than
/// stored. The fragment itself is never retained.
fn scan_mcp_config_names(value: &str) -> Vec<String> {
    let bytes = value.as_bytes();
    let bytes = &bytes[..bytes.len().min(MCP_SCAN_BYTES)];
    const KEY: &[u8] = b"\"mcpServers\"";
    let Some(start) = bytes.windows(KEY.len()).position(|w| w == KEY) else { return Vec::new() };
    let mut i = start + KEY.len();
    // Only whitespace and the colon may stand between the key and its object;
    // anything else means this is not the shape it claims to be.
    while i < bytes.len() && bytes[i] != b'{' {
        if !bytes[i].is_ascii_whitespace() && bytes[i] != b':' {
            return Vec::new();
        }
        i += 1;
    }
    let mut names = Vec::new();
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                i += 1;
            }
            b'"' => {
                let mut j = i + 1;
                let mut text = Vec::new();
                let mut closed = false;
                while j < bytes.len() {
                    match bytes[j] {
                        // Whatever it escapes is not a character a name may
                        // contain, so the backslash stands in for it and
                        // `plausible_name` refuses the result.
                        b'\\' => {
                            text.push(b'\\');
                            j += 2;
                        }
                        b'"' => {
                            closed = true;
                            break;
                        }
                        byte => {
                            text.push(byte);
                            j += 1;
                        }
                    }
                }
                // A string the fragment cut in half is the end of what can be
                // read: there is no way to tell a key from a value after it.
                if !closed {
                    break;
                }
                if depth == 1 {
                    let mut after = j + 1;
                    while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                        after += 1;
                    }
                    if after < bytes.len() && bytes[after] == b':' {
                        names.push(plausible_name(&String::from_utf8_lossy(&text)));
                    }
                }
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    names
}

/// codex borrows through `-c mcp_servers.<name>.<field>=<value>`. The value
/// half is the part that holds a command line or an env block, so only the
/// key is read — and only when it addresses an MCP server, because `-c` is a
/// general config override and most of what it sets is not a capability.
fn codex_mcp_key(value: &str) -> Option<String> {
    let name = value.strip_prefix("mcp_servers.")?.split('.').next()?;
    Some(plausible_name(name))
}

/// What `.git/HEAD` says, told apart rather than flattened.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Head {
    Branch(String),
    /// The short object id. A detached HEAD has no branch, and printing the
    /// id where a branch name goes is a lie a person will act on.
    Detached(String),
}

/// The `.git` directory that governs `cwd`, wherever it actually lives.
///
/// A worktree and a submodule both leave a `.git` *file* holding
/// `gitdir: <path>`, and that path is routinely relative — to the directory
/// holding the file, not to the process's cwd. Resolving it against the wrong
/// one finds nothing and reports no branch for every submodule on the machine.
fn git_dir(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        let candidate = dir.join(".git");
        let Ok(meta) = std::fs::metadata(&candidate) else { continue };
        if meta.is_dir() {
            return Some(candidate);
        }
        let text = std::fs::read_to_string(&candidate).ok()?;
        let target = text.lines().find_map(|line| line.trim().strip_prefix("gitdir:"))?.trim();
        let target = Path::new(target);
        return Some(if target.is_absolute() { target.to_path_buf() } else { dir.join(target) });
    }
    None
}

/// The branch a run is working on, read the way `project::git` reads one:
/// straight off disk, with no subprocess anywhere near the gather path.
///
/// A cwd outside a repository yields nothing — not an error, not a row.
pub(crate) fn head_of(cwd: &str) -> Option<Head> {
    if cwd.is_empty() {
        return None;
    }
    let text = std::fs::read_to_string(git_dir(Path::new(cwd))?.join("HEAD")).ok()?;
    let head = text.trim();
    if let Some(name) = head.strip_prefix("ref: refs/heads/") {
        return (!name.is_empty()).then(|| Head::Branch(name.to_string()));
    }
    let hex = head.chars().all(|c| c.is_ascii_hexdigit()) && head.len() >= 7;
    hex.then(|| Head::Detached(head.chars().take(7).collect()))
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

struct Found {
    pid: String,
    ppid: String,
    agent: &'static str,
    etime: String,
    started: u64,
    batch: bool,
    requested: Option<String>,
    /// Capability *names* only. The argument vector this came out of is not
    /// kept anywhere, in this struct or beyond it.
    skills: Vec<String>,
    mcp: Vec<String>,
}

/// Find the fleet. Expensive, and therefore cached.
///
/// Records identity only — which agent, which pid, which directory, and an
/// explicit resume id when the process supplied one. Nothing here says what a
/// run is
/// *doing*: that is decided live, because a state read out of a cache is a
/// state that was true some minutes ago.
pub fn fleet() -> Vec<Item> {
    let ps = crate::exec::run(
        &["ps", "-Ao", "pid=,ppid=,etime=,command="],
        Duration::from_secs(5),
    );
    let mut found: Vec<Found> = Vec::new();
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
        let started = now().saturating_sub(elapsed_seconds(etime).unwrap_or(0));
        let requested = requested_session(agent, &rest[1..]);
        // The one place the argument vector exists, and the last place it is
        // allowed to. Both extractions take a relation and a name; `rest` is
        // dropped with the loop iteration.
        let (skills, mcp) = borrowed_capabilities(agent, &rest[1..]);
        found.push(Found {
            pid: pid.into(),
            ppid: ppid.into(),
            agent,
            etime: etime.into(),
            started,
            batch,
            requested,
            skills,
            mcp,
        });
    }
    if found.is_empty() {
        return Vec::new();
    }
    let cwds = cwd_of_agents();

    found
        .into_iter()
        .map(|found| {
            let Found { pid, ppid, agent, etime, started, batch, requested, skills, mcp } = found;
            let _ = &ppid;
            let cwd = cwds.get(&pid).cloned().unwrap_or_default();
            let run_id = format!("{agent}:{pid}:{started}");
            let mut it = Item::new(format!("kill {pid}"), Kind::Run)
                .title(agent)
                .put("agent", agent)
                .put("run_id", run_id)
                .put("started", started.to_string())
                .put("cwd", cwd.clone())
                .put("path", cwd)
                .put("pid", pid)
                .put("etime", etime);
            if batch {
                it = it.put("batch", "1");
            }
            if let Some(id) = requested {
                it = it.put("requested_session", id);
            }
            // Two keys with two names, because "claude has forty skills" and
            // "this run loaded one" are different facts and a single field
            // would let them be read as the same number.
            if !skills.is_empty() {
                it = it.put("run_skills", skills.join(", "));
            }
            if !mcp.is_empty() {
                it = it.put("run_mcp", mcp.join(", "));
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
    for spec in crate::agent::SPECS {
        argv.push("-c");
        argv.push(spec.name);
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

fn canonical_session_id(session: &Item) -> String {
    let existing = session.get("session_id");
    if !existing.is_empty() {
        existing.to_string()
    } else {
        format!("{}:{}", session.get("agent"), session.get("id"))
    }
}

fn attach(run: &mut Item, session: &Item, relation: &str) {
    run.data.insert("session".into(), session.get("file").into());
    run.data.insert("session_native_id".into(), session.get("id").into());
    run.data.insert("session_id".into(), canonical_session_id(session));
    run.data.insert("session_match".into(), relation.into());
    run.data.insert("subject".into(), session.title.clone());
}

/// Relate live processes to conversations without pretending an ambiguous
/// cwd match is exact. An explicit resume id wins. A cwd-latest inference is
/// made only when exactly one run of that agent exists in that directory;
/// two Claude processes in one project otherwise remain honestly unlinked.
pub(crate) fn attach_sessions(runs: &mut [Item], sessions: &[Item]) {
    use std::collections::{HashMap, HashSet};
    let mut group_size: HashMap<(String, String), usize> = HashMap::new();
    for run in runs.iter() {
        *group_size
            .entry((run.get("agent").to_string(), run.get("cwd").to_string()))
            .or_default() += 1;
    }

    let mut claimed = HashSet::new();
    for run in runs.iter_mut().filter(|r| !r.get("requested_session").is_empty()) {
        let requested = run.get("requested_session");
        let candidates: Vec<&Item> = sessions
            .iter()
            .filter(|s| {
                s.get("agent") == run.get("agent")
                    && (s.get("id") == requested || s.get("id").starts_with(requested))
            })
            .collect();
        if candidates.len() == 1 {
            let id = canonical_session_id(candidates[0]);
            attach(run, candidates[0], "explicit");
            claimed.insert(id);
        } else {
            run.data.insert("session_native_id".into(), requested.to_string());
            run.data.insert("session_match".into(), "requested-missing".into());
        }
    }

    for run in runs.iter_mut().filter(|r| r.get("session_match").is_empty()) {
        let key = (run.get("agent").to_string(), run.get("cwd").to_string());
        if key.1.is_empty() {
            continue;
        }
        if group_size.get(&key).copied().unwrap_or(0) != 1 {
            run.data.insert("session_match".into(), "ambiguous".into());
            continue;
        }
        let session = sessions
            .iter()
            .filter(|s| s.get("agent") == key.0 && s.get("cwd") == key.1)
            .filter(|s| !claimed.contains(&canonical_session_id(s)))
            .max_by(|a, b| {
                let x = a.get("ts").parse::<f64>().unwrap_or(0.0);
                let y = b.get("ts").parse::<f64>().unwrap_or(0.0);
                x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some(session) = session {
            let id = canonical_session_id(session);
            attach(run, session, "cwd-latest");
            claimed.insert(id);
        }
    }
}

/// Put the reverse edge on sessions so their preview can say whether the
/// conversation is active and which process owns it.
pub fn annotate_sessions(mut sessions: Vec<Item>, runs: &[Item]) -> Vec<Item> {
    let active: std::collections::HashMap<&str, &Item> = runs
        .iter()
        .filter_map(|run| (!run.get("session_id").is_empty()).then_some((run.get("session_id"), run)))
        .collect();
    for session in &mut sessions {
        let id = canonical_session_id(session);
        session.data.insert("session_id".into(), id.clone());
        if let Some(run) = active.get(id.as_str()) {
            session.data.insert("active_run".into(), run.get("run_id").into());
            session.data.insert("active_pid".into(), run.get("pid").into());
            session.data.insert("active_state".into(), run.get("state").into());
            session.data.insert("active_addr".into(), run.get("addr").into());
        }
    }
    sessions
}

/// Relationship-only view for per-keystroke Session search. It reads cached
/// identities, checks pids with syscalls and attaches Sessions, but never
/// starts a subprocess. State may be absent; ownership is still current enough
/// to prevent a duplicate resume.
pub fn linked_identities(sessions: &[Item]) -> Vec<Item> {
    let mut runs: Vec<Item> = crate::cache::read_cached("fleet")
        .into_iter()
        .filter(|it| alive(it.get("pid")))
        .collect();
    attach_sessions(&mut runs, sessions);
    runs
}

/// Fresh identities for a destructive decision. This deliberately pays for
/// `ps` and lsof again: a launcher row may have been open long enough for a
/// Session to start after its snapshot was built.
pub fn fresh_identities_with_sessions(sessions: &[Item]) -> Vec<Item> {
    let mut runs = fleet();
    attach_sessions(&mut runs, sessions);
    runs
}

/// The fleet as it stands *now*: cached identities, live state.
///
/// Every read here is a syscall, so a hundred runs cost well under a
/// millisecond. A run whose process has gone is dropped rather than shown.
pub fn live() -> Vec<Item> {
    let sessions = crate::cache::read_cached("sessions");
    live_with_sessions(&sessions)
}

/// The same live view for a caller that already parsed the session cache.
pub fn live_with_sessions(sessions: &[Item]) -> Vec<Item> {
    let now = now();
    // Which runs still exist is a `kill(pid, 0)` each, and it settles the
    // common case — nothing running — before anything else is read.
    let mut runs: Vec<Item> = crate::cache::read_cached("fleet")
        .into_iter()
        .filter(|it| alive(it.get("pid")))
        .collect();
    if runs.is_empty() {
        return Vec::new();
    }
    attach_sessions(&mut runs, sessions);
    // Runs in one project share a branch, and a project is usually where
    // several of them are. Two file reads is cheap; twenty is still worth not
    // doing.
    let mut branches: HashMap<String, Option<Head>> = HashMap::new();
    runs.into_iter()
        .map(|mut it| {
            // One clock: the session file, which every agent appends to as it
            // works and leaves alone while it waits. A pane's redraw time used
            // to be consulted alongside it and was the more direct answer for
            // the runs that had one — but only for those, and a fleet view
            // that is sharper about some of its rows than others is harder to
            // read than one that treats them alike.
            let last = mtime_of(it.get("session"));
            let silent = last.map(|t| now.saturating_sub(t)).unwrap_or(0);
            // A batch run writes to a pipe and keeps no conversation file, so
            // silence tells you nothing about it, and neither does a run with
            // no clock at all. Both are working as far as anything here can
            // see.
            let blind = last.is_none() || it.get("batch") == "1";
            // The conversation says what the clock cannot: whether this run is
            // quiet because it is thinking or quiet because it asked you
            // something and stopped.
            //
            // Only asked when the clock is about to say "waiting", which is
            // the only answer it can change — a run that moved a second ago is
            // working whatever its last entry was. That keeps the cost
            // proportional to the number of *quiet* runs rather than to the
            // size of the fleet: eighty busy agents cost nothing, and reading
            // 64KB per row on every keystroke would not have fit the budget.
            let turn = (!blind && silent >= QUIET)
                .then(|| last_turn(it.get("session")))
                .flatten();
            let state = classify(false, if blind { 0 } else { silent }, turn);
            let cwd = it.get("cwd").to_string();
            match branches.entry(cwd.clone()).or_insert_with(|| head_of(&cwd)) {
                Some(Head::Branch(name)) => {
                    it.data.insert("branch".into(), name.clone());
                }
                Some(Head::Detached(id)) => {
                    it.data.insert("detached".into(), id.clone());
                }
                None => {}
            }
            let project = match cwd.rsplit('/').next().unwrap_or("") {
                "" => crate::paths::tilde(&cwd),
                p => p.to_string(),
            };
            let addr = format!("pid {}", it.get("pid"));
            let subject = it.get("subject").to_string();
            it.fields = vec![project.clone(), state.label(silent), addr.clone(), subject];
            // `finish` dedupes on (kind, cmd), so this must differ per run or
            // two agents in the same project collapse into one row — which is
            // precisely the case this source exists for. The pid is what makes
            // it unique now that no address does.
            it.cmd = format!("kill {}", it.get("pid"));
            it.data.insert("project".into(), project);
            it.data.insert("addr".into(), addr);
            it.data.insert("state".into(), state.key().into());
            it.score = Kind::Run.priority() as f64 + state.rank() as f64;
            it
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Effective context
// ---------------------------------------------------------------------------

/// Everything known about one run, as label/value pairs in reading order.
///
/// Rendering-agnostic on purpose. Quick Look, `prelude fleet` and the control
/// table all want the same facts in the same order, and a second surface
/// re-deriving them is how two views of one run start disagreeing. An empty
/// value means "nothing says so" and is the caller's to drop — a run outside a
/// repository has no branch line at all, rather than a line saying it has none.
///
/// The expensive part is here rather than on the row for a reason: the model
/// is a bounded read of the session file, and it is wanted only when somebody
/// is looking at one run. Gather never calls this.
pub fn effective_context(run: &Item) -> Vec<(&'static str, String)> {
    let agent = run.get("agent");
    let (skills, mcp) = installed_counts(agent);
    let mut out = vec![
        ("agent", agent.to_string()),
        ("project", project_label(run)),
        ("branch", branch_label(run).unwrap_or_default()),
        ("session", session_label(run)),
        ("state", state_label(run)),
        ("started", started_label(run)),
        ("model", model_of(agent, run.get("session")).unwrap_or_default()),
        // Confirmed, never inferred: these are the capabilities this run's own
        // command line named. "none" is said out loud so it can never be read
        // as the inventory line below.
        ("skills loaded by this run", or_none(run.get("run_skills"))),
        ("mcp loaded by this run", or_none(run.get("run_mcp"))),
        (
            "installed for this agent",
            format!("{skills} skills · {mcp} mcp servers, none loaded unless named above"),
        ),
    ];
    out.push(("last said", transcript_tail(run.get("session"), 1).join(" ")));
    out
}

fn or_none(value: &str) -> String {
    if value.is_empty() { "none".into() } else { value.to_string() }
}

/// The branch, or the honest admission that HEAD is not on one.
pub fn branch_label(run: &Item) -> Option<String> {
    match (run.get("branch"), run.get("detached")) {
        ("", "") => None,
        ("", id) => Some(format!("detached at {id}")),
        (name, _) => Some(name.to_string()),
    }
}

fn project_label(run: &Item) -> String {
    match run.get("project") {
        "" => crate::paths::tilde(run.get("cwd")),
        project => project.to_string(),
    }
}

fn state_label(run: &Item) -> String {
    // `fields` already carries the state with its silence in it ("waiting
    // 4m"), computed once by the live path; recomputing it here from `state`
    // alone would drop the duration.
    match run.fields.get(1) {
        Some(field) if !field.is_empty() => field.clone(),
        _ => run.get("state").to_string(),
    }
}

fn started_label(run: &Item) -> String {
    let started: u64 = run.get("started").parse().unwrap_or(0);
    if started == 0 {
        return String::new();
    }
    format!("{} ago", short_dur(now().saturating_sub(started)))
}

/// The conversation this run is in, and how confidently it was matched.
///
/// An ambiguous or missing match says so rather than showing nothing: "two
/// runs of this agent are in this project" is the reason there is no session
/// here, and a blank line invites somebody to go looking for a bug.
fn session_label(run: &Item) -> String {
    let relation = run.get("session_match");
    match (run.get("session_id"), relation) {
        ("", "ambiguous") => "not linked · more than one run of this agent here".into(),
        ("", "requested-missing") => {
            format!("{} · no such conversation on this machine", run.get("session_native_id"))
        }
        ("", _) => String::new(),
        (id, relation) => {
            let subject = match run.get("subject") {
                "" => id.to_string(),
                subject => subject.to_string(),
            };
            if relation.is_empty() { subject } else { format!("{subject} · {relation}") }
        }
    }
}

/// What the *agent* has, which is a different question from what this run
/// took. Counted rather than listed: the two numbers are the whole point.
///
/// Directory entries only — no frontmatter parsing, no hashing, nothing that
/// belongs behind the capability cache tier. The count is therefore an
/// arithmetic restatement of the launcher's Skill rows and `control`'s
/// `AgentRecord::skills`, and it has to come to the same number as both, so it
/// counts on their rule and not a looser one.
///
/// Two things it used to do differently, and both put a number on screen that
/// contradicted the list right beside it. `~/.agents/skills` is a *location*
/// rather than an agent — `missing_agents` says so, by reporting a skill that
/// lives only there as missing from every agent — so counting it as the
/// agent's own inventory made claude's nine skills into eighteen and gave
/// codex nine it does not have. And the same skill name can appear under
/// several roots, so names are counted once however many directories hold
/// them.
fn installed_counts(agent: &str) -> (usize, usize) {
    if agent.is_empty() {
        return (0, 0);
    }
    let mut names = std::collections::BTreeSet::new();
    for (dir, _) in
        crate::sources::agents::skill_dirs().into_iter().filter(|(_, owner)| *owner == agent)
    {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !entry.path().is_dir() {
                continue;
            }
            names.insert(name);
        }
    }
    let skills = names.len();
    let mcp = crate::cache::read_cached("mcp")
        .iter()
        .filter(|server| server.get("agent").split(',').map(str::trim).any(|a| a == agent))
        .count();
    (skills, mcp)
}

/// The capability names a run confirmed, split by kind. The reverse of this —
/// which runs loaded a given capability — is built in `control.rs`.
pub fn confirmed_capabilities(run: &Item) -> (Vec<String>, Vec<String>) {
    let split = |value: &str| -> Vec<String> {
        value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
    };
    (split(run.get("run_skills")), split(run.get("run_mcp")))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory nothing else is using, removed with the test.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "prelude-running-{name}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("scratch directory");
            Scratch(root)
        }
        fn write(&self, rel: &str, text: &str) -> PathBuf {
            let path = self.0.join(rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).expect("fixture directory");
            }
            std::fs::write(&path, text).expect("fixture");
            path
        }
        fn dir(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(&path).expect("fixture directory");
            path
        }
        fn at(&self, rel: &str) -> String {
            self.0.join(rel).to_string_lossy().into_owned()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_branch_is_read_off_disk_in_every_shape_a_repository_takes() {
        let scratch = Scratch::new("git");

        // The ordinary case, and from a subdirectory: a run's cwd is rarely
        // the repository root.
        scratch.write("repo/.git/HEAD", "ref: refs/heads/feature/agent-control-plane\n");
        scratch.dir("repo/src/sources");
        assert_eq!(
            head_of(&scratch.at("repo/src/sources")),
            Some(Head::Branch("feature/agent-control-plane".into()))
        );

        // Detached: there is no branch, and the id is labelled as one by
        // `branch_label` rather than printed where a name goes.
        scratch.write("bisect/.git/HEAD", "9f1c2d3e4b5a60718293a4b5c6d7e8f901234567\n");
        assert_eq!(head_of(&scratch.at("bisect")), Some(Head::Detached("9f1c2d3".into())));

        // A worktree keeps HEAD elsewhere and leaves a file saying where.
        let elsewhere = scratch.dir("repo/.git/worktrees/spike");
        std::fs::write(elsewhere.join("HEAD"), "ref: refs/heads/spike\n").expect("worktree HEAD");
        scratch.write("tree/.git", &format!("gitdir: {}\n", elsewhere.display()));
        assert_eq!(head_of(&scratch.at("tree")), Some(Head::Branch("spike".into())));

        // A submodule's is relative — to the directory holding the file, not
        // to the process's working directory.
        scratch.write("super/.git/modules/lib/HEAD", "ref: refs/heads/vendor\n");
        scratch.write("super/lib/.git", "gitdir: ../.git/modules/lib\n");
        assert_eq!(head_of(&scratch.at("super/lib")), Some(Head::Branch("vendor".into())));

        // Not a repository at all, and an empty cwd. No branch, no error.
        assert_eq!(head_of(&scratch.at("nowhere")), None);
        assert_eq!(head_of(""), None);
        // A `.git` file pointing at nothing readable degrades the same way.
        scratch.write("broken/.git", "gitdir: /no/such/place\n");
        assert_eq!(head_of(&scratch.at("broken")), None);
    }

    #[test]
    fn a_borrowed_capability_keeps_its_name_and_never_its_path() {
        let (skills, mcp) = borrowed_capabilities(
            "claude",
            &["--plugin-dir", "/Users/someone/.cache/prelude/borrow/deploy", "fix the build"],
        );
        assert_eq!(skills, vec!["deploy".to_string()], "the last path component is the name");
        assert!(mcp.is_empty());

        // Both spellings, and the staged file's extension is not part of it.
        let (_, mcp) = borrowed_capabilities(
            "claude",
            &["--mcp-config=/Users/someone/.cache/prelude/borrow/node_repl.json"],
        );
        assert_eq!(mcp, vec!["node_repl".to_string()]);

        // pi points straight at the owner's directory, SKILL.md and all.
        let (skills, _) = borrowed_capabilities(
            "pi",
            &["--skill", "/Users/someone/.agents/skills/cnipa-ooa/SKILL.md", "start"],
        );
        assert_eq!(skills, vec!["cnipa-ooa".to_string()], "the file is not the skill");

        // codex addresses config by dotted path; only the key is a name.
        let (_, mcp) = borrowed_capabilities(
            "codex",
            &["-c", "mcp_servers.node_repl.command=\"npx\"", "-c", "model=\"gpt-5\""],
        );
        assert_eq!(mcp, vec!["node_repl".to_string()], "-c is not only for capabilities");

        // Three of the eight pairings have no flag, and extract nothing.
        assert_eq!(borrowed_capabilities("opencode", &["--skill", "/s/deploy"]).0, Vec::<String>::new());

        // Anything that cannot be reduced to a plausible name is recorded as
        // having happened, without saying what it was.
        assert_eq!(
            borrowed_capabilities("claude", &["--plugin-dir=/tmp/@@not a name!!"]).0,
            vec![UNNAMED.to_string()]
        );
        assert_eq!(
            borrowed_capabilities("pi", &["--skill=/tmp/sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01"]).0,
            vec![UNNAMED.to_string()],
            "a credential arriving where a name was expected is not stored as one"
        );
    }

    #[test]
    fn preludes_own_borrow_command_records_the_server_it_staged() {
        // Exactly what `prelude _lend-mcp claude computer-use` emits on a
        // machine where that server lives in an application bundle — and
        // exactly what `ps` gives back, because it joins the argument vector
        // with spaces and `fleet` splits it again on them. The value is
        // therefore a fragment: valid JSON up to where it was cut, and
        // nothing serde will take.
        let emitted = "--mcp-config={\"mcpServers\":{\"computer-use\":{\"args\":[\"mcp\"],\
                       \"command\":\"./Codex Computer Use.app/Contents/MacOS/Client\"}}}";
        let split: Vec<&str> = emitted.split(' ').collect();
        assert!(split.len() > 1, "the space is the whole problem");
        let (_, mcp) = borrowed_capabilities("claude", &split);
        assert_eq!(mcp, vec!["computer-use".to_string()], "the name is at the front of the object");

        // Whole and unbroken, serde still does it, and several servers still
        // come back as several names.
        let (_, mcp) = borrowed_capabilities(
            "claude",
            &["--mcp-config={\"mcpServers\":{\"one\":{\"command\":\"a\"},\"two\":{\"command\":\"b\"}}}"],
        );
        assert_eq!(mcp, vec!["one".to_string(), "two".to_string()]);

        // The scan reads keys and never values. A command, an argument list
        // and an env block sit one level deeper and are not names.
        let (_, mcp) = borrowed_capabilities(
            "claude",
            &["--mcp-config={\"mcpServers\":{\"gh\":{\"command\":\"npx\",\"env\":{\"TOKEN\":\"x\"}},"],
        );
        assert_eq!(mcp, vec!["gh".to_string()], "only the server key is a name");

        // A fragment that cuts inside the name itself yields no name — there
        // is no way to tell how much of it there was.
        let (_, mcp) = borrowed_capabilities("claude", &["--mcp-config={\"mcpServers\":{\"gh"]);
        assert_eq!(mcp, vec![UNNAMED.to_string()]);
        // And an object that is not a config at all still says nothing.
        let (_, mcp) = borrowed_capabilities("claude", &["--mcp-config={\"model\":\"opus\"}"]);
        assert_eq!(mcp, vec![UNNAMED.to_string()]);
    }

    #[test]
    fn a_capability_named_twice_on_one_command_line_is_recorded_once() {
        // codex sets one server with several `-c` overrides, and nothing says
        // they arrive together: `dedup()` removes only *adjacent* repeats, so
        // this reached the row, `mcp_confirmed` and Quick Look as three.
        let (_, mcp) = borrowed_capabilities(
            "codex",
            &[
                "-c", "mcp_servers.a.command=\"npx\"",
                "-c", "mcp_servers.b.command=\"node\"",
                "-c", "mcp_servers.a.env={\"K\":\"v\"}",
            ],
        );
        assert_eq!(mcp, vec!["a".to_string(), "b".to_string()], "two servers, in the order named");

        let (skills, _) = borrowed_capabilities(
            "claude",
            &["--plugin-dir=/c/borrow/deploy", "--plugin-dir=/c/borrow/lint",
              "--plugin-dir=/c/borrow/deploy"],
        );
        assert_eq!(skills, vec!["deploy".to_string(), "lint".to_string()]);
    }

    #[test]
    fn a_command_line_carrying_an_api_key_leaves_none_of_it_on_the_run() {
        let key = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01";
        let inline = format!(
            "{{\"mcpServers\":{{\"gh\":{{\"command\":\"npx\",\"args\":[\"-y\",\"gh-mcp\"],\
             \"env\":{{\"GITHUB_TOKEN\":\"{key}\"}}}}}}}}"
        );
        let args = [
            "--resume",
            "0cc7d054-2bef-4e1b-ba7b-56cba3958ac6",
            "--mcp-config",
            inline.as_str(),
            "--plugin-dir",
            "/Users/someone/.cache/prelude/borrow/deploy",
            "rotate the key and redeploy",
        ];
        let (skills, mcp) = borrowed_capabilities("claude", &args);
        assert_eq!(mcp, vec!["gh".to_string()], "the server's name, and nothing under it");
        assert_eq!(skills, vec!["deploy".to_string()]);

        // The row itself: everything `fleet` records from an argument vector.
        let mut run = Item::new("kill 4242", Kind::Run)
            .title("claude")
            .put("agent", "claude")
            .put("run_id", "claude:4242:1000")
            .put("run_skills", skills.join(", "))
            .put("run_mcp", mcp.join(", "));
        if let Some(id) = requested_session("claude", &args) {
            run = run.put("requested_session", id);
        }
        let stored = serde_json::to_string(&run).expect("serialize");
        for leak in [key, "GITHUB_TOKEN", "npx", "gh-mcp", "rotate the key", "/Users/someone"] {
            assert!(!stored.contains(leak), "the run kept {leak}: {stored}");
        }
        assert!(stored.contains("\"gh\"") && stored.contains("deploy"), "the names survive");

        // `ps` joins the argument vector with spaces and we split it back on
        // them, so a hand-written definition with spaces in it arrives in
        // pieces. A fragment is not a definition and not a name: the run
        // records that something was loaded, and no piece of the key can
        // reach the row through the fragments this never looks at.
        let spaced = format!(
            "--mcp-config {{ \"mcpServers\": {{ \"gh\": {{ \"env\": {{ \"T\": \"{key}\" }} }} }} }}"
        );
        let split: Vec<&str> = spaced.split(' ').collect();
        let (_, mcp) = borrowed_capabilities("claude", &split);
        assert_eq!(mcp, vec![UNNAMED.to_string()], "half a definition is not a name");
        assert!(!mcp.join(", ").contains("sk-"));
    }

    #[test]
    fn a_model_is_read_only_where_a_native_file_records_one() {
        let scratch = Scratch::new("model");

        let claude = scratch.write(
            "claude.jsonl",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n\
             {\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"model\":\"claude-opus-5\",\
             \"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n",
        );
        assert_eq!(
            model_of("claude", &claude.to_string_lossy()),
            Some("claude-opus-5".to_string())
        );

        let codex = scratch.write(
            "codex.jsonl",
            "{\"type\":\"session_meta\",\"payload\":{\"model_provider\":\"openai\"}}\n\
             {\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-5.6-luna\",\"effort\":\"high\"}}\n",
        );
        assert_eq!(model_of("codex", &codex.to_string_lossy()), Some("gpt-5.6-luna".to_string()));

        // pi writes `model_change` when the session opens and never again, so
        // a long conversation puts it far outside the tail window.
        let mut long = String::from("{\"type\":\"model_change\",\"modelId\":\"gpt-5.6-terra\"}\n");
        while long.len() < (TAIL_BYTES as usize) * 2 {
            long.push_str(
                "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\
                 \"content\":[{\"type\":\"text\",\"text\":\"working\"}]}}\n",
            );
        }
        let pi = scratch.write("pi.jsonl", &long);
        assert_eq!(model_of("pi", &pi.to_string_lossy()), Some("gpt-5.6-terra".to_string()));

        // Nothing structured says so: a model named in prose, in a flag or in
        // a field that is not the one the format records, is not evidence.
        let quiet = scratch.write(
            "quiet.jsonl",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\
             \"content\":[{\"type\":\"text\",\"text\":\"running claude --model opus\"}]}}\n\
             {\"type\":\"response_item\",\"payload\":{\"model\":\"gpt-9\"}}\n",
        );
        assert_eq!(model_of("claude", &quiet.to_string_lossy()), None);
        assert_eq!(model_of("codex", &quiet.to_string_lossy()), None, "only turn_context counts");
        assert_eq!(model_of("claude", ""), None);
        assert_eq!(model_of("claude", &scratch.at("no-such-file.jsonl")), None);
        // An agent whose format we cannot read says nothing rather than
        // guessing from a field that happens to be called model.
        assert_eq!(model_of("opencode", &claude.to_string_lossy()), None);
    }

    #[test]
    fn what_a_run_loaded_is_never_what_its_agent_has_installed() {
        let loaded = Item::new("kill 1", Kind::Run)
            .title("claude")
            .put("agent", "claude")
            .put("run_id", "claude:1:100")
            .put("state", "working")
            .put("run_skills", "deploy")
            .fields(["Prelude", "working", "pid 1", ""]);
        let bare = Item::new("kill 2", Kind::Run)
            .title("claude")
            .put("agent", "claude")
            .put("run_id", "claude:2:100")
            .put("state", "working");

        assert_eq!(
            confirmed_capabilities(&loaded),
            (vec!["deploy".to_string()], Vec::new()),
            "confirmed means this run's own command line named it"
        );
        assert_eq!(
            confirmed_capabilities(&bare),
            (Vec::new(), Vec::new()),
            "an agent that has a skill installed has not loaded it"
        );

        let context = effective_context(&bare);
        let value = |label: &str| {
            context.iter().find(|(l, _)| *l == label).map(|(_, v)| v.clone()).unwrap_or_default()
        };
        assert_eq!(value("skills loaded by this run"), "none");
        assert_eq!(value("mcp loaded by this run"), "none");
        assert!(
            value("installed for this agent").contains("skills"),
            "the inventory is counted under its own label: {:?}",
            value("installed for this agent")
        );
        assert_ne!(
            value("skills loaded by this run"),
            value("installed for this agent"),
            "the two numbers must never be the same field"
        );
        assert_eq!(
            effective_context(&loaded)
                .iter()
                .find(|(l, _)| *l == "skills loaded by this run")
                .map(|(_, v)| v.as_str()),
            Some("deploy")
        );
    }

    #[test]
    fn effective_context_reports_what_it_knows_and_stays_silent_otherwise() {
        let scratch = Scratch::new("context");
        scratch.write("repo/.git/HEAD", "ref: refs/heads/main\n");

        let run = Item::new("kill 77", Kind::Run)
            .title("claude")
            .put("agent", "claude")
            .put("run_id", "claude:77:1000")
            .put("project", "repo")
            .put("branch", "main")
            .put("started", (now().saturating_sub(600)).to_string())
            .put("state", "waiting")
            .put("session_id", "claude:abc")
            .put("session_match", "explicit")
            .put("subject", "milestone five")
            .fields(["repo", "waiting 4m", "pid 77", "milestone five"]);

        let context = effective_context(&run);
        let value = |label: &str| {
            context.iter().find(|(l, _)| *l == label).map(|(_, v)| v.clone()).unwrap_or_default()
        };
        assert_eq!(value("agent"), "claude");
        assert_eq!(value("project"), "repo");
        assert_eq!(value("branch"), "main");
        assert_eq!(value("session"), "milestone five · explicit");
        assert_eq!(value("state"), "waiting 4m", "the silence is part of the state");
        assert_eq!(value("started"), "10m ago");
        assert_eq!(value("model"), "", "no native file, no model line");

        // A detached HEAD is labelled, never presented as a branch name.
        let detached = Item::new("kill 5", Kind::Run).put("detached", "9f1c2d3");
        assert_eq!(branch_label(&detached).as_deref(), Some("detached at 9f1c2d3"));
        assert_eq!(branch_label(&Item::new("kill 6", Kind::Run)), None);

        // The conversation is what a run is judged on when nothing else says
        // anything, so the line is always there — empty when there is no file.
        let quiet = Item::new("kill 8", Kind::Run).put("agent", "claude").put("run_id", "claude:8:1");
        assert!(effective_context(&quiet).iter().any(|(label, _)| *label == "last said"));
    }
}
