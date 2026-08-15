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
    crate::agent::get(agent)?.fork(id)
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

/// Should this conversation appear where nobody asked for archived rows?
///
/// Archive is a statement about a *finished* conversation — put it away, it is
/// done. A conversation somebody has just resumed is not finished, whatever
/// was true of it last week, and hiding a live agent because of a label the
/// same person set months ago is the launcher contradicting the machine. So an
/// archived Session with a live Run is visible again; the flag is left alone
/// rather than cleared, because the resume is the exception and the archive is
/// still the intent once that Run exits.
///
/// The rule lives here and `apply_filters` asks it, rather than each side
/// testing the flag for itself: two answers to one question is how an archived
/// conversation somebody had just resumed came to be on the launcher home and
/// missing from `s:`, the one scope whose job is finding every conversation.
/// `is:archived` is the exception and reads the flag directly, because that is
/// a question about metadata and must answer literally; `is:active` and
/// `is:all` include archived rows by design.
///
/// Called for every session on every gather, so it stays two `data` lookups.
pub fn visible(session: &Item) -> bool {
    session.get("archived") != "true" || !session.get("active_run").is_empty()
}

/// What a Session row hands over, or the bare id when its Agent has no way to
/// resume a named conversation.
///
/// The fallback is deliberately not a command. Several CLIs offer only
/// `--continue` — the most recent session, no id — and inventing
/// `<agent> --resume <id>` for those produces a line that looks right on the
/// clipboard and fails once the launcher has closed, which is the failure this
/// registry exists to prevent. The id is at least true, and it is what those
/// CLIs' own session pickers ask for.
fn resume_cmd(agent: &str, id: &str) -> String {
    crate::agent::get(agent)
        .and_then(|spec| spec.resume(id))
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

/// omp: ~/.omp/agent/sessions/<encoded-cwd>/<timestamp>_<uuid>.jsonl
///
/// The most generous of these formats: the head carries a `title` record that
/// omp writes itself, and a `session` record with the id, the timestamp and
/// the working directory. Nothing has to be inferred from a first message, and
/// nothing below the head has to be read.
fn scan_omp(out: &mut Vec<Raw>) {
    let root = paths::home().join(".omp/agent/sessions");
    walk_jsonl(&root, &mut |p, mtime| {
        let mut id = String::new();
        let mut cwd = String::new();
        let mut title = String::new();
        for line in head_lines(p, 12) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            match json_str(&v, "type").as_str() {
                "session" if id.is_empty() => {
                    id = json_str(&v, "id");
                    cwd = json_str(&v, "cwd");
                }
                "title" if title.is_empty() => title = crate::width::flatten(&json_str(&v, "title")),
                _ => {}
            }
        }
        if title.is_empty() {
            title = folder_label(&cwd, p);
        }
        if !id.is_empty() {
            let opening = title.clone();
            out.push(Raw { agent: "omp", id, path: p.clone(), title, opening, cwd, mtime });
        }
    });
}

/// Kimi Code: ~/.kimi-code/sessions/<workdir-slug>/session_<uuid>/state.json
///
/// The only one of these whose conversation is not a JSONL to be parsed. A
/// small sidecar carries everything a row needs — title, working directory and
/// both timestamps — so this reads one bounded JSON per session and never
/// opens the transcript beside it.
///
/// Kimi titles a session from its opening message, and leaves `New Session`
/// on one that was opened and abandoned. Nineteen of the thirty-four on the
/// machine this was written on were exactly that. They are skipped when the
/// two timestamps agree, which is what "created and never touched again"
/// looks like without reading the wire log to find out.
fn scan_kimi(out: &mut Vec<Raw>) {
    let root = paths::home().join(".kimi-code/sessions");
    let Ok(projects) = std::fs::read_dir(&root) else { return };
    for project in projects.flatten() {
        let Ok(sessions) = std::fs::read_dir(project.path()) else { continue };
        for entry in sessions.flatten() {
            let dir = entry.path();
            let Some(id) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            let state = dir.join("state.json");
            let Some(v) = read_json(&state) else { continue };
            let created = json_str(&v, "createdAt");
            let updated = json_str(&v, "updatedAt");
            let mut title = crate::width::flatten(&json_str(&v, "title"));
            let cwd = json_str(&v, "workDir");
            if title == "New Session" {
                if created == updated {
                    continue;
                }
                title = folder_label(&cwd, &state);
            }
            let mtime = std::fs::metadata(&state)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let opening = title.clone();
            out.push(Raw { agent: "kimi", id, path: state, title, opening, cwd, mtime });
        }
    }
}

/// Cursor: ~/.cursor/chats/<workspace>/<chat-id>/meta.json
///
/// The transcript lives in a SQLite `store.db` beside this file, which is why
/// a Cursor row carries no title: reading it would mean a database engine on
/// the launch path, and the rule against new dependencies is not worth
/// spending on a display string. The sidecar has the working directory and
/// both timestamps, so the row says which project and when — enough to pick a
/// conversation out, which is what resuming needs.
///
/// `hasConversation` is Cursor's own word for a chat that was opened and never
/// used, and there are dozens of them. It is the same exclusion `scan_kimi`
/// makes by comparing timestamps, stated by the format itself.
fn scan_cursor(out: &mut Vec<Raw>) {
    for root in [paths::home().join(".cursor/chats"), paths::home().join(".cursor/acp-sessions")] {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for entry in entries.flatten() {
            // `chats/` nests one level deeper than `acp-sessions/`: a
            // workspace directory holding the chats belonging to it.
            let holders: Vec<PathBuf> = if entry.path().join("meta.json").is_file() {
                vec![entry.path()]
            } else {
                std::fs::read_dir(entry.path())
                    .map(|inner| inner.flatten().map(|e| e.path()).collect())
                    .unwrap_or_default()
            };
            for dir in holders {
                let meta = dir.join("meta.json");
                let Some(v) = read_json(&meta) else { continue };
                if v.get("hasConversation").and_then(|x| x.as_bool()) == Some(false) {
                    continue;
                }
                let Some(id) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                    continue;
                };
                let cwd = json_str(&v, "cwd");
                let title = folder_label(&cwd, &meta);
                let mtime = std::fs::metadata(&meta)
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                let opening = title.clone();
                out.push(Raw {
                    agent: "cursor-agent",
                    id,
                    path: meta,
                    title,
                    opening,
                    cwd,
                    mtime,
                });
            }
        }
    }
}

