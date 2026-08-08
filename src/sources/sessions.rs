//! Past agent conversations, so you can resume one without hunting for a
//! uuid in your shell history.
//!
//! Each agent stores sessions in its own private format, none of them
//! documented. Parsing is therefore best-effort and silently yields nothing
//! when a format changes — the same contract as the docker source.
//!
//! Reading every file would be far too slow (hundreds of sessions), so only
//! the first few lines of each are read, and the whole set is cached and
//! refreshed in the background like $PATH.

use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const PIN_RANK: f64 = 10_000.0;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct SessionMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct MetadataFile {
    #[serde(default = "metadata_schema")]
    schema: u32,
    #[serde(default)]
    sessions: BTreeMap<String, SessionMeta>,
}

fn is_false(value: &bool) -> bool { !*value }
fn metadata_schema() -> u32 { 1 }

fn metadata_path() -> PathBuf {
    paths::data().join("sessions.json")
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("could not create {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("could not protect {}: {e}", path.display()))?;
    }
    Ok(())
}

fn empty_metadata() -> MetadataFile {
    MetadataFile { schema: metadata_schema(), sessions: BTreeMap::new() }
}

fn read_metadata_result() -> Result<MetadataFile, String> {
    match std::fs::read(metadata_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("session metadata is not valid JSON: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(empty_metadata()),
        Err(e) => Err(format!("could not read session metadata: {e}")),
    }
}

fn read_metadata() -> MetadataFile {
    // Sources degrade to nothing. Interactive writes use the strict reader
    // below so malformed metadata is never silently overwritten.
    read_metadata_result().unwrap_or_else(|_| empty_metadata())
}

fn write_metadata(metadata: &MetadataFile) -> Result<(), String> {
    use std::io::Write;
    let path = metadata_path();
    let dir = path.parent().ok_or_else(|| "no metadata directory".to_string())?;
    create_private_dir(dir)?;
    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|e| format!("could not encode session metadata: {e}"))?;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(|e| format!("could not write session metadata: {e}"))?;
    file.write_all(&bytes).map_err(|e| format!("could not write session metadata: {e}"))?;
    file.sync_all().map_err(|e| format!("could not sync session metadata: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("could not replace session metadata: {e}"))
}

fn clean_metadata(metadata: &mut MetadataFile) {
    metadata.sessions.retain(|_, value| {
        value.pinned || value.archived || value.title.as_ref().is_some_and(|s| !s.is_empty())
    });
}

fn refresh_caches() {
    let sessions = all();
    crate::cache::write_cached("sessions", &sessions);
    let runs = super::running::linked_identities(&sessions);
    let linked = super::running::annotate_sessions(sessions, &runs);
    crate::cache::write_cached("sessions-linked", &linked);
}

fn update_metadata(id: &str, edit: impl FnOnce(&mut SessionMeta)) -> Result<(), String> {
    if id.is_empty() || !id.contains(':') {
        return Err("that session has no stable identity".into());
    }
    let mut metadata = read_metadata_result()?;
    if metadata.schema != metadata_schema() {
        return Err(format!("session metadata schema {} is newer than this Prelude", metadata.schema));
    }
    edit(metadata.sessions.entry(id.to_string()).or_default());
    clean_metadata(&mut metadata);
    write_metadata(&metadata)?;
    refresh_caches();
    Ok(())
}

pub fn fork_cmd(agent: &str, id: &str) -> Option<String> {
    match agent {
        "claude" => Some(format!("claude --resume {} --fork-session", shq(id))),
        "codex" => Some(format!("codex fork {}", shq(id))),
        "pi" => Some(format!("pi --fork {}", shq(id))),
        _ => None,
    }
}

pub fn set_pinned(id: &str, pinned: bool) -> Result<(), String> {
    update_metadata(id, |meta| meta.pinned = pinned)
}

pub fn set_archived(id: &str, archived: bool) -> Result<(), String> {
    update_metadata(id, |meta| meta.archived = archived)
}

pub fn rename(id: &str, title: &str) -> Result<(), String> {
    let title = crate::width::flatten(title).trim().chars().take(200).collect::<String>();
    update_metadata(id, |meta| meta.title = (!title.is_empty()).then_some(title))
}

pub(crate) fn decorate_sessions(
    sessions: &mut [Item],
    metadata: &BTreeMap<String, SessionMeta>,
) {
    for session in sessions {
        let Some(meta) = metadata.get(session.get("session_id")) else { continue };
        if let Some(title) = meta.title.as_ref().filter(|title| !title.is_empty()) {
            session.data.insert("native_title".into(), session.title.clone());
            session.title = title.clone();
        }
        if meta.pinned {
            let rank = session.get("rank").parse::<f64>().unwrap_or(0.0) + PIN_RANK;
            session.data.insert("rank".into(), format!("{rank:.3}"));
            session.data.insert("pinned".into(), "true".into());
            session.score = session.kind.priority() as f64 + rank;
        }
        if meta.archived {
            session.data.insert("archived".into(), "true".into());
        }
        if meta.pinned || meta.archived {
            let mut tags = Vec::new();
            if meta.pinned { tags.push("pinned"); }
            if meta.archived { tags.push("archived"); }
            let owner = session.get("agent").to_string();
            if let Some(agent) = session.fields.first_mut() {
                *agent = format!("{owner} · {}", tags.join(" · "));
            }
        }
    }
}

pub fn visible(session: &Item) -> bool {
    session.get("archived") != "true"
}

pub struct Agent {
    pub name: &'static str,
    /// How to resume, given a session id.
    pub resume: fn(&str) -> String,
    /// How to start with an opening prompt. These genuinely differ: claude,
    /// codex and pi take it positionally, opencode needs a subcommand.
    pub prompt: fn(&str) -> String,
    /// Non-interactive form: print the answer to stdout and exit, so it can
    /// be rendered inside the launcher instead of taking over the screen.
    pub ask: fn(&str) -> Vec<String>,
}

pub const AGENTS: &[Agent] = &[
    Agent {
        name: "claude",
        resume: |id| format!("claude --resume {id}"),
        prompt: |p| format!("claude {}", shq(p)),
        ask: |p| vec!["claude".into(), "-p".into(), p.into()],
    },
    Agent {
        name: "codex",
        resume: |id| format!("codex resume {id}"),
        prompt: |p| format!("codex {}", shq(p)),
        // --skip-git-repo-check: codex exec refuses to run outside a git
        // repo, and the launcher is used from any directory.
        ask: |p| vec![
            "codex".into(),
            "exec".into(),
            "--skip-git-repo-check".into(),
            p.into(),
        ],
    },
    Agent {
        name: "pi",
        resume: |id| format!("pi --session {id}"),
        prompt: |p| format!("pi {}", shq(p)),
        ask: |p| vec!["pi".into(), "--print".into(), p.into()],
    },
    Agent {
        name: "opencode",
        resume: |id| format!("opencode --session {id}"),
        prompt: |p| format!("opencode run {}", shq(p)),
        ask: |p| vec!["opencode".into(), "run".into(), p.into()],
    },
];

/// Agents that are actually installed, for the `@` completion.
pub fn installed() -> Vec<&'static str> {
    AGENTS
        .iter()
        .map(|a| a.name)
        .filter(|n| crate::exec::which(n).is_some())
        .collect()
}

fn resume_cmd(agent: &str, id: &str) -> String {
    AGENTS
        .iter()
        .find(|a| a.name == agent)
        .map(|a| (a.resume)(id))
        .unwrap_or_else(|| id.to_string())
}

struct Raw {
    agent: &'static str,
    id: String,
    /// The file itself. Its mtime is the only universal way to tell whether
    /// a running agent is working or waiting — see `running.rs`.
    path: PathBuf,
    title: String,
    /// The opening message verbatim. The displayed title may be an
    /// AI-generated summary, which loses the `/skill-name` that started it.
    opening: String,
    cwd: String,
    mtime: SystemTime,
}

/// Read only the head of a session file; these can be megabytes.
fn head_lines(p: &Path, n: usize) -> Vec<String> {
    use std::io::{BufRead, BufReader};
    let Ok(f) = std::fs::File::open(p) else { return Vec::new() };
    BufReader::new(f)
        .lines()
        .take(n)
        .map_while(Result::ok)
        .collect()
}

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

/// Claude Code: ~/.claude/projects/<encoded-cwd>/<uuid>.jsonl
///
/// Worth the extra effort because it records an `ai-title` — a generated
/// summary of the conversation, which beats truncating the first message.
fn scan_claude(out: &mut Vec<Raw>) {
    let root = paths::home().join(".claude/projects");
    let Ok(projects) = std::fs::read_dir(&root) else { return };
    for proj in projects.flatten() {
        let Ok(files) = std::fs::read_dir(proj.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
                continue;
            };
            let mtime = f.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            let (mut title, mut cwd) = (String::new(), String::new());
            let mut first_user = String::new();
            for line in head_lines(&p, 400) {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                if cwd.is_empty() {
                    cwd = json_str(&v, "cwd");
                }
                match json_str(&v, "type").as_str() {
                    "ai-title" => {
                        let t = ["aiTitle", "title", "text", "content"]
                            .iter()
                            .map(|k| json_str(&v, k))
                            .find(|s| !s.is_empty())
                            .unwrap_or_default();
                        if !t.is_empty() {
                            title = t;
                        }
                    }
                    "user" if first_user.is_empty() => {
                        first_user = user_text(&v);
                    }
                    _ => {}
                }
                if !title.is_empty() && !cwd.is_empty() {
                    break;
                }
            }
            let opening = first_user.clone();
            if title.is_empty() {
                title = first_user;
            }
            if title.is_empty() {
                continue;
            }
            out.push(Raw { agent: "claude", id, path: p.clone(), title, opening, cwd, mtime });
        }
    }
}

