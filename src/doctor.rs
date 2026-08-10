//! Diagnose the setup, and measure the one thing that cannot be guessed.
//!
//! Five doors: `prelude doctor` for the launcher itself, and one specialized
//! report per half of the Agent Control Plane — `agents`, `sessions`, `skills`
//! and `mcp`.
//!
//! **A doctor reports. It never repairs.** Every check here returns data; the
//! renderers turn that data into a table for a person or into JSON for an
//! agent, and neither of them writes anything. The one door that changes the
//! machine is `--repair`, and it is not a switch that applies the report: it
//! walks the findings that carry a repair *one at a time*, puts each one in
//! front of `ui::confirm` with Cancel first, and does nothing at all to the
//! ones a person does not say yes to. That shape is deliberate. A single
//! "fix everything" prompt turns a list of unrelated decisions — this staged
//! file is litter, that one is a shim somebody is using — into one keystroke,
//! which is exactly the accident the confirmation exists to prevent.
//!
//! A confirmation is also not a warrant. Minutes pass between a report being
//! printed and a question being answered, so every repair carries the evidence
//! its finding was made on and re-checks it before acting — see `Repair`.
//!
//! Only one thing is ever repairable: Prelude's own private staging files.
//! Nothing here deletes a conversation, a Skill or an agent's configuration —
//! those have their own confirmed actions in the launcher, with their own
//! comparison steps, and a diagnostic is not a second way in.
//!
//! Nothing in this file may be reached from `cache::gather` or the
//! per-keystroke helper. `doctor skills` hashes every Skill tree on the
//! machine and `doctor agents` starts one subprocess per installed agent
//! CLI; both are fine for a command a person typed and waits for, and both
//! would be catastrophic on a path that runs on every keystroke.

use crate::ansi::*;
use crate::exec::which;
use crate::item::Kind;
use crate::paths;
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

// ---------------------------------------------------------------------------
// What a report is
// ---------------------------------------------------------------------------

/// Something Prelude is willing to do about a finding, once a person has said
/// yes to that finding in particular.
///
/// Deliberately a closed set of two. Both act only on files and records
/// Prelude wrote itself: a repair that reached into an agent's own directories
/// would be a destructive action arriving through a diagnostic, which is not a
/// door this codebase has anywhere else.
///
/// **Each one carries the evidence the finding was made on**, and re-checks it
/// before acting. A report is printed, read and then answered one question at
/// a time, so minutes pass between the observation and the act — and staging
/// names are deterministic (`borrow/<server>.json`, `borrow/<skill>/`), so a
/// borrow staged *while the confirmation is on screen* has exactly the name
/// the question is about. Without the evidence, "yes" to a sentence about a
/// week-old file trashes the one written two seconds ago. `agents_report`
/// already re-finds the fleet rather than trusting the launcher's snapshot;
/// this is the same rule applied to the half that actually changes something.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Repair {
    /// Move one of Prelude's own private staging entries to the Trash. Never
    /// `unlink`, never `remove_dir_all`, and never anything outside `borrow/`.
    Trash {
        path: String,
        /// Modification time in seconds, as observed. Every staging finding —
        /// stale, wrong mode, dangling shim — is a statement about the entry
        /// that was sitting there, and a rewrite is what makes it a different
        /// entry under the same name.
        modified: u64,
        /// Permission bits, as observed. `staged-permissions` is literally
        /// about these, and a file chmodded back to 0600 in the meantime is no
        /// longer the finding.
        mode: u32,
    },
}

impl Repair {
    /// The affirmative row in the confirmation. A decision, not a reflex —
    /// "Move the staged file to the Trash", never "Are you sure?".
    fn go_ahead(&self) -> String {
        match self {
            Repair::Trash { path, .. } => {
                let name = Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone());
                format!("Move {name} to the Trash")
            }
        }
    }

    fn detail(&self) -> String {
        match self {
            Repair::Trash { path, .. } => paths::tilde(path),
        }
    }

    fn apply(&self) -> Result<String, String> {
        match self {
            Repair::Trash { .. } => self.trash_in(&staging_root(), paths::trash),
        }
    }

    /// The staging half, with the root and the mover passed in so the decision
    /// can be tested without a test suite moving a real file through the real
    /// Trash. Production passes `paths::trash`, and nothing else ever should:
    /// it is the only mover in this codebase that leaves the thing recoverable.
    fn trash_in(
        &self,
        root: &Path,
        mv: impl Fn(&Path) -> Result<std::path::PathBuf, String>,
    ) -> Result<String, String> {
        use std::os::unix::fs::PermissionsExt;
        let Repair::Trash { path, modified, mode } = self;
        let path = Path::new(path);
        let shown = paths::tilde(&path.to_string_lossy());
        // The boundary is re-checked here rather than trusted from the report:
        // a report can be minutes old, and this is the last moment before
        // something moves.
        if !inside(root, path) {
            return Err(format!("{shown} is not one of Prelude's own staging entries"));
        }
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| format!("{shown} could not be read any more: {e}"))?;
        let now = (mtime_of(&meta), meta.permissions().mode() & 0o777);
        if now != (*modified, *mode) {
            return Err(format!(
                "{shown} has been written or chmodded since the report — this is no longer the \
                 entry that finding was about, so it is left alone; run the report again"
            ));
        }
        let dest = mv(path)?;
        Ok(format!("moved to {}", paths::tilde(&dest.to_string_lossy())))
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Issue {
    /// Stable machine code. An agent reading `--json` matches on this; the
    /// prose beside it is for a person and may be reworded.
    pub code: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair: Option<Repair>,
}

/// One subject — an agent, a conversation, a Skill, a server — and everything
/// wrong with it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Row {
    pub subject: String,
    /// The dim trailing summary on the subject's own line.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<Issue>,
    /// Statements of fact that are not faults: an unknown that the CLI will
    /// not answer, a deliberate disabled state, a count worth knowing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// What this Agent's CLI said about the configuration a run actually
    /// resolves — structured here, and rendered from this same record in the
    /// table, so an agent reading `--json` matches fields rather than parsing
    /// the prose a person reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<crate::sources::agents::ConfigEvidence>,
}

impl Row {
    fn new(subject: impl Into<String>, summary: impl Into<String>) -> Row {
        Row {
            subject: subject.into(),
            summary: summary.into(),
            ok: true,
            ..Default::default()
        }
    }

    fn issue(&mut self, code: &str, detail: impl Into<String>) {
        self.ok = false;
        self.issues.push(Issue { code: code.into(), detail: detail.into(), repair: None });
    }

    fn fixable(&mut self, code: &str, detail: impl Into<String>, repair: Repair) {
        self.ok = false;
        self.issues.push(Issue {
            code: code.into(),
            detail: detail.into(),
            repair: Some(repair),
        });
    }

    fn note(&mut self, text: impl Into<String>) {
        self.notes.push(text.into());
    }
}

pub struct Report {
    check: &'static str,
    rows: Vec<Row>,
}

impl Report {
    fn new(check: &'static str, rows: Vec<Row>) -> Report {
        Report { check, rows }
    }

    fn attention(&self) -> usize {
        self.rows.iter().filter(|row| !row.ok).count()
    }

    fn exit(&self) -> i32 {
        if self.attention() == 0 { 0 } else { 1 }
    }

    /// Every finding that carries a repair, paired with the subject it belongs
    /// to. Reading this list changes nothing; `--repair` is what asks.
    fn repairable(&self) -> Vec<(&str, &Issue)> {
        self.rows
            .iter()
            .flat_map(|row| {
                row.issues
                    .iter()
                    .filter(|issue| issue.repair.is_some())
                    .map(move |issue| (row.subject.as_str(), issue))
            })
            .collect()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "check": self.check,
            "ok": self.attention() == 0,
            "attention": self.attention(),
            "rows": self.rows,
        })
    }

    /// How many findings of one kind a table shows before it says how many
    /// more there are.
    ///
    /// A cap on the *rendering*, never on the data: a machine with six hundred
    /// conversations in an iCloud folder that is no longer synced has sixty
    /// missing projects, and sixty identical paragraphs is a report nobody
    /// reads. `--json` still carries every one, because an agent counting them
    /// is exactly the case this cap must not break.
    fn print(&self) {
        const MAX_SHOWN: usize = 8;
        println!("\n{CYAN}Prelude doctor · {}{RESET}\n", self.check);
        if self.rows.is_empty() {
            println!("  {DIM}nothing to check here{RESET}\n");
            return;
        }
        let width = self
            .rows
            .iter()
            .map(|row| crate::width::dwidth(&row.subject))
            .max()
            .unwrap_or(0)
            .clamp(8, 40);
        for row in &self.rows {
            let mark = if row.ok { format!("{GREEN}✓{RESET}") } else { format!("{YELLOW}✗{RESET}") };
            let subject = crate::width::pad_to(&crate::width::dtrunc(&row.subject, width), width, false);
            let summary = if row.summary.is_empty() {
                String::new()
            } else {
                format!("  {DIM}{}{RESET}", row.summary)
            };
            println!("  {mark} {subject}{summary}");
            for issue in row.issues.iter().take(MAX_SHOWN) {
                let fix = if issue.repair.is_some() { format!("  {DIM}· --repair offers this{RESET}") } else { String::new() };
                println!("      {YELLOW}{}{RESET}  {DIM}[{}]{RESET}{fix}", issue.detail, issue.code);
            }
            if row.issues.len() > MAX_SHOWN {
                println!("      {DIM}… and {} more like it — `--json` prints every one{RESET}",
                         row.issues.len() - MAX_SHOWN);
            }
            for note in row.notes.iter().take(MAX_SHOWN) {
                println!("      {DIM}{note}{RESET}");
            }
            if row.notes.len() > MAX_SHOWN {
                println!("      {DIM}… and {} more{RESET}", row.notes.len() - MAX_SHOWN);
            }
            // Never capped. Effective configuration is a handful of lines by
            // construction — the allowlists in `agents.rs` see to that — and
            // hiding half of an answer about what a run is configured with
            // would be worse than not asking.
            for line in row.config.iter().flat_map(crate::sources::agents::evidence_lines) {
                println!("      {DIM}{line}{RESET}");
            }
        }
        let attention = self.attention();
        println!();
        if attention == 0 {
            println!("  {DIM}{} subject{} checked · nothing needs attention{RESET}",
                     self.rows.len(), if self.rows.len() == 1 { "" } else { "s" });
        } else {
            println!("  {DIM}{attention} of {} need attention{RESET}", self.rows.len());
        }
        let repairs = self.repairable().len();
        if repairs > 0 {
            println!(
                "  {DIM}{repairs} of those can be repaired — `prelude doctor {} --repair` \
                 asks about each one separately{RESET}",
                self.check
            );
        }
        println!();
    }
}

