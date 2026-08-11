//! One surface over the skills and MCP servers of every agent CLI installed.
//! Each agent keeps them somewhere different; this is the union.

use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::paths;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Duration;

pub(crate) fn skill_dirs() -> Vec<(std::path::PathBuf, &'static str)> {
    let h = paths::home();
    vec![
        (h.join(".claude/skills"), "claude"),
        (h.join(".agents/skills"), "shared"),
        (h.join(".codex/skills"), "codex"),
        (h.join(".pi/agent/skills"), "pi"),
        (h.join(".config/opencode/skills"), "opencode"),
    ]
}

#[derive(Default)]
struct Skill {
    agents: Vec<&'static str>,
    dir: String,
    file: String,
    desc: String,
    /// Every copy, one per agent that has it.
    ///
    /// `dir` is only the first one found, which is all borrowing and copying
    /// ever needed. Deleting needs the rest: the whole point of merging by
    /// name is that one row can stand for four directories, and a "delete"
    /// that silently took one of them would leave the row on screen and look
    /// like it had failed.
    copies: Vec<(&'static str, String)>,
    copy_info: Vec<crate::capability::SkillCopy>,
}

/// Where each agent keeps its copy of a skill, off the row.
pub fn copies_of(it: &Item) -> Vec<(String, String)> {
    serde_json::from_str(it.get("copies")).unwrap_or_default()
}

/// Every directory this row stands for, once each and in discovery order.
///
/// `dir` is only ever the first copy found, which is all borrowing and
/// copying ever needed and not enough to *act on all of them*: a skill merged
/// across four agents is four directories behind one row, and an "open all
/// copies" that quietly opened one would look like it had failed. This is
/// that target, made explicit — the copy list, backed by the hashed copies so
/// a row written before one of the two fields existed still answers
/// completely, and never a comma-joined agent list masquerading as an agent.
pub fn copy_paths(item: &Item) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    let hashed = crate::capability::copies(item).into_iter().map(|copy| (copy.agent, copy.dir));
    for (agent, dir) in copies_of(item).into_iter().chain(hashed) {
        if !dir.is_empty() && seen.insert(dir.clone()) {
            out.push((agent, dir));
        }
    }
    if out.is_empty() {
        let dir = item.get("dir");
        let agent = item.get("agent").split(',').next().unwrap_or_default().trim();
        if !dir.is_empty() {
            out.push((agent.to_string(), dir.to_string()));
        }
    }
    out
}

/// What one invocation is worth.
///
/// Wide enough that a single one clears anything the launcher's own frecency
/// can add (`MAX_BONUS`, 60), because the two are not the same evidence.
/// Selecting a skill row in Prelude may mean reading its instructions,
/// installing it, or preparing a one-off loan. Actually invoking it in a
/// conversation is the only unambiguous statement that this is a skill you
/// use, so it decides, and clicks only separate skills you use equally.
const PER_USE: f64 = 100.0;

/// How much recency can move a skill *without* crossing a count.
const RECENCY: f64 = 60.0;

/// A skill's place among skills: how often you actually invoke it.
///
/// Count first and by a wide margin — the column has said `used 8× · 1d ago`
/// all along, and sorting by anything else makes that number decorative.
/// Recency then separates skills you reach for equally often, but is bounded
/// below `PER_USE` so it can never lift a skill over one you have used more.
/// That ordering is stable in a way a decayed count is not: a skill used
/// eight times over a month is *yours*, and should not fall behind one used
/// once yesterday.
pub fn usage_rank(times: u32, last: f64) -> f64 {
    if times == 0 {
        return 0.0;
    }
    let days = (crate::frecency::now() - last).max(0.0) / 86_400.0;
    times as f64 * PER_USE + RECENCY / (1.0 + days)
}

/// Skills across every agent, merged by name and ordered by use.
///
/// The same skill commonly lives in several agents' directories. Listing it
/// once and naming the agents that have it is more useful than either three
/// duplicate rows or silently dropping two of them.
pub fn skills() -> Vec<Item> {
    let mut skills = skills_with(&crate::cache::read_cached("sessions"));
    crate::archive::decorate(&mut skills);
    skills
}

/// The same, for a caller that already holds the session list.
///
/// Ranking a skill means counting the conversations that invoked it, and the
/// session cache is the largest thing `gather` reads. Passing it in is what
/// keeps a gather from parsing it once for the skills, once for the agent
/// summary, and once again for the recent list.
pub fn skills_with(sessions: &[Item]) -> Vec<Item> {
    let hashes: std::collections::HashMap<String, crate::capability::SkillCopy> =
        crate::cache::read_cached("skill-hashes")
            .iter()
            .map(|item| (item.get("dir").to_string(), crate::capability::copy_from_item(item)))
            .collect();
    let mut merged: BTreeMap<String, Skill> = BTreeMap::new();
    for (dir, agent) in skill_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut subs: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        subs.sort();
        for sub in subs {
            let base = sub.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // e.g. codex's internal .system directory
            if !sub.is_dir() || base.starts_with('.') {
                continue;
            }
            let md = ["SKILL.md", "skill.md"]
                .iter()
                .map(|n| sub.join(n))
                .find(|p| p.exists());
            let (name, desc) = match &md {
                Some(p) => frontmatter(p, &base),
                None => (base.clone(), String::new()),
            };
            let rec = merged.entry(name).or_default();
            if !rec.agents.contains(&agent) {
                rec.agents.push(agent);
            }
            if rec.desc.is_empty() && !desc.is_empty() {
                rec.desc = desc;
            }
            let sub_path = sub.to_string_lossy().into_owned();
            rec.copies.push((agent, sub_path.clone()));
            rec.copy_info.push(hashes.get(&sub_path).cloned().unwrap_or_else(|| {
                crate::capability::SkillCopy { agent: agent.into(), dir: sub_path, ..Default::default() }
            }));
            if rec.file.is_empty() {
                if let Some(p) = &md {
                    rec.file = p.to_string_lossy().into_owned();
                    rec.dir = sub.to_string_lossy().into_owned();
                }
            }
        }
    }
    let names: Vec<String> = merged.keys().cloned().collect();
    let usage = super::sessions::skill_usage(&names, sessions);
    merged
        .into_iter()
        .map(|(name, rec)| {
            let (times, last) = usage.get(&name.to_lowercase()).copied().unwrap_or((0, 0.0));
            let used = if times == 0 {
                "never used".to_string()
            } else {
                format!("used {times}× · {}", super::user::ago(last))
            };
            let agents = rec.agents.join(", ");
            let missing = missing_agents(&agents);
            let unique: std::collections::BTreeSet<&str> = rec.copy_info.iter()
                .map(|copy| copy.fingerprint.as_str())
                .filter(|hash| !hash.is_empty())
                .collect();
            // The five states live in `capability`, because the row, the
            // Quick Look matrix and `doctor skills` must not each hold their
            // own idea of what "identical" means.
            let integrity = crate::capability::integrity(&rec.copy_info);
            let mut notes = Vec::new();
            if integrity == "divergent" {
                notes.push("divergent copies".to_string());
            } else if integrity == "private-unknown" {
                notes.push("private lines omitted".to_string());
            }
            if !missing.is_empty() {
                notes.push(format!("missing: {}", missing.join(", ")));
            }
            let gap = notes.join(" · ");
            let fingerprint = if unique.len() == 1 {
                unique.iter().next().copied().unwrap_or("")
            } else {
                ""
            };
            let source_sensitive = rec.copy_info.iter()
                .find(|copy| copy.dir == rec.dir)
                .is_some_and(|copy| copy.sensitive_files > 0);
            Item::new(format!("/{name}"), Kind::Skill)
                .rank(usage_rank(times, last))
                .title(&name)
                .fields([agents.clone(), used, gap, rec.desc.clone()])
                .put("missing", missing.join(","))
                .put("name", &name)
                .put("agent", agents)
                .put("dir", rec.dir)
                .put("file", rec.file)
                .put("copies", serde_json::to_string(&rec.copies).unwrap_or_default())
                .put("copy_info", serde_json::to_string(&rec.copy_info).unwrap_or_default())
                .put("integrity", integrity)
                .put("fingerprint", fingerprint)
                .put("source_sensitive", source_sensitive.to_string())
                .put("desc", rec.desc)
        })
        .collect()
}

/// May a previously cached record stand in for walking this tree again?
///
/// Two independent questions, and they have to stay independent.
///
/// *Is the tree unchanged* is the stamp: a cheap metadata walk that skips the
/// expensive one. *Is the record complete* is `capability::RECORD`, an
/// explicit schema version, because the walk is exactly what an unchanged
/// stamp skips — so a field added after a cache was written would never once
/// be computed for a Skill nobody edits, and would read as zero for ever.
///
/// That second question used to be asked as `copy.modified > 0`, which is a
/// value the filesystem chooses rather than a version we control. A tree whose
/// newest mtime is at or before the epoch hashes to `modified == 0` and a
/// perfectly good fingerprint, so the gate rejected the record it had just
/// written — re-walking and re-hashing that whole tree on every refresh, for
/// ever, at a cost that grows with the tree. A derived value cannot be a
/// sentinel.
fn reusable(current_record: bool, copy: &crate::capability::SkillCopy, stamp: &str) -> bool {
    current_record && !stamp.is_empty() && copy.stamp == stamp && !copy.fingerprint.is_empty()
}

