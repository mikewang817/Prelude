//! One surface over the skills and MCP servers of every agent CLI installed.
//! Each agent keeps them somewhere different; this is the union.

use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::paths;
use std::collections::BTreeMap;

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
}

/// Skills across every agent, merged by name.
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
            if rec.file.is_empty() {
                if let Some(p) = &md {
                    rec.file = p.to_string_lossy().into_owned();
                    rec.dir = sub.to_string_lossy().into_owned();
                }
            }
        }
    }
    merged
        .into_iter()
        .map(|(name, rec)| {
            let agents = rec.agents.join(", ");
            let missing = missing_agents(&agents);
            let gap = if missing.is_empty() {
                String::new()
            } else {
                format!("missing: {}", missing.join(", "))
            };
            Item::new(format!("/{name}"), Kind::Skill)
                .title(&name)
                .fields([agents.clone(), gap, rec.desc.clone()])
                .put("missing", missing.join(","))
                .put("name", &name)
                .put("agent", agents)
                .put("dir", rec.dir)
                .put("file", rec.file)
                .put("desc", rec.desc)
        })
        .collect()
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

/// MCP servers across every agent that has any configured.
pub fn mcp() -> Vec<Item> {
    let mut items = Vec::new();
    let home = paths::home();

    // codex — ~/.codex/config.toml  [mcp_servers.NAME]
    let codex = home.join(".codex/config.toml");
    if let Ok(text) = std::fs::read_to_string(&codex) {
        for (section, body) in crate::minitoml::parse(&text) {
            let Some(name) = section.strip_prefix("mcp_servers.") else { continue };
            if name.contains('.') {
                continue; // nested tables like mcp_servers.x.env
            }
            let cmd = body.get("command").cloned().unwrap_or_default();
            let args = body.get("args").cloned().unwrap_or_default();
            items.push(
                Item::new(format!("codex mcp get {}", shq(name)), Kind::Mcp)
                    .title(name)
                    .fields(["codex".to_string(), format!("{cmd} {args}").trim().to_string()])
                    .put("agent", "codex")
                    .put("name", name)
                    .put("config", codex.to_string_lossy()),
            );
        }
    }

    // claude — ~/.claude.json, global and per-project
    let cj = home.join(".claude.json");
    if let Ok(text) = std::fs::read_to_string(&cj) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let mk = |name: &str, detail: String| {
                Item::new(format!("claude mcp get {}", shq(name)), Kind::Mcp)
                    .title(name)
                    .fields(["claude".to_string(), detail])
                    .put("agent", "claude")
                    .put("name", name)
                    .put("config", cj.to_string_lossy())
            };
            if let Some(obj) = v.get("mcpServers").and_then(|m| m.as_object()) {
                for (name, body) in obj {
                    let cmd = body.get("command").and_then(|c| c.as_str()).unwrap_or("");
                    items.push(mk(name, cmd.to_string()));
                }
            }
            if let Some(projects) = v.get("projects").and_then(|p| p.as_object()) {
                for (proj, body) in projects {
                    let Some(servers) = body.get("mcpServers").and_then(|m| m.as_object()) else {
                        continue;
                    };
                    let short = proj.rsplit('/').next().unwrap_or(proj).to_string();
                    for name in servers.keys() {
                        items.push(mk(name, short.clone()));
                    }
                }
            }
        }
    }

    // a project-local .mcp.json, if we're standing in one
    if let Some(root) = super::project::root() {
        let local = root.join(".mcp.json");
        if let Ok(text) = std::fs::read_to_string(&local) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(obj) = v.get("mcpServers").and_then(|m| m.as_object()) {
                    for name in obj.keys() {
                        items.push(
                            Item::new(format!("claude mcp get {}", shq(name)), Kind::Mcp)
                                .title(name)
                                .fields(["claude".to_string(), "this project".to_string()])
                                .put("agent", "claude")
                                .put("name", name)
                                .put("config", local.to_string_lossy()),
                        );
                    }
                }
            }
        }
    }
    items
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
        (h.join(".claude/CLAUDE.md"), "claude · global instructions"),
        (h.join(".claude/settings.json"), "claude · settings"),
        (h.join(".claude.json"), "claude · mcp + projects"),
        (h.join(".codex/config.toml"), "codex · config"),
        (h.join(".codex/AGENTS.md"), "codex · instructions"),
        (h.join(".pi/agent/settings.json"), "pi · settings"),
        (h.join(".config/opencode/opencode.jsonc"), "opencode · config"),
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
                .put("path", s)
        })
        .collect()
}