/// How a report is being asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Text,
    Json,
    Repair,
}

/// `--json` and `--repair` are refused together rather than ordered.
///
/// `--json` exists so an agent can read fields; `--repair` is a sequence of
/// questions put to a person on a terminal. A command that claimed to be both
/// would either print JSON nobody could answer or block an agent on an fzf
/// prompt it cannot see.
fn mode_of(flags: &[&str]) -> Result<Mode, String> {
    let json = flags.contains(&"--json");
    let repair = flags.contains(&"--repair");
    if let Some(unknown) = flags.iter().find(|f| !matches!(**f, "--json" | "--repair")) {
        return Err(format!("unknown option {unknown}"));
    }
    match (json, repair) {
        (true, true) => Err(
            "--json and --repair cannot be combined: --json is for an agent reading fields, \
             --repair asks a person one question per finding"
                .into(),
        ),
        (true, false) => Ok(Mode::Json),
        (false, true) => Ok(Mode::Repair),
        (false, false) => Ok(Mode::Text),
    }
}

fn emit(report: Report, mode: Mode) -> i32 {
    match mode {
        Mode::Json => {
            println!("{}", serde_json::to_string_pretty(&report.json()).unwrap_or_else(|_| "{}".into()));
            report.exit()
        }
        Mode::Text => {
            report.print();
            report.exit()
        }
        Mode::Repair if !interactive() => {
            report.print();
            eprintln!(
                "prelude: --repair asks one question per finding and needs a terminal to ask on. \
                 Run it yourself, or read the findings with --json."
            );
            2
        }
        Mode::Repair => {
            report.print();
            walk_repairs(&report)
        }
    }
}

/// Is there a person here to answer?
///
/// `ui::confirm` draws with fzf, which takes over the controlling terminal and
/// waits for a keypress. With no terminal it does not fail — it sits there
/// for ever, so `prelude doctor skills --repair` in a script hangs the script
/// rather than declining. The door is closed before it is opened, which is
/// also the safer failure: a confirmation nobody can see must never be
/// answered on their behalf.
fn interactive() -> bool {
    unsafe extern "C" {
        unsafe fn isatty(fd: i32) -> i32;
    }
    unsafe { isatty(0) == 1 }
}

/// Ask about each repairable finding on its own, and do nothing to the rest.
///
/// The exit code is still the report's: repairing three of five findings does
/// not make the machine healthy, and a script that trusted a zero here would
/// be trusting a claim nobody made.
fn walk_repairs(report: &Report) -> i32 {
    let pending = report.repairable();
    if pending.is_empty() {
        println!("  {DIM}nothing in this report can be repaired automatically{RESET}\n");
        return report.exit();
    }
    println!("  {CYAN}repairs{RESET} {DIM}· each one is asked separately, and Cancel is the default{RESET}\n");
    for (subject, issue) in pending {
        let Some(repair) = issue.repair.as_ref() else { continue };
        if !crate::ui::confirm(&format!(" {subject} — {} ", issue.code), &repair.go_ahead(), &repair.detail()) {
            println!("    {DIM}left alone:{RESET} {subject}  {DIM}[{}]{RESET}", issue.code);
            continue;
        }
        match repair.apply() {
            Ok(done) => println!("    {GREEN}✓{RESET} {subject}  {DIM}{done}{RESET}"),
            Err(e) => println!("    {YELLOW}✗{RESET} {subject}  {YELLOW}{e}{RESET}"),
        }
    }
    println!();
    report.exit()
}

/// `prelude doctor <check> [--json|--repair]`.
pub fn dispatch(rest: &[&str]) -> i32 {
    let (check, flags) = match rest.split_first() {
        Some((check, flags)) => (*check, flags),
        None => return run(),
    };
    let mode = match mode_of(flags) {
        Ok(mode) => mode,
        Err(e) => {
            eprintln!("prelude: {e}");
            return 2;
        }
    };
    match check {
        "agents" => emit(agents_report(), mode),
        "sessions" => emit(sessions_report(), mode),
        "skills" => emit(skills_report(), mode),
        "mcp" => emit(mcp_report(), mode),
        other => {
            eprintln!(
                "prelude: no doctor called {other} — try agents, sessions, skills or mcp"
            );
            2
        }
    }
}

// ---------------------------------------------------------------------------
// doctor agents
// ---------------------------------------------------------------------------

/// How an Agent CLI answers "who am I logged in as".
///
/// One variant per *CLI*, not per shape of answer, because these genuinely do
/// not agree with each other and the differences are the whole reason honest
/// reporting is hard here. Nothing is inferred from a config file or a
/// keychain: if the CLI will not say, the answer is `unknown`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Login {
    /// `claude auth status --json` → `{"loggedIn": bool, "authMethod": "…"}`.
    /// The same object also carries the account's e-mail address, organization
    /// id and organization name. Those are read past and never printed: a
    /// diagnostic that a person pastes into an issue must not carry their
    /// identity with it.
    ClaudeJson,
    /// `codex login status` → one line, on **stderr**, `Logged in using
    /// ChatGPT` or `Not logged in`.
    CodexLine,
    /// `opencode auth list` → a count of stored provider credentials. Not a
    /// login state: opencode is logged in to zero or more providers, and
    /// "which one would this run use" is not a question it answers here.
    OpencodeCount,
    /// The CLI has no whole-account login at all. pi authenticates per
    /// provider (`pi auth check --provider …`), so there is no single state to
    /// report and Prelude reports none.
    PerProvider,
}

struct Probe {
    agent: &'static str,
    /// Every Agent CLI here prints its version to stdout for `--version`.
    version: &'static [&'static str],
    login: Login,
}

/// The four Agents the Control Plane models — kept in the same order as the
/// typed `agent::SPECS` registry and pinned to it by a test. Other agent-shaped CLIs on
/// PATH are named in a note rather than given a row: Prelude does not track
/// their Runs or Sessions, so it has nothing to say about their health.
const PROBES: &[Probe] = &[
    Probe { agent: "claude", version: &["claude", "--version"], login: Login::ClaudeJson },
    Probe { agent: "codex", version: &["codex", "--version"], login: Login::CodexLine },
    Probe { agent: "pi", version: &["pi", "--version"], login: Login::PerProvider },
    Probe { agent: "opencode", version: &["opencode", "--version"], login: Login::OpencodeCount },
];

/// What a login probe concluded. `Unknown` is a first-class answer.
#[derive(Clone, PartialEq, Eq, Debug)]
enum LoginState {
    In(String),
    Out(String),
    Unknown(String),
}

/// A short descriptor is safe to print; anything else is dropped.
///
/// `codex login status` says "Logged in using ChatGPT" today and could say
/// "Logged in as someone@example.com" tomorrow. The method word is worth
/// showing and an account identifier is not, so only a short run of plain
/// characters with no `@` in it survives.
fn safe_method(raw: &str) -> String {
    let m = raw.trim().trim_matches('"');
    let plain = !m.is_empty()
        && m.chars().count() <= 24
        && m.chars().all(|c| c.is_ascii_alphanumeric() || " ._+-".contains(c));
    if plain && !crate::secrets::looks_secret(m) { m.to_string() } else { String::new() }
}

/// Decide a login state from what a CLI actually printed.
///
/// Pure, so the four CLIs' real output can be pinned by tests without any of
/// them being installed.
fn read_login(login: Login, text: &str) -> LoginState {
    let low = text.to_lowercase();
    match login {
        Login::ClaudeJson => {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
                return LoginState::Unknown("`claude auth status --json` printed no JSON".into());
            };
            // Read exactly two fields. `email`, `orgId` and `orgName` are in
            // the same object and are deliberately never touched.
            let method = v.get("authMethod").and_then(|m| m.as_str()).map(safe_method).unwrap_or_default();
            match v.get("loggedIn").and_then(|l| l.as_bool()) {
                Some(true) => LoginState::In(if method.is_empty() { "signed in".into() } else { method }),
                Some(false) => LoginState::Out("`claude auth login` to sign in".into()),
                None => LoginState::Unknown("`claude auth status` reported no loggedIn field".into()),
            }
        }
        Login::CodexLine => {
            if low.contains("not logged in") {
                LoginState::Out("`codex login` to sign in".into())
            } else if low.contains("logged in") {
                let method = text.split_once("using ").map(|(_, m)| safe_method(m)).unwrap_or_default();
                LoginState::In(if method.is_empty() { "signed in".into() } else { method })
            } else {
                LoginState::Unknown("`codex login status` said nothing this understands".into())
            }
        }
        Login::OpencodeCount => {
            // The count is the only field; the surrounding box is drawn with
            // ANSI and box-drawing characters, so the digits are found rather
            // than parsed positionally.
            let n = low.split_whitespace().collect::<Vec<_>>().windows(2).find_map(|pair| {
                (pair[1].starts_with("credential")).then(|| pair[0].parse::<usize>().ok()).flatten()
            });
            match n {
                Some(0) => LoginState::Out("`opencode auth login` — no provider credentials stored".into()),
                Some(n) => LoginState::In(format!("{n} provider credential{}", if n == 1 { "" } else { "s" })),
                None => LoginState::Unknown("`opencode auth list` reported no credential count".into()),
            }
        }
        Login::PerProvider => LoginState::Unknown(
            "pi authenticates per provider — `pi auth check --provider NAME` is the only answer there is"
                .into(),
        ),
    }
}

/// Run a login probe, merging stderr where the CLI writes its answer there.
///
/// `codex login status` prints to stderr, and `exec::run` discards stderr by
/// design — a source must not be noisy. The redirection is a fixed string with
/// no interpolation, so there is nothing for a shell to expand.
fn login_output(login: Login) -> String {
    let d = Duration::from_secs(20);
    match login {
        Login::ClaudeJson => crate::exec::run(&["claude", "auth", "status", "--json"], d),
        Login::CodexLine => crate::exec::run(&["sh", "-c", "codex login status 2>&1"], d),
        Login::OpencodeCount => crate::exec::run(&["opencode", "auth", "list"], d),
        Login::PerProvider => String::new(),
    }
}