/// Full Skill-tree fingerprints. This is a background cache source: scripts,
/// references and symlinks are part of a capability, so hashing only
/// `SKILL.md` would call divergent copies identical.
pub fn skill_hashes() -> Vec<Item> {
    let previous: std::collections::HashMap<String, (bool, crate::capability::SkillCopy)> =
        crate::cache::read_cached("skill-hashes").iter()
            .map(|item| {
                let current = crate::capability::is_current_record(item);
                (item.get("dir").to_string(), (current, crate::capability::copy_from_item(item)))
            })
            .collect();
    let mut out = Vec::new();
    for (root, agent) in skill_dirs() {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        let mut dirs: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        dirs.sort();
        for dir in dirs.into_iter().filter(|dir| {
            dir.is_dir() && !dir.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'))
        }) {
            let path = dir.to_string_lossy().into_owned();
            let stamp = crate::capability::skill_stamp(&dir);
            let copy = match previous.get(&path).filter(|(current, copy)| reusable(*current, copy, &stamp)) {
                Some((_, copy)) => copy.clone(),
                None => {
                    let mut copy = crate::capability::hash_skill(agent, &dir);
                    copy.stamp = stamp;
                    copy
                }
            };
            out.push(crate::capability::cache_item(&copy));
        }
    }
    out
}

/// Remove a skill, recoverably.
///
/// This is the only destructive thing Prelude does to a user's files, and it
/// is built so that being wrong is survivable rather than so that it cannot
/// happen. Three rules:
///
/// **It goes to the Trash, never to `unlink`.** A skill is somebody's work —
/// often the only copy, often not in git. `rename` into `~/.Trash` costs the
/// same as deleting and leaves the whole directory sitting there to be
/// dragged back out. Nothing here is worth a permanent delete.
///
/// **The path must be a skill directory**, meaning a direct child of one of
/// the five directories `skill_dirs` knows about. The path arrives off a row
/// that has been through JSON and a shell, and a launcher that will remove
/// whatever it is handed is one malformed field away from removing something
/// else. This is the check that makes that impossible rather than unlikely.
///
/// **One copy at a time.** The caller names the agent, so deleting a skill
/// shared by four of them is four decisions, not one.
pub fn sync_skill(
    from: &str,
    to: &str,
    expected_source: &str,
    expected_target: &str,
) -> Result<std::path::PathBuf, String> {
    let source = std::path::Path::new(from);
    let target = std::path::Path::new(to);
    if source == target || !is_skill_dir(source) || !is_skill_dir(target) {
        return Err("both source and target must be distinct installed Skill directories".into());
    }
    let source_hash = crate::capability::hash_skill("source", source);
    let target_hash = crate::capability::hash_skill("target", target);
    if source_hash.fingerprint.is_empty() || target_hash.fingerprint.is_empty() {
        return Err("one of those Skill copies could not be read completely".into());
    }
    if source_hash.sensitive_files > 0 {
        return Err("the source contains credential-like material; refusing to copy it".into());
    }
    if source_hash.fingerprint != expected_source || target_hash.fingerprint != expected_target {
        return Err("a Skill copy changed after the comparison; compare again".into());
    }
    if source_hash.fingerprint == target_hash.fingerprint {
        return Err("those Skill copies are already identical".into());
    }
    let trashed = crate::paths::trash(target)?;
    if let Err(error) = copy_tree(source, target) {
        // A failed recursive copy must not leave a half-Skill looking
        // installed. It too goes to the Trash; the original target is still
        // recoverable at `trashed`.
        if target.exists() {
            let _ = crate::paths::trash(target);
        }
        return Err(format!(
            "copy failed: {error}; the previous target is safe at {}",
            crate::paths::tilde(&trashed.to_string_lossy())
        ));
    }
    Ok(trashed)
}

pub fn delete_skill(dir: &str) -> Result<std::path::PathBuf, String> {
    let path = std::path::Path::new(dir);
    if !is_skill_dir(path) {
        return Err(format!("{dir} is not a skill directory — refusing"));
    }
    crate::paths::trash(path)
}

/// Is this the directory of an installed skill, rather than some other path?
///
/// A direct child of a known skills directory, and nothing else — not the
/// skills directory itself, not something nested deeper, not a sibling
/// reached by `..`. Compared after `canonicalize`, so `~/.claude/skills/x/..`
/// cannot masquerade as one.
pub fn is_skill_dir(p: &std::path::Path) -> bool {
    let Ok(real) = p.canonicalize() else { return false };
    let Some(parent) = real.parent() else { return false };
    skill_dirs()
        .into_iter()
        .filter_map(|(d, _)| d.canonicalize().ok())
        .any(|d| d == parent)
}

/// What a SKILL.md header actually says, before any fallback is applied.
///
/// The display path below answers with the folder name when a header is
/// missing a `name:`, which is right for a row and useless for validation:
/// "there is no header" and "there is a header with no name" are different
/// faults with different fixes. Both come off this one parser, because two
/// parsers over one file eventually disagree — and that disagreement shows up
/// as a row rendering one name beside a diagnostic complaining about another.
pub(crate) struct Front {
    /// The file opens with `---`.
    pub opened: bool,
    /// …and the block is closed again.
    pub closed: bool,
    pub name: Option<String>,
    pub desc: Option<String>,
}

pub(crate) fn parse_front(text: &str) -> Front {
    // An editor that writes a UTF-8 BOM puts three bytes in front of the `---`,
    // and a header test spelled `starts_with("---")` then says the file has no
    // frontmatter at all — for a file every agent parses without complaint. The
    // launcher row would fall back to the folder name and `doctor skills` would
    // report a working Skill as broken. Stripping it changes nothing for any
    // other input: `trim_start_matches` on a string that does not start with
    // the BOM returns that same string.
    let text = text.trim_start_matches('\u{feff}');
    let mut front = Front { opened: false, closed: false, name: None, desc: None };
    let mut lines = text.lines();
    if !lines.next().is_some_and(|l| l.starts_with("---")) {
        return front;
    }
    front.opened = true;
    let mut in_desc = false;
    for line in lines {
        if line.starts_with("---") {
            front.closed = true;
            break;
        }
        if let Some(v) = line.strip_prefix("name:") {
            front.name = Some(v.trim().trim_matches(['\'', '"']).to_string());
            in_desc = false;
        } else if let Some(v) = line.strip_prefix("description:") {
            front.desc = Some(v.trim().trim_matches(['\'', '"']).to_string());
            in_desc = true;
        } else if in_desc && line.starts_with([' ', '\t']) {
            // folded continuation line
            if let Some(desc) = front.desc.as_mut() {
                desc.push(' ');
                desc.push_str(line.trim());
            }
        } else {
            in_desc = false;
        }
    }
    front
}

/// Pull name/description out of a SKILL.md YAML header, for display.
fn frontmatter(p: &std::path::Path, fallback: &str) -> (String, String) {
    let Ok(text) = std::fs::read_to_string(p) else {
        return (fallback.to_string(), String::new());
    };
    let front = parse_front(&text);
    (
        front.name.unwrap_or_else(|| fallback.to_string()),
        crate::width::flatten(&front.desc.unwrap_or_default()),
    )
}

/// MCP servers, with the status each agent actually reports.
///
/// Reading the config files was wrong: it missed every claude.ai-hosted
/// server (they are not in `mcpServers`) and every HTTP server codex keeps
/// elsewhere, and it could not tell you the one thing that matters — whether
/// the server works. A panel that lists an MCP server without saying it is
/// disabled or logged out is worse than no panel.
///
/// `claude mcp list` performs a network health check, so this is slow and
/// always runs behind the cache.
pub fn mcp() -> Vec<Item> {
    let checked_at = crate::frecency::now() as u64;
    let mut items = Vec::new();
    mcp_claude(&mut items, checked_at);
    mcp_codex(&mut items, checked_at);
    crate::mcp_tools::attach_cached(&mut items);
    enrich_mcp_matrix(&mut items);
    items
}

#[derive(Clone, Copy, PartialEq)]
pub enum Health {
    Ok,
    Disabled,
    NeedsAuth,
    Failed,
    Unknown,
}

impl Health {
    fn label(self) -> &'static str {
        match self {
            Health::Ok => "✔ connected",
            Health::Disabled => "⏸ disabled",
            Health::NeedsAuth => "⚠ not logged in",
            Health::Failed => "✘ failed",
            Health::Unknown => "· unknown",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Health::Ok => "ok",
            Health::Disabled => "disabled",
            Health::NeedsAuth => "auth",
            Health::Failed => "failed",
            Health::Unknown => "unknown",
        }
    }
}

