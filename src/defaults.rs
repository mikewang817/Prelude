//! What Enter does, and why it depends on where you are.
//!
//! "Enter inserts, it does not execute" exists to stop a launcher from
//! silently running destructive shell commands. It was over-generalised into
//! "everything must go through the command line", which is wrong: the list
//! holds two different kinds of thing.
//!
//!   * **Commands** — history, scripts, $PATH, snippets, ports, processes.
//!     Inserting is exactly right; you want to read them before they run.
//!   * **Objects** — files, apps, links, skills, results. You wanted to *use*
//!     the thing. Getting a path pasted onto your prompt is a step backwards,
//!     and opening a file is harmless and reversible in a way that
//!     `kill $(lsof -ti tcp:3000)` is not.
//!
//! The second half is that the right answer changes with the host. Selecting
//! a file at a shell prompt means "open it". Selecting the same file from the
//! popup over an agent's input box means "here, look at this path". Prelude
//! knows which it is, so it should act on that.

use crate::item::{Item, Kind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Host {
    /// A shell prompt, via the zsh widget.
    Shell,
    /// Someone else's input box — an agent, vim, a REPL — via `paste`.
    Agent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Default_ {
    /// Put the command on the prompt for review. The original behaviour.
    Insert,
    /// Put a path/name/result on the prompt rather than a command.
    InsertText(Text),
    /// Do the obvious harmless thing to the object.
    Act(Verb),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Text {
    AbsolutePath,
    Name,
    Result,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    OpenInEditor,
    Launch,
    OpenUrl,
    CopyResult,
    OpenConfig,
    RunSkill,
    ResumeSession,
    /// Run it right here in the launcher and show the output.
    RunHere,
}

/// The one place that decides what Enter means.
pub fn on_enter(item: &Item, host: Host) -> Default_ {
    use Default_::*;
    use Kind::*;
    if std::env::var_os("PRELUDE_CLASSIC_ENTER").is_some() {
        return Insert;
    }
    // Starting an agent with a prompt is a request for an answer, not for a
    // command to review. Run it here and show what it says. Resuming an
    // existing session is different — you want that in a real terminal, so
    // it still goes onto the prompt.
    if item.kind == Session && item.get("mode") == "start" {
        return Act(Verb::RunHere);
    }
    match (item.kind, host) {
        // Commands: unchanged in both hosts. Destructive ones especially.
        (History | Script | Path | Snippet | Ssh | Container | Git, _) => Insert,
        (Port | Proc | Sys, _) => Insert,

        // Objects: act at a shell, hand over the text to an agent.
        (File | Find, Host::Shell) => Act(Verb::OpenInEditor),
        (File | Find, Host::Agent) => InsertText(Text::AbsolutePath),

        (App, Host::Shell) => Act(Verb::Launch),
        (App, Host::Agent) => InsertText(Text::Name),

        (Link, Host::Shell) => Act(Verb::OpenUrl),
        (Link, Host::Agent) => InsertText(Text::Name),

        (Calc | Translate, Host::Shell) => Act(Verb::CopyResult),
        (Calc | Translate, Host::Agent) => InsertText(Text::Result),

        (Mcp, Host::Shell) => Act(Verb::OpenConfig),
        (Mcp, Host::Agent) => InsertText(Text::Name),

        // A skill name is meaningless at a shell prompt but is exactly what
        // an agent wants to hear.
        (Skill, Host::Shell) => Act(Verb::RunSkill),
        (Skill, Host::Agent) => InsertText(Text::Name),

        (Session, _) => Act(Verb::ResumeSession),
        // Starting an agent is the obvious thing to do with one.
        (Agent, Host::Shell) => Insert,
        (Agent, Host::Agent) => InsertText(Text::Name),
        (Config, Host::Shell) => Act(Verb::OpenInEditor),
        (Config, Host::Agent) => InsertText(Text::AbsolutePath),

        // cd has to happen in *your* shell, so it stays an inserted command.
        (Dir, Host::Shell) => Insert,
        (Dir, Host::Agent) => InsertText(Text::AbsolutePath),

        (Clip, _) => Insert,
    }
}

/// A human-readable name for the current default, shown as the first entry of
/// the action panel so the behaviour is never a mystery.
pub fn describe(item: &Item, host: Host) -> &'static str {
    match on_enter(item, host) {
        Default_::Insert => "Insert into prompt",
        Default_::InsertText(Text::AbsolutePath) => "Insert the full path",
        Default_::InsertText(Text::Name) => "Insert its name",
        Default_::InsertText(Text::Result) => "Insert the result",
        Default_::Act(Verb::OpenInEditor) => "Open in editor",
        Default_::Act(Verb::Launch) => "Launch it",
        Default_::Act(Verb::OpenUrl) => "Open in browser",
        Default_::Act(Verb::CopyResult) => "Copy the result",
        Default_::Act(Verb::OpenConfig) => "Open its config",
        Default_::Act(Verb::RunSkill) => "Run it with an agent",
        Default_::Act(Verb::ResumeSession) => "Resume this session",
        Default_::Act(Verb::RunHere) => "Run it here and show the output",
    }
}

/// The text an `InsertText` default should produce.
pub fn text_for(it: &Item, what: Text) -> String {
    match what {
        Text::AbsolutePath => {
            let p = it.get("path");
            if !p.is_empty() {
                return p.to_string();
            }
            // dir items carry the path inside a `cd <path>` command
            it.cmd.strip_prefix("cd ").map(|r| r.trim_matches('\'').to_string())
                .unwrap_or_else(|| it.cmd.clone())
        }
        Text::Name => match it.kind {
            Kind::App => it.title.clone(),
            Kind::Link => it.get("url").to_string(),
            Kind::Mcp => it.get("name").to_string(),
            _ => it.cmd.clone(),
        },
        Text::Result => it.cmd.clone(),
    }
}