/// One bounded JSON sidecar, or nothing.
///
/// `read_bounded` for the reason every read of somebody else's file here uses
/// it: the size is another program's decision, and these are read on the
/// session refresh for every conversation on the machine.
fn read_json(p: &Path) -> Option<serde_json::Value> {
    let text = crate::paths::read_bounded(p, 64 * 1024)?;
    serde_json::from_slice(&text).ok()
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
    scan_omp(&mut raw);
    scan_kimi(&mut raw);
    scan_cursor(&mut raw);
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

/// The newest conversation this agent had, if there is one.
///
/// The cache is written newest-first, so this is the first match. It is what
/// "resume" means on an agent row: you almost never want a *particular* old
/// session from the panel — you want the one you were just in, and anything
/// else you would search for with `s:`.
///
/// The linked snapshot is read first because `visible` asks a question only
/// that file can answer: the raw `sessions` cache never carries `active_run`,
/// so the archived-but-resumed clause would be inert here and the panel most
/// likely to want that conversation would be the one place that hid it. The
/// fallback is not a third answer — it is the same rule with no Run known yet,
/// which is what archive meant before the Run edge existed.
pub fn latest_for(agent: &str) -> Option<Item> {
    let mut sessions = crate::cache::read_cached("sessions-linked");
    if sessions.is_empty() {
        sessions = crate::cache::read_cached("sessions");
    }
    sessions.into_iter().find(|s| s.get("agent") == agent && visible(s))
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

/// What a `s:` query asked for, before any conversation has been looked at.
///
/// Kept as data rather than as four booleans threaded through a closure so the
/// parse can be tested on its own. It runs on every keystroke of an `s:`
/// query, so it does exactly what its name says: no filesystem, no clock, no
/// Agent CLI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filters {
    /// `is:archived` — only rows carrying the flag.
    pub archived: bool,
    /// Archived rows are allowed through at all (`is:all`, `is:active`).
    pub include_archived: bool,
    pub pinned: bool,
    pub active: bool,
    /// `project:<name>`. Several are an *or*: one conversation cannot be in
    /// two projects, so anding them would always produce nothing.
    pub projects: Vec<String>,
    /// `agent:<name>`, likewise an or.
    pub agents: Vec<String>,
    /// `since:<7d|24h|30m>`, as a number of seconds. Applied against `ts`,
    /// which is the session file's mtime — when the conversation last moved.
    pub since: Option<f64>,
    /// Filter-shaped words whose value meant nothing: `since:banana`,
    /// `is:yesterday`, a bare `project:`. Kept apart from the needles so a
    /// surface can name them, and then searched for anyway — `apply_filters`
    /// adds them to the needles, which empties the list.
    ///
    /// That is the point. Dropping them was the same lie in the more
    /// dangerous direction: an unrecognised filter that quietly matches
    /// *everything* answers a question nobody asked and looks exactly like a
    /// filter that worked, so `since:banana` returned all eighty rows and
    /// `since:24h` returned thirteen. A list that visibly collapses is a
    /// question you can see went wrong.
    pub unknown: Vec<String>,
}

/// `24h` → 86400 seconds. A bare number is refused: `since:7` is either a
/// week or seven minutes depending on what the reader assumes, and guessing
/// wrong silently hides conversations. Zero is refused for the same reason
/// from the other end — `since:0d` is a window nothing can fall inside, so it
/// empties the list while looking like a filter that ran.
fn parse_duration(value: &str) -> Option<f64> {
    let digits: String = value.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let scale = match &value[digits.len()..] {
        "m" | "min" | "mins" => 60.0,
        "h" | "hr" | "hrs" => 3600.0,
        "d" | "day" | "days" => 86_400.0,
        "w" | "week" | "weeks" => 604_800.0,
        _ => return None,
    };
    digits.parse::<f64>().ok().map(|n| n * scale).filter(|seconds| *seconds > 0.0)
}

/// Split a query into words, keeping a quoted run together.
///
/// `project:"my project"` has to survive as one word: split on whitespace it
/// becomes a filter for a project called "my" plus a stray needle, which
/// matches nothing and says nothing about why. Quotes are dropped rather than
/// kept — nothing downstream wants them, and a title containing one is
/// searched for by typing it inside the other kind.
pub(crate) fn split_words(term: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for (i, c) in term.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            // A quote only quotes when it is closed later. Otherwise `don't`
            // would swallow the rest of the query, and this runs on every
            // keystroke of an ordinary English search.
            None if matches!(c, '"' | '\'') && term[i + c.len_utf8()..].contains(c) => {
                quote = Some(c)
            }
            None if c.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            None => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Split an `s:` query into what it filters by and what it searches for.
///
/// The remainder comes back as words rather than as a slice of the input: a
/// filter removed from the middle of a query leaves no contiguous string
/// behind, so a `&str` would either have to lie about the original or be
/// re-split by every caller.
pub fn parse_filters(term: &str) -> (Filters, Vec<String>) {
    let mut filters = Filters::default();
    let mut needles = Vec::new();
    for word in split_words(term).iter().map(|word| word.to_lowercase()) {
        let Some((key, value)) = word.split_once(':') else {
            needles.push(word);
            continue;
        };
        match key {
            "is" => match value {
                "archived" => {
                    filters.archived = true;
                    filters.include_archived = true;
                }
                "all" => filters.include_archived = true,
                "pinned" => filters.pinned = true,
                "active" => {
                    filters.active = true;
                    // An active conversation is shown even when it is
                    // archived; see `visible`.
                    filters.include_archived = true;
                }
                _ => filters.unknown.push(word),
            },
            "project" if !value.is_empty() => filters.projects.push(value.to_string()),
            "agent" if !value.is_empty() => filters.agents.push(value.to_string()),
            "since" => match parse_duration(value) {
                Some(seconds) => filters.since = Some(seconds),
                None => filters.unknown.push(word),
            },
            "project" | "agent" => filters.unknown.push(word),
            // `s:` shares its box with scope prefixes and paths, so anything
            // else containing a colon is an ordinary search word.
            _ => needles.push(word),
        }
    }
    (filters, needles)
}

/// Does this conversation belong to the project the query named?
///
/// The directory's own name, or the whole path, and nothing in between. An
/// exact name is never widened into a substring match — the same rule the bus
/// follows when it resolves a recipient, and for the same reason: `project:app`
/// silently including every project whose path contains "app" is a filter
/// pretending to be an answer.
fn in_project(cwd: &str, want: &str) -> bool {
    if cwd.is_empty() {
        return false;
    }
    let trimmed = cwd.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
    base.to_lowercase() == want || trimmed.to_lowercase() == want
}

/// The filter half of `s:`, with the clock passed in so it can be pinned.
pub(crate) fn apply_filters(
    all: Vec<Item>,
    filters: &Filters,
    needles: &[String],
    now: f64,
) -> Vec<Item> {
    // A word that looked like a filter and was not one is searched for
    // literally. It will match no title, which is the visible answer; being
    // dropped made it match everything, which is not an answer at all.
    let needles: Vec<&str> = needles
        .iter()
        .chain(filters.unknown.iter())
        .map(String::as_str)
        .collect();
    all.into_iter()
        .filter(|session| {
            if filters.archived {
                // `is:archived` asks about metadata and answers literally:
                // carries the flag, whatever the machine is doing with it.
                if session.get("archived") != "true" {
                    return false;
                }
            } else if !filters.include_archived && !visible(session) {
                // The default view and the launcher home must agree. Asking
                // the raw flag here hid a conversation somebody had just
                // resumed from `s:` — the one scope whose job is finding every
                // conversation — while the home was still showing it.
                return false;
            }
            if filters.pinned && session.get("pinned") != "true" {
                return false;
            }
            if filters.active && session.get("active_run").is_empty() {
                return false;
            }
            if !filters.projects.is_empty()
                && !filters.projects.iter().any(|want| in_project(session.get("cwd"), want))
            {
                return false;
            }
            if !filters.agents.is_empty()
                && !filters.agents.iter().any(|want| session.get("agent").to_lowercase() == *want)
            {
                return false;
            }
            if let Some(window) = filters.since {
                let ts = session.get("ts").parse::<f64>().unwrap_or(0.0);
                if ts <= 0.0 || now - ts > window {
                    return false;
                }
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
            needles.iter().all(|needle| hay.contains(needle))
        })
        .collect()
}

pub(crate) fn filter_search(all: Vec<Item>, term: &str) -> Vec<Item> {
    let (filters, needles) = parse_filters(term);
    apply_filters(all, &filters, &needles, crate::frecency::now())
}

fn safe_export_stem(session: &Item) -> String {
    let mut name: String = session.title.chars()
        .map(|c| if matches!(c, '/' | ':' | '\0') { '-' } else { c })
        .take(80)
        .collect();
    name = name.trim().trim_matches('.').to_string();
    if name.is_empty() {
        name = session.get("id").chars().take(36).collect();
    }
    if name.is_empty() {
        name = "conversation".into();
    }
    name
}

/// A name in the export directory that is not already taken.
fn unique_export(dir: &Path, stem: &str, ext: &str) -> Result<PathBuf, String> {
    let mut destination = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while destination.exists() {
        destination = dir.join(format!("{stem} {n}.{ext}"));
        n += 1;
        if n > 999 {
            return Err("too many exports with that name".into());
        }
    }
    Ok(destination)
}

pub fn export_raw(session: &Item) -> Result<PathBuf, String> {
    let source = Path::new(session.get("file"));
    if !safe_session_path(source, &paths::home()) {
        return Err("that is not a recognised native session file".into());
    }
    let dir = paths::data().join("exports");
    create_private_dir(&dir)?;
    let destination = unique_export(&dir, &safe_export_stem(session), "jsonl")?;
    // Created 0600, not chmodded afterwards: a copy-then-chmod leaves a window
    // in which a whole conversation is world readable, and this is the larger
    // of the two exports.
    let mut from = std::fs::File::open(source)
        .map_err(|e| format!("could not export conversation: {e}"))?;
    let mut to = private_file(&destination)?;
    std::io::copy(&mut from, &mut to).map_err(|e| format!("could not export conversation: {e}"))?;
    to.sync_all().map_err(|e| format!("could not export conversation: {e}"))?;
    Ok(destination)
}

/// One readable turn, in whichever of the three formats recorded it.
struct Turn {
    role: &'static str,
    /// The native timestamp, when the format writes one. Never computed: a
    /// clock of Prelude's own would be a claim about the conversation that no
    /// Agent made.
    stamp: String,
    text: String,
    /// Tool names only. What a tool was *asked* is the single largest source
    /// of file contents, diffs and credentials in any of these files, so the
    /// arguments are read past rather than exported.
    tools: Vec<String>,
}

/// Turn one recorded line into a turn of conversation, or nothing.
///
/// The three formats disagree about everything except that a turn has a role
/// and some content, so this reads for that shape rather than branching on the
/// agent — which also means a format that gains a wrapper keeps working.
///
/// * claude puts the turn in `message`, tool calls in `tool_use` blocks and
///   tool results in a *user* turn, which is not something a person said. It
///   also delivers its own harness material — the output of a slash command,
///   a reminder to itself, a whole injected skill document — as `user` text,
///   wrapped in the tags `INJECTED_SPANS` names; those are stripped, and a
///   turn that was nothing else is dropped exactly as `developer` is.
/// * pi puts it in `message` too, but gives a tool result its own
///   `toolResult` role and calls its call blocks `toolCall`.
/// * codex nests everything in `payload`, writes each turn twice — once as a
///   `response_item` for the record and again as an `event_msg` for its own
///   display — and speaks to the model through a `developer` role.
fn transcript_turn(v: &serde_json::Value) -> Option<Turn> {
    // The event copy is dropped rather than printed twice.
    if json_str(v, "type") == "event_msg" {
        return None;
    }
    let body = v.get("message").or_else(|| v.get("payload")).unwrap_or(v);
    let role = match json_str(body, "role").as_str() {
        "user" => "user",
        "assistant" => "assistant",
        // Tool results and harness instructions. Neither is conversation.
        "toolresult" | "toolResult" | "tool" | "developer" | "system" => return None,
        "" => match json_str(body, "type").as_str() {
            // codex names the tool on the call item itself, with no role.
            "function_call" | "custom_tool_call" | "local_shell_call" => "assistant",
            _ => return None,
        },
        _ => return None,
    };
    let mut text = String::new();
    let mut tools: Vec<String> = Vec::new();
    if let Some(name) = body.get("name").and_then(|n| n.as_str()).filter(|n| !n.is_empty()) {
        tools.push(name.to_string());
    }
    match body.get("content") {
        Some(serde_json::Value::String(s)) => text.push_str(s),
        Some(serde_json::Value::Array(parts)) => {
            for part in parts {
                match json_str(part, "type").as_str() {
                    // codex distinguishes what went in from what came out;
                    // both are prose.
                    "text" | "input_text" | "output_text" => {
                        let said = json_str(part, "text");
                        if !said.is_empty() {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&said);
                        }
                    }
                    "tool_use" | "toolCall" => tools.push(json_str(part, "name")),
                    // Reasoning, images and tool results are deliberately not
                    // part of a portable transcript.
                    _ => {}
                }
            }
        }
        _ => {}
    }
    tools.retain(|name| !name.is_empty());
    if role == "user" {
        text = without_injected_spans(&text);
    }
    if text.trim().is_empty() && tools.is_empty() {
        return None;
    }
    Some(Turn { role, stamp: json_str(v, "timestamp"), text, tools })
}

/// What Claude Code wraps around content it generated itself and recorded as
/// though the person had typed it: the caveat it prefixes to local command
/// output, the three parts of a slash command, that command's stdout, and the
/// reminders and documents it injects mid-conversation.
///
/// codex and pi give this class of material its own role (`developer`,
/// `toolResult`) and it is refused there by role alone. Claude does not, so
/// the only signal left is the wrapper — and without this a transcript
/// promising that tool output is never copied carries shell output, slash
/// command metadata and entire skill files as `## user` prose.
const INJECTED_SPANS: [&str; 6] = [
    "local-command-caveat",
    "command-name",
    "command-message",
    "command-args",
    "local-command-stdout",
    "system-reminder",
];

fn without_injected_spans(text: &str) -> String {
    let mut out = text.to_string();
    for tag in INJECTED_SPANS {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        let mut kept = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(start) = rest.find(open.as_str()) {
            kept.push_str(&rest[..start]);
            match rest[start..].find(close.as_str()) {
                Some(end) => rest = &rest[start + end + close.len()..],
                // An unclosed injection runs to the end of the turn. Keeping
                // the remainder would export precisely what the tag exists to
                // mark as not-conversation.
                None => {
                    rest = "";
                    break;
                }
            }
        }
        kept.push_str(rest);
        out = kept;
    }
    out.trim().to_string()
}

/// Never let a credential out of the conversation, and take the block it sits
/// in with it.
///
/// A line at a time is the wrong window, and in the one direction that costs a
/// key: nobody writes a credential on the line that names it. The label is
/// typed and the value is *pasted underneath* — an `Authorization:` header and
/// its bearer blob, `-----BEGIN OPENSSH PRIVATE KEY-----` and forty lines of
/// base64, a `.env` heading and the variables below it — so testing each line
/// alone tests only the harmless half and exports the rest.
///
/// So a match redacts its line and every non-blank line following it: the run
/// of text a paste lands in. A blank line ends it, which is what keeps this
/// from swallowing the transcript — paragraphs are already separated by one,
/// and this is applied per turn rather than to the finished document, so a key
/// in one turn can never reach into the next. The trade is deliberate and
/// asymmetric, because the output is a file a person then mails or commits:
/// over-redacting costs a paragraph about tokens, under-redacting costs the
/// key itself, and the raw export still has everything.
///
/// The header Prelude writes is *not* passed through this. Its lines are
/// single self-contained fields, not a paste, so a project path with the word
/// "secret" in it would otherwise take the line under it with no paste
/// anywhere; `redacted_field` tests those one at a time.
fn redacted(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            in_block = false;
            out.push(line);
            continue;
        }
        if crate::secrets::looks_secret(line) {
            if !in_block {
                out.push("[redacted: this looked like a credential]");
                in_block = true;
            }
            continue;
        }
        if !in_block {
            out.push(line);
        }
    }
    out.join("\n")
}