pub(crate) fn safe_mcp_detail(detail: &str) -> String {
    let credential_url = detail.split_once("://").is_some_and(|(_, rest)| {
        rest.split('/').next().is_some_and(|authority| authority.contains('@'))
    });
    if credential_url || crate::secrets::looks_secret(detail) {
        "private target omitted".into()
    } else {
        detail.to_string()
    }
}

pub(crate) fn normalize_transport(raw: &str) -> &str {
    let low = raw.to_ascii_lowercase();
    if low.contains("stdio") { "stdio" }
    else if low.contains("sse") { "sse" }
    else if low.contains("http") { "http" }
    else if low.contains("hosted") { "hosted" }
    else { "unknown" }
}

fn mcp_item(
    agent: &str,
    name: &str,
    detail: &str,
    health: Health,
    transport: &str,
    checked_at: u64,
) -> Item {
    let detail = safe_mcp_detail(detail);
    let public_definition = serde_json::json!({
        "type": normalize_transport(transport),
        "display_target": detail.clone(),
    });
    Item::new(format!("{agent} mcp get {}", shq(name)), Kind::Mcp)
        .title(name)
        .fields([agent.to_string(), health.label().to_string(), detail.clone()])
        .put("agent", agent)
        .put("name", name)
        .put("health", health.key())
        .put("transport", normalize_transport(transport))
        .put("health_checked_at", checked_at.to_string())
        .put("portable", "false")
        .put("definition_hash", crate::capability::fingerprint(public_definition.to_string().as_bytes()))
        .put("definition_source", "display")
        .put("definition_public", public_definition.to_string())
}

/// `claude mcp list` prints `<name>: <target> - <status>` per server.
/// May this agent's answer replace what is already cached for it?
///
/// The question can only be settled *after* parsing, because the decisive
/// case is a command that succeeded in the shell's sense and said nothing an
/// inventory could be read out of. Three outcomes, and the middle one is what
/// an earlier version of this got wrong by asking too early:
///
/// * **exit 0, zero records** — an authoritative empty. An agent whose last
///   server was removed has to be able to say so, or the launcher shows it
///   forever.
/// * **non-zero exit, records parsed** — accept the records. Several of these
///   CLIs report a warning through their status while still printing a
///   perfectly good list.
/// * **non-zero exit, zero records** — not an answer. This is the shape of
///   `Error: authentication required`: a refusal, printed on either stream,
///   which read as "this agent now has no servers" and took every cached row
///   with it. Checking `stdout` for emptiness was not enough — the error text
///   *is* stdout, and it is not an inventory.
///
/// A run that never started or had to be killed is never an answer, whatever
/// it managed to print: a partial list must not replace a complete one.
pub(crate) fn trusted(probe: &crate::exec::Output, agent: &str, parsed: usize) -> bool {
    // Never finished. Whatever it printed is a fragment, and a fragment must
    // not replace a complete answer — which is the rule the two middle cases
    // here were quietly outside of. `truncated` and a `None` status both make
    // `ok()` false, so a run that hit the output cap or was killed by a
    // signal fell through to `refused` and passed it the moment it had parsed
    // one record. Four conditions, one meaning: we do not know what the rest
    // of the answer was.
    let unfinished =
        probe.spawn_failed || probe.timed_out || probe.truncated || probe.status.is_none();
    // Finished, and said no: exited non-zero with nothing convincing.
    let refused = !probe.ok() && parsed == 0;
    if unfinished || refused {
        crate::exec::note_incomplete(agent);
        return false;
    }
    true
}

/// One line of `claude mcp list`, read once for every caller that needs it.
///
/// There were two readings of this output — the inventory's and the tool
/// scanner's — and two readings of one format is one too many: they filtered
/// differently, counted differently, and only one of them had been taught
/// that an error message parses as a server. `Error: authentication required`
/// splits on `": "` exactly as an entry does, so the other reading not only
/// trusted it, it went on to run `claude mcp get Error` against a server
/// nobody has.
pub(crate) struct ClaudeServer<'a> {
    pub name: &'a str,
    pub target: &'a str,
    /// The health `claude mcp list` prints after ` - `, or empty when the
    /// line carried none.
    pub status: &'a str,
}

pub(crate) fn parse_claude_list(text: &str) -> Vec<ClaudeServer<'_>> {
    text.lines()
        .filter_map(|line| {
            let (name, rest) = line.trim().split_once(": ")?;
            if name.is_empty() || name.contains("Checking") || crate::secrets::looks_secret(name) {
                return None;
            }
            let (target, status) = match rest.rsplit_once(" - ") {
                Some((target, status)) => (target.trim(), status.trim()),
                None => (rest.trim(), ""),
            };
            Some(ClaudeServer { name, target, status })
        })
        .collect()
}

/// How much of that answer counts as evidence that it *is* an answer.
///
/// A clean exit is taken at its word, including an empty list — an agent
/// whose last server was removed has to be able to say so. A non-zero exit
/// has to show a health status, which is the part of the format an error
/// message cannot produce by accident. Rows without one are still displayed;
/// only the decision to replace the cache needs the stronger evidence, so a
/// format that stops printing statuses degrades to showing rows rather than
/// to erasing them.
pub(crate) fn claude_evidence(probe: &crate::exec::Output, servers: &[ClaudeServer]) -> usize {
    if probe.ok() {
        servers.len()
    } else {
        servers.iter().filter(|server| !server.status.is_empty()).count()
    }
}

fn mcp_claude(into: &mut Vec<Item>, checked_at: u64) {
    if crate::exec::require("claude").is_none() {
        return;
    }
    let probe = crate::exec::capture(&["claude", "mcp", "list"], Duration::from_secs(30));
    let text = probe.stdout_text();
    let servers = parse_claude_list(&text);
    if !trusted(&probe, "claude", claude_evidence(&probe, &servers)) {
        return;
    }
    let mut rows = Vec::new();
    for server in &servers {
        let low = server.status.to_lowercase();
        let health = if low.contains("connect") && !low.contains("fail") {
            Health::Ok
        } else if low.contains("pending") || low.contains("approve") {
            Health::NeedsAuth
        } else if low.is_empty() {
            Health::Unknown
        } else {
            Health::Failed
        };
        // claude.ai-hosted servers expose a display URL but their account
        // credentials are not a transferable local definition.
        let portable = !server.name.starts_with("claude.ai ");
        let transport = if !portable {
            "hosted"
        } else if server.target.starts_with("http://") || server.target.starts_with("https://") {
            "http"
        } else {
            "stdio"
        };
        rows.push(
            mcp_item("claude", server.name, server.target, health, transport, checked_at)
                .put("portable", portable.to_string()),
        );
    }
    into.extend(rows);
}

/// codex has --json, which also reports auth_status and why it is disabled.
fn mcp_codex(into: &mut Vec<Item>, checked_at: u64) {
    if crate::exec::require("codex").is_none() {
        return;
    }
    let probe = crate::exec::capture(&["codex", "mcp", "list", "--json"], Duration::from_secs(20));
    let text = probe.stdout_text();
    let mut rows = Vec::new();
    let out = &mut rows;
    let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
        // Bytes we cannot read are not "no servers". Something answered, and
        // we do not know what it said — the previous answer is still better
        // than replacing it with nothing.
        crate::exec::note_incomplete("codex");
        return;
    };
    for s in list {
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() || crate::secrets::looks_secret(name) {
            continue;
        }
        let enabled = s.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        let auth = s.get("auth_status").and_then(|v| v.as_str()).unwrap_or("");
        let health = if !enabled {
            Health::Disabled
        // codex reports this as `not_logged_in`, underscores and all.
        } else if auth.replace('_', " ").to_lowercase().contains("not logged") {
            Health::NeedsAuth
        } else {
            Health::Ok
        };
        let tr = s.get("transport");
        let detail = tr
            .and_then(|t| t.get("url").and_then(|u| u.as_str()).map(str::to_string))
            .or_else(|| tr.and_then(|t| t.get("command").and_then(|c| c.as_str()).map(str::to_string)))
            .or_else(|| tr.and_then(|t| t.get("type").and_then(|c| c.as_str()).map(str::to_string)))
            .unwrap_or_default();
        let detail = detail.rsplit('/').next().unwrap_or(&detail).to_string();
        let transport = tr.and_then(|transport| transport.get("type"))
            .and_then(|kind| kind.as_str()).unwrap_or("unknown");
        let mut it = mcp_item("codex", name, &detail, health, transport, checked_at);
        if let Some(r) = s.get("disabled_reason").and_then(|v| v.as_str()) {
            it = it.put("reason", r);
        }
        // Keep the definition itself, not just how it looks. It is the one
        // thing needed to lend the server to another agent, and it is free
        // here — the alternative is a second `mcp get` per server at the
        // moment someone asks, on a path that has to feel instant.
        if let Some(def) = tr.and_then(|t| crate::lend::Mcp::from_codex(name, t)) {
            let sensitive = def.has_sensitive_fields();
            it = it
                .put("portable", "true")
                .put("definition_hash", def.public_fingerprint())
                .put("definition_public", def.public_definition().to_string())
                .put("definition_source", "semantic")
                .put("sensitive", sensitive.to_string());
            // Complete definitions never enter an Item or cache. Even a
            // currently plain argument can become a credential after an
            // Agent upgrade; resolve from the owner CLI on explicit action.
        }
        out.push(it);
    }
    into.extend(rows);
}