/// Start asking every installed Agent CLI what configuration it actually
/// resolves, and hand back the threads rather than the answers.
///
/// One subprocess each, and `codex doctor --json` alone costs about 1.8
/// seconds because it checks provider reachability and completes a websocket
/// handshake before it prints. Sequentially the report would pay the sum of
/// every probe; started together and collected where they are needed, it pays
/// the slowest one, and the version and login subprocesses below run inside
/// that same window. It is the shape `cache`'s FAST sources are built around,
/// for the same reason. Nothing here may be reached from `gather` or the
/// per-keystroke helper.
type ConfigProbes =
    std::collections::BTreeMap<&'static str, std::thread::JoinHandle<crate::sources::agents::ConfigEvidence>>;

fn start_config_probes() -> ConfigProbes {
    PROBES
        .iter()
        .filter(|probe| which(probe.agent).is_some())
        .map(|probe| {
            let agent = probe.agent;
            (agent, std::thread::spawn(move || crate::sources::agents::effective_config(agent)))
        })
        .collect()
}

fn agent_rows(mut configs: ConfigProbes) -> Vec<Row> {
    let mut rows = Vec::new();
    for probe in PROBES {
        let Some(path) = which(probe.agent) else {
            let mut row = Row::new(probe.agent, "not installed");
            row.note("no binary on PATH; nothing to check");
            rows.push(row);
            continue;
        };
        // A version costs a subprocess, which is exactly what a command a
        // person typed is allowed to spend.
        let version = crate::exec::run(probe.version, Duration::from_secs(20));
        let version = version.lines().next().unwrap_or("").trim().to_string();
        let mut row = Row::new(
            probe.agent,
            if version.is_empty() { paths::tilde(&path.to_string_lossy()) } else { version.clone() },
        );
        if version.is_empty() {
            row.issue("no-version", format!("`{} --version` printed nothing", probe.agent));
        }
        match read_login(probe.login, &login_output(probe.login)) {
            LoginState::In(how) => row.note(format!("logged in · {how}")),
            LoginState::Out(how) => row.issue("not-logged-in", format!("not logged in — {how}")),
            LoginState::Unknown(why) => row.note(format!("login state unknown · {why}")),
        }
        match crate::sources::agents::config_for(probe.agent) {
            None => row.note("no settings file of its own here"),
            Some(config) => match std::fs::read_to_string(&config) {
                Err(e) => row.issue("config-unreadable", format!("{}: {e}", paths::tilde(&config))),
                Ok(text) => {
                    // Only strict `.json` is parse-checked. opencode's file is
                    // `.jsonc` and codex's is TOML; declaring a valid file
                    // broken because Prelude read it with the wrong parser
                    // would be worse than not checking.
                    if config.ends_with(".json")
                        && serde_json::from_str::<serde_json::Value>(&text).is_err()
                    {
                        row.issue(
                            "config-malformed",
                            format!("{} is not valid JSON, so the agent is running on defaults", paths::tilde(&config)),
                        );
                    } else {
                        row.note(format!("settings readable · {}", paths::tilde(&config)));
                    }
                }
            },
        }
        // Readable is not the same as effective. The check above says a file
        // parses; this says what the CLI resolved out of every file, variable
        // and flag together — or, where the CLI will not say, that it will not.
        if let Some(evidence) = configs.remove(probe.agent).and_then(|probe| probe.join().ok()) {
            if let Some(trouble) = evidence.trouble.clone() {
                row.issue("config-effective-unreadable", trouble);
            } else if evidence.status.as_deref().is_some_and(|status| status != "ok") {
                row.issue(
                    "config-effective-not-ok",
                    format!(
                        "{} reports its own configuration load as `{}`",
                        probe.agent,
                        evidence.status.clone().unwrap_or_default()
                    ),
                );
            }
            row.config = Some(evidence);
        }
        rows.push(row);
    }
    let others: Vec<&str> = ["cursor-agent", "gemini"]
        .iter()
        .copied()
        .filter(|c| which(c).is_some())
        .collect();
    if !others.is_empty() {
        let mut row = Row::new("other CLIs", others.join("  "));
        row.note("agent-shaped, but Prelude models no Runs or Sessions for them");
        rows.push(row);
    }
    rows
}

/// Runs whose relationship to a conversation is missing or unresolved.
///
/// `running.rs` already decided this and wrote it down; nothing here
/// recomputes it. `session_match` is `explicit`, `cwd-latest`, `ambiguous`,
/// `requested-missing`, or absent when one run stands alone in a project with
/// no conversation to attach to.
fn run_rows(runs: &[crate::item::Item]) -> Vec<Row> {
    let mut rows = Vec::new();
    for run in runs {
        let id = run.get("run_id");
        let subject = if id.is_empty() { run.get("agent").to_string() } else { id.to_string() };
        let project = paths::tilde(run.get("cwd"));
        let mut row = Row::new(subject, format!("{} · {project}", run.get("agent")));
        match run.get("session_match") {
            "ambiguous" => row.issue(
                "session-ambiguous",
                format!(
                    "more than one {} runs in this project, so no conversation can be attached to this one",
                    run.get("agent")
                ),
            ),
            "requested-missing" => row.issue(
                "session-requested-missing",
                format!(
                    "resumed `{}`, and no conversation file with that id was found",
                    run.get("session_native_id")
                ),
            ),
            "explicit" | "cwd-latest" => {}
            // Batch runs (`claude -p`, `codex exec`) keep no conversation file
            // at all, so having no Session is what they are, not a fault.
            _ if run.get("batch") == "1" => {
                row.note("batch run · keeps no conversation file, so it has no Session by design");
            }
            // A run with no readable working directory had nothing to match a
            // conversation against in the first place. Same missing edge,
            // different cause, and saying "no conversation" without saying why
            // sends somebody looking for a session file that was never the
            // problem.
            _ if run.get("cwd").is_empty() => row.issue(
                "run-without-session",
                "no conversation is attached, and its working directory could not be read — so there was nothing to match one against",
            ),
            _ => row.issue(
                "run-without-session",
                "no conversation file is attached to this run",
            ),
        }
        rows.push(row);
    }
    rows
}

/// Inbox records addressed to something that no longer exists.
///
/// A message left for an agent is collected by working directory, so a
/// directory that has gone leaves a record nothing can ever pick up. Reported
/// and never swept here: an uncollected instruction is exactly the thing that
/// must not disappear because a diagnostic tidied up.
fn inbox_rows() -> Vec<Row> {
    let mut rows = Vec::new();
    let messages = crate::bus::all();
    let mut inbox_row = Row::new("inbox", format!("{} messages on the bus", messages.len()));
    let stranded: Vec<&crate::bus::Msg> = messages
        .iter()
        .filter(|m| m.to != "human" && m.answer.is_none())
        .filter(|m| {
            undeliverable(&m.to_cwd, !m.to_cwd.is_empty() && Path::new(&m.to_cwd).is_dir())
        })
        .collect();
    if !stranded.is_empty() {
        for m in stranded.iter().take(6) {
            inbox_row.issue(
                "inbox-unreachable",
                format!(
                    "{} for `{}` names a project that is not there, so nobody can collect it",
                    m.id, m.to
                ),
            );
        }
        if stranded.len() > 6 {
            inbox_row.note(format!("and {} more like it", stranded.len() - 6));
        }
        inbox_row.note("a diagnostic reports these; it will not collect or answer them for you");
    }
    rows.push(inbox_row);
    rows
}

/// Can this message still reach anybody?
///
/// The working directory is the whole of the address, so this is the whole of
/// the question. There used to be a second address, a tmux pane, and a message
/// counted as stranded only when both were gone — a closed pane was normal
/// while the project was still there, and a moved project was normal while the
/// pane was open.
fn undeliverable(cwd: &str, cwd_exists: bool) -> bool {
    cwd.is_empty() || !cwd_exists
}

fn agents_report() -> Report {
    // Started first and collected last. The slowest of them is `codex doctor
    // --json`, which talks to the network, and re-finding the fleet below is
    // the other second this report spends — running them past each other costs
    // one of those seconds rather than both.
    let configs = start_config_probes();
    // An explicit diagnostic re-finds the fleet rather than reading the
    // launcher's snapshot: `fleet.rs` does the same, for the same reason —
    // somebody asking now wants what is true now.
    let sessions = crate::sources::sessions::all();
    let runs = crate::sources::running::fresh_identities_with_sessions(&sessions);
    let sessions = crate::sources::running::annotate_sessions(sessions, &runs);
    let mut rows = agent_rows(configs);
    let mut fleet = Row::new("fleet", format!("{} run{} found", runs.len(), if runs.len() == 1 { "" } else { "s" }));
    fleet.note(format!("{} conversations recorded across every agent", sessions.len()));
    // Said here rather than repeated under every run: the effective
    // configuration above belongs to the agents, not to these processes.
    fleet.note(format!("effective config is {}", crate::sources::agents::RUN_SCOPE));
    rows.push(fleet);
    rows.extend(run_rows(&runs));
    rows.extend(inbox_rows());
    Report::new("agents", rows)
}

// ---------------------------------------------------------------------------
// doctor sessions
// ---------------------------------------------------------------------------

fn trouble_code(trouble: &crate::sources::sessions::Trouble) -> &'static str {
    use crate::sources::sessions::Trouble::*;
    match trouble {
        MissingProject => "missing-project",
        UnreadableFile => "unreadable-file",
        MalformedIndex => "malformed-index",
        UnreadableRoot => "unreadable-root",
        UnreadableMetadata => "unreadable-metadata",
    }
}

/// The subject a trouble belongs under, and what its heading says.
///
/// Grouped by *kind* rather than by conversation, because these are not one
/// fault per conversation. One iCloud folder that stopped syncing takes sixty
/// conversations with it, and sixty rows saying the same sentence about the
/// same directory is a report that hides the one fact in it: a directory is
/// missing, and here is how much is behind it.
fn trouble_subject(trouble: &crate::sources::sessions::Trouble) -> &'static str {
    use crate::sources::sessions::Trouble::*;
    match trouble {
        MissingProject => "missing projects",
        UnreadableFile => "unreadable conversations",
        MalformedIndex => "malformed conversations",
        UnreadableRoot => "session roots",
        UnreadableMetadata => "sessions.json",
    }
}

