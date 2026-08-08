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
    /// Where a skill's instructions live, phrased as something to do.
    ///
    /// The one form of borrowing that needs no flag, no restart and no
    /// cooperation from the agent's CLI: a skill is a file of instructions,
    /// and every agent can read a file. It is the only way in for codex and
    /// opencode, which have no way to load a skill they do not own.
    SkillFile,
}

/// Which agent's input box we are typing into, when tmux can say.
///
/// Ambient rather than an argument, for the same reason the column widths
/// are: the per-keystroke footer helper is a separate process, so this
/// arrives on its argv and is set once. Threading it through thirty-odd
/// call sites of `Host` would buy nothing.
static HOST_AGENT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

pub fn set_host_agent(a: Option<String>) {
    let _ = HOST_AGENT.set(a);
}

pub fn host_agent() -> Option<&'static str> {
    HOST_AGENT.get().and_then(|o| o.as_deref())
}

/// Does the agent we are typing into already have this skill?
///
/// `/name` is a slash command only to an agent that has it. To any other it
/// is a line of prose that means nothing — and it fails silently, which is
/// the worst way for it to fail.
///
/// An unknown host is treated as owning it: that keeps the slash command for
/// everyone who opened the launcher from the agent that has the skill, and
/// the file pointer stays one key away on the secondary either way.
/// `shared` is a directory rather than an agent, so it is not ownership.
fn host_owns(it: &Item) -> bool {
    match host_agent() {
        None => true,
        Some(h) => it.get("agent").split(',').map(str::trim).any(|a| a == h),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    /// Hand the file to whichever application owns it — the user's choice
    /// if they have made one, macOS's otherwise. See `openwith`.
    Open,
    Launch,
    OpenUrl,
    CopyResult,
    RunSkill,
    ResumeSession,
    /// Start an agent CLI in a pane beside the conversation you are in —
    /// same window, so both are on screen at once.
    SplitPane,
    /// Put the cursor in the pane a running agent lives in.
    JumpTo,
    /// Type an answer to a question an agent is blocked on.
    Answer,
    /// Run it right here in the launcher and show the output.
    RunHere,
    RunInShell,
    Inspect,
    CdThere,
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
        // A question someone is blocked on. There is exactly one thing to do
        // with it, and it is the same wherever you are standing.
        (Msg, _) => Act(Verb::Answer),

        // Commands: unchanged in both hosts. Destructive ones especially.
        (History | Script | Path | Snippet | Ssh | Container | Git, _) => Insert,
        (Port | Proc | Sys | Search, _) => Insert,

        // Objects: act at a shell, hand over the text to an agent.
        // Opening means "give it to an application", not "give it to $EDITOR"
        // — you pick a file out of a launcher for every reason, and only
        // sometimes to edit it in this terminal. ^K is where you say which
        // application, once or from now on.
        (File | Find, Host::Shell) => Act(Verb::Open),
        (File | Find, Host::Agent) => InsertText(Text::AbsolutePath),

        (App, Host::Shell) => Act(Verb::Launch),
        (App, Host::Agent) => InsertText(Text::Name),

        // A URL is an external object wherever Prelude was opened. Enter
        // asks Launch Services to open it directly; handing the URL to an
        // agent remains the secondary action in ^K.
        (Link, _) => Act(Verb::OpenUrl),

        (Calc | Translate, Host::Shell) => Act(Verb::CopyResult),
        (Calc | Translate, Host::Agent) => InsertText(Text::Result),

        // An MCP server exists for the tools it exposes, not for the config
        // file that happens to describe it. Details are therefore the useful
        // default; configuration remains an explicit action.
        (Mcp, Host::Shell) => Act(Verb::RunHere),
        (Mcp, Host::Agent) => InsertText(Text::Name),

        // A skill name is meaningless at a shell prompt but is exactly what
        // an agent wants to hear.
        (Skill, Host::Shell) => Act(Verb::RunSkill),
        // Mid-conversation, the useful answer depends on who you are talking
        // to. Its owner takes the slash command; anyone else is handed the
        // file, which needs no restart and works even for the agents that
        // cannot load a borrowed skill at all.
        (Skill, Host::Agent) if host_owns(item) => InsertText(Text::Name),
        (Skill, Host::Agent) => InsertText(Text::SkillFile),

        // A running agent: go to it. At three of them this is a
        // convenience; at eighty it is the only way to use the machine at
        // all. A stray has no address to jump to, so the useful thing left
        // is its directory, on the prompt.
        (Run, Host::Shell) if !item.get("pane").is_empty() => Act(Verb::JumpTo),
        (Run, Host::Shell) => Act(Verb::CdThere),
        (Run, Host::Agent) => InsertText(Text::AbsolutePath),

        (Session, _) => Act(Verb::ResumeSession),
        // At a shell this is a command line like any other, and the reason
        // to hand it over rather than run it is not safety — `claude` is
        // harmless — but that it is so often the *start* of a command.
        // `--resume`, a model, an opening prompt: one keystroke buys the
        // chance to add them, and costs nothing when you do not.
        (Agent, Host::Shell) => Insert,
        // In a conversation there is no prompt to paste onto. Typing
        // `codex` into claude's input box sends claude the word "codex".
        // The only reading of "start it" that means anything here is a
        // second agent beside the first — in the same window, because the
        // point of starting one while talking to another is watching both.
        // tmux is already underneath us; that popup is how we got here.
        (Agent, Host::Agent) => Act(Verb::SplitPane),
        (Config, Host::Shell) => Act(Verb::Open),
        (Config, Host::Agent) => InsertText(Text::AbsolutePath),

        // A folder is an object like a file: Finder is the harmless default.
        // `cd` remains the first explicit alternative in ^K. In a
        // conversation the path is still what the agent needs.
        (Dir, Host::Shell) => Act(Verb::Open),
        (Dir, Host::Agent) => InsertText(Text::AbsolutePath),

        (Clip, _) => Insert,
    }
}