fn enrich_mcp_matrix(items: &mut [Item]) {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        groups.entry(item.get("name").to_lowercase()).or_default().push(index);
    }
    let installed: Vec<&str> = crate::agent::installed()
        .into_iter()
        .filter(|agent| {
            crate::agent::get(agent).is_some_and(|spec| spec.capabilities.install_mcp)
        })
        .collect();
    for (name, indexes) in groups {
        let variants: Vec<crate::capability::McpVariant> = indexes.iter().map(|index| {
            let item = &items[*index];
            crate::capability::McpVariant {
                agent: item.get("agent").to_string(),
                health: item.get("health").to_string(),
                transport: item.get("transport").to_string(),
                health_checked_at: item.get("health_checked_at").parse().unwrap_or(0),
                summary: item.fields.get(2).cloned().unwrap_or_default(),
                fingerprint: item.get("definition_hash").to_string(),
                source: item.get("definition_source").to_string(),
                public_definition: serde_json::from_str(item.get("definition_public"))
                    .unwrap_or(serde_json::Value::Null),
                sensitive: item.get("sensitive") == "true",
                portable: item.get("portable") == "true",
                tools_status: item.get("tools_status").to_string(),
                tools_checked_at: item.get("tools_checked_at").parse().unwrap_or(0),
                tools: serde_json::from_str(item.get("tools")).unwrap_or_default(),
            }
        }).collect();
        let owners: Vec<String> = variants.iter().map(|variant| variant.agent.clone()).collect();
        let missing: Vec<&str> = installed.iter().copied()
            .filter(|agent| !owners.iter().any(|owner| owner == agent))
            .collect();
        let sources: std::collections::BTreeSet<&str> = variants.iter()
            .map(|variant| variant.source.as_str()).filter(|source| !source.is_empty()).collect();
        let hashes: std::collections::BTreeSet<&str> = variants.iter()
            .map(|variant| variant.fingerprint.as_str()).filter(|hash| !hash.is_empty()).collect();
        let known = variants.iter().filter(|variant| !variant.fingerprint.is_empty()).count();
        let comparison = if variants.len() == 1 {
            "single"
        } else if known != variants.len() {
            "unknown"
        } else if sources.len() != 1 {
            "incomparable"
        } else if hashes.len() == 1 && variants.iter().any(|variant| variant.sensitive) {
            "private-unknown"
        } else if hashes.len() == 1 {
            "identical"
        } else {
            "divergent"
        };
        let variants_json = serde_json::to_string(&variants).unwrap_or_default();
        let owners_json = serde_json::to_string(&owners).unwrap_or_default();
        let missing_json = serde_json::to_string(&missing).unwrap_or_default();
        for index in indexes {
            items[index].data.insert("capability_id".into(), format!("mcp:{name}"));
            items[index].data.insert("owners".into(), owners_json.clone());
            items[index].data.insert("missing_agents".into(), missing_json.clone());
            items[index].data.insert("variants".into(), variants_json.clone());
            items[index].data.insert("comparison".into(), comparison.into());
        }
    }
}

/// Which agents are *missing* a skill — the other half of "who has it".
///
/// Keeping `~/.claude/skills` and `~/.agents/skills` in sync is a real chore;
/// knowing where a skill exists means the gaps are computable, and copying it
/// across is a one-line action rather than a background sync daemon that
/// resurrects things you deliberately deleted.
pub fn missing_agents(have: &str) -> Vec<&'static str> {
    let have: Vec<&str> = have.split(',').map(str::trim).collect();
    skill_dirs()
        .into_iter()
        .filter(|(dir, agent)| dir.parent().is_some_and(|p| p.exists()) && !have.contains(agent))
        .map(|(_, agent)| agent)
        .collect()
}

pub fn skill_dir_for(agent: &str) -> Option<std::path::PathBuf> {
    skill_dirs().into_iter().find(|(_, a)| *a == agent).map(|(d, _)| d)
}

/// Copy a skill directory into another agent. Never overwrites.
pub fn copy_skill(from_dir: &str, agent: &str, name: &str) -> Result<String, String> {
    let Some(dest_root) = skill_dir_for(agent) else {
        return Err(format!("unknown agent {agent}"));
    };
    let dest = dest_root.join(name);
    if dest.exists() {
        return Err(format!("{agent} already has {name}"));
    }
    let source = std::path::Path::new(from_dir);
    let scan = crate::capability::hash_skill("source", source);
    if scan.fingerprint.is_empty() {
        return Err("the Skill could not be read completely".into());
    }
    if scan.sensitive_files > 0 {
        return Err("the Skill contains credential-like material; refusing to copy it".into());
    }
    std::fs::create_dir_all(&dest_root).map_err(|e| e.to_string())?;
    copy_tree(source, &dest).map_err(|e| e.to_string())?;
    Ok(crate::paths::tilde(&dest.to_string_lossy()))
}

