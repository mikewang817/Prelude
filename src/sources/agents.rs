//! One surface over the skills and MCP servers of every agent CLI installed.
//! Each agent keeps them somewhere different; this is the union.

use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::paths;
use std::collections::BTreeMap;
use std::time::Duration;

fn skill_dirs() -> Vec<(std::path::PathBuf, &'static str)> {
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
}

/// Where each agent keeps its copy of a skill, off the row.
pub fn copies_of(it: &Item) -> Vec<(String, String)> {
    serde_json::from_str(it.get("copies")).unwrap_or_default()
}

/// What one invocation is worth.
///
/// Wide enough that a single one clears anything the launcher's own frecency
/// can add (`MAX_BONUS`, 60), because the two are not the same evidence.
/// Selecting a skill row in Prelude is usually reading its description,
/// inserting its name, or lending it somewhere — the row has nine actions
/// and only one of them runs the thing. Actually invoking it in a
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
            rec.copies.push((agent, sub.to_string_lossy().into_owned()));
            if rec.file.is_empty() {
                if let Some(p) = &md {
                    rec.file = p.to_string_lossy().into_owned();
                    rec.dir = sub.to_string_lossy().into_owned();
                }
            }
        }
    }
    let names: Vec<String> = merged.keys().cloned().collect();
    let usage = super::sessions::skill_usage(&names);
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
            let gap = if missing.is_empty() {
                String::new()
            } else {
                format!("missing: {}", missing.join(", "))
            };
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
                .put("desc", rec.desc)
        })
        .collect()
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
pub fn delete_skill(dir: &str) -> Result<std::path::PathBuf, String> {
    let path = std::path::Path::new(dir);
    if !is_skill_dir(path) {
        return Err(format!("{dir} is not a skill directory — refusing"));
    }
    if !path.is_dir() {
        return Err(format!("{dir} is not there any more"));
    }
    let name = path.file_name().ok_or_else(|| "no name".to_string())?;
    let trash = paths::home().join(".Trash");
    std::fs::create_dir_all(&trash).map_err(|e| format!("no Trash: {e}"))?;

    // Two agents' copies of one skill share a name, and so does anything you
    // deleted last week. Never overwrite what is already in there.
    let mut dest = trash.join(name);
    let mut n = 2;
    while dest.exists() {
        dest = trash.join(format!("{} {n}", name.to_string_lossy()));
        n += 1;
        if n > 999 {
            return Err("too many copies of that name in the Trash".into());
        }
    }
    std::fs::rename(path, &dest).map_err(|e| format!("could not move it to the Trash: {e}"))?;
    Ok(dest)
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

/// Pull name/description out of a SKILL.md YAML header.
fn frontmatter(p: &std::path::Path, fallback: &str) -> (String, String) {
    let Ok(text) = std::fs::read_to_string(p) else {
        return (fallback.to_string(), String::new());
    };
    let mut lines = text.lines();
    if !lines.next().is_some_and(|l| l.starts_with("---")) {
        return (fallback.to_string(), String::new());
    }
    let (mut name, mut desc) = (fallback.to_string(), String::new());
    let mut in_desc = false;
    for line in lines {
        if line.starts_with("---") {
            break;
        }
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().trim_matches(['\'', '"']).to_string();
            in_desc = false;
        } else if let Some(v) = line.strip_prefix("description:") {
            desc = v.trim().trim_matches(['\'', '"']).to_string();
            in_desc = true;
        } else if in_desc && line.starts_with([' ', '\t']) {
            // folded continuation line
            desc.push(' ');
            desc.push_str(line.trim());
        } else {
            in_desc = false;
        }
    }
    (name, crate::width::flatten(&desc))
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
    let mut items = Vec::new();
    mcp_claude(&mut items);
    mcp_codex(&mut items);
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

fn mcp_item(agent: &str, name: &str, detail: &str, health: Health) -> Item {
    Item::new(format!("{agent} mcp get {}", shq(name)), Kind::Mcp)
        .title(name)
        .fields([agent.to_string(), health.label().to_string(), detail.to_string()])
        .put("agent", agent)
        .put("name", name)
        .put("health", health.key())
}

/// `claude mcp list` prints `<name>: <target> - <status>` per server.
fn mcp_claude(out: &mut Vec<Item>) {
    if crate::exec::which("claude").is_none() {
        return;
    }
    let text = crate::exec::run(&["claude", "mcp", "list"], Duration::from_secs(30));
    for line in text.lines() {
        let line = line.trim();
        let Some((name, rest)) = line.split_once(": ") else { continue };
        if name.is_empty() || name.contains("Checking") {
            continue;
        }
        let (target, status) = match rest.rsplit_once(" - ") {
            Some((t, s)) => (t.trim(), s.trim()),
            None => (rest.trim(), ""),
        };
        let low = status.to_lowercase();
        let health = if low.contains("connect") && !low.contains("fail") {
            Health::Ok
        } else if low.contains("pending") || low.contains("approve") {
            Health::NeedsAuth
        } else if low.is_empty() {
            Health::Unknown
        } else {
            Health::Failed
        };
        out.push(mcp_item("claude", name, target, health));
    }
}

/// codex has --json, which also reports auth_status and why it is disabled.
fn mcp_codex(out: &mut Vec<Item>) {
    if crate::exec::which("codex").is_none() {
        return;
    }
    let text = crate::exec::run(&["codex", "mcp", "list", "--json"], Duration::from_secs(20));
    let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else { return };
    for s in list {
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() {
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
        let mut it = mcp_item("codex", name, &detail, health);
        if let Some(r) = s.get("disabled_reason").and_then(|v| v.as_str()) {
            it = it.put("reason", r);
        }
        // Keep the definition itself, not just how it looks. It is the one
        // thing needed to lend the server to another agent, and it is free
        // here — the alternative is a second `mcp get` per server at the
        // moment someone asks, on a path that has to feel instant.
        if let Some(def) = tr.and_then(|t| crate::lend::Mcp::from_codex(name, t)) {
            if let Ok(j) = serde_json::to_string(&def) {
                it = it.put("def", j);
            }
        }
        out.push(it);
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
    std::fs::create_dir_all(&dest_root).map_err(|e| e.to_string())?;
    copy_tree(std::path::Path::new(from_dir), &dest).map_err(|e| e.to_string())?;
    Ok(crate::paths::tilde(&dest.to_string_lossy()))
}

pub fn copy_tree(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
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
pub fn configs() -> Vec<Item> {
    let h = crate::paths::home();
    let mut v: Vec<(std::path::PathBuf, &str)> = vec![
        (h.join(".claude/CLAUDE.md"), "claude"),
        (h.join(".claude/settings.json"), "claude"),
        (h.join(".claude.json"), "claude"),
        (h.join(".codex/config.toml"), "codex"),
        (h.join(".codex/AGENTS.md"), "codex"),
        (h.join(".pi/agent/settings.json"), "pi"),
        (h.join(".config/opencode/opencode.jsonc"), "opencode"),
    ];
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

/// One row per installed agent, saying what it holds. The first thing you
/// see when the launcher opens, because it is the thing most worth seeing.
pub fn summary() -> Vec<Item> {
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
    for it in skills() {
        tally(&it);
    }
    for it in crate::cache::read_cached("mcp") {
        tally(&it);
    }
    for it in crate::cache::read_cached("sessions") {
        tally(&it);
    }
    super::sessions::installed()
        .into_iter()
        .map(|name| {
            let (sk, mc, se) = per.get(name).copied().unwrap_or_default();
            Item::new(name, Kind::Agent)
                .title(name)
                .fields([
                    format!("{sk} skills"),
                    format!("{mc} mcp"),
                    format!("{se} sessions"),
                ])
                .put("agent", name)
        })
        .collect()
}