fn user_text(v: &serde_json::Value) -> String {
    let Some(m) = v.get("message") else { return String::new() };
    match m.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .map(|b| json_str(b, "text"))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Codex: ~/.codex/sessions/rollout-<ts>-<uuid>.jsonl, first line is meta.
fn scan_codex(out: &mut Vec<Raw>) {
    for dir in ["sessions", "archived_sessions"] {
        let root = paths::home().join(".codex").join(dir);
        walk_jsonl(&root, &mut |p, mtime| {
            let lines = head_lines(p, 12);
            let mut id = String::new();
            let mut cwd = String::new();
            let mut title = String::new();
            for l in &lines {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
                if let Some(pl) = v.get("payload") {
                    if id.is_empty() {
                        id = json_str(pl, "session_id");
                    }
                    if cwd.is_empty() {
                        cwd = ["cwd", "workdir", "cd"].iter().map(|k| json_str(pl, k))
                            .find(|s| !s.is_empty()).unwrap_or_default();
                    }
                }
                if title.is_empty() && json_str(&v, "type").contains("user") {
                    title = crate::width::flatten(&user_text(&v));
                }
            }
            if id.is_empty() {
                // fall back to the uuid embedded in the filename
                id = p.file_stem().map(|s| s.to_string_lossy().into_owned())
                    .and_then(|s| s.rsplit('-').take(5).collect::<Vec<_>>()
                        .into_iter().rev().collect::<Vec<_>>().join("-").into())
                    .unwrap_or_default();
            }
            if title.is_empty() {
                title = folder_label(&cwd, p);
            }
            if !id.is_empty() {
                let opening = title.clone();
                out.push(Raw { agent: "codex", id, path: p.clone(), title, opening, cwd, mtime });
            }
        });
    }
}

/// pi: ~/.pi/agent/sessions/<ts>_<uuid>.jsonl, first line carries id and cwd.
fn scan_pi(out: &mut Vec<Raw>) {
    let root = paths::home().join(".pi/agent/sessions");
    walk_jsonl(&root, &mut |p, mtime| {
        let lines = head_lines(p, 12);
        let mut id = String::new();
        let mut cwd = String::new();
        let mut title = String::new();
        for l in &lines {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else { continue };
            if id.is_empty() && json_str(&v, "type") == "session" {
                id = json_str(&v, "id");
                cwd = json_str(&v, "cwd");
            }
            if title.is_empty() && json_str(&v, "type").contains("user") {
                title = crate::width::flatten(&user_text(&v));
            }
        }
        if title.is_empty() {
            title = folder_label(&cwd, p);
        }
        if !id.is_empty() {
            let opening = title.clone();
            out.push(Raw { agent: "pi", id, path: p.clone(), title, opening, cwd, mtime });
        }
    });
}

fn folder_label(cwd: &str, p: &Path) -> String {
    if !cwd.is_empty() {
        return cwd.rsplit('/').next().unwrap_or(cwd).to_string();
    }
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn walk_jsonl(root: &Path, f: &mut dyn FnMut(&PathBuf, SystemTime)) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_jsonl(&p, f);
        } else if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            let mtime = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            f(&p, mtime);
        }
    }
}