pub fn copy_tree(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        if crate::capability::ignored_path(&e.path()) {
            continue;
        }
        let to = dst.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

/// Agent configuration files worth jumping to directly.
/// The file this agent is configured by, if it has one here.
///
/// Its *settings*, not its instructions: `CLAUDE.md` is prose you write for
/// it and is a row of its own in the main list, while this is the thing you
/// go looking for when the agent is behaving oddly.
pub fn config_for(agent: &str) -> Option<String> {
    crate::agent::get(agent)?
        .existing_settings()
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn configs() -> Vec<Item> {
    let h = crate::paths::home();
    let mut v: Vec<(std::path::PathBuf, &str)> = crate::agent::SPECS
        .iter()
        .map(|spec| (spec.settings_path(), spec.name))
        .collect();
    // Instructions and legacy/global files are useful Config objects but are
    // not an Agent's primary settings path, so they stay beside the registry
    // rather than pretending to be part of its invocation contract.
    v.extend([
        (h.join(".claude/CLAUDE.md"), "claude"),
        (h.join(".claude.json"), "claude"),
        (h.join(".codex/AGENTS.md"), "codex"),
    ]);
    // whatever the project you're standing in defines
    if let Some(root) = super::project::root() {
        for name in ["CLAUDE.md", "AGENTS.md", ".mcp.json"] {
            v.push((root.join(name), "this project"));
        }
    }
    v.into_iter()
        .filter(|(p, _)| p.exists())
        .map(|(p, what)| {
            let s = p.to_string_lossy().into_owned();
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Item::new(format!("$EDITOR {}", shq(&s)), Kind::Config)
                .title(name)
                .fields([what.to_string(), crate::paths::tilde(&s)])
                .put("agent", what)
                .put("path", s)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Effective configuration
// ---------------------------------------------------------------------------

/// How an Agent CLI answers "what settings is a run actually operating under".
///
/// `configs()` above lists *files*, and a file is not an answer. Every one of
/// these CLIs layers several sources — a home config, a project config,
/// environment variables, flags on the command line — so reading one of them
/// off disk and calling the result effective would be Prelude inventing
/// capability information. This is the same rule the MCP sources already
/// follow: ask the owner, or say you cannot.
///
/// The four installed CLIs were asked, and they do not agree:
///
/// **codex** has `codex doctor --json`, documented as "Emit a redacted
/// machine-readable report" and covering "installation, config, auth, and
/// runtime health". Its `checks["config.load"]` is genuinely *resolved* rather
/// than read: `codex doctor --json -c model="o3-fake-test"` reports
/// `"model": "o3-fake-test"`, and the same command run from another directory
/// reports that directory as `cwd`. That is an effective-config reporter.
///
/// **opencode** has `opencode debug config`, whose own help says "show
/// resolved configuration". It is directory-sensitive: a project holding an
/// `opencode.json` sees that file's `model` and `instructions` merged into the
/// object. Its values are the dangerous half — a `provider` block routinely
/// holds an API key — so only key *names* and a short allowlist of scalars are
/// carried out of it.
///
/// **claude** has no resolved-settings command at all. `claude config` is not
/// a subcommand: the CLI takes `config` as a *prompt* and starts answering it.
/// `claude doctor` reports the installation — version, package manager, update
/// channel — and no settings. The one thing that does report effective
/// configuration is `claude auto-mode config`, "Print the effective auto mode
/// config as JSON: your settings where set, defaults otherwise", and it covers
/// the auto-mode classifier alone. It is worth reporting as exactly that.
///
/// **pi** has `pi config`, which opens a TUI and has no non-interactive form
/// (`--json` is refused: "Unknown option --json"). Nothing else prints
/// settings. So pi gets no answer, said out loud, in the same voice its
/// per-provider login already uses.
///
/// None of these reports the configuration of *another process*. See
/// `RUN_SCOPE`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effective {
    /// `codex doctor --json` → `checks["config.load"]` and
    /// `checks["sandbox.helpers"]`.
    CodexDoctorJson,
    /// `opencode debug config` → the merged config object for the directory.
    OpencodeDebugConfig,
    /// `claude auto-mode config` → the auto-mode classifier's effective rules.
    ClaudeAutoMode,
    /// The CLI has no non-interactive resolved-settings command.
    None,
}

/// Why none of this is per-Run, said once so every caller says it the same way.
///
/// A Run's effective configuration is its agent's layered files *plus* the
/// flags it was started with *plus* the environment it inherited, and the last
/// two are exactly what Prelude may not keep: a full process command line can
/// hold a credential, which is why `running.rs` extracts a hint and throws the
/// rest away. Re-running the CLI in a Run's directory would answer a different
/// question — what a run started there *now* would resolve — and presenting
/// that as the Run's own configuration would be a guess wearing evidence's
/// clothes. No installed CLI reports another process's resolved settings, so
/// this evidence is agent-level and says so.
pub const RUN_SCOPE: &str = "agent-level, not run-level · no installed Agent CLI reports the \
                             resolved configuration of another process, and a Run's own flags and \
                             environment are never retained";

/// One key and one value that survived the credential filter.
#[derive(Clone, Debug, Serialize)]
pub struct ConfigFact {
    pub key: String,
    pub value: String,
}

/// What one Agent CLI said about its own effective configuration, and when.
#[derive(Clone, Debug, Serialize)]
pub struct ConfigEvidence {
    pub agent: String,
    /// Exactly what was asked, so the answer can be checked by hand. Absent
    /// when there was nothing to ask.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// How far the answer reaches: `directory`, `subsystem` or `none`. Never
    /// `run` — see `RUN_SCOPE`.
    pub scope: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    /// The CLI's own word for the state of its configuration load, where it
    /// has one. Never Prelude's opinion of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<ConfigFact>,
    /// The honest limitation, always present.
    pub note: String,
    /// Set when the CLI was asked and did not answer usefully.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trouble: Option<String>,
}

pub fn effective_kind(agent: &str) -> Effective {
    match crate::agent::get(agent).map(|spec| spec.capabilities.effective_config) {
        Some("directory") if agent == "codex" => Effective::CodexDoctorJson,
        Some("directory") if agent == "opencode" => Effective::OpencodeDebugConfig,
        Some("subsystem") => Effective::ClaudeAutoMode,
        _ => Effective::None,
    }
}

fn effective_command(kind: Effective) -> Option<&'static [&'static str]> {
    match kind {
        Effective::CodexDoctorJson => Some(&["codex", "doctor", "--json"]),
        Effective::OpencodeDebugConfig => Some(&["opencode", "debug", "config"]),
        Effective::ClaudeAutoMode => Some(&["claude", "auto-mode", "config"]),
        Effective::None => None,
    }
}

/// Ask one Agent CLI. A subprocess, and for codex a network round trip — this
/// belongs to `doctor agents` and to explicit commands, never to `gather`, a
/// cache source or the per-keystroke helper.
pub fn effective_config(agent: &str) -> ConfigEvidence {
    let kind = effective_kind(agent);
    let text = match effective_command(kind) {
        // `codex doctor` performs provider reachability and a websocket
        // handshake before it prints, which is why this one gets a minute.
        Some(argv) if kind == Effective::CodexDoctorJson => {
            crate::exec::run(argv, Duration::from_secs(60))
        }
        Some(argv) => crate::exec::run(argv, Duration::from_secs(30)),
        None => String::new(),
    };
    let mut evidence = read_effective(kind, agent, &text);
    if kind != Effective::None {
        evidence.checked_at = Some(crate::frecency::now() as u64);
    }
    evidence
}

/// The keys of `checks["config.load"].details` worth carrying, named exactly.
///
/// An allowlist rather than a filter, because the same object also holds
/// `enabled feature flags` — a single value listing thirty-nine flag names,
/// hundreds of characters long — and the two keys differ only by word order.
/// `feature flags enabled` is the count.
const CODEX_CONFIG_KEYS: &[&str] = &[
    "CODEX_HOME",
    "config.toml",
    "config.toml parse",
    "cwd",
    "model",
    "model provider",
    "mcp servers",
    "feature flags enabled",
    "feature flag overrides",
];

/// Sandbox and approval policy are configuration a run operates under just as
/// much as the model is, and codex resolves them the same way.
const CODEX_SANDBOX_KEYS: &[&str] =
    &["approval policy", "filesystem sandbox", "network sandbox"];

/// The opencode keys whose *values* may be shown. Everything else in that
/// object is reported by name only: `provider` holds API keys, `mcp` holds
/// commands, environments and headers, and `username` is an account identity —
/// the same thing `doctor`'s login probe already refuses to print.
const OPENCODE_VALUE_KEYS: &[&str] =
    &["model", "small_model", "theme", "share", "autoupdate"];

/// Turn what a CLI printed into evidence. Pure, so the real output of all four
/// CLIs can be pinned by tests without any of them being installed.
pub fn read_effective(kind: Effective, agent: &str, text: &str) -> ConfigEvidence {
    let mut evidence = ConfigEvidence {
        agent: agent.to_string(),
        command: effective_command(kind).map(|argv| argv.join(" ")),
        scope: match kind {
            Effective::CodexDoctorJson | Effective::OpencodeDebugConfig => "directory",
            Effective::ClaudeAutoMode => "subsystem",
            Effective::None => "none",
        },
        checked_at: None,
        status: None,
        facts: Vec::new(),
        note: match kind {
            Effective::CodexDoctorJson =>
                "`codex doctor --json` resolves config.toml, CODEX_HOME and `-c` overrides, so this \
                 is what a codex run started in this directory would use — not what a Run already \
                 going was started with"
                    .into(),
            Effective::OpencodeDebugConfig =>
                "`opencode debug config` shows the merged configuration for this directory — not \
                 that of a Run already going; provider, MCP and account values are named but never \
                 printed"
                    .into(),
            Effective::ClaudeAutoMode =>
                "claude exposes no resolved-settings command — `claude config` is not a subcommand \
                 and would run `config` as a prompt, and `claude doctor` reports the installation. \
                 `claude auto-mode config` is the only effective-config reporter it has, and it \
                 covers the auto-mode classifier alone"
                    .into(),
            Effective::None =>
                "effective configuration unknown · pi's only configuration surface is `pi config`, \
                 an interactive TUI with no non-interactive form — so nothing is claimed here"
                    .into(),
        },
        trouble: None,
    };
    if kind == Effective::None {
        return evidence;
    }
    if text.trim().is_empty() {
        evidence.trouble = Some(format!(
            "`{}` printed nothing",
            evidence.command.clone().unwrap_or_default()
        ));
        return evidence;
    }
    match kind {
        Effective::CodexDoctorJson => read_codex(&mut evidence, text),
        Effective::OpencodeDebugConfig => read_opencode(&mut evidence, text),
        Effective::ClaudeAutoMode => read_claude_auto_mode(&mut evidence, text),
        Effective::None => {}
    }
    evidence
}

fn read_codex(evidence: &mut ConfigEvidence, text: &str) {
    let Ok(report) = serde_json::from_str::<serde_json::Value>(text) else {
        evidence.trouble = Some("`codex doctor --json` printed no JSON".into());
        return;
    };
    let checks = report.get("checks");
    let Some(load) = checks.and_then(|checks| checks.get("config.load")) else {
        evidence.trouble =
            Some("`codex doctor --json` reported no config.load check, so nothing was resolved".into());
        return;
    };
    evidence.status = load.get("status").and_then(|s| s.as_str()).map(str::to_string);
    for (source, keys) in [
        (load.get("details"), CODEX_CONFIG_KEYS),
        (checks.and_then(|c| c.get("sandbox.helpers")).and_then(|c| c.get("details")), CODEX_SANDBOX_KEYS),
    ] {
        let Some(details) = source.and_then(|d| d.as_object()) else { continue };
        for key in keys {
            if let Some(value) = details.get(*key).and_then(|v| v.as_str()) {
                push_fact(&mut evidence.facts, key, value);
            }
        }
    }
    if evidence.facts.is_empty() {
        evidence.trouble =
            Some("`codex doctor --json` named none of the configuration keys this understands".into());
    }
}

fn read_opencode(evidence: &mut ConfigEvidence, text: &str) {
    let Ok(config) = serde_json::from_str::<serde_json::Value>(text) else {
        evidence.trouble = Some("`opencode debug config` printed no JSON".into());
        return;
    };
    let Some(object) = config.as_object() else {
        evidence.trouble = Some("`opencode debug config` printed no configuration object".into());
        return;
    };
    // Names, not values. A key called `apiKey` is dropped even as a name,
    // because the safest thing to say about it is nothing.
    let mut names: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|name| !name.starts_with('$') && !crate::secrets::looks_secret(name))
        .collect();
    names.sort_unstable();
    if !names.is_empty() {
        push_fact(&mut evidence.facts, "defines", &crate::width::dtrunc(&names.join(", "), 96));
    }
    for key in OPENCODE_VALUE_KEYS {
        let Some(value) = object.get(*key) else { continue };
        let shown = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Number(n) => n.to_string(),
            // An object or an array under one of these keys is a shape this
            // allowlist was not written for; naming it is enough.
            _ => continue,
        };
        push_fact(&mut evidence.facts, key, &shown);
    }
    if evidence.facts.is_empty() {
        evidence.trouble = Some("`opencode debug config` resolved to an empty configuration".into());
    }
}

fn read_claude_auto_mode(evidence: &mut ConfigEvidence, text: &str) {
    let Ok(config) = serde_json::from_str::<serde_json::Value>(text) else {
        evidence.trouble = Some("`claude auto-mode config` printed no JSON".into());
        return;
    };
    let Some(object) = config.as_object() else {
        evidence.trouble = Some("`claude auto-mode config` printed no configuration object".into());
        return;
    };
    // Counts, never the rules. An auto-mode rule is free prose a person wrote,
    // so it is the one thing in that object that could hold anything at all.
    for key in ["allow", "soft_deny", "hard_deny", "environment"] {
        if let Some(rules) = object.get(key).and_then(|v| v.as_array()) {
            push_fact(
                &mut evidence.facts,
                &format!("auto-mode {} rules", key.replace('_', "-")),
                &rules.len().to_string(),
            );
        }
    }
    if evidence.facts.is_empty() {
        evidence.trouble =
            Some("`claude auto-mode config` named no rule set this understands".into());
    }
}

/// The gate every reported value passes, and the reason this can be shown at
/// all.
///
/// Rejects rather than truncates: a value too long to be a setting is most
/// likely a blob, and the first eighty characters of a blob is still eighty
/// characters of it. Control characters go the same way — a report is a line
/// of text, and anything carrying newlines or escapes is not one.
fn push_fact(facts: &mut Vec<ConfigFact>, key: &str, raw: &str) {
    if crate::secrets::looks_secret(key) {
        return;
    }
    let value = paths::tilde(raw.trim());
    let unsafe_value = value.is_empty()
        || value.chars().count() > 120
        || value.chars().any(char::is_control)
        || crate::secrets::looks_secret(&value)
        || crate::secrets::looks_secret_material(&format!("{key}={value}"))
        || looks_like_a_blob(&value);
    if unsafe_value {
        return;
    }
    facts.push(ConfigFact { key: key.to_string(), value });
}

/// Twenty unbroken alphanumerics mixing letters and digits, which is a token
/// and not a setting.
///
/// `secrets::looks_secret` stays the filter every value goes through; this is
/// a stricter one on top of it, for the one kind of output where credentials
/// live by design. The shared filter recognises `sk-` followed by twenty
/// *unbroken* alphanumerics, and every current provider key is segmented
/// instead — `sk-proj-…`, `sk-ant-…`, `sk_live_…` — so the prefix rule misses
/// the body it was written for. Length and shape catch it whatever the prefix
/// turns out to be next year.
///
/// The digit requirement is what keeps real settings: a model id is
/// `claude-sonnet-4-5-20250929`, whose longest unbroken run is eight
/// characters, and a long camel-case config key is letters alone.
fn looks_like_a_blob(value: &str) -> bool {
    value
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|run| {
            run.len() >= 20
                && run.bytes().any(|b| b.is_ascii_digit())
                && run.bytes().any(|b| b.is_ascii_alphabetic())
        })
}

