//! The canonical Agent/Run/Session/Capability graph.
//!
//! Launcher rows are views. This module is the relationship layer beneath
//! them: stable ids, explicit edges, and only facts that can be traced to an
//! agent CLI, process or session file. It never retains a process command
//! line or prompt, because either may contain credentials.

use crate::item::Item;
use serde::Serialize;

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
    pub copies: Vec<(String, String)>,
}

#[derive(Serialize)]
pub struct McpRecord {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub health: String,
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
            .map(|run| RunRecord {
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
            .map(|skill| SkillRecord {
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
                copies: crate::sources::agents::copies_of(skill),
            })
            .collect();
        let mcp_records: Vec<McpRecord> = mcp
            .iter()
            .map(|server| McpRecord {
                id: format!("{}:{}", server.get("agent"), server.get("name")),
                name: server.get("name").to_string(),
                agent: server.get("agent").to_string(),
                health: server.get("health").to_string(),
            })
            .collect();

        let agents = crate::sources::sessions::AGENTS
            .iter()
            .map(|spec| {
                let executable = crate::exec::which(spec.name)
                    .map(|path| path.to_string_lossy().into_owned());
                AgentRecord {
                    id: spec.name.to_string(),
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
                        .filter(|server| server.agent == spec.name)
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
            schema: 1,
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