/// How much being the newest session is worth.
///
/// Larger than a use record can be (`frecency::MAX_BONUS`, 60) because for a
/// conversation recency *is* the question — you resume what you were just
/// doing. A favourite still climbs: at ten hours old this is worth 18, so a
/// session you keep coming back to passes one from two hours ago that you
/// have never picked. Nothing passes one touched a minute ago.
const RECENCY_WEIGHT: f64 = 200.0;

/// Newest first, said out loud.
///
/// This used to be a property of the *order rows were generated in* — `all`
/// sorts by mtime and a stable sort preserved it — which held only while
/// every session scored the same. The moment one picked up a frecency bonus
/// the recency order broke, and a six-hour-old session sat above one from a
/// minute ago with nothing on screen to explain why.
fn recency_rank(ts: f64) -> f64 {
    let hours = (crate::frecency::now() - ts).max(0.0) / 3600.0;
    RECENCY_WEIGHT / (1.0 + hours)
}

/// All sessions, newest first. Expensive — always called behind the cache.
pub fn all() -> Vec<Item> {
    let mut raw = Vec::new();
    scan_claude(&mut raw);
    scan_codex(&mut raw);
    scan_pi(&mut raw);
    raw.sort_by_key(|r| std::cmp::Reverse(r.mtime));

    let mut sessions: Vec<Item> = raw.into_iter()
        .map(|r| {
            let ts = r.mtime.duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs_f64()).unwrap_or(0.0);
            let where_ = if r.cwd.is_empty() {
                String::new()
            } else {
                paths::tilde(&r.cwd)
            };
            Item::new(resume_cmd(r.agent, &r.id), Kind::Session)
                .rank(recency_rank(ts))
                .title(crate::width::flatten(&r.title))
                .fields([r.agent.to_string(), where_, crate::sources::user::ago(ts)])
                .put("agent", r.agent)
                .put("session_id", format!("{}:{}", r.agent, r.id))
                .put("id", r.id)
                .put("cwd", r.cwd)
                .put("ts", format!("{ts:.0}"))
                .put("file", r.path.to_string_lossy().into_owned())
                .put("opening", crate::width::flatten(&r.opening))
        })
        .collect();
    decorate_sessions(&mut sessions, &read_metadata().sessions);
    sessions
}