fn sessions_report() -> Report {
    use crate::sources::sessions;
    use std::collections::BTreeMap;

    // `all()`, never the finished launcher list. `cache::finish` dedupes on
    // `(kind, cmd)` and two files sharing one Session id produce an identical
    // `cmd`, so a finished list has already thrown away the exact rows
    // `duplicate_sessions` exists to find — it would report nothing, for ever,
    // and look correct doing it.
    let all = sessions::all();
    let mut rows = Vec::new();

    let mut per_agent: BTreeMap<&str, usize> = BTreeMap::new();
    for session in &all {
        *per_agent.entry(session.get("agent")).or_default() += 1;
    }
    let mut inventory = Row::new(
        "inventory",
        format!("{} conversation{}", all.len(), if all.len() == 1 { "" } else { "s" }),
    );
    if per_agent.is_empty() {
        inventory.note("no native conversation files found on this machine");
    } else {
        inventory.note(
            per_agent.iter().map(|(agent, n)| format!("{agent} {n}")).collect::<Vec<_>>().join(" · "),
        );
    }
    rows.push(inventory);

    // Grouped by kind, and within `missing projects` by the directory itself:
    // the fact worth reporting is that one directory has gone, not that each
    // of the forty conversations in it noticed separately.
    let problems = sessions::session_problems(&all);
    let mut grouped: BTreeMap<&'static str, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for problem in &problems {
        grouped
            .entry(trouble_subject(&problem.trouble))
            .or_default()
            .entry(problem.detail.clone())
            .or_default()
            .push(problem.session.clone());
    }
    for (subject, by_detail) in grouped {
        let sessions_hit: usize = by_detail.values().map(Vec::len).sum();
        let mut row = Row::new(
            subject,
            format!(
                "{} · {} conversation{}",
                by_detail.len(),
                sessions_hit,
                if sessions_hit == 1 { "" } else { "s" }
            ),
        );
        for (detail, ids) in &by_detail {
            let shown = paths::tilde(detail);
            let code = problems
                .iter()
                .find(|p| p.detail == *detail)
                .map(|p| trouble_code(&p.trouble))
                .unwrap_or("session-problem");
            let tail = match ids.len() {
                0 | 1 => String::new(),
                n => format!(" ({n} conversations)"),
            };
            row.issue(code, format!("{shown}{tail}"));
        }
        row.note(match subject {
            "missing projects" => "resuming these would start the agent somewhere else, or not at all",
            "unreadable conversations" => "moved, deleted, or on a volume that is not mounted",
            "malformed conversations" => "truncated, or written in a format this Prelude cannot read",
            "session roots" => "every conversation under an unopenable root is simply absent from the launcher",
            _ => "local names, pins and archive flags are being ignored until this parses",
        });
        row.note("reported only: which file to keep is a decision Prelude does not have the context to make");
        rows.push(row);
    }

    let duplicates = sessions::duplicate_sessions(&all);
    let mut same_id = Row::new("duplicate ids", "");
    let mut same_file = Row::new("duplicate records", "");
    for duplicate in &duplicates {
        match duplicate.kind {
            sessions::DuplicateKind::SameIdManyFiles => same_id.issue(
                "duplicate-session-id",
                format!(
                    "{} is claimed by {} files, so resuming it may continue the wrong conversation — newest is {}",
                    duplicate.ids.first().cloned().unwrap_or_default(),
                    duplicate.paths.len(),
                    paths::tilde(&duplicate.newest)
                ),
            ),
            sessions::DuplicateKind::SameFileManyIds => same_file.issue(
                "duplicate-session-file",
                format!(
                    "{} appears under {} ids, so its name, pin and archive state attach to only one of them — newest is {}",
                    paths::tilde(&duplicate.paths.first().cloned().unwrap_or_default()),
                    duplicate.ids.len(),
                    duplicate.newest
                ),
            ),
        }
    }
    for mut row in [same_id, same_file] {
        if row.issues.is_empty() {
            continue;
        }
        row.summary = format!("{} found", row.issues.len());
        row.note("reported only: either file may be the one you want, and this cannot know which");
        rows.push(row);
    }
    Report::new("sessions", rows)
}

// ---------------------------------------------------------------------------
// doctor skills
// ---------------------------------------------------------------------------

fn skills_report() -> Report {
    let started = std::time::Instant::now();
    // This walks and hashes every Skill tree on the machine. It is the reason
    // the doc comment at the top of this file says what it says.
    let report = crate::capability::skill_diagnostics();
    let ms = started.elapsed().as_secs_f64() * 1000.0;

    let mut rows = Vec::new();
    let (mut files, mut bytes, mut copies) = (0u64, 0u64, 0usize);
    for skill in &report.skills {
        for copy in &skill.copies {
            copies += 1;
            files += copy.files;
            bytes += copy.bytes;
        }
        rows.push(skill_row(skill));
    }

    for collision in &report.collisions {
        let mut row = Row::new(
            format!("{} · {}", collision.agent, collision.name),
            "name collision",
        );
        let detail = collision.paths.iter().map(|p| paths::tilde(p)).collect::<Vec<_>>().join("  ");
        match collision.kind.as_str() {
            "case" => row.issue(
                "case-collision",
                format!(
                    "two directories differ only by case, which is one directory here and two on a case-sensitive filesystem: {detail}"
                ),
            ),
            _ => row.issue(
                "duplicate-name",
                format!("two directories claim this name under one agent, so one of them is invisible: {detail}"),
            ),
        }
        rows.push(row);
    }

    rows.extend(staging_rows(Staged::Shims));

    let mut cost = Row::new("cost", format!("{ms:.0}ms"));
    cost.note(format!(
        "hashed {copies} Skill cop{} · {files} files · {} — fresh every time, and never on the gather path",
        if copies == 1 { "y" } else { "ies" },
        human_bytes(bytes)
    ));
    rows.push(cost);
    Report::new("skills", rows)
}

/// One Skill's row, from the health record and nothing else, so the whole
/// rendering can be tested against fixtures rather than against whatever this
/// machine happens to have installed.
fn skill_row(skill: &crate::capability::SkillHealth) -> Row {
    let mut row = Row::new(
        skill.name.clone(),
        format!(
            "{} · {} cop{}",
            skill.integrity,
            skill.copies.len(),
            if skill.copies.len() == 1 { "y" } else { "ies" }
        ),
    );
    match skill.integrity.as_str() {
        "divergent" => row.issue(
            "divergent-copies",
            format!(
                "{} copies of this Skill differ; the launcher's Diff and Replace actions are the way to reconcile them",
                skill.copies.len()
            ),
        ),
        // Worded exactly as `compute::skill_is_sound` words it, because it is
        // one state and two reports saying different things about it is how a
        // person ends up believing the state means whichever they read first.
        // The *cause* differs by where you are standing and is said here
        // rather than assumed: these hashes were computed a moment ago, so a
        // copy with no fingerprint is one that could not be read — where the
        // launcher, reading the background cache, is usually just early.
        "unknown" => row.issue(
            "unhashed-copy",
            "at least one copy has no fingerprint, so nothing can be said about whether they match \
             — hashed just now, so that copy could not be read completely",
        ),
        "private-unknown" => row.note(
            "copies contain redacted credential-like lines, so equality cannot be claimed either way",
        ),
        _ => {}
    }
    // Which copies, and which of them is the newer.
    //
    // `skill_diagnostics` already carries a path, a fingerprint and an mtime
    // per copy, and this report used to discard all three — so `doctor skills`
    // could say two copies had diverged and nothing about where they were or
    // which way round to reconcile them, which is the only question the person
    // reading it has. Listed only where there is something to reconcile: on a
    // healthy machine an inventory of every copy of every Skill is the noise
    // that stops the faults being read.
    if unsettled(&skill.integrity) {
        for copy in &skill.copies {
            row.note(copy_line(copy));
        }
    }
    for copy in &skill.copies {
        for fault in &copy.faults {
            row.issue(&fault.code, format!("{}: {}", copy.agent, fault.detail));
        }
        if copy.unreadable > 0 {
            row.issue(
                "unreadable-files",
                format!("{}: {} file(s) in the tree could not be read", copy.agent, copy.unreadable),
            );
        }
        if copy.sensitive_files > 0 {
            row.note(format!(
                "{}: {} file(s) hold credential-like lines — fingerprinted as redaction markers, and never copied or lent",
                copy.agent, copy.sensitive_files
            ));
        }
    }
    row
}

/// Is this Skill's integrity a question rather than an answer? `single` and
/// `identical` are settled; everything else needs the copies named.
fn unsettled(integrity: &str) -> bool {
    matches!(integrity, "divergent" | "unknown" | "private-unknown")
}

/// One copy, worded like the Quick Look copy matrix on purpose: agent, short
/// fingerprint, path and age. Two descriptions of one comparison eventually
/// disagree about it.
///
/// A copy with no fingerprint is the reason the whole Skill is `unknown`, and
/// saying `unhashed` is the point of listing it — an empty column would read
/// as a rendering fault rather than as the finding.
fn copy_line(copy: &crate::capability::SkillCopyHealth) -> String {
    let hash = if copy.fingerprint.is_empty() {
        "unhashed"
    } else {
        copy.fingerprint.strip_prefix("fnv1a64-v1:").unwrap_or(&copy.fingerprint)
    };
    let when = crate::sources::user::ago(copy.modified as f64);
    let when = if when.is_empty() { String::new() } else { format!(" · {when}") };
    format!("{}: {hash} · {}{when}", copy.agent, paths::tilde(&copy.dir))
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: &[(&str, u64)] = &[("GB", 1 << 30), ("MB", 1 << 20), ("KB", 1 << 10)];
    for (unit, size) in UNITS {
        if bytes >= *size {
            return format!("{:.1}{unit}", bytes as f64 / *size as f64);
        }
    }
    format!("{bytes}B")
}

// ---------------------------------------------------------------------------
// doctor mcp
// ---------------------------------------------------------------------------

/// A server has not been health-checked recently enough to believe.
const HEALTH_TTL: u64 = 120;
/// The tool inventory's own, longer, TTL — it starts servers, so it runs far
/// less often.
const TOOLS_TTL: u64 = 600;