/// One rendering, so the table and `--json` cannot describe the same evidence
/// differently.
pub fn evidence_lines(evidence: &ConfigEvidence) -> Vec<String> {
    let mut lines = Vec::new();
    match (&evidence.command, evidence.checked_at) {
        (Some(command), Some(at)) => lines.push(format!(
            "effective config · `{command}` · asked {}",
            super::user::ago(at as f64)
        )),
        (Some(command), None) => lines.push(format!("effective config · `{command}`")),
        (None, _) => {}
    }
    if let Some(status) = &evidence.status {
        lines.push(format!("  it calls its own configuration load {status}"));
    }
    for fact in &evidence.facts {
        lines.push(format!("  {}: {}", fact.key, fact.value));
    }
    // The limitation, always, and always last: it is the sentence that stops
    // the facts above it being read as more than they are. `RUN_SCOPE` itself
    // is said once per report rather than once per agent — four copies of one
    // paragraph is how a caveat stops being read at all — and each note ends
    // by naming the same boundary in its own CLI's terms.
    lines.push(evidence.note.clone());
    lines
}

/// One row per installed agent, saying what it holds. The first thing you
/// see when the launcher opens, because it is the thing most worth seeing.
/// Counted from lists the caller has already built rather than gathered
/// again: this is a tally of the three sources above it, and computing them
/// a second time to count them made a summary cost more than the thing it
/// summarises.
pub fn summary(skills: &[Item], mcp: &[Item], sessions: &[Item], runs: &[Item]) -> Vec<Item> {
    use std::collections::BTreeMap;
    let mut per: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut tally = |it: &Item| {
        for a in it.get("agent").split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let e = per.entry(a.to_string()).or_default();
            match it.kind {
                Kind::Skill => e.0 += 1,
                Kind::Mcp => e.1 += 1,
                Kind::Session => e.2 += 1,
                _ => {}
            }
        }
    };
    for it in skills.iter().chain(mcp).chain(sessions) {
        tally(it);
    }
    crate::agent::installed()
        .into_iter()
        .map(|name| {
            let (sk, mc, se) = per.get(name).copied().unwrap_or_default();
            let own_runs: Vec<&Item> = runs.iter().filter(|run| run.get("agent") == name).collect();
            let waiting = own_runs.iter().filter(|run| run.get("state") == "waiting").count();
            let projects: Vec<&str> = own_runs
                .iter()
                .map(|run| run.get("project"))
                .filter(|project| !project.is_empty())
                .collect();
            let run_ids: Vec<&str> = own_runs.iter().map(|run| run.get("run_id")).collect();
            let latest = sessions
                .iter()
                .filter(|session| session.get("agent") == name)
                .max_by(|a, b| {
                    a.get("ts").parse::<f64>().unwrap_or(0.0)
                        .partial_cmp(&b.get("ts").parse::<f64>().unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            let spec = crate::agent::get(name);
            let executable = spec.and_then(crate::agent::Spec::executable)
                .map(|path| path.to_string_lossy().into_owned()).unwrap_or_default();
            let settings = spec.map(crate::agent::Spec::settings_path)
                .map(|path| path.to_string_lossy().into_owned()).unwrap_or_default();
            let operations = spec.map(crate::agent::Spec::operation_labels).unwrap_or_default();
            let item = Item::new(name, Kind::Agent)
                .title(name)
                .fields([
                    format!("{sk} skills"),
                    format!("{mc} mcp"),
                    format!("{se} sessions"),
                ])
                .put("agent", name)
                .put("agent_id", name)
                .put("installed", "true")
                .put("executable", executable)
                .put("settings", settings)
                .put("operations", operations.join(", "))
                .put("run_count", own_runs.len().to_string())
                .put("waiting_count", waiting.to_string())
                .put("projects", serde_json::to_string(&projects).unwrap_or_default())
                .put("run_ids", serde_json::to_string(&run_ids).unwrap_or_default());
            match latest {
                Some(session) => item
                    .put("latest_session", session.title.clone())
                    .put("latest_session_id", session.get("session_id"))
                    .put("latest_session_at", session.get("ts")),
                None => item,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {

    /// The three cases, and the middle one is the whole reason this decision
    /// happens after parsing rather than before it.
    #[test]
    fn an_agents_answer_is_trusted_only_when_it_is_one() {
        use crate::exec::Output;
        let ok = |code: i32| Output { status: Some(code), ..Output::default() };

        // Exit 0 with nothing: an authoritative empty. The last server was
        // removed and the row has to go, or the launcher shows it forever.
        assert!(trusted(&ok(0), "claude", 0));
        assert!(trusted(&ok(0), "claude", 3));

        // Non-zero but it printed a usable list: take the list. These CLIs
        // report warnings through their status while answering perfectly.
        assert!(trusted(&ok(1), "claude", 3));

        // Non-zero and nothing parseable — `Error: authentication required`.
        // Checking stdout for emptiness could not see this: the error text
        // *is* stdout, and it is not an inventory.
        let auth_error = Output {
            status: Some(1),
            stdout: b"Error: authentication required\n".to_vec(),
            ..Output::default()
        };
        assert!(!trusted(&auth_error, "claude", 0));

        // And the count it is given has to be evidence, not merely a number.
        // That error line splits on `": "` exactly as a server line does, so
        // the parser read a server *named* "Error", and "a record was parsed"
        // was true while three real rows were replaced by one imaginary one.
        let refused = parse_claude_list("Error: authentication required");
        assert_eq!(refused.len(), 1, "it does parse — that is the whole trap");
        assert_eq!(refused[0].name, "Error");
        assert_eq!(claude_evidence(&ok(1), &refused), 0, "and it is not evidence");
        assert!(!trusted(&ok(1), "claude", claude_evidence(&ok(1), &refused)));

        // A real listing carries a health status on every entry, which is
        // what an error message cannot produce by accident.
        let real = parse_claude_list(
            "Checking MCP server health...\n\nnode_repl: /bin/x - ✓ Connected\ndrive: /bin/y - ✘ Failed",
        );
        assert_eq!(real.len(), 2);
        assert_eq!(real[0].name, "node_repl");
        assert_eq!(real[0].target, "/bin/x");
        assert_eq!(real[0].status, "✓ Connected");
        assert_eq!(claude_evidence(&ok(1), &real), 2);
        assert!(trusted(&ok(1), "claude", claude_evidence(&ok(1), &real)));

        // A clean exit is taken at its word, statuses or not — so a format
        // that stops printing them degrades to showing rows, never to
        // erasing them.
        let statusless = parse_claude_list("node_repl: /bin/x");
        assert_eq!(statusless.len(), 1);
        assert_eq!(claude_evidence(&ok(0), &statusless), 1);
        assert_eq!(claude_evidence(&ok(1), &statusless), 0);

        // Never started, or killed part-way: never an answer, whatever it
        // managed to print. A partial list must not replace a complete one.
        let killed = Output { timed_out: true, status: None, ..Output::default() };
        assert!(!trusted(&killed, "claude", 0));
        assert!(!trusted(&killed, "claude", 7), "a partial list is not an answer");
        let missing = Output { spawn_failed: true, ..Output::default() };
        assert!(!trusted(&missing, "claude", 0));

        // The two that used to slip through, because both make `ok()` false
        // and neither was named — so having parsed one record was enough to
        // reach the `refused` test and pass it.
        let cut_off = Output { status: Some(0), truncated: true, ..Output::default() };
        assert!(!trusted(&cut_off, "claude", 7), "output that hit the cap is a fragment");
        let signalled = Output { status: None, ..Output::default() };
        assert!(!trusted(&signalled, "claude", 7), "a process killed by a signal never finished");

        // …and each refusal names the partition, which is what keeps its
        // cached rows through `cache::carry_over`.
        assert!(crate::exec::incomplete_partitions().iter().any(|p| p == "claude"));
    }
    use super::*;
    use crate::capability::SkillCopy;

    #[test]
    fn every_copy_of_a_merged_skill_is_reachable_from_the_row() {
        let copies = vec![
            ("claude", "/s/claude/deploy"),
            ("shared", "/s/shared/deploy"),
            ("claude", "/s/claude/deploy"),
        ];
        let copy_info = vec![
            SkillCopy { agent: "shared".into(), dir: "/s/shared/deploy".into(), ..Default::default() },
            SkillCopy { agent: "codex".into(), dir: "/s/codex/deploy".into(), ..Default::default() },
        ];
        let row = Item::new("/deploy", Kind::Skill)
            .put("agent", "claude, shared, codex")
            .put("dir", "/s/claude/deploy")
            .put("copies", serde_json::to_string(&copies).unwrap())
            .put("copy_info", serde_json::to_string(&copy_info).unwrap());
        assert_eq!(
            copy_paths(&row),
            vec![
                ("claude".to_string(), "/s/claude/deploy".to_string()),
                ("shared".to_string(), "/s/shared/deploy".to_string()),
                ("codex".to_string(), "/s/codex/deploy".to_string()),
            ],
            "one entry per directory, in discovery order, and the hashed copies fill any gap",
        );

        // A row from before either list existed still opens its one copy —
        // and never with the comma-joined agent column as an agent name.
        let bare = Item::new("/solo", Kind::Skill)
            .put("agent", "claude, codex")
            .put("dir", "/s/claude/solo");
        assert_eq!(copy_paths(&bare), vec![("claude".to_string(), "/s/claude/solo".to_string())]);
        assert!(copy_paths(&Item::new("/none", Kind::Skill)).is_empty());
    }

    #[test]
    fn one_parser_answers_both_the_row_and_the_diagnostic() {
        let text = "---\nname: deploy\ndescription: ships it\n  carefully\n---\nbody\n";
        let front = parse_front(text);
        assert!(front.opened && front.closed);
        assert_eq!(front.name.as_deref(), Some("deploy"));
        assert_eq!(front.desc.as_deref(), Some("ships it carefully"));

        let none = parse_front("# no header\n");
        assert!(!none.opened && !none.closed);
        assert!(none.name.is_none() && none.desc.is_none());

        // Present but empty is not absent: the row falls back to the folder
        // name, the diagnostic still has to say the header declares nothing.
        let empty = parse_front("---\nname:\n---\n");
        assert_eq!(empty.name.as_deref(), Some(""));
    }

    #[test]
    fn a_byte_order_mark_is_not_a_missing_header() {
        let body = "---\nname: deploy\ndescription: ships it\n---\nbody\n";
        let front = parse_front(&format!("\u{feff}{body}"));
        assert!(front.opened && front.closed, "agents parse this file; so must we");
        assert_eq!(front.name.as_deref(), Some("deploy"));
        assert_eq!(front.desc.as_deref(), Some("ships it"));

        // Nothing else moves: for every other input the BOM-stripped parse is
        // the parse it always was.
        for text in [body, "# no header\n", "", "---\n", "-- not quite\n", "\n---\nname: x\n---\n"] {
            let plain = parse_front(text);
            let marked = parse_front(&format!("\u{feff}{text}"));
            assert_eq!(
                (plain.opened, plain.closed, plain.name, plain.desc),
                (marked.opened, marked.closed, marked.name, marked.desc),
                "input {text:?}",
            );
        }
    }

    /// What each Agent CLI on this machine actually prints when asked for its
    /// effective configuration, pinned so nothing here starts inferring a
    /// setting nobody reported.
    #[test]
    fn effective_config_is_read_from_what_the_cli_said() {
        // codex: `codex doctor --json`, abridged to the two checks read.
        let codex = r#"{
          "schemaVersion": 1, "overallStatus": "ok", "codexVersion": "0.147.0",
          "checks": {
            "config.load": { "status": "ok", "summary": "config loaded", "details": {
              "CODEX_HOME": "/Users/mike/.codex",
              "config.toml": "/Users/mike/.codex/config.toml",
              "config.toml parse": "ok",
              "cwd": "/Users/mike/App/Prelude",
              "model": "gpt-5.6-terra",
              "model provider": "openai",
              "mcp servers": "2",
              "feature flags enabled": "39",
              "feature flag overrides": "none",
              "enabled feature flags": "shell_tool, view_image, unified_exec, shell_snapshot, code_mode_host, terminal_resize_reflow, sqlite, hooks, enable_request_compression, multi_agent, apps, tool_search_always_defer_mcp_tools, tool_suggest, plugins, in_app_browser"
            }},
            "sandbox.helpers": { "status": "ok", "details": {
              "approval policy": "OnRequest",
              "filesystem sandbox": "restricted",
              "network sandbox": "restricted",
              "execve wrapper helper": "/Users/mike/.codex/tmp/arg0/codex-arg0i24IeM/codex-execve-wrapper"
            }}
          }}"#
            .replace("/Users/mike", &crate::paths::home().to_string_lossy());
        let evidence = read_effective(Effective::CodexDoctorJson, "codex", &codex);
        assert_eq!(evidence.scope, "directory");
        assert_eq!(evidence.status.as_deref(), Some("ok"));
        assert_eq!(evidence.trouble, None);
        let fact = |key: &str| {
            evidence.facts.iter().find(|f| f.key == key).map(|f| f.value.clone())
        };
        assert_eq!(fact("model").as_deref(), Some("gpt-5.6-terra"));
        assert_eq!(fact("approval policy").as_deref(), Some("OnRequest"));
        assert_eq!(fact("cwd").as_deref(), Some("~/App/Prelude"), "paths are shown the way every other report shows them");
        // The two keys differ only by word order, and one of them is a list
        // hundreds of characters long. Only the count is carried.
        assert_eq!(fact("feature flags enabled").as_deref(), Some("39"));
        assert!(fact("enabled feature flags").is_none(), "the flag list is not a setting worth a report");
        assert!(fact("execve wrapper helper").is_none(), "an allowlist, not a filter");

        // opencode: `opencode debug config`, whose help calls it resolved.
        let opencode = r#"{"$schema":"https://opencode.ai/config.json","model":"anthropic/claude-x",
          "share":"manual","agent":{},"mode":{},"plugin":[],"command":{},"username":"mike",
          "provider":{"anthropic":{"options":{"apiKey":"sk-ant-0123456789abcdefghij"}}}}"#;
        let evidence = read_effective(Effective::OpencodeDebugConfig, "opencode", opencode);
        let fact = |key: &str| evidence.facts.iter().find(|f| f.key == key).map(|f| f.value.clone());
        assert_eq!(fact("model").as_deref(), Some("anthropic/claude-x"));
        assert_eq!(fact("share").as_deref(), Some("manual"));
        assert!(fact("provider").is_none(), "named, never printed");
        assert!(fact("username").is_none(), "an account identity is not a setting");
        let defines = fact("defines").unwrap_or_default();
        assert!(defines.contains("provider") && defines.contains("username"), "{defines}");
        assert!(!defines.contains("$schema"), "{defines}");

        // claude: one subsystem, reported as one subsystem.
        let claude = r#"{"allow":[1,2,3],"soft_deny":[1],"hard_deny":[],"environment":[1,2]}"#;
        let evidence = read_effective(Effective::ClaudeAutoMode, "claude", claude);
        assert_eq!(evidence.scope, "subsystem");
        let fact = |key: &str| evidence.facts.iter().find(|f| f.key == key).map(|f| f.value.clone());
        assert_eq!(fact("auto-mode allow rules").as_deref(), Some("3"));
        assert_eq!(fact("auto-mode hard-deny rules").as_deref(), Some("0"));
        assert!(evidence.note.contains("auto-mode classifier alone"));

        // pi: nothing to ask, and it says so instead of reading a file.
        let evidence = read_effective(Effective::None, "pi", "");
        assert_eq!(evidence.scope, "none");
        assert_eq!(evidence.command, None, "an answer nobody can reproduce is not evidence");
        assert_eq!(evidence.trouble, None, "not a fault — pi simply has no such command");
        assert!(evidence.note.starts_with("effective configuration unknown ·"));

        // Asked, and no usable answer, is a third thing again.
        assert!(read_effective(Effective::CodexDoctorJson, "codex", "").trouble.is_some());
        assert!(read_effective(Effective::CodexDoctorJson, "codex", "not json").trouble.is_some());
        assert!(read_effective(Effective::OpencodeDebugConfig, "opencode", "{}").trouble.is_some());
    }

    /// Effective configuration is where an agent's credentials live. Nothing
    /// that looks like one may reach a report a person pastes into an issue,
    /// and neither may an unbounded blob.
    #[test]
    fn effective_config_never_carries_a_credential() {
        let codex = r#"{"checks":{"config.load":{"status":"ok","details":{
            "model":"gpt-5.6-terra",
            "config.toml":"/Users/mike/.codex/config.toml",
            "cwd":"https://user:hunter2@example.com/checkout",
            "model provider":"sk-proj-0123456789abcdefghijklmno",
            "mcp servers":"2"
        }}}}"#;
        let evidence = read_effective(Effective::CodexDoctorJson, "codex", codex);
        let rendered = format!(
            "{}{}",
            serde_json::to_string(&evidence).unwrap(),
            evidence_lines(&evidence).join("\n")
        );
        for leak in ["hunter2", "sk-proj-0123456789abcdefghijklmno"] {
            assert!(!rendered.contains(leak), "{leak} reached the report:\n{rendered}");
        }
        assert!(rendered.contains("gpt-5.6-terra"), "and the safe facts still arrive");

        // Rejected, not truncated: the first eighty characters of a blob are
        // still eighty characters of it.
        let mut facts = Vec::new();
        push_fact(&mut facts, "model", &"x".repeat(200));
        push_fact(&mut facts, "apiKey", "plainly-harmless-looking");
        push_fact(&mut facts, "cwd", "one line\nand another");
        push_fact(&mut facts, "auth_token", "aaaaaaaaaaaaaaaaaaaaaaaa");
        push_fact(&mut facts, "model", "sk_live_9Xy2Qa7Bc4De6Fg8Hi0Jk");
        assert!(facts.is_empty(), "{facts:?}");

        // And the settings that *look* like blobs are still shown, or the gate
        // would quietly empty the report it exists to make safe.
        let mut facts = Vec::new();
        for keep in ["claude-sonnet-4-5-20250929", "gpt-5.6-terra", "~/.codex/config.toml", "OnRequest"] {
            push_fact(&mut facts, "model", keep);
        }
        assert_eq!(facts.len(), 4, "{facts:?}");

        // An opencode config is mostly credentials, and only names escape it.
        let opencode = r#"{"provider":{"openai":{"options":{"apiKey":"sk-0123456789abcdefghijkl"}}},
            "mcp":{"x":{"environment":{"GITHUB_TOKEN":"ghp_0123456789abcdefghijkl"}}},
            "model":"openai/gpt-5"}"#;
        let evidence = read_effective(Effective::OpencodeDebugConfig, "opencode", opencode);
        let rendered = serde_json::to_string(&evidence).unwrap();
        assert!(!rendered.contains("sk-0123456789") && !rendered.contains("ghp_"), "{rendered}");
        assert!(rendered.contains("openai/gpt-5"));
    }

    /// The claim is agent-level, and every rendering of it says so. A Run's
    /// own flags and environment are exactly what Prelude does not keep, so
    /// there is no honest way to answer this per Run.
    #[test]
    fn effective_config_never_claims_to_be_per_run() {
        for kind in [
            Effective::CodexDoctorJson,
            Effective::OpencodeDebugConfig,
            Effective::ClaudeAutoMode,
            Effective::None,
        ] {
            let evidence = read_effective(kind, "a", "{}");
            assert_ne!(evidence.scope, "run");
            let lines = evidence_lines(&evidence).join("\n");
            assert!(lines.contains(&evidence.note), "the limitation is always rendered");
            // Every answer that exists names the boundary in its own words,
            // and the paragraph that explains why is said once per report.
            if evidence.command.is_some() {
                assert!(
                    evidence.note.contains("not what a Run already going")
                        || evidence.note.contains("not that of a Run already going")
                        || evidence.scope == "subsystem",
                    "{}", evidence.note,
                );
            }
        }
        assert!(RUN_SCOPE.starts_with("agent-level, not run-level"));
    }

    #[test]
    fn a_pre_epoch_tree_is_hashed_once_and_then_reused() {
        let root = std::env::temp_dir().join(format!("prelude-epoch-skill-{}", std::process::id()));
        // Cleared going in as well as coming out, so a previous run that
        // panicked cannot make this one fail for a different reason.
        let _ = std::fs::remove_dir_all(&root);
        let skill = root.join("ancient");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: ancient\ndescription: d\n---\n").unwrap();
        // `touch -t 197001010000` west of Greenwich, as a fixture: an mtime at
        // or before the epoch, which `mtime()` reports as 0.
        let epoch = std::fs::FileTimes::new()
            .set_accessed(std::time::SystemTime::UNIX_EPOCH)
            .set_modified(std::time::SystemTime::UNIX_EPOCH);
        for path in [skill.join("SKILL.md"), skill.clone()] {
            std::fs::File::options().read(true).open(&path).unwrap().set_times(epoch).unwrap();
        }

        let mut copy = crate::capability::hash_skill("claude", &skill);
        let stamp = crate::capability::skill_stamp(&skill);
        copy.stamp = stamp.clone();
        assert_eq!(copy.modified, 0, "an epoch mtime is a real answer, not a failure");
        assert!(!copy.fingerprint.is_empty(), "and the tree was read completely");

        // The second pass, exactly as `skill_hashes` performs it: write the
        // record, read it back, ask whether it may stand.
        let item = crate::capability::cache_item(&copy);
        let back = crate::capability::copy_from_item(&item);
        assert!(
            reusable(crate::capability::is_current_record(&item), &back, &stamp),
            "a record must satisfy the gate it was just written by, or this tree is re-hashed for ever",
        );

        // A record from before the version field is recomputed — once: the
        // rewrite carries the current version, so the next pass reuses it.
        let mut legacy = item.clone();
        legacy.data.remove("record");
        assert!(!reusable(crate::capability::is_current_record(&legacy), &back, &stamp));

        // And the tree itself is still what decides the rest.
        assert!(!reusable(true, &back, "fnv1a64-v1:0000000000000000"), "an edited tree");
        assert!(!reusable(true, &back, ""), "an unreadable tree");
        let mut unhashed = back.clone();
        unhashed.fingerprint.clear();
        assert!(!reusable(true, &unhashed, &stamp), "a record with no fingerprint");
        let _ = std::fs::remove_dir_all(&root);
    }
}