/// Only the most recent handful go in the main list; `s:` searches them all.
/// Without this the 500+ sessions here would swamp everything else.
pub const IN_MAIN_LIST: usize = 15;

/// The newest conversation this agent had, if there is one.
///
/// The cache is written newest-first, so this is the first match. It is what
/// "resume" means on an agent row: you almost never want a *particular* old
/// session from the panel — you want the one you were just in, and anything
/// else you would search for with `s:`.
pub fn latest_for(agent: &str) -> Option<Item> {
    crate::cache::read_cached("sessions")
        .into_iter()
        .find(|s| s.get("agent") == agent && visible(s))
}

/// `s:query` — search every session, not just the recent ones.
pub fn search(term: &str) -> Vec<Item> {
    // `gather` joins the graph once and writes this derived snapshot. This
    // function runs on every keypress, so it must filter that snapshot rather
    // than repeat even syscall-only relationship work hundreds of times.
    let mut all = crate::cache::read_cached("sessions-linked");
    if all.is_empty() {
        let raw = crate::cache::read_cached("sessions");
        let runs = super::running::linked_identities(&raw);
        all = super::running::annotate_sessions(raw, &runs);
    }
    filter_search(all, term).into_iter().take(80).collect()
}

pub(crate) fn filter_search(all: Vec<Item>, term: &str) -> Vec<Item> {
    let mut archived = false;
    let mut include_archived = false;
    let mut pinned = false;
    let mut active = false;
    let mut needles = Vec::new();
    for word in term.split_whitespace().map(|word| word.to_lowercase()) {
        match word.as_str() {
            "is:archived" => { archived = true; include_archived = true; }
            "is:all" => include_archived = true,
            "is:pinned" => pinned = true,
            "is:active" => {
                active = true;
                include_archived = true;
            }
            _ => needles.push(word),
        }
    }
    all.into_iter()
        .filter(|session| {
            let is_archived = session.get("archived") == "true";
            if archived != is_archived && (archived || !include_archived) {
                return false;
            }
            if pinned && session.get("pinned") != "true" {
                return false;
            }
            if active && session.get("active_run").is_empty() {
                return false;
            }
            if needles.is_empty() {
                return true;
            }
            let hay = format!(
                "{} {} {} {}",
                session.title,
                session.get("native_title"),
                session.get("agent"),
                session.get("cwd")
            ).to_lowercase();
            needles.iter().all(|needle| hay.contains(needle.as_str()))
        })
        .collect()
}