/// One value of Prelude's own — a title, a path, an id — with the same broad
/// test but no block: there is nothing above or below it to be part of.
fn redacted_field(value: &str) -> String {
    if crate::secrets::looks_secret(value) {
        "[redacted: this looked like a credential]".to_string()
    } else {
        value.to_string()
    }
}

/// The whole conversation as Markdown, or an honest error.
fn render_transcript(session: &Item, source: &Path) -> Result<String, String> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(source)
        .map_err(|e| format!("could not read that conversation: {e}"))?;
    let mut turns: Vec<Turn> = Vec::new();
    let mut parsed = 0usize;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        parsed += 1;
        if let Some(turn) = transcript_turn(&v) {
            turns.push(turn);
        }
    }
    if parsed == 0 {
        return Err("that file is not a conversation this Prelude can read".into());
    }
    if turns.is_empty() {
        // Parseable, but nothing in it was a turn: the format moved. Say so
        // rather than write an empty file that looks like an empty
        // conversation.
        return Err(format!(
            "{} recorded this conversation in a shape this Prelude does not \
             recognise; the raw export still has everything",
            session.get("agent"),
        ));
    }
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", redacted_field(&session.title)));
    for (label, value) in [
        ("Agent", session.get("agent").to_string()),
        ("Session", session.get("session_id").to_string()),
        ("Project", paths::tilde(session.get("cwd"))),
        ("Source", paths::tilde(&source.to_string_lossy())),
        ("Turns", turns.len().to_string()),
    ] {
        if !value.is_empty() {
            out.push_str(&format!("- {label}: {}\n", redacted_field(&value)));
        }
    }
    out.push_str(
        "\nExported by Prelude from the native conversation. Tool arguments, \
         tool output and reasoning are named or omitted, never copied.\n",
    );
    for turn in &turns {
        out.push_str(&format!("\n## {}", turn.role));
        if !turn.stamp.is_empty() {
            out.push_str(&format!(" · {}", turn.stamp));
        }
        out.push('\n');
        if !turn.text.trim().is_empty() {
            out.push_str(&format!("\n{}\n", redacted(turn.text.trim_end())));
        }
        for tool in &turn.tools {
            out.push_str(&format!("\n- called tool: {}\n", redacted_field(tool)));
        }
    }
    Ok(out)
}

