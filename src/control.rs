//! The canonical Agent/Run/Session/Capability graph.
//!
//! Launcher rows are views. This module is the relationship layer beneath
//! them: stable ids, explicit edges, and only facts that can be traced to an
//! agent CLI, process or session file. It never retains a process command
//! line or prompt, because either may contain credentials.

use crate::item::Item;
use serde::Serialize;

/// Schema 3 adds Run effective context — the branch, the model and the
/// capabilities a run actually loaded — and the reverse edges from Skill and
/// MCP back to the Runs that borrowed them. Every field schema 2 had is still
/// there and still means what it did; a reader that knows only the old shape
/// keeps working.
const SCHEMA: u32 = 3;

#[derive(Serialize)]
pub struct Snapshot {
    pub schema: u32,
    pub generated_at: u64,
    pub agents: Vec<AgentRecord>,
    pub runs: Vec<RunRecord>,
    pub sessions: Vec<SessionRecord>,
    pub skills: Vec<SkillRecord>,
    pub mcp: Vec<McpRecord>,
}

#[derive(Serialize)]
pub struct AgentRecord {
    pub id: String,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    pub runs: Vec<String>,
    pub sessions: Vec<String>,
    pub skills: Vec<String>,
    pub mcp: Vec<String>,
    pub configs: Vec<String>,
    pub capabilities: crate::agent::Capabilities,
}

#[derive(Serialize)]
pub struct RunRecord {
    pub id: String,
    pub agent: String,
    pub pid: String,
    pub started: u64,
    pub state: String,
    pub cwd: String,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_match: Option<String>,
    pub batch: bool,
    /// The branch its working directory is on, or `detached at <id>`. Absent
    /// outside a repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Only ever what a native session file recorded as a structured field.
    /// Never a `--model` flag, a config default or a guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Capabilities **this run's own command line named**. Not the agent's
    /// inventory: `AgentRecord::skills` and `AgentRecord::mcp` are what is
    /// installed and available, and these are what was actually loaded.
    pub skills_confirmed: Vec<String>,
    pub mcp_confirmed: Vec<String>,
}

#[derive(Serialize)]
pub struct SessionRecord {
    pub id: String,
    pub native_id: String,
    pub agent: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_title: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    pub cwd: String,
    pub file: String,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_state: Option<String>,
}

#[derive(Serialize)]
pub struct SkillRecord {
    pub id: String,
    pub name: String,
    pub owners: Vec<String>,
    pub missing: Vec<String>,
    pub integrity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    pub copies: Vec<crate::capability::SkillCopy>,
    pub borrow_targets: Vec<String>,
    pub install_targets: Vec<String>,
    /// Runs that explicitly loaded this skill for one run. Never "runs of an
    /// agent that has it installed" — that is `owners`, and conflating the two
    /// is the mistake Milestone 5 exists to prevent.
    pub runs: Vec<String>,
}

#[derive(Serialize)]
pub struct McpRecord {
    pub id: String,
    pub name: String,
    pub owners: Vec<String>,
    pub variants: Vec<crate::capability::McpVariant>,
    pub comparison: String,
    pub borrow_targets: Vec<String>,
    pub install_targets: Vec<String>,
    /// Runs that explicitly borrowed this server, on the same rule as
    /// `SkillRecord::runs`.
    pub runs: Vec<String>,
}