fn safe_export_name(session: &Item) -> String {
    let mut name: String = session.title.chars()
        .map(|c| if matches!(c, '/' | ':' | '\0') { '-' } else { c })
        .take(80)
        .collect();
    name = name.trim().trim_matches('.').to_string();
    if name.is_empty() {
        name = session.get("id").chars().take(36).collect();
    }
    format!("{name}.jsonl")
}

pub fn export_raw(session: &Item) -> Result<PathBuf, String> {
    let source = Path::new(session.get("file"));
    if !safe_session_path(source, &paths::home()) {
        return Err("that is not a recognised native session file".into());
    }
    let dir = paths::data().join("exports");
    create_private_dir(&dir)?;
    let name = safe_export_name(session);
    let stem = Path::new(&name).file_stem().unwrap_or_default().to_string_lossy();
    let mut destination = dir.join(&name);
    let mut n = 2;
    while destination.exists() {
        destination = dir.join(format!("{stem} {n}.jsonl"));
        n += 1;
        if n > 999 {
            return Err("too many exports with that name".into());
        }
    }
    std::fs::copy(source, &destination).map_err(|e| format!("could not export conversation: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600));
    }
    Ok(destination)
}

pub(crate) fn safe_session_path(path: &Path, home: &Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
        return false;
    }
    let Ok(real) = path.canonicalize() else { return false };
    let roots = [
        home.join(".claude/projects"),
        home.join(".codex/sessions"),
        home.join(".codex/archived_sessions"),
        home.join(".pi/agent/sessions"),
    ];
    roots.into_iter().filter_map(|root| root.canonicalize().ok()).any(|root| real.starts_with(root))
}