/// A conversation as Markdown anyone can read, beside the raw JSONL export.
///
/// The raw export is the authoritative one and stays: it is what you hand
/// back to the Agent that wrote it. This is the other half — what you send to
/// a person, who has no interest in `tool_use` ids and cannot be given a file
/// that carries a key, a diff of a private repository or forty kilobytes of
/// one tool result.
///
/// Explicit action only. It reads a file that can be megabytes and must never
/// be reached from gather or the per-keystroke helper.
pub fn export_transcript(session: &Item) -> Result<PathBuf, String> {
    export_transcript_into(session, &paths::data().join("exports"), &paths::home())
}

/// The directory and the home root are parameters so a test can prove the
/// boundary, the permissions and the redaction without an export landing in
/// the person's own data directory.
fn export_transcript_into(session: &Item, dir: &Path, home: &Path) -> Result<PathBuf, String> {
    let source = Path::new(session.get("file"));
    if !safe_session_path(source, home) {
        return Err("that is not a recognised native session file".into());
    }
    let markdown = render_transcript(session, source)?;
    create_private_dir(dir)?;
    let destination = unique_export(dir, &safe_export_stem(session), "md")?;
    write_private(&destination, markdown.as_bytes())?;
    Ok(destination)
}

/// A new file only this user can read, created 0600 rather than chmodded
/// afterwards: a chmod leaves a window in which a transcript is world
/// readable. `create_new` also means an export never lands on top of
/// something already there.
fn private_file(path: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|e| format!("could not write the export: {e}"))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut file = private_file(path)?;
    file.write_all(bytes).map_err(|e| format!("could not write the export: {e}"))?;
    file.sync_all().map_err(|e| format!("could not write the export: {e}"))
}

/// The four directories the native CLIs record conversations in.
///
/// One list, because two would drift: it is both what the Trash and export
/// boundary trusts and what the diagnostic checks it can still read.
fn native_session_roots(home: &Path) -> [PathBuf; 8] {
    [
        home.join(".claude/projects"),
        home.join(".codex/sessions"),
        home.join(".codex/archived_sessions"),
        home.join(".pi/agent/sessions"),
        home.join(".kimi-code/sessions"),
        home.join(".cursor/chats"),
        home.join(".cursor/acp-sessions"),
        home.join(".omp/agent/sessions"),
    ]
}

pub(crate) fn safe_session_path(path: &Path, home: &Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
        return false;
    }
    let Ok(real) = path.canonicalize() else { return false };
    native_session_roots(home)
        .into_iter()
        .filter_map(|root| root.canonicalize().ok())
        .any(|root| real.starts_with(root))
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