fn some(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn session_id(item: &Item) -> String {
    match item.get("session_id") {
        "" => format!("{}:{}", item.get("agent"), item.get("id")),
        id => id.to_string(),
    }
}

/// The one spelling a display name and a command line can be compared in.
///
/// A confirmed capability name is read off an argument vector, and what is
/// written there is `lend::Mcp::key` — every character outside `[A-Za-z0-9_-]`
/// replaced, because that is both the staged file's name and the dotted path
/// segment codex addresses config by. `McpRecord::name` is the display name
/// the owner gave it. On this machine those are `claude_ai_Gmail` and
/// `claude.ai Gmail`, which no case-insensitive comparison can bridge: every
/// account-hosted server that became portable had an empty `runs` array for
/// ever. Both sides are put through the same rule instead, lower-cased on top
/// because MCP capability ids already are.
///
/// It restates `lend::Mcp::key` rather than calling it, which needs a parsed
/// definition rather than a name — and definitions are exactly what this layer
/// must not hold.
fn capability_key(name: &str) -> String {
    let key: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    key.trim_matches('_').to_ascii_lowercase()
}

/// Capability names a run confirmed, matched to a capability's own name.
fn named_by(runs: &[RunRecord], is_skill: bool, name: &str) -> Vec<String> {
    let name = capability_key(name);
    if name.is_empty() {
        return Vec::new();
    }
    runs.iter()
        .filter(|run| {
            let confirmed = if is_skill { &run.skills_confirmed } else { &run.mcp_confirmed };
            confirmed.iter().any(|loaded| capability_key(loaded) == name)
        })
        .map(|run| run.id.clone())
        .collect()
}

impl Snapshot {
    pub fn from_items(
        runs: &[Item],
        sessions: &[Item],
        skills: &[Item],
        mcp: &[Item],
        configs: &[Item],
    ) -> Self {
        let run_records: Vec<RunRecord> = runs
            .iter()
            .map(|run| {
                let (skills_confirmed, mcp_confirmed) =
                    crate::sources::running::confirmed_capabilities(run);
                RunRecord {
                    id: run.get("run_id").to_string(),
                    agent: run.get("agent").to_string(),
                    pid: run.get("pid").to_string(),
                    started: run.get("started").parse().unwrap_or(0),
                    state: run.get("state").to_string(),
                    cwd: run.get("cwd").to_string(),
                    project: run.get("project").to_string(),
                    pane: some(run.get("pane")),
                    address: some(run.get("addr")),
                    session: some(run.get("session_id")),
                    session_match: some(run.get("session_match")),
                    batch: run.get("batch") == "1",
                    branch: crate::sources::running::branch_label(run),
                    // A bounded tail read of the session file, on an explicit
                    // command. Absent unless the native format recorded it.
                    model: crate::sources::running::model_of(
                        run.get("agent"),
                        run.get("session"),
                    ),
                    skills_confirmed,
                    mcp_confirmed,
                }
            })
            .collect();
        let session_records: Vec<SessionRecord> = sessions
            .iter()
            .map(|session| SessionRecord {
                id: session_id(session),
                native_id: session.get("id").to_string(),
                agent: session.get("agent").to_string(),
                title: session.title.clone(),
                native_title: some(session.get("native_title")),
                pinned: session.get("pinned") == "true",
                archived: session.get("archived") == "true",
                cwd: session.get("cwd").to_string(),
                file: session.get("file").to_string(),
                updated_at: session.get("ts").parse().unwrap_or(0),
                active_run: some(session.get("active_run")),
                active_state: some(session.get("active_state")),
            })
            .collect();
        let skill_records: Vec<SkillRecord> = skills
            .iter()
            .map(|skill| {
                let mut copies = crate::capability::copies(skill);
                if copies.is_empty() {
                    copies = crate::sources::agents::copies_of(skill).into_iter()
                        .map(|(agent, dir)| crate::capability::SkillCopy {
                            agent, dir, ..Default::default()
                        }).collect();
                }
                SkillRecord {
                runs: named_by(&run_records, true, skill.get("name")),
                id: format!("skill:{}", skill.get("name")),
                name: skill.get("name").to_string(),
                owners: skill
                    .get("agent")
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
                missing: skill
                    .get("missing")
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
                integrity: skill.get("integrity").to_string(),
                fingerprint: some(skill.get("fingerprint")),
                copies,
                borrow_targets: if skill.get("source_sensitive") == "true" {
                    Vec::new()
                } else {
                    skill.get("missing").split(',').map(str::trim)
                        .filter(|agent| !agent.is_empty() && crate::lend::can_borrow_skill(agent))
                        .map(str::to_string).collect()
                },
                install_targets: if skill.get("source_sensitive") == "true" {
                    Vec::new()
                } else {
                    skill.get("missing").split(',').map(str::trim)
                        .filter(|agent| {
                            crate::agent::get(agent)
                                .is_some_and(|spec| spec.capabilities.install_skill)
                        })
                        .map(str::to_string).collect()
                },
                }
            })
            .collect();
        let mut mcp_records: Vec<McpRecord> = Vec::new();
        let mut seen_mcp = std::collections::BTreeSet::new();
        for server in mcp {
            let id = if server.get("capability_id").is_empty() {
                format!("mcp:{}", server.get("name").to_lowercase())
            } else {
                server.get("capability_id").to_string()
            };
            if !seen_mcp.insert(id.clone()) {
                continue;
            }
            let variants = {
                let variants = crate::capability::mcp_variants(server);
                if variants.is_empty() {
                    vec![crate::capability::McpVariant {
                        agent: server.get("agent").to_string(),
                        health: server.get("health").to_string(),
                        transport: server.get("transport").to_string(),
                        health_checked_at: server.get("health_checked_at").parse().unwrap_or(0),
                        summary: server.fields.get(2).cloned().unwrap_or_default(),
                        fingerprint: server.get("definition_hash").to_string(),
                        source: server.get("definition_source").to_string(),
                        public_definition: serde_json::from_str(server.get("definition_public"))
                            .unwrap_or(serde_json::Value::Null),
                        sensitive: server.get("sensitive") == "true",
                        portable: server.get("portable") == "true",
                        tools_status: server.get("tools_status").to_string(),
                        tools_checked_at: server.get("tools_checked_at").parse().unwrap_or(0),
                        tools: serde_json::from_str(server.get("tools")).unwrap_or_default(),
                    }]
                } else {
                    variants
                }
            };
            let owners: Vec<String> = variants.iter().map(|variant| variant.agent.clone()).collect();
            let targets: Vec<String> = crate::agent::installed().into_iter()
                .filter(|agent| {
                    crate::agent::get(agent).is_some_and(|spec| spec.capabilities.install_mcp)
                })
                .filter(|agent| !owners.iter().any(|owner| owner == agent))
                .map(str::to_string)
                .collect();
            let borrow_targets = targets.iter().filter(|target| {
                variants.iter().any(|variant| {
                    variant.portable && (target.as_str() == "claude" || !variant.sensitive)
                })
            }).cloned().collect();
            let install_targets = targets.iter().filter(|_| {
                variants.iter().any(|variant| variant.portable && !variant.sensitive)
            }).cloned().collect();
            mcp_records.push(McpRecord {
                runs: named_by(&run_records, false, server.get("name")),
                id,
                name: server.get("name").to_string(),
                owners,
                variants,
                comparison: server.get("comparison").to_string(),
                borrow_targets,
                install_targets,
            });
        }

        let agents = crate::agent::SPECS.iter().map(|spec| {
                let executable = spec.executable().map(|path| path.to_string_lossy().into_owned());
                AgentRecord {
                    id: spec.name.to_string(),
                    capabilities: spec.capabilities,
                    installed: executable.is_some(),
                    executable,
                    runs: run_records
                        .iter()
                        .filter(|run| run.agent == spec.name)
                        .map(|run| run.id.clone())
                        .collect(),
                    sessions: session_records
                        .iter()
                        .filter(|session| session.agent == spec.name)
                        .map(|session| session.id.clone())
                        .collect(),
                    skills: skill_records
                        .iter()
                        .filter(|skill| skill.owners.iter().any(|owner| owner == spec.name))
                        .map(|skill| skill.id.clone())
                        .collect(),
                    mcp: mcp_records
                        .iter()
                        .filter(|server| server.owners.iter().any(|owner| owner == spec.name))
                        .map(|server| server.id.clone())
                        .collect(),
                    configs: configs
                        .iter()
                        .filter(|config| config.get("agent") == spec.name)
                        .map(|config| config.get("path").to_string())
                        .collect(),
                }
            })
            .collect();

        Self {
            schema: SCHEMA,
            generated_at: crate::frecency::now() as u64,
            agents,
            runs: run_records,
            sessions: session_records,
            skills: skill_records,
            mcp: mcp_records,
        }
    }
}

pub fn snapshot() -> Snapshot {
    let sessions = crate::cache::read_cached("sessions");
    let mcp = crate::cache::read_cached("mcp");
    let runs = crate::sources::running::live_with_sessions(&sessions);
    let sessions = crate::sources::running::annotate_sessions(sessions, &runs);
    let skills = crate::sources::agents::skills_with(&sessions);
    let configs = crate::sources::agents::configs();
    Snapshot::from_items(&runs, &sessions, &skills, &mcp, &configs)
}

pub fn list(json: bool) -> i32 {
    // An explicit control-plane command wants current process identities. MCP
    // health and the hundreds of sessions stay on their existing cache tiers.
    crate::cache::refresh_named("fleet");
    for source in ["sessions", "mcp"] {
        if crate::cache::stale(source) {
            crate::cache::spawn_self(&["_refresh", source]);
        }
    }
    let snapshot = snapshot();
    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| "{}".into()));
        return 0;
    }

    println!("agent       installed  runs  waiting  sessions  skills  mcp");
    for agent in &snapshot.agents {
        let waiting = snapshot
            .runs
            .iter()
            .filter(|run| run.agent == agent.id && run.state == "waiting")
            .count();
        println!(
            "{:<11} {:<9}  {:>4}  {:>7}  {:>8}  {:>6}  {:>3}",
            agent.id,
            if agent.installed { "yes" } else { "no" },
            agent.runs.len(),
            waiting,
            agent.sessions.len(),
            agent.skills.len(),
            agent.mcp.len(),
        );
    }
    0
}