pub(crate) fn may_be_active(session: &Item, runs: &[Item]) -> bool {
    runs.iter().any(|run| {
        run.get("agent") == session.get("agent")
            && (run.get("session_id") == session.get("session_id")
                || (!session.get("cwd").is_empty() && run.get("cwd") == session.get("cwd")))
    })
}

pub fn trash_session(session: &Item) -> Result<PathBuf, String> {
    if !session.get("active_run").is_empty() {
        return Err("an active conversation cannot be moved to the Trash".into());
    }
    // The row is a snapshot. Re-find processes now, and be conservative when
    // a same-Agent, same-project Run cannot be attached unambiguously.
    let mut sessions = crate::cache::read_cached("sessions");
    if !sessions.iter().any(|candidate| candidate.get("session_id") == session.get("session_id")) {
        sessions.push(session.clone());
    }
    let runs = super::running::fresh_identities_with_sessions(&sessions);
    if may_be_active(session, &runs) {
        return Err("that conversation may be active now; refresh before moving it".into());
    }
    let source = Path::new(session.get("file"));
    if !safe_session_path(source, &paths::home()) {
        return Err("that is not a recognised native session file".into());
    }
    let destination = paths::trash(source)?;
    // Keep its Prelude metadata. Restoring the native file from Finder then
    // restores its name and pin as well; an orphaned metadata row is tiny and
    // never appears without an authoritative native Session.
    refresh_caches();
    Ok(destination)
}

/// The non-interactive invocation for an agent, if we know one.
pub fn ask_cmd(agent: &str, prompt: &str) -> Option<Vec<String>> {
    AGENTS.iter().find(|a| a.name == agent).map(|a| (a.ask)(prompt))
}

/// Start a fresh agent session in a directory, optionally invoking a skill.
pub fn start_cmd(agent: &str, cwd: Option<&str>, prompt: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(d) = cwd {
        s.push_str(&format!("cd {} && ", shq(d)));
    }
    match prompt {
        Some(p) => {
            let spec = AGENTS.iter().find(|a| a.name == agent);
            s.push_str(&match spec {
                Some(a) => (a.prompt)(p),
                None => format!("{agent} {}", shq(p)),
            });
        }
        None => s.push_str(agent),
    }
    s
}

/// How often each skill has actually been invoked, mined from past sessions.
///
/// Only Prelude can answer this: it is the one place that sees both the
/// skills you have written and the conversations you have had. A skill you
/// wrote months ago and never used once is worth knowing about.
///
/// Restricted to names we know are skills — otherwise every `/Users/...`
/// path in a message would look like an invocation.
pub fn skill_usage(
    known: &[String],
    sessions: &[Item],
) -> std::collections::HashMap<String, (u32, f64)> {
    let mut out: std::collections::HashMap<String, (u32, f64)> = Default::default();
    if known.is_empty() {
        return out;
    }
    // Lowercased once. The inner loop runs for every session times every
    // skill — six hundred by however many you have written — so a marker
    // built inside it is thousands of allocations to compare a dozen
    // strings that never change.
    let markers: Vec<String> = known.iter().map(|n| format!("/{}", n.to_lowercase())).collect();
    for it in sessions {
        let hay = format!("{} {}", it.title, it.get("opening")).to_lowercase();
        if !hay.contains('/') {
            continue;
        }
        let ts = it.get("ts").parse::<f64>().unwrap_or(0.0);
        for (name, marker) in known.iter().zip(&markers) {
            // Scan for the marker anywhere: CJK text has no spaces, so the
            // invocation is often glued to the words around it.
            if hay.contains(marker.as_str()) {
                let e = out.entry(name.to_lowercase()).or_insert((0, 0.0));
                e.0 += 1;
                e.1 = e.1.max(ts);
            }
        }
    }
    out
}