/// Which way round a duplicate is, because the two are different accidents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DuplicateKind {
    /// One Session id, more than one file. A conversation copied between
    /// machines, or restored from the Trash beside the original: whichever
    /// file the launcher happens to read second wins the row, and resuming it
    /// continues the wrong one.
    SameIdManyFiles,
    /// One file, more than one Session id. The id was read out of the file's
    /// contents on one pass and out of its name on another, so the same
    /// conversation appears twice and its metadata — name, pin, archive —
    /// attaches to only one of them.
    SameFileManyIds,
}

/// A conversation that exists more than once, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicate {
    pub kind: DuplicateKind,
    /// The Session ids involved: one for `SameIdManyFiles`, several otherwise.
    pub ids: Vec<String>,
    /// The files involved: several for `SameIdManyFiles`, one otherwise.
    pub paths: Vec<String>,
    /// The newest member — the file, or the id, whose conversation last moved.
    /// A report needs this to say which of them is the live one.
    pub newest: String,
}

fn newest_of(group: &[&Item], field: &str) -> String {
    group
        .iter()
        .max_by(|a, b| {
            let x = a.get("ts").parse::<f64>().unwrap_or(0.0);
            let y = b.get("ts").parse::<f64>().unwrap_or(0.0);
            x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.get(field).cmp(b.get(field)))
        })
        .map(|it| it.get(field).to_string())
        .unwrap_or_default()
}

/// Conversations recorded twice, in either direction.
///
/// A pure function over the list the launcher already has, so a doctor pays
/// nothing to ask and a test can pin the answer. It reports; it repairs
/// nothing and deletes nothing — either file may be the one somebody wants,
/// and this cannot know which.
///
/// **Feed it `read_cached("sessions")` or `all()`, never the finished launcher
/// list.** `cache::finish` dedupes on `(kind, cmd)`, and two files sharing one
/// Session id produce the same `cmd` — so by the time a list has been
/// finished, the duplicate this function exists to find is the one row that
/// survived, and it reports nothing at all.
pub fn duplicate_sessions(sessions: &[Item]) -> Vec<Duplicate> {
    let mut by_id: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    let mut by_file: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for session in sessions {
        let (id, file) = (session.get("session_id"), session.get("file"));
        if !id.is_empty() {
            by_id.entry(id).or_default().push(session);
        }
        if !file.is_empty() {
            by_file.entry(file).or_default().push(session);
        }
    }
    let mut out = Vec::new();
    for (id, group) in &by_id {
        // The same row twice is one fact stated twice, not a duplicate
        // conversation; `finish` drops those anyway.
        let mut distinct: Vec<&Item> = Vec::new();
        for session in group {
            if !distinct.iter().any(|kept| kept.get("file") == session.get("file")) {
                distinct.push(session);
            }
        }
        if distinct.len() < 2 {
            continue;
        }
        let mut paths: Vec<String> = distinct.iter().map(|s| s.get("file").to_string()).collect();
        paths.sort();
        out.push(Duplicate {
            kind: DuplicateKind::SameIdManyFiles,
            ids: vec![(*id).to_string()],
            paths,
            newest: newest_of(&distinct, "file"),
        });
    }
    for (file, group) in &by_file {
        let mut distinct: Vec<&Item> = Vec::new();
        for session in group {
            if !distinct.iter().any(|kept| kept.get("session_id") == session.get("session_id")) {
                distinct.push(session);
            }
        }
        if distinct.len() < 2 {
            continue;
        }
        let mut ids: Vec<String> = distinct.iter().map(|s| s.get("session_id").to_string()).collect();
        ids.sort();
        out.push(Duplicate {
            kind: DuplicateKind::SameFileManyIds,
            ids,
            paths: vec![(*file).to_string()],
            newest: newest_of(&distinct, "session_id"),
        });
    }
    out
}

/// What is wrong with a conversation the launcher is still listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The project directory is gone, so resuming it would start the agent
    /// somewhere else — or not at all.
    MissingProject,
    /// The native file cannot be opened. Moved, deleted, or on a volume that
    /// is not mounted.
    UnreadableFile,
    /// The file opens but holds no JSON: truncated by a full disk, or a
    /// format this Prelude cannot read.
    MalformedIndex,
    /// A native session directory exists but will not open. This is the
    /// failure with no other symptom: every conversation under it is simply
    /// absent, and a report that walked only the sessions it was handed would
    /// look at an empty inventory and call it healthy.
    UnreadableRoot,
    /// Prelude's own `sessions.json` overlay will not parse, so every local
    /// name, pin and archive flag is currently being ignored.
    UnreadableMetadata,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The stable Session id. Empty for `UnreadableMetadata`, which belongs
    /// to no single conversation.
    pub session: String,
    pub trouble: Trouble,
    /// The path or the operating system's own words. Never conversation
    /// content.
    pub detail: String,
}

/// How much of a session file to read before calling it unparseable.
///
/// Enough to clear any header a format puts first, small enough that checking
/// six hundred conversations is a few hundred short reads.
const INDEX_PROBE_LINES: usize = 20;

/// Everything wrong with this list of conversations, and nothing repaired.
///
/// Diagnostic only, and deliberately not on any hot path: it stats a directory
/// and opens a file per conversation, which is far too much for a gather and
/// pointless per keystroke. A person asks; then it answers.
pub fn session_problems(sessions: &[Item]) -> Vec<Problem> {
    // The strict reader already knows exactly why the overlay would not load;
    // restating that here would be a second opinion able to disagree with the
    // one the writes use.
    session_problems_in(sessions, &paths::home(), read_metadata_result().err())
}

/// The home root and the overlay's verdict are parameters so a test can pin
/// every branch — including the unreadable one — without reading the person's
/// real `sessions.json` or their real `~/.claude`.
fn session_problems_in(
    sessions: &[Item],
    home: &Path,
    metadata_error: Option<String>,
) -> Vec<Problem> {
    use std::io::{BufRead, BufReader};
    let mut out = Vec::new();
    if let Some(detail) = metadata_error {
        out.push(Problem {
            session: String::new(),
            trouble: Trouble::UnreadableMetadata,
            detail,
        });
    }
    // Before the conversations, the shelves they sit on. `scan_claude` and
    // `walk_jsonl` both bail silently when a root will not open — that is the
    // right answer for a source, which degrades to nothing, and exactly the
    // wrong one for a diagnostic: a `chmod 000` on `~/.claude/projects` takes
    // the entire Claude inventory out of the launcher, and the only trace left
    // is the absence itself. A directory that is not there at all is not a
    // fault; it means that agent has never run here.
    for root in native_session_roots(home) {
        if root.exists() {
            if let Err(e) = std::fs::read_dir(&root) {
                out.push(Problem {
                    session: String::new(),
                    trouble: Trouble::UnreadableRoot,
                    detail: format!("{}: {e}", root.display()),
                });
            }
        }
    }
    for session in sessions {
        let id = session.get("session_id").to_string();
        let cwd = session.get("cwd");
        if !cwd.is_empty() && !Path::new(cwd).is_dir() {
            out.push(Problem {
                session: id.clone(),
                trouble: Trouble::MissingProject,
                detail: cwd.to_string(),
            });
        }
        let file = session.get("file");
        if file.is_empty() {
            continue;
        }
        let opened = match std::fs::File::open(file) {
            Ok(opened) => opened,
            Err(e) => {
                out.push(Problem {
                    session: id,
                    trouble: Trouble::UnreadableFile,
                    detail: format!("{file}: {e}"),
                });
                continue;
            }
        };
        // Line-wise first, because most of these are JSONL transcripts and a
        // bounded probe of the head is all that is affordable across hundreds
        // of files. Whole-document second, because not every Agent stores a
        // conversation that way: Kimi and Cursor keep a small pretty-printed
        // JSON sidecar, in which *no single line* is valid JSON. Probing only
        // line-wise reported every one of them as malformed — seventeen
        // perfectly good sessions, in the report whose job is to tell real
        // damage from noise.
        let readable = BufReader::new(opened)
            .lines()
            .take(INDEX_PROBE_LINES)
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .any(|line| serde_json::from_str::<serde_json::Value>(&line).is_ok())
            || crate::paths::read_bounded(Path::new(file), 64 * 1024)
                .is_some_and(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).is_ok());
        if !readable {
            out.push(Problem {
                session: id,
                trouble: Trouble::MalformedIndex,
                detail: file.to_string(),
            });
        }
    }
    out
}