/// The secondary action — Enter's opposite, and the second entry of the
/// ^K panel.
///
/// Raycast's manual defines ⌘↵ as "execute the secondary action", so it is
/// per-item like the primary one rather than a fixed verb. The pattern
/// throughout is that the two are opposites: where the primary *does*
/// something, the secondary hands you the text, and where the primary hands
/// you text, the secondary does the thing.
///
/// It has no key of its own. fzf rejects `ctrl-enter` and `shift-enter`
/// outright, a terminal never receives Cmd, and Option is spent on composing
/// characters unless the terminal is told otherwise — so every candidate was
/// either impossible or silently dead on someone's machine. Naming it in the
/// panel costs one keystroke and needs no explaining.
pub fn on_secondary(item: &Item, host: Host) -> Option<Default_> {
    use Default_::*;
    use Kind::*;
    let alt = match item.kind {
        // Primary answers it from here; the secondary takes you to the
        // conversation, for when the question needs more context than a line.
        Msg => Act(Verb::JumpTo),
        // Primary inserts a command, so the secondary runs it.
        History | Script | Path | Snippet | Ssh | Container | Git | Sys => {
            Act(Verb::RunInShell)
        }
        // Primary hands you the command; the secondary runs it unedited.
        Agent => Act(Verb::RunInShell),
        // Primary goes there; the secondary tells you where "there" is.
        Run => InsertText(Text::AbsolutePath),
        // Primary kills or acts; the secondary shows you what you would hit.
        Port | Proc => Act(Verb::Inspect),
        // Primary does something to the object, so the secondary yields text.
        File | Find | Config => InsertText(Text::AbsolutePath),
        App | Mcp | Skill => InsertText(Text::Name),
        Link => InsertText(Text::Name),
        // Primary copies the result, so the secondary puts it on the prompt.
        Calc | Translate => Insert,
        // Primary pastes it; the secondary puts it on the system clipboard.
        Clip => Act(Verb::CopyResult),
        Session => Act(Verb::CdThere),
        Dir => InsertText(Text::AbsolutePath),
        Search => return None,
    };
    if host == Host::Agent {
        // A skill is the one row with two genuinely different texts to hand
        // over, so the slot that would otherwise go unused carries the other
        // one — whichever of the two Enter did not give you.
        // Enter opens a window; the other one gives the conversation the
        // name as text, for "run codex on this for me".
        if item.kind == Agent {
            return Some(InsertText(Text::Name));
        }
        if item.kind == Skill {
            return Some(if host_owns(item) {
                InsertText(Text::SkillFile)
            } else {
                InsertText(Text::Name)
            });
        }
        // Links are external objects in both hosts; the text is still a
        // useful alternative when the intent was to hand it to the agent.
        if item.kind == Link {
            return Some(alt);
        }
        // Otherwise the primary is already "hand over the text", so an
        // inserting secondary would be the same keystroke twice.
        if matches!(alt, InsertText(_) | Insert) {
            return None;
        }
    }
    Some(alt)
}