fn mcp_report() -> Report {
    // An explicit diagnostic may pay for authoritative owner CLI health.
    let _ = crate::cache::refresh_named("mcp");
    let servers = crate::cache::read_cached("mcp");
    let now = crate::frecency::now() as u64;
    let mut rows = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if servers.is_empty() {
        let mut row = Row::new("mcp", "none");
        row.issue("no-servers", "no MCP servers reported by the installed Agent CLIs");
        rows.push(row);
        return Report::new("mcp", rows);
    }

    for server in &servers {
        let owner = server.get("agent");
        let name = server.get("name");
        let transport = server.get("transport");
        let health = server.get("health");
        let tools_status = server.get("tools_status");
        let mut row = Row::new(
            format!("{owner} · {name}"),
            format!("{transport} · {health} · tools {tools_status}"),
        );

        if !seen.insert((owner.to_string(), name.to_lowercase())) {
            row.issue("duplicate-definition", "this owner defines this server name more than once");
        }

        // Health, with auth split out from failure. They arrive through the
        // same field and mean opposite things: an auth failure is a server
        // that works and does not know you, and telling somebody to debug a
        // connection when they simply need to log in wastes the afternoon.
        match health {
            "ok" => {}
            "auth" => row.issue(
                "needs-auth",
                format!(
                    "{owner} reports this server is not logged in — an authentication state, not a connection failure; \
                     the launcher's Login action for this row is the way in"
                ),
            ),
            "failed" => row.issue(
                "connection-failed",
                format!("{owner} could not reach this server"),
            ),
            "disabled" => {
                let reason = server.get("reason");
                row.note(if reason.is_empty() {
                    "deliberately disabled by its owner — not a fault".to_string()
                } else {
                    format!("deliberately disabled by its owner: {reason}")
                });
            }
            other => row.issue(
                "health-unknown",
                format!("{owner} reported no usable status for this server ({other})"),
            ),
        }

        let health_at = server.get("health_checked_at").parse::<u64>().unwrap_or(0);
        if health_at == 0 || now.saturating_sub(health_at) > HEALTH_TTL {
            row.issue("health-stale", "the health snapshot is stale or missing");
        }

        // Tool inventory, said in full. `unsupported` (Prelude has no owner
        // auth for an HTTP or account-hosted server), `disabled` (nothing to
        // ask) and `failed` (asked and it broke) are three different answers,
        // and only the last is a fault. An empty successful list is a fourth.
        let tools: Vec<crate::mcp_tools::Tool> =
            serde_json::from_str(server.get("tools")).unwrap_or_default();
        let error = server.get("tools_error");
        match tools_status {
            "ok" if tools.is_empty() => {
                row.note("tool inventory succeeded and this server offers no tools")
            }
            "ok" => row.note(format!(
                "{} tool{} inventoried: {}",
                tools.len(),
                if tools.len() == 1 { "" } else { "s" },
                tools.iter().take(8).map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")
            )),
            "unsupported" => row.note(format!(
                "tool inventory is not supported for this server{}",
                if error.is_empty() { String::new() } else { format!(" — {error}") }
            )),
            "disabled" => row.note("disabled, so there is nothing to inventory"),
            "pending" => row.note("tool inventory has not run yet; it is a background five-minute cache"),
            "failed" => row.issue(
                "tools-failed",
                format!(
                    "the tools/list handshake failed{}",
                    if error.is_empty() { String::new() } else { format!(" — {error}") }
                ),
            ),
            other => row.issue("tools-unknown", format!("unrecognised tool inventory state `{other}`")),
        }
        let tools_at = server.get("tools_checked_at").parse::<u64>().unwrap_or(0);
        let tools_countable = !matches!(tools_status, "unsupported" | "disabled" | "pending");
        if tools_countable && (tools_at == 0 || now.saturating_sub(tools_at) > TOOLS_TTL) {
            row.issue("tools-stale", "the tool inventory is stale or missing");
        }

        if !matches!(transport, "stdio" | "http" | "sse" | "hosted") {
            row.issue("transport-unknown", "the transport could not be normalized");
        }
        if !server.get("def").is_empty() {
            row.issue(
                "definition-retained",
                "a complete server definition was retained in the cache — a privacy violation",
            );
        }
        if server.get("sensitive") == "true" {
            row.note("private definition fields are omitted from retained data");
        }
        if server.get("portable") == "false" {
            row.note("owner-account only; no transferable local definition");
        }
        rows.push(row);
    }

    rows.extend(staging_rows(Staged::Files));
    Report::new("mcp", rows)
}

// ---------------------------------------------------------------------------
// Prelude's own private staging area
// ---------------------------------------------------------------------------

/// Anything under `borrow/` untouched for this long is litter rather than a
/// staged borrow: a borrow is used by the command it was staged for, minutes
/// later at the outside.
const STALE_STAGED: u64 = 7 * 24 * 60 * 60;

/// Which half of `borrow/` a report is asking about.
///
/// Two kinds of thing live there and they belong to different reports: the
/// claude plugin shims are Skill borrowing (`doctor skills`), and the 0600
/// JSON files are MCP server definitions (`doctor mcp`). One function finds
/// both so the ownership rule is written once.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Staged {
    Shims,
    Files,
}

fn staging_root() -> std::path::PathBuf {
    paths::cache().join("borrow")
}

/// A path is repairable only if it is a *direct child* of the staging root,
/// compared after `canonicalize` so `..` cannot dress something up as one.
///
/// The same rule `is_skill_dir` uses, for the same reason. A broken symlink
/// does not canonicalize, so the parent is canonicalized and the name checked
/// separately — otherwise the one entry most worth cleaning up would be the
/// one entry that could not be.
///
/// The root is a parameter, on `sessions.rs`'s reasoning:
/// repointing `$XDG_CACHE_HOME` for a test mutates state the whole process
/// shares, and the alternative is a test suite that writes into the person's
/// real `borrow/` directory to check a boundary.
fn inside(root: &Path, path: &Path) -> bool {
    let Ok(root) = root.canonicalize() else { return false };
    let Some(parent) = path.parent().and_then(|p| p.canonicalize().ok()) else { return false };
    parent == root && path.file_name().is_some()
}

fn age_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Modification time as a plain number of seconds, which is what a finding can
/// carry and compare later. Unreadable reads as 0 on both sides, so an entry
/// whose clock cannot be asked is simply never *seen* to have changed — the
/// boundary and the mode still have to hold.
fn mtime_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn staging_rows(want: Staged) -> Vec<Row> {
    staging_rows_in(&staging_root(), want)
}

fn staging_rows_in(root: &Path, want: Staged) -> Vec<Row> {
    use std::os::unix::fs::PermissionsExt;
    let subject = match want {
        Staged::Shims => "borrowed skills",
        Staged::Files => "staged definitions",
    };
    let shown_root = paths::tilde(&root.to_string_lossy());
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        // Never staged anything, or the directory has been cleaned. Not a
        // fault; a source that finds nothing says nothing.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut row = Row::new(subject, "none staged");
            row.note(format!("{shown_root} does not exist yet"));
            return vec![row];
        }
        // A root that is there and will not open is the opposite finding, and
        // reporting it as "does not exist yet" is the report saying everything
        // is fine about a directory it could not look inside. Prelude's own
        // cache being unreadable also means borrowing is broken, not idle.
        Err(e) => {
            let mut row = Row::new(subject, "unreadable");
            row.issue("staging-unreadable", format!("{shown_root} exists and could not be read: {e}"));
            row.note("staged borrows cannot be listed, and borrowing writes here — this is not an empty directory");
            return vec![row];
        }
    };
    let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();

    let mut row = Row::new(subject, String::new());
    let mut counted = 0usize;
    for path in paths {
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        let is_dir = meta.is_dir();
        if (want == Staged::Shims) != is_dir {
            continue;
        }
        counted += 1;
        let shown = paths::tilde(&path.to_string_lossy());
        let evidence = |path: &Path| Repair::Trash {
            path: path.to_string_lossy().into_owned(),
            modified: mtime_of(&meta),
            mode: meta.permissions().mode() & 0o777,
        };
        // A symlink is never something Prelude staged: a shim is a directory
        // and a definition is a 0600 file, and the symlinks borrowing does
        // create live one level down, inside a shim's `skills/`. It is also
        // the one entry a Trash repair cannot honour — `paths::trash` gates on
        // `exists()`, which follows the link, so a dangling one is refused at
        // the very moment it is confirmed. Offering a repair that can only
        // fail is worse than offering none, so this is reported and left.
        if meta.file_type().is_symlink() {
            row.issue(
                "staged-symlink",
                format!(
                    "{shown} is a symlink, which Prelude never stages here{} — remove it by hand",
                    if path.exists() { "" } else { ", and its target is gone" }
                ),
            );
            continue;
        }
        let age = age_of(&meta);
        if is_dir {
            for (code, detail) in shim_faults(&path) {
                row.fixable(code, format!("{shown}: {detail}"), evidence(&path));
            }
        } else {
            // A staged MCP definition routinely holds an API key. The mode is
            // set at creation for exactly that reason, so a file that is not
            // 0600 is either not ours or has been tampered with, and either
            // way it should not be sitting there.
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o600 {
                row.fixable(
                    "staged-permissions",
                    format!("{shown} is mode {mode:o}, not 0600 — a staged server definition can hold an API key"),
                    evidence(&path),
                );
                continue;
            }
        }
        if age > STALE_STAGED {
            row.fixable(
                "staged-stale",
                format!("{shown} was last written {} ago and no run is waiting on it", crate::sources::running::short_dur(age)),
                evidence(&path),
            );
        }
    }
    row.summary = format!("{counted} in {shown_root}");
    if counted == 0 {
        row.note("nothing staged");
    }
    vec![row]
}

/// What is wrong with one claude plugin shim.
///
/// The shim exists to point at somebody else's Skill; when that Skill has
/// moved or been deleted the shim is a dangling symlink inside a plugin claude
/// will still try to load.
fn shim_faults(root: &Path) -> Vec<(&'static str, String)> {
    let mut faults = Vec::new();
    if !root.join(".claude-plugin").join("plugin.json").is_file() {
        faults.push(("shim-incomplete", "the plugin manifest is missing, so claude cannot load it".to_string()));
    }
    let skills = root.join("skills");
    match std::fs::read_dir(&skills) {
        Err(_) => faults.push((
            "shim-incomplete",
            "the shim holds no skills directory".to_string(),
        )),
        Ok(entries) => {
            let mut any = false;
            for entry in entries.flatten() {
                any = true;
                // `exists` follows the link, which is exactly the question:
                // is the owner's Skill still there?
                if !entry.path().exists() {
                    faults.push((
                        "shim-target-missing",
                        format!(
                            "it links to a Skill that is no longer there ({})",
                            entry.file_name().to_string_lossy()
                        ),
                    ));
                }
            }
            if !any {
                faults.push(("shim-incomplete", "the shim links to no Skill at all".to_string()));
            }
        }
    }
    faults
}