/// The non-interactive invocation for an agent, if we know one.
pub fn ask_cmd(agent: &str, prompt: &str) -> Option<Vec<String>> {
    crate::agent::get(agent)?.ask(prompt)
}

/// Start a fresh agent session in a directory, optionally invoking a skill.
pub fn start_cmd(agent: &str, cwd: Option<&str>, prompt: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(d) = cwd {
        s.push_str(&format!("cd {} && ", shq(d)));
    }
    match prompt {
        Some(p) => {
            let spec = crate::agent::get(agent);
            s.push_str(&match spec {
                Some(spec) => spec.prompt(p),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Kind;

    /// None of these tests sets `XDG_DATA_HOME`, and that is deliberate:
    /// environment variables are process-wide while `cargo test` runs these
    /// alongside every other test in the binary on parallel threads, so two
    /// tests sharing one variable race and the loser reads the person's real
    /// data directory. Everything that would have needed the variable takes
    /// what it would have read as a parameter instead — `export_transcript_into`
    /// the export directory and the home root, `session_problems_in` the home
    /// root and the overlay's verdict — and each test that touches the disk
    /// owns a temporary directory named after itself and this process.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("prelude-sessions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn session(agent: &str, id: &str, file: &Path) -> Item {
        Item::new(format!("{agent} resume"), Kind::Session)
            .title("a conversation")
            .put("agent", agent)
            .put("session_id", format!("{agent}:{id}"))
            .put("id", id)
            .put("file", file.to_string_lossy().into_owned())
    }

    fn write_session(home: &Path, relative: &str, lines: &[&str]) -> PathBuf {
        let path = home.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
        path
    }

    #[test]
    fn a_transcript_export_reads_every_native_format_and_keeps_secrets_out() {
        let root = scratch("transcript");
        let home = root.join("home");
        let exports = root.join("exports");

        let claude_file = write_session(&home, ".claude/projects/proj/abc.jsonl", &[
            r#"{"type":"user","timestamp":"2026-08-08T10:00:00Z","message":{"role":"user","content":"please read the file"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"private-reasoning"},{"type":"tool_use","name":"Read","input":{"file_path":"/dangerous-argument"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"tool-output-body"}]}}"#,
            // Claude Code records its own harness material as a `user` turn:
            // a slash command's name and arguments, the shell output it
            // produced, and whatever it injected mid-conversation. Every one
            // of these is wrapped, and every one of them must go.
            r#"{"type":"user","message":{"role":"user","content":"<local-command-caveat>Caveat: the messages below were generated by the user while running local commands.</local-command-caveat>\n<command-name>/deploy</command-name>\n<command-message>deploy-message</command-message>\n<command-args>--production</command-args>"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<local-command-stdout>shell-output-body</local-command-stdout>"}}"#,
            r#"{"type":"user","message":{"role":"user","content":"<system-reminder>injected-skill-document</system-reminder>ship it"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"all done\napi_key=sk-0123456789abcdefghijklmno"}]}}"#,
        ]);
        let claude = session("claude", "abc", &claude_file).put("cwd", "/tmp");
        let exported = export_transcript_into(&claude, &exports, &home).unwrap();
        let md = std::fs::read_to_string(&exported).unwrap();
        assert!(md.contains("## user"), "{md}");
        assert!(md.contains("please read the file"), "{md}");
        assert!(md.contains("- called tool: Read"), "{md}");
        assert!(md.contains("all done"), "{md}");
        assert!(md.contains("[redacted:"), "a credential line must not be exported: {md}");
        assert!(!md.contains("sk-0123456789"), "{md}");
        assert!(!md.contains("private-reasoning"), "{md}");
        assert!(!md.contains("dangerous-argument"), "tool arguments are named, never dumped: {md}");
        assert!(!md.contains("tool-output-body"), "{md}");
        for injected in [
            "Caveat:",
            "/deploy",
            "deploy-message",
            "--production",
            "shell-output-body",
            "injected-skill-document",
        ] {
            assert!(
                !md.contains(injected),
                "claude delivers harness material as a `user` turn; {injected} is not \
                 something a person said: {md}",
            );
        }
        assert!(
            md.contains("ship it"),
            "what the person actually typed beside an injection stays: {md}",
        );
        assert_eq!(
            md.matches("## user").count(),
            2,
            "a turn that was nothing but injections is dropped, like `developer`: {md}",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&exported).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "an export is private");
            let dir_mode = std::fs::metadata(&exports).unwrap().permissions().mode() & 0o777;
            assert_eq!(dir_mode, 0o700);
        }

        // Eight credential shapes a person would actually paste into a
        // conversation, each in its own block so that catching one cannot
        // stand in for catching another. Two of them are the case a
        // line-at-a-time test cannot see: the label is typed and the value is
        // pasted on the line below it.
        let pasted_file = write_session(&home, ".claude/projects/proj/pasted.jsonl", &[
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"notes\n\nmy API key is below\nhunter2-plaintext-value\n\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n\nDATABASE_URL=postgres://admin:hunter2@db.internal/app\n\nexport GITHUB_PAT=github_pat_11ABCDEFG0123456789abcdefghijklmnopqrstuvwxyz\n\nAIzaSyA1234567890abcdefghijklmnopqrstuv\n\neyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk\n\nthe export continues"}]}}"#,
        ]);
        let md = std::fs::read_to_string(
            export_transcript_into(&session("claude", "pasted", &pasted_file), &exports, &home).unwrap(),
        ).unwrap();
        for leaked in [
            "hunter2",
            "b3BlbnNzaC1rZXktdjEAAAAA",
            "github_pat_11",
            "AIzaSy",
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
        ] {
            assert!(!md.contains(leaked), "{leaked} was exported: {md}");
        }
        assert_eq!(
            md.matches("[redacted:").count(),
            6,
            "each shape must be caught on its own; a block is redacted with the \
             line that named it, because nobody writes the value on that line: {md}",
        );
        assert!(md.contains("notes"), "{md}");
        assert!(
            md.contains("the export continues"),
            "a blank line ends a redacted block, or one key would eat the transcript: {md}",
        );

        let codex_file = write_session(&home, ".codex/sessions/2026/rollout-1.jsonl", &[
            r#"{"timestamp":"t","type":"session_meta","payload":{"session_id":"cx","cwd":"/tmp"}}"#,
            r#"{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello codex"}]}}"#,
            r#"{"timestamp":"t","type":"event_msg","payload":{"type":"user_message","message":"hello codex"}}"#,
            r#"{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"harness-instructions"}]}}"#,
            r#"{"timestamp":"t","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"dangerous-argument"}}"#,
            r#"{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"codex answered"}]}}"#,
        ]);
        let codex = session("codex", "cx", &codex_file);
        let md = std::fs::read_to_string(export_transcript_into(&codex, &exports, &home).unwrap()).unwrap();
        assert_eq!(md.matches("hello codex").count(), 1, "codex records each turn twice: {md}");
        assert!(md.contains("- called tool: shell"), "{md}");
        assert!(md.contains("codex answered"), "{md}");
        assert!(!md.contains("harness-instructions"), "a developer turn is the harness, not the person");
        assert!(!md.contains("dangerous-argument"), "{md}");

        let pi_file = write_session(&home, ".pi/agent/sessions/proj/1.jsonl", &[
            r#"{"type":"session","id":"p1","cwd":"/tmp"}"#,
            r#"{"type":"message","message":{"role":"user","content":[{"type":"text","text":"hello pi"}]}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"thinking","thinking":"private-reasoning"},{"type":"toolCall","name":"bash","arguments":"dangerous-argument"}]}}"#,
            r#"{"type":"message","message":{"role":"toolResult","toolName":"bash","content":[{"type":"text","text":"tool-output-body"}]}}"#,
        ]);
        let pi = session("pi", "p1", &pi_file);
        let md = std::fs::read_to_string(export_transcript_into(&pi, &exports, &home).unwrap()).unwrap();
        assert!(md.contains("hello pi"), "{md}");
        assert!(md.contains("- called tool: bash"), "{md}");
        assert!(!md.contains("tool-output-body"), "{md}");
        assert!(!md.contains("private-reasoning"), "{md}");

        // A format that cannot be read says so. An empty file that looked
        // like an empty conversation would be the worse answer.
        let broken = write_session(&home, ".pi/agent/sessions/proj/2.jsonl", &["not json at all"]);
        assert!(export_transcript_into(&session("pi", "p2", &broken), &exports, &home).is_err());
        let shapeless = write_session(&home, ".pi/agent/sessions/proj/3.jsonl", &[r#"{"type":"session","id":"p3"}"#]);
        assert!(export_transcript_into(&session("pi", "p3", &shapeless), &exports, &home).is_err());
        // The trash boundary applies to exports too: only a native file.
        let outside = write_session(&home, "notes/4.jsonl", &[r#"{"type":"message"}"#]);
        assert!(export_transcript_into(&session("pi", "p4", &outside), &exports, &home).is_err());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_conversation_recorded_twice_is_reported_both_ways_round() {
        let one = session("claude", "abc", Path::new("/a/abc.jsonl")).put("ts", "100");
        let two = session("claude", "abc", Path::new("/b/abc.jsonl")).put("ts", "200");
        let three = session("pi", "p1", Path::new("/c/shared.jsonl")).put("ts", "300");
        let four = session("pi", "p2", Path::new("/c/shared.jsonl")).put("ts", "400");
        let alone = session("codex", "cx", Path::new("/d/cx.jsonl")).put("ts", "500");

        let found = duplicate_sessions(&[one.clone(), two.clone(), three, four, alone, one.clone()]);
        assert_eq!(found.len(), 2, "{found:?}");
        let by_id = found.iter().find(|d| d.kind == DuplicateKind::SameIdManyFiles).unwrap();
        assert_eq!(by_id.ids, vec!["claude:abc".to_string()]);
        assert_eq!(by_id.paths, vec!["/a/abc.jsonl".to_string(), "/b/abc.jsonl".to_string()]);
        assert_eq!(by_id.newest, "/b/abc.jsonl", "a report has to say which one is live");
        let by_file = found.iter().find(|d| d.kind == DuplicateKind::SameFileManyIds).unwrap();
        assert_eq!(by_file.paths, vec!["/c/shared.jsonl".to_string()]);
        assert_eq!(by_file.ids, vec!["pi:p1".to_string(), "pi:p2".to_string()]);
        assert_eq!(by_file.newest, "pi:p2");
        assert!(duplicate_sessions(&[two]).is_empty());
    }

    #[test]
    fn broken_conversations_are_named_without_being_repaired() {
        let root = scratch("problems");
        // A home of this test's own. `session_problems` reads the person's
        // real `sessions.json` and their real `~/.claude`; nothing here may.
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".pi/agent/sessions")).unwrap();
        let good = root.join("good.jsonl");
        std::fs::write(&good, "{\"type\":\"session\"}\n").unwrap();
        let empty = root.join("empty.jsonl");
        std::fs::write(&empty, "not json\nstill not json\n").unwrap();

        let healthy = session("claude", "ok", &good).put("cwd", root.to_string_lossy().into_owned());
        let gone = session("claude", "gone", &root.join("missing.jsonl"))
            .put("cwd", root.join("no-such-project").to_string_lossy().into_owned());
        let malformed = session("pi", "bad", &empty);

        let found = session_problems_in(&[healthy.clone(), gone, malformed], &home, None);
        let troubles: Vec<(&str, &Trouble)> =
            found.iter().map(|p| (p.session.as_str(), &p.trouble)).collect();
        assert!(troubles.contains(&("claude:gone", &Trouble::MissingProject)), "{found:?}");
        assert!(troubles.contains(&("claude:gone", &Trouble::UnreadableFile)), "{found:?}");
        assert!(troubles.contains(&("pi:bad", &Trouble::MalformedIndex)), "{found:?}");
        assert!(!troubles.iter().any(|(id, _)| *id == "claude:ok"), "{found:?}");
        // A root that is simply absent is not a fault: that agent has never
        // run here. Only one of the four exists in this home.
        assert!(
            !found.iter().any(|p| p.trouble == Trouble::UnreadableRoot),
            "{found:?}",
        );
        // Nothing was touched: this reports.
        assert!(good.exists() && empty.exists());
        assert!(session_problems_in(std::slice::from_ref(&healthy), &home, None).is_empty());
        let overlay = session_problems_in(std::slice::from_ref(&healthy), &home, Some("bad json".into()));
        assert_eq!(overlay.len(), 1);
        assert_eq!(overlay[0].trouble, Trouble::UnreadableMetadata);
        assert!(overlay[0].session.is_empty(), "the overlay belongs to no one conversation");

        // The failure with no other symptom. A native root that will not open
        // removes every conversation under it from the launcher, silently,
        // because both scanners degrade to nothing — so a diagnostic that only
        // walked the sessions it was handed reported a clean bill of health on
        // an empty inventory.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = home.join(".claude/projects");
            std::fs::create_dir_all(&locked).unwrap();
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            // root ignores the mode bits, and CI sometimes runs as root. The
            // check is then unenforceable rather than failing, so say so and
            // move on.
            if std::fs::read_dir(&locked).is_ok() {
                eprintln!("skipping the unreadable-root check: this user can read 0o000");
            } else {
                let found = session_problems_in(std::slice::from_ref(&healthy), &home, None);
                let root_faults: Vec<&Problem> = found
                    .iter()
                    .filter(|p| p.trouble == Trouble::UnreadableRoot)
                    .collect();
                assert_eq!(root_faults.len(), 1, "{found:?}");
                assert!(
                    root_faults[0].detail.contains(".claude/projects"),
                    "a report has to name the root: {found:?}",
                );
            }
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn session_filters_are_parsed_before_a_single_conversation_is_read() {
        let (f, needles) = parse_filters("is:pinned project:Prelude agent:Claude since:24h auth fix");
        assert!(f.pinned);
        assert_eq!(f.projects, vec!["prelude".to_string()]);
        assert_eq!(f.agents, vec!["claude".to_string()]);
        assert_eq!(f.since, Some(86_400.0));
        assert_eq!(needles, vec!["auth".to_string(), "fix".to_string()]);
        assert!(f.unknown.is_empty());

        let (f, needles) = parse_filters("since:banana project: is:yesterday http://example.com");
        assert_eq!(f.since, None);
        assert!(f.projects.is_empty());
        assert_eq!(
            f.unknown,
            vec!["since:banana".to_string(), "project:".to_string(), "is:yesterday".to_string()],
        );
        assert_eq!(
            needles,
            vec!["http://example.com".to_string()],
            "an ordinary word with a colon in it is still a search word",
        );

        assert_eq!(parse_filters("since:30m").0.since, Some(1_800.0));
        assert_eq!(parse_filters("since:7d").0.since, Some(604_800.0));
        assert_eq!(parse_filters("since:2w").0.since, Some(1_209_600.0));
        assert_eq!(parse_filters("since:7").0.since, None, "a bare number is ambiguous");
        let (f, _) = parse_filters("since:0d");
        assert_eq!(f.since, None, "a zero window can hold nothing and explains nothing");
        assert_eq!(f.unknown, vec!["since:0d".to_string()]);

        // A project name with a space in it is one word if it was quoted;
        // split on whitespace it becomes a filter for "my" plus a stray needle.
        let (f, needles) = parse_filters("project:\"Patent Exam\" fix");
        assert_eq!(f.projects, vec!["patent exam".to_string()]);
        assert_eq!(needles, vec!["fix".to_string()]);
        assert!(f.unknown.is_empty());
        assert!(in_project("/Users/me/Patent Exam", "patent exam"));
        assert_eq!(parse_filters("project:'my project'").0.projects, vec!["my project".to_string()]);
        assert_eq!(
            parse_filters("don't panic").1,
            vec!["don't".to_string(), "panic".to_string()],
            "an apostrophe is a letter until it is closed; this runs on every keystroke",
        );
    }

    #[test]
    fn session_filters_group_by_project_agent_and_time() {
        let now = 1_000_000.0;
        let mut items = vec![
            session("claude", "a", Path::new("/x/a.jsonl"))
                .put("cwd", "/Users/me/App/Prelude")
                .put("ts", format!("{}", now - 60.0)),
            session("codex", "b", Path::new("/x/b.jsonl"))
                .put("cwd", "/Users/me/App/Prelude")
                .put("ts", format!("{}", now - 90_000.0)),
            session("claude", "c", Path::new("/x/c.jsonl"))
                .put("cwd", "/Users/me/App/Preludex")
                .put("ts", format!("{}", now - 30.0)),
        ];
        items[2].title = "another conversation".into();

        let run = |term: &str| {
            let (f, needles) = parse_filters(term);
            apply_filters(items.clone(), &f, &needles, now)
                .into_iter()
                .map(|it| it.get("id").to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(run("project:prelude"), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(
            run("project:prelu"),
            Vec::<String>::new(),
            "an exact project name must never widen into a substring match",
        );
        assert_eq!(run("project:/users/me/app/preludex"), vec!["c".to_string()]);
        assert_eq!(run("agent:claude"), vec!["a".to_string(), "c".to_string()]);
        assert_eq!(run("agent:claude agent:codex").len(), 3, "two agents are an or");
        assert_eq!(run("since:24h"), vec!["a".to_string(), "c".to_string()]);
        assert_eq!(run("since:24h project:prelude agent:claude"), vec!["a".to_string()]);
        assert!(run("project:prelude agent:pi").is_empty());

        // A filter nobody can honour must not answer the question anyway.
        // Dropped, `since:banana` returned every conversation and looked
        // exactly like a filter that had worked.
        assert_eq!(run("").len(), 3);
        for nonsense in ["since:banana", "since:0d", "since:7", "is:yesterday", "project:"] {
            assert!(
                run(nonsense).is_empty(),
                "{nonsense} is not a filter this can apply, and it is not everything either",
            );
        }
    }

    #[test]
    fn archiving_gives_way_to_a_conversation_somebody_has_resumed() {
        let archived = session("claude", "abc", Path::new("/x/a.jsonl")).put("archived", "true");
        assert!(!visible(&archived), "an archived conversation is put away");
        let resumed = archived.clone().put("active_run", "claude:7:1");
        assert!(
            visible(&resumed),
            "archive says a conversation is finished; a live Run says it is not, \
             and the machine outranks a label set last week",
        );
        assert!(visible(&session("claude", "abc", Path::new("/x/a.jsonl"))));
        // The flag itself is untouched, so the row goes back to being hidden
        // when the Run exits — and `s:is:archived` still finds it meanwhile.
        assert_eq!(resumed.get("archived"), "true");
        let (f, needles) = parse_filters("is:archived");
        assert_eq!(apply_filters(vec![resumed.clone()], &f, &needles, 0.0).len(), 1);

        // The default `s:` view has to give the same answer as `visible`, or
        // the one scope whose job is finding every conversation is the one
        // place a resumed conversation disappears from.
        let show = |rows: Vec<Item>, term: &str| filter_search(rows, term).len();
        assert_eq!(show(vec![resumed.clone()], ""), 1, "s: must agree with the launcher home");
        assert_eq!(show(vec![archived.clone()], ""), 0);
        assert_eq!(show(vec![archived], "is:archived"), 1, "a flag question answers literally");
        assert_eq!(show(vec![resumed], "is:active"), 1);
    }
}