/// A human-readable name for the current default, shown as the first entry of
/// the action panel so the behaviour is never a mystery.
pub fn describe(item: &Item, host: Host) -> &'static str {
    if item.kind == Kind::Search {
        return if !item.get("ask").is_empty() {
            "Add question"
        } else if item.get("provider").is_empty() {
            "Open this search"
        } else {
            "Add search term"
        };
    }
    name(item, on_enter(item, host))
}

pub fn describe_secondary(item: &Item, host: Host) -> Option<&'static str> {
    on_secondary(item, host).map(|d| name(item, d))
}

/// One verb can read two ways depending on what it is pointed at.
///
/// `Inspect` is "show what is using it" for a port and "show its full
/// command" for a process — the same action, two different questions. The
/// kind used to carry a second entry with the right wording, which was the
/// same action listed twice; once Enter's row left the panel the two sat
/// adjacent and it stopped being arguable.
fn name(item: &Item, d: Default_) -> &'static str {
    if d == Default_::Act(Verb::Inspect) && item.kind == Kind::Proc {
        return "Show its full command";
    }
    if d == Default_::Act(Verb::RunHere) && item.kind == Kind::Mcp {
        return "Show what it exposes";
    }
    if d == Default_::Act(Verb::RunInShell) && item.kind == Kind::Agent {
        return "Start now";
    }
    if d == Default_::Act(Verb::CopyResult) && item.kind == Kind::Clip {
        return "Copy text";
    }
    if d == Default_::InsertText(Text::Name) && item.kind == Kind::Link {
        return "Insert URL";
    }
    if d == Default_::Act(Verb::Open) && item.kind == Kind::Dir {
        return "Open in Finder";
    }
    describe_action(d)
}

fn describe_action(d: Default_) -> &'static str {
    match d {
        Default_::Insert => "Insert into prompt",
        Default_::InsertText(Text::AbsolutePath) => "Insert the full path",
        Default_::InsertText(Text::Name) => "Insert its name",
        Default_::InsertText(Text::Result) => "Insert the result",
        Default_::InsertText(Text::SkillFile) => "Point the agent at its file",
        Default_::Act(Verb::Open) => "Open it",
        Default_::Act(Verb::SplitPane) => "Start it in a pane beside this one",
        Default_::Act(Verb::JumpTo) => "Go to it",
        Default_::Act(Verb::Answer) => "Answer it",
        Default_::Act(Verb::Launch) => "Launch it",
        Default_::Act(Verb::OpenUrl) => "Open in browser",
        Default_::Act(Verb::CopyResult) => "Copy the result",
        Default_::Act(Verb::RunSkill) => "Hand it to an agent",
        Default_::Act(Verb::ResumeSession) => "Resume this session",
        Default_::Act(Verb::RunHere) => "Run it here and show the output",
        Default_::Act(Verb::RunInShell) => "Run it in the shell",
        Default_::Act(Verb::Inspect) => "Show what is using it",
        Default_::Act(Verb::CdThere) => "Go to its directory",
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
        // An instruction rather than a bare path: the point is for the
        // agent to follow the skill, and a path on its own invites it to
        // summarise the file instead.
        Text::SkillFile => {
            let p = it.get("file");
            let p = if p.is_empty() { it.get("dir") } else { p };
            if p.is_empty() { it.cmd.clone() } else { format!("Read {p} and follow it.") }
        }
        Text::Result => it.cmd.clone(),
    }
}