// ---------------------------------------------------------------------------
// The original whole-setup doctor
// ---------------------------------------------------------------------------

pub fn run() -> i32 {
    let mut ok = true;
    let mut check = |label: String, good: bool, detail: &str| {
        let mark = if good { format!("{GREEN}✓{RESET}") } else { format!("{YELLOW}✗{RESET}") };
        let d = if detail.is_empty() { String::new() } else { format!("  {DIM}{detail}{RESET}") };
        println!("  {mark} {label}{d}");
        if !good {
            ok = false;
        }
    };

    println!("\n{CYAN}Prelude doctor{RESET}\n");
    let fzf = which("fzf");
    check("fzf installed".into(), fzf.is_some(),
          &fzf.map(|p| p.to_string_lossy().into_owned()).unwrap_or("brew install fzf".into()));
    check("zoxide (folder ranking)".into(), which("zoxide").is_some(), "optional");

    let global = crate::global::status();
    if global.app_installed || global.launch_agent_installed {
        check(
            "launcher panel installed".into(),
            global.app_installed && global.launch_agent_installed,
            "ghostty quick terminal",
        );
        check(
            "launcher panel supervision".into(),
            global.helper_supervised,
            "launchd keeps the panel available after a crash · prelude global start",
        );
        check(
            "launcher panel instance".into(),
            global.helper_running,
            "prelude global start",
        );
        check(
            "Ghostty Accessibility".into(),
            global.accessibility_granted == Some(true),
            "enable Ghostty in System Settings → Privacy & Security → Accessibility, then run: prelude global start",
        );
        check(
            "launcher zsh widget".into(),
            global.zsh_widget_available,
            "Ctrl+R only · add eval \"$(prelude init zsh)\" to ~/.zshrc",
        );
        let owner = global
            .hotkey_owner
            .clone()
            .unwrap_or_else(|| "another application".into());
        check(
            format!("global hotkey {} registered", global.selected_hotkey),
            global.hotkey_registered,
            &format!("{owner} may own it; free it or choose another, then run: prelude global start"),
        );
        check(
            "launcher panel running".into(),
            global.helper_running,
            if global.helper_running {
                "hidden Ghostty instance up"
            } else {
                "prelude global open"
            },
        );
    } else {
        check(
            "launcher panel".into(),
            true,
            "optional · install with: prelude global install",
        );
    }

    let hist = std::env::var("HISTFILE").map(std::path::PathBuf::from)
        .unwrap_or_else(|_| paths::home().join(".zsh_history"));
    check("shell history readable".into(), hist.exists(), &hist.to_string_lossy());

    let t = std::time::Instant::now();
    let items = crate::cache::gather();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    check(format!("gathered {} candidates", items.len()), !items.is_empty(), &format!("{ms:.0}ms"));
    check("gather under 40ms budget".into(), ms < 40.0, &format!("{ms:.0}ms"));

    // The ambiguous-width probe: measured, never inferred.
    match crate::probe::ambiguous_width() {
        Some(w) => {
            let _ = crate::cache::write_atomic(&paths::cache().join("ambiguous_width"),
                                               w.to_string().as_bytes());
            check(format!("ambiguous-width probe: · = {w} column{}", if w > 1 { "s" } else { "" }),
                  true, "measured from your terminal");
        }
        None => check("ambiguous-width probe".into(), false, "needs a real terminal; assuming 1"),
    }

    let mut counts = std::collections::BTreeMap::new();
    for i in &items {
        *counts.entry(i.kind.style().1).or_insert(0usize) += 1;
    }
    println!("\n  {DIM}by source:{RESET} {}",
             counts.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("  "));

    println!("\n  {CYAN}computed rows{RESET} {DIM}(type these, no search needed){RESET}");
    let u = which("units").is_some();
    println!("    {} units    {DIM}10kg to lb · 1gb to mb · 100 degF to degC{RESET}",
             if u { format!("{GREEN}✓{RESET}") } else { format!("{YELLOW}✗{RESET}") });
    println!("    {GREEN}✓{RESET} time     {DIM}now + 3 days · 1699999999{RESET}");
    println!("    {GREEN}✓{RESET} web      {DIM}github.com · localhost:3000 · https://example.com{RESET}");
    let ql = crate::compute::quicklinks();
    println!("    {GREEN}✓{RESET} links    {DIM}{}{RESET}",
             ql.keys().filter(|k| !k.is_empty()).cloned().collect::<Vec<_>>().join(" "));
    // The roots decide what `f:` can find, and an index built before they
    // last changed answers from a set nothing on screen describes. Silence
    // there is the failure worth naming here.
    let roots = crate::settings::root_rows().len();
    match crate::settings::index_count() {
        Some(n) if crate::settings::index_stale() => println!(
            "    {YELLOW}✗{RESET} f:name   {DIM}{n} files from {roots} roots · roots changed — run:  prelude index{RESET}"
        ),
        Some(n) => println!(
            "    {GREEN}✓{RESET} f:name   {DIM}{n} files from {roots} roots · set: to change them{RESET}"
        ),
        None => println!("    {YELLOW}✗{RESET} f:name   {DIM}no index — run:  prelude index{RESET}"),
    }

    println!("\n  {CYAN}translation{RESET} {DIM}(Apple, on-device){RESET}");
    if !crate::compute::translate_app().exists() {
        println!("    {YELLOW}✗{RESET} not built — run:  {DIM}prelude build-translate{RESET}");
    } else {
        match crate::compute::translate("这是一个测试", "en") {
            Ok(v) => println!("    {GREEN}✓{RESET} working  {DIM}这是一个测试 → {v}{RESET}"),
            Err(e) => println!("    {YELLOW}✗{RESET} {e}"),
        }
    }

    let skill_rows: Vec<&crate::item::Item> = items.iter().filter(|i| i.kind == Kind::Skill).collect();
    let skills = skill_rows.len();
    let divergent = skill_rows.iter().filter(|skill| skill.get("integrity") == "divergent").count();
    let unknown = skill_rows.iter().filter(|skill| skill.get("integrity") == "unknown").count();
    let sensitive = skill_rows.iter().flat_map(|skill| crate::capability::copies(skill))
        .filter(|copy| copy.sensitive_files > 0).count();
    let mcp_rows: Vec<&crate::item::Item> = items.iter().filter(|i| i.kind == Kind::Mcp).collect();
    let mcps: Vec<&str> = mcp_rows.iter().map(|i| i.title.as_str()).collect();
    let unhealthy = mcp_rows.iter().filter(|server| server.get("health") != "ok").count();
    let private = mcp_rows.iter().filter(|server| server.get("sensitive") == "true").count();
    let tools_ok = mcp_rows.iter().filter(|server| server.get("tools_status") == "ok").count();
    let tools_failed = mcp_rows.iter().filter(|server| server.get("tools_status") == "failed").count();
    let tools_pending = mcp_rows.iter().filter(|server| server.get("tools_status") == "pending").count();
    println!("\n  {CYAN}agents{RESET}");
    println!("    {DIM}skills:{RESET} {skills} unique · {divergent} divergent · {unknown} unhashed");
    println!("    {DIM}skill privacy:{RESET} {sensitive} copies contain redacted credential-like lines");
    println!("    {DIM}mcp servers:{RESET} {}", if mcps.is_empty() { "none".into() } else { mcps.join("  ") });
    println!("    {DIM}mcp health:{RESET} {unhealthy} need attention · {private} definitions keep private fields out of cache");
    println!("    {DIM}mcp tools:{RESET} {tools_ok} inventoried · {tools_failed} failed · {tools_pending} pending");
    let clis: Vec<&str> = ["claude", "codex", "pi", "cursor-agent", "opencode", "gemini"]
        .iter().copied().filter(|c| which(c).is_some()).collect();
    println!("    {DIM}CLIs on PATH:{RESET} {}", clis.join("  "));

    println!("\n  {CYAN}clipboard{RESET}");
    println!("    {DIM}watcher:{RESET} {}", if crate::clipd::is_running() {
        format!("{GREEN}running{RESET}") } else { format!("{YELLOW}not running{RESET} (starts on first search)") });
    println!("\n  {DIM}deeper: prelude doctor agents · sessions · skills · mcp   (--json, --repair){RESET}");
    println!();
    if ok { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "prelude-doctor-{name}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn diagnostics_cover_exactly_the_registered_agents() {
        let registered: Vec<&str> = crate::agent::SPECS.iter().map(|spec| spec.name).collect();
        let diagnosed: Vec<&str> = PROBES.iter().map(|probe| probe.agent).collect();
        assert_eq!(diagnosed, registered);
    }

    #[test]
    fn a_clean_report_exits_zero_and_a_faulty_one_does_not() {
        let clean = Report::new("agents", vec![Row::new("claude", "2.0.0")]);
        assert_eq!(clean.exit(), 0);
        assert_eq!(clean.attention(), 0);

        let mut bad = Row::new("codex", "");
        bad.issue("not-logged-in", "not logged in");
        let faulty = Report::new("agents", vec![Row::new("claude", ""), bad]);
        assert_eq!(faulty.exit(), 1);
        assert_eq!(faulty.attention(), 1);
    }

    #[test]
    fn a_note_is_not_a_fault() {
        let mut row = Row::new("pi", "");
        row.note("login state unknown");
        assert!(row.ok);
        assert_eq!(Report::new("agents", vec![row]).exit(), 0);
    }

    /// A divergence nobody can act on is a divergence not worth reporting.
    /// `skill_diagnostics` hashes every copy and records its path and mtime;
    /// this is the assertion that the report actually says so, because for a
    /// while it computed all three and printed none of them.
    #[test]
    fn a_divergence_report_names_the_copies_and_says_which_is_newer() {
        use crate::capability::{SkillCopyHealth, SkillHealth};
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let copy = |agent: &str, hash: &str, age: u64| SkillCopyHealth {
            agent: agent.into(),
            dir: format!("/tmp/{agent}/skills/deploy"),
            fingerprint: hash.into(),
            modified: now.saturating_sub(age),
            ..Default::default()
        };

        let divergent = SkillHealth {
            name: "deploy".into(),
            integrity: "divergent".into(),
            copies: vec![
                copy("claude", "fnv1a64-v1:aaaa", 3 * 86_400),
                copy("codex", "fnv1a64-v1:bbbb", 2 * 3600),
            ],
        };
        let row = skill_row(&divergent);
        assert!(!row.ok, "a divergence is a fault");
        let notes = row.notes.join("\n");
        // Where each one is, what it hashes to with the algorithm prefix
        // stripped, and how long ago it was touched.
        assert!(notes.contains("/tmp/claude/skills/deploy"), "{notes}");
        assert!(notes.contains("/tmp/codex/skills/deploy"), "{notes}");
        assert!(notes.contains("aaaa") && notes.contains("bbbb"), "{notes}");
        assert!(!notes.contains("fnv1a64-v1:"), "the prefix is noise on every line: {notes}");
        assert!(notes.contains("3d ago") && notes.contains("2h ago"), "{notes}");

        // A copy that could not be hashed is the reason the Skill is unknown,
        // and saying so is the finding rather than a blank column.
        let unreadable = SkillHealth {
            name: "deploy".into(),
            integrity: "unknown".into(),
            copies: vec![copy("claude", "", 60)],
        };
        assert!(skill_row(&unreadable).notes.join("\n").contains("unhashed"));

        // Settled Skills stay quiet. A machine with forty healthy Skills on it
        // must not print a hash and a path for every copy of every one.
        let settled = SkillHealth {
            name: "deploy".into(),
            integrity: "identical".into(),
            copies: vec![copy("claude", "fnv1a64-v1:aaaa", 60), copy("codex", "fnv1a64-v1:aaaa", 60)],
        };
        let row = skill_row(&settled);
        assert!(row.ok && row.notes.is_empty(), "{:?}", row.notes);
    }

    /// A `Trash` repair for a path, carrying whatever is on disk right now as
    /// its evidence — which is what `staging_rows_in` records.
    fn trash_of(path: &std::path::Path) -> Repair {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::symlink_metadata(path).expect("staged entry");
        Repair::Trash {
            path: path.to_string_lossy().into_owned(),
            modified: mtime_of(&meta),
            mode: meta.permissions().mode() & 0o777,
        }
    }

    /// A mover that stands in for `paths::trash` so a test never touches the
    /// real one: same contract — the entry is renamed somewhere it can be
    /// recovered from, never unlinked.
    fn into_dir(dir: &std::path::Path) -> impl Fn(&Path) -> Result<std::path::PathBuf, String> + '_ {
        move |path: &Path| {
            let dest = dir.join(path.file_name().ok_or("no name")?);
            std::fs::rename(path, &dest).map_err(|e| e.to_string())?;
            Ok(dest)
        }
    }

    #[test]
    fn json_carries_codes_and_repairs_and_the_same_verdict_as_the_table() {
        let mut row = Row::new("borrowed skills", "1 staged");
        row.fixable(
            "shim-target-missing",
            "it links to a Skill that is no longer there",
            Repair::Trash { path: "/tmp/nope".into(), modified: 1, mode: 0o600 },
        );
        let report = Report::new("skills", vec![row]);
        let json = report.json();
        assert_eq!(json["ok"], serde_json::json!(false));
        assert_eq!(json["attention"], serde_json::json!(1));
        assert_eq!(json["check"], serde_json::json!("skills"));
        let issue = &json["rows"][0]["issues"][0];
        assert_eq!(issue["code"], serde_json::json!("shim-target-missing"));
        assert_eq!(issue["repair"]["action"], serde_json::json!("trash"));
        // The renderers agree; a JSON reader and a person get one verdict.
        assert_eq!(report.exit(), 1);
    }

    #[test]
    fn json_and_repair_cannot_be_asked_for_together() {
        assert_eq!(mode_of(&[]).unwrap(), Mode::Text);
        assert_eq!(mode_of(&["--json"]).unwrap(), Mode::Json);
        assert_eq!(mode_of(&["--repair"]).unwrap(), Mode::Repair);
        assert!(mode_of(&["--json", "--repair"]).is_err());
        assert!(mode_of(&["--fix"]).is_err());
    }

    /// A report is a report. Building and rendering one must not move a file,
    /// however loudly the finding says it should be moved.
    #[test]
    fn rendering_a_report_repairs_nothing() {
        let dir = temp("render");
        let file = dir.join("staged.json");
        std::fs::write(&file, b"{}").expect("write");
        let mut row = Row::new("staged definitions", "");
        row.fixable("staged-stale", "old", trash_of(&file));
        let report = Report::new("mcp", vec![row]);
        report.print();
        let _ = report.json();
        assert!(file.exists(), "a report moved a file");
        assert_eq!(report.repairable().len(), 1, "and it should still be offered");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The boundary that stops `--repair` from trashing anything that is not
    /// Prelude's own staging. Same rule as `is_skill_dir`: a direct child of
    /// one known root, compared after `canonicalize`.
    ///
    /// The root is a temporary directory, not the person's real `borrow/`.
    /// A test that has to create files in the cache it is checking is a test
    /// that can leave them there.
    #[test]
    fn only_direct_children_of_the_staging_root_can_be_trashed() {
        let root = temp("boundary");
        assert!(inside(&root, &root.join("borrowed.json")));
        assert!(inside(&root, &root.join("some-skill")));
        // Not a child at all.
        assert!(!inside(&root, &paths::home().join(".zshrc")));
        assert!(!inside(&root, Path::new("/etc/hosts")));
        // A grandchild is not a direct child.
        assert!(!inside(&root, &root.join("some-skill").join("SKILL.md")));
        // And `..` cannot dress something up as one.
        assert!(!inside(&root, &root.join("..").join("mcp")));

        // The guard is enforced at the moment of repair, not only in the
        // report that proposed it — and before anything is read, so a path
        // outside the root is refused whatever is sitting at it.
        let outside = Repair::Trash {
            path: paths::home().join(".zshrc").to_string_lossy().into_owned(),
            modified: 0,
            mode: 0o600,
        };
        let refused = outside.trash_in(&root, into_dir(&root)).expect_err("outside the root");
        assert!(refused.contains("not one of Prelude's own staging entries"), "{refused}");
        // The production door refuses it too, without moving anything.
        assert!(outside.apply().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A repair that is confirmed really does happen, and it happens by
    /// *moving* — the entry is recoverable afterwards, which is the whole
    /// reason destructive actions here are never `unlink`.
    ///
    /// The mover is injected. Production passes `paths::trash`; a test suite
    /// that moved a real file into the person's real Trash to prove a rename
    /// works is a test suite that puts litter in their Trash on every run.
    #[test]
    fn a_confirmed_trash_repair_moves_the_entry_and_leaves_it_recoverable() {
        let root = temp("trash-root");
        let bin = temp("trash-bin");
        let staged = root.join("borrowed.json");
        std::fs::write(&staged, b"{}").expect("stage");

        let repair = trash_of(&staged);
        let done = repair.trash_in(&root, into_dir(&bin)).expect("repair");
        assert!(!staged.exists(), "the staged file is gone from the cache");
        let moved = bin.join("borrowed.json");
        assert!(moved.exists(), "and it is sitting where it can be recovered from");
        assert!(done.contains("moved to"), "{done}");
        assert_eq!(std::fs::read(&moved).expect("read"), b"{}", "contents intact, not truncated");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&bin);
    }

    /// Staging names are deterministic, so the borrow staged while the
    /// confirmation is on screen has exactly the name the question is about.
    /// The evidence the finding was made on is what tells them apart.
    #[test]
    fn a_trash_repair_declines_when_the_entry_is_no_longer_the_one_reported() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp("trash-race");
        let bin = temp("trash-race-bin");
        let staged = root.join("borrowed.json");
        std::fs::write(&staged, b"{}").expect("stage");
        let repair = trash_of(&staged);

        // Re-staged under the same name, one second later: a different file
        // wearing the answered question's name.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::write(&staged, b"{\"new\":true}").expect("re-stage");
        std::fs::File::open(&staged)
            .and_then(|f| f.set_times(std::fs::FileTimes::new().set_modified(later)))
            .expect("touch");
        let refused = repair.trash_in(&root, into_dir(&bin)).expect_err("a rewrite is a new entry");
        assert!(refused.contains("written or chmodded since the report"), "{refused}");
        assert!(staged.exists(), "and the new borrow is still there");

        // The other half of the same evidence: `staged-permissions` is a
        // finding about the mode, and a file chmodded back to 0600 while the
        // question was on screen is no longer that finding.
        let loose = root.join("loose.json");
        std::fs::write(&loose, b"{}").expect("stage");
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let repair = trash_of(&loose);
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        assert!(repair.trash_in(&root, into_dir(&bin)).is_err(), "the fault was fixed by hand");

        // An entry that has not been touched is still repaired.
        let untouched = root.join("stale.json");
        std::fs::write(&untouched, b"{}").expect("stage");
        assert!(trash_of(&untouched).trash_in(&root, into_dir(&bin)).is_ok());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&bin);
    }

    /// `paths::trash` gates on `exists()`, which follows the link, so a
    /// dangling symlink is the one entry a Trash repair is guaranteed to fail
    /// on — at the moment somebody has just said yes to it. It is reported
    /// with no repair rather than offered one that cannot work.
    #[test]
    fn a_broken_symlink_is_reported_and_never_offered_a_repair_that_cannot_work() {
        let root = temp("symlink");
        std::os::unix::fs::symlink(root.join("gone"), root.join("dangling.json")).expect("link");
        let rows = staging_rows_in(&root, Staged::Files);
        let codes: Vec<&str> = rows[0].issues.iter().map(|i| i.code.as_str()).collect();
        assert_eq!(codes, ["staged-symlink"]);
        assert!(rows[0].issues[0].repair.is_none(), "a repair here could only ever fail");
        assert!(rows[0].issues[0].detail.contains("its target is gone"), "{:?}", rows[0].issues[0]);
        // And the proof that it could only fail: `paths::trash` refuses it.
        assert!(paths::trash(&root.join("dangling.json")).is_err());

        // A symlink whose target exists is the same anomaly — Prelude stages
        // directories and 0600 files, never links — and is reported without
        // being mistaken for a file with the wrong mode.
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(root.join("real.json"), b"{}").expect("write");
        std::fs::set_permissions(root.join("real.json"), std::fs::Permissions::from_mode(0o600))
            .expect("chmod");
        std::os::unix::fs::symlink(root.join("real.json"), root.join("live.json")).expect("link");
        let rows = staging_rows_in(&root, Staged::Files);
        let codes: Vec<&str> = rows[0].issues.iter().map(|i| i.code.as_str()).collect();
        assert_eq!(codes, ["staged-symlink", "staged-symlink"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A directory that is not there and a directory that will not open are
    /// different findings. Saying "does not exist yet" about the second is the
    /// report claiming everything is fine about a place it could not look.
    #[test]
    fn an_unreadable_staging_root_is_not_an_empty_one() {
        use std::os::unix::fs::PermissionsExt;
        let missing = temp("absent");
        let root = missing.join("never-created");
        let rows = staging_rows_in(&root, Staged::Files);
        assert!(rows[0].ok && rows[0].issues.is_empty(), "nothing staged is not a fault");
        assert!(rows[0].notes.join(" ").contains("does not exist yet"));

        let closed = temp("closed");
        std::fs::write(closed.join("borrowed.json"), b"{}").expect("stage");
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        let rows = staging_rows_in(&closed, Staged::Files);
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        assert!(!rows[0].ok, "an unreadable root is a fault");
        assert_eq!(rows[0].issues[0].code, "staging-unreadable");
        assert!(rows[0].issues[0].repair.is_none());

        let _ = std::fs::remove_dir_all(&missing);
        let _ = std::fs::remove_dir_all(&closed);
    }

    #[test]
    fn a_shim_pointing_at_a_deleted_skill_is_a_fault_and_a_healthy_one_is_not() {
        let dir = temp("shim");
        let owner = dir.join("owner-skill");
        std::fs::create_dir_all(&owner).expect("owner");
        let shim = dir.join("shim");
        std::fs::create_dir_all(shim.join(".claude-plugin")).expect("meta");
        std::fs::write(shim.join(".claude-plugin").join("plugin.json"), b"{}").expect("manifest");
        std::fs::create_dir_all(shim.join("skills")).expect("skills");
        std::os::unix::fs::symlink(&owner, shim.join("skills").join("owner-skill")).expect("link");
        assert!(shim_faults(&shim).is_empty(), "a healthy shim has no faults");

        std::fs::remove_dir_all(&owner).expect("delete the owner's skill");
        let faults = shim_faults(&shim);
        assert_eq!(faults.len(), 1);
        assert_eq!(faults[0].0, "shim-target-missing");

        // A shim with no manifest cannot be loaded at all.
        let empty = dir.join("empty");
        std::fs::create_dir_all(empty.join("skills")).expect("skills");
        let codes: Vec<&str> = shim_faults(&empty).into_iter().map(|(code, _)| code).collect();
        assert_eq!(codes, vec!["shim-incomplete", "shim-incomplete"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What each Agent CLI on this machine actually prints, pinned so a
    /// diagnostic cannot start inferring a login state nobody reported.
    #[test]
    fn login_states_are_read_from_what_the_cli_said() {
        // claude: JSON, and the two fields that may be read out of it.
        let claude = r#"{"loggedIn":true,"authMethod":"claude.ai","email":"someone@example.com",
                         "orgId":"cf3dd62e","orgName":"An Org","subscriptionType":"max"}"#;
        assert_eq!(read_login(Login::ClaudeJson, claude), LoginState::In("claude.ai".into()));
        assert!(matches!(
            read_login(Login::ClaudeJson, r#"{"loggedIn":false}"#),
            LoginState::Out(_)
        ));
        assert!(matches!(read_login(Login::ClaudeJson, "not json"), LoginState::Unknown(_)));

        // codex: one line, on stderr.
        assert_eq!(read_login(Login::CodexLine, "Logged in using ChatGPT\n"), LoginState::In("ChatGPT".into()));
        assert!(matches!(read_login(Login::CodexLine, "Not logged in\n"), LoginState::Out(_)));
        assert!(matches!(read_login(Login::CodexLine, ""), LoginState::Unknown(_)));

        // opencode: a credential count, drawn inside an ANSI box.
        assert!(matches!(
            read_login(Login::OpencodeCount, "  Credentials ~/auth.json\n\n  0 credentials\n"),
            LoginState::Out(_)
        ));
        assert_eq!(
            read_login(Login::OpencodeCount, "  2 credentials\n"),
            LoginState::In("2 provider credentials".into())
        );

        // pi has no whole-account login, and says so rather than guessing.
        assert!(matches!(read_login(Login::PerProvider, ""), LoginState::Unknown(_)));
    }

    /// `claude auth status --json` carries an e-mail address, an organization
    /// id and an organization name. None of them may reach a report a person
    /// pastes into an issue.
    #[test]
    fn a_login_report_never_carries_an_account_identity() {
        let claude = r#"{"loggedIn":true,"authMethod":"claude.ai","email":"someone@example.com",
                         "orgId":"cf3dd62e-a1e5-44c2","orgName":"An Org"}"#;
        let state = read_login(Login::ClaudeJson, claude);
        let rendered = format!("{state:?}");
        assert!(!rendered.contains('@'), "{rendered}");
        assert!(!rendered.contains("cf3dd62e"), "{rendered}");
        assert!(!rendered.contains("An Org"), "{rendered}");

        // And an account identifier arriving where a method word is expected
        // is dropped rather than shown.
        assert_eq!(safe_method("someone@example.com"), "");
        assert_eq!(safe_method("ChatGPT"), "ChatGPT");
        assert_eq!(safe_method(&"x".repeat(64)), "");
    }

    /// The trap `duplicate_sessions` documents: two files sharing one Session
    /// id produce an identical `cmd`, so `cache::finish` keeps one of them and
    /// the duplicate becomes invisible. `sessions_report` must feed it
    /// `sessions::all()` for exactly this reason.
    #[test]
    fn duplicates_must_be_looked_for_before_the_list_is_finished() {
        let session = |file: &str| {
            Item::new("claude --resume abc", Kind::Session)
                .title("a conversation")
                .put("agent", "claude")
                .put("session_id", "claude:abc")
                .put("id", "abc")
                .put("file", file)
        };
        let raw = vec![session("/tmp/one.jsonl"), session("/tmp/two.jsonl")];
        assert_eq!(
            crate::sources::sessions::duplicate_sessions(&raw).len(),
            1,
            "the raw inventory has the duplicate in it"
        );
        let finished = crate::cache::finish(raw);
        assert!(
            crate::sources::sessions::duplicate_sessions(&finished).is_empty(),
            "finish() dedupes on (kind, cmd), which is precisely the pair that shares one cmd"
        );
    }

    #[test]
    fn a_message_is_stranded_when_its_project_is_gone() {
        // The working directory is the whole of the address, so it is the
        // whole of the question. A second address — a tmux pane — used to
        // keep a message reachable after its project moved, and vice versa.
        assert!(!undeliverable("/here", true));
        assert!(undeliverable("/gone", false));
        // Nothing was ever recorded, so nobody can ever collect it.
        assert!(undeliverable("", false));
    }

    /// Batch runs keep no conversation file, so silence about a Session says
    /// nothing about them and they are never reported as broken.
    #[test]
    fn run_findings_read_the_relationship_rather_than_recomputing_it() {
        let run = |id: &str| {
            Item::new(format!("kill {id}"), Kind::Run)
                .title("claude")
                .put("agent", "claude")
                .put("run_id", format!("claude:{id}:1"))
                .put("cwd", "/tmp/project")
        };
        let runs = vec![
            run("1").put("session_match", "explicit"),
            run("2").put("session_match", "ambiguous"),
            run("3").put("session_match", "requested-missing").put("session_native_id", "abc"),
            run("4"),
            run("5").put("batch", "1"),
        ];
        let rows = run_rows(&runs);
        let codes: Vec<&str> = rows
            .iter()
            .flat_map(|row| row.issues.iter().map(|i| i.code.as_str()))
            .collect();
        assert_eq!(
            codes,
            vec!["session-ambiguous", "session-requested-missing", "run-without-session"]
        );
        assert!(rows[0].ok, "an explicit edge is healthy");
        assert!(rows[4].ok, "a batch run has no Session by design");
        assert!(rows.iter().all(|row| row.issues.iter().all(|i| i.repair.is_none())),
                "a relationship is never repaired by a diagnostic");
    }

    /// Every check either offers no repair or offers the one there is, which
    /// touches only files Prelude staged itself.
    ///
    /// Driven by fixtures rather than by running the real reports. Calling
    /// `skills_report()` here hashed every Skill tree on the machine and
    /// `sessions_report()` read every conversation on it — tens of megabytes
    /// and hundreds of files, to check a property about which *variant* a
    /// finding carries. The three row builders that can produce a repair are
    /// exercised directly instead, and the ones that cannot are pinned as
    /// producing none.
    #[test]
    fn the_only_repairs_are_preludes_own_records() {
        use crate::capability::{SkillCopyHealth, SkillHealth};
        let root = temp("closed-set");
        let shim = root.join("borrowed-skill");
        std::fs::create_dir_all(shim.join("skills")).expect("shim");
        std::fs::write(root.join("server.json"), b"{}").expect("stage");

        let mut rows = staging_rows_in(&root, Staged::Shims);
        rows.extend(staging_rows_in(&root, Staged::Files));
        let report = Report::new("skills", rows);
        assert!(!report.repairable().is_empty(), "the fixtures have to produce some");
        for (_, issue) in report.repairable() {
            let Repair::Trash { path, .. } = issue.repair.as_ref().expect("filtered");
            assert!(inside(&root, Path::new(path)), "{path} is outside the staging root");
        }

        // The rest of every report: a Skill's health, a Session's troubles and
        // a Run's relationships are reported and never repaired.
        let unsound = SkillHealth {
            name: "deploy".into(),
            integrity: "divergent".into(),
            copies: vec![SkillCopyHealth { agent: "claude".into(), unreadable: 2, ..Default::default() }],
        };
        assert!(skill_row(&unsound).issues.iter().all(|i| i.repair.is_none()));
        let _ = std::fs::remove_dir_all(&root);
    }
}
