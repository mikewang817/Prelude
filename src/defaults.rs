//! What Enter does, and why it depends on what the row is.
//!
//! "Enter hands commands over; it does not execute them" stops a launcher
//! from silently running destructive shell commands. It must not become
//! "everything goes through a command line", because the list holds two
//! different kinds of thing.
//!
//!   * **Commands** — history, scripts, $PATH, snippets, ports, processes.
//!     Copying is right; you can read and edit them before they run.
//!   * **Objects** — files, apps, links, skills, results. You wanted to *use*
//!     the thing. Getting only a path copied is a step backwards, and opening
//!     a file is harmless and reversible in a way that
//!     `kill $(lsof -ti tcp:3000)` is not.
//!
//! There is one handoff. A command goes to the clipboard where the person can
//! read and edit it before it runs, whether Prelude was opened from a shell or
//! from the global chord. An object is handed to the application that owns it.
//! The two entry points must not produce two action vocabularies.

use crate::item::{Item, Kind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Default_ {
    /// Hand the command over for review.
    Insert,
    /// Hand over a path/name/result rather than a command.
    InsertText(Text),
    /// Do the obvious harmless thing to the object.
    Act(Verb),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Text {
    AbsolutePath,
    Name,
    /// Where a skill's instructions live, phrased as something to do.
    ///
    /// The one form of borrowing that needs no flag, no restart and no
    /// cooperation from the agent's CLI: a skill is a file of instructions,
    /// and every agent can read a file. It is the only way in for codex and
    /// opencode, which have no way to load a skill they do not own.
    ///
    /// Prelude used to choose between this and `/name` by asking tmux which
    /// agent the pane underneath was running. Nothing can answer that now, so
    /// the choice belongs to the person: both are in `^K`, named for what
    /// they are, and the one you want goes on the clipboard.
    SkillFile,
}

/// Where handed-over text lands.
///
/// Runtime entry points all use `Clipboard`: opening Prelude from a terminal
/// and opening its Quick Terminal are one launcher, not two subtly different
/// products. `Prompt` remains an explicit input to the pure action rules so
/// their historical opposite can still be tested, but no launcher selects it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    /// The retired prompt-insertion surface, retained for pure rule tests.
    #[allow(dead_code)]
    Prompt,
    /// The launcher surface: commands copy, objects act.
    Clipboard,
}

/// The one runtime surface. Everything below still takes it as a parameter so
/// action rules stay decidable without reading process state.
pub fn surface() -> Surface {
    Surface::Clipboard
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
    /// Type an answer to a question an agent is blocked on.
    Answer,
    /// Run it right here in the launcher and show the output.
    RunHere,
    RunInShell,
    Inspect,
    CdThere,
    /// Change one of Prelude's own preferences, in whatever way that
    /// particular preference is changed. A chord is typed, Quick Look is
    /// toggled, a rules file is opened — one verb, because from the row's
    /// point of view they are one intent, and `settings::edit` dispatches.
    EditSetting,
}

/// The one place that decides what Enter means.
pub fn on_enter(item: &Item) -> Default_ {
    use Default_::*;
    use Kind::*;
    // Settings are controls, not payload. Making `copy everything` copy
    // `set:roots` instead of opening its manager made the very screen that can
    // turn the preference off unusable. Control-plane rows always act.
    if item.kind == Setting {
        return Act(Verb::EditSetting);
    }
    if crate::settings::classic_enter() {
        return Insert;
    }
    // Starting an agent with a prompt is a request for an answer, not for a
    // command to review. Run it here and show what it says. Resuming an
    // existing session is different — you want that in a real terminal, so
    // its resume command is copied for you to place there.
    if item.kind == Session && item.get("mode") == "start" {
        return Act(Verb::RunHere);
    }
    // Resuming a conversation a live process already owns starts a competing
    // copy of it. The relationship graph makes the honest default possible:
    // point at the project that run is working in instead.
    if item.kind == Session && !item.get("active_run").is_empty() {
        return Act(Verb::CdThere);
    }
    // A newer release. `prelude update` has no arguments anybody would add,
    // so there is nothing to hand over and nothing to edit — the reason
    // commands are copied for review does not apply, and the row's whole
    // purpose
    // is the thing it would have made you paste somewhere.
    if item.get("update") == "available" {
        return Act(Verb::RunHere);
    }
    match item.kind {
        // A question someone is blocked on. There is exactly one thing to do
        // with it.
        Msg => Act(Verb::Answer),

        // Commands. Destructive ones especially.
        History | Script | Path | Snippet | Ssh | Container | Git => Insert,
        Port | Proc | Sys | Search => Insert,

        // Objects act.
        // Opening means "give it to an application", not "give it to $EDITOR"
        // — you pick a file out of a launcher for every reason, and only
        // sometimes to edit it. ^K is where you say which application, once
        // or from now on.
        // An indexed folder keeps Kind::Find for compatibility with the old
        // file-only index, but acts exactly like every other Folder row.
        Find if item.get("index_kind") == "folder" => Act(Verb::Open),
        File | Find => Act(Verb::Open),

        App => Act(Verb::Launch),

        Link => Act(Verb::OpenUrl),

        Calc | Translate => Act(Verb::CopyResult),

        // An MCP server exists for the tools it exposes, not for the config
        // file that happens to describe it. Details are therefore the useful
        // default; configuration remains an explicit action.
        Mcp => Act(Verb::RunHere),

        // A skill name is meaningless on its own, so it is handed over
        // attached to an agent that has it. `^K` carries the two bare forms:
        // the slash command, and the instruction to read its file.
        Skill => Act(Verb::RunSkill),

        // A running agent. Nothing can put the cursor in somebody else's
        // terminal any more, so the useful thing left is where it is working.
        Run => Act(Verb::CdThere),

        Session => Act(Verb::ResumeSession),
        // A command line like any other, and the reason to hand it over
        // rather than run it is not safety — `claude` is harmless — but that
        // it is so often the *start* of a command. `--resume`, a model, an
        // opening prompt: one keystroke buys the chance to add them, and
        // costs nothing when you do not.
        Agent => Insert,
        Config => Act(Verb::Open),

        // A setting's row already states its value, so Enter is the change
        // rather than another look at it.
        Setting => unreachable!("settings return before the payload policy"),

        // A folder is an object like a file: Finder is the harmless default.
        // `cd` remains the first explicit alternative in ^K.
        Dir => Act(Verb::Open),

        Clip => Insert,
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
pub fn on_secondary(item: &Item, surface: Surface) -> Option<Default_> {
    use Default_::*;
    use Kind::*;
    let alt = match item.kind {
        // Primary answers it, which is the whole of what a question is for.
        // The alternatives — a canned yes, a canned no, the conversation it
        // came out of — are named in the panel rather than compressed into
        // one unlabelled opposite.
        Msg => return None,
        // On the retired prompt surface, inserting and running are opposites.
        History | Script | Path | Snippet | Ssh | Container | Git | Sys => {
            Act(Verb::RunInShell)
        }
        // Primary hands you the command; the secondary runs it unedited.
        Agent => Act(Verb::RunInShell),
        // Primary hands over the `cd`; the secondary hands over the bare
        // path, which is what you want when it is going somewhere other than
        // a shell.
        Run => InsertText(Text::AbsolutePath),
        // Primary kills or acts; the secondary shows you what you would hit.
        Port | Proc => Act(Verb::Inspect),
        // Primary does something to the object, so the secondary yields text.
        File | Find | Config => InsertText(Text::AbsolutePath),
        App | Mcp | Skill => InsertText(Text::Name),
        Link => InsertText(Text::Name),
        // These were text-vs-clipboard opposites on the prompt surface.
        Calc | Translate => Insert,
        Clip => Act(Verb::CopyResult),
        Session => Act(Verb::CdThere),
        Dir => InsertText(Text::AbsolutePath),
        // Enter changes it; the panel names every other thing that can be
        // done to it, and they differ too much per setting to compress into
        // one unlabelled opposite.
        Setting => return None,
        Search => return None,
    };
    // "Insert it" and "run it" are opposites only where there is a shell to
    // run it in. On the clipboard they are the same bytes, and two rows
    // saying the same thing is worse than one.
    if surface == Surface::Clipboard
        && (alt == Act(Verb::RunInShell) || matches!(item.kind, Calc | Translate | Clip))
    {
        return None;
    }
    Some(alt)
}

/// A human-readable name for the current default, shown as the first entry of
/// the action panel so the behaviour is never a mystery.
pub fn describe(item: &Item, surface: Surface) -> &'static str {
    if item.kind == Kind::Search {
        return if !item.get("ask").is_empty() {
            "Add question"
        } else if item.get("provider").is_empty() {
            "Open this search"
        } else {
            "Add search term"
        };
    }
    name(item, on_enter(item), surface)
}

pub fn describe_secondary(item: &Item, surface: Surface) -> Option<&'static str> {
    on_secondary(item, surface).map(|d| name(item, d, surface))
}

/// One verb can read two ways depending on what it is pointed at.
///
/// `Inspect` is "show what is using it" for a port and "show its full
/// command" for a process — the same action, two different questions. The
/// kind used to carry a second entry with the right wording, which was the
/// same action listed twice; once Enter's row left the panel the two sat
/// adjacent and it stopped being arguable.
fn name(item: &Item, d: Default_, surface: Surface) -> &'static str {
    // Enter means something different for each setting, and the footer is
    // where that has to be said — "Edit setting" would tell you nothing about
    // whether you are about to be asked for a chord or shown a file.
    if d == Default_::Act(Verb::EditSetting) {
        return match item.get("setting") {
            "roots" => "Manage folders",
            "index" => "Rebuild the index",
            "hotkey" => "Change the chord…",
            "paneldir" => "Change the directory…",
            "key" => "Change the key…",
            "preview" => {
                if item.fields.first().map(String::as_str) == Some("on") {
                    "Turn Quick Look off"
                } else {
                    "Turn Quick Look on"
                }
            }
            "update" => "Choose update mode…",
            "enter" => {
                if item.fields.first().map(String::as_str) == Some("per kind") {
                    "Switch to copy-everything"
                } else {
                    "Switch to per-kind"
                }
            }
            "openwith" => "Manage open-with rules",
            "snippets" => "Manage snippets",
            "quicklinks" => "Manage Quicklinks",
            "favorites" => "Manage Favorites",
            _ => "Open the file",
        };
    }
    if d == Default_::Act(Verb::Inspect) && item.kind == Kind::Proc {
        return "Show its full command";
    }
    if d == Default_::Act(Verb::RunHere) && item.get("update") == "available" {
        return "Update now";
    }
    if d == Default_::Act(Verb::RunHere) && item.kind == Kind::Mcp {
        return "Show what it exposes";
    }
    if d == Default_::Act(Verb::RunInShell) && item.kind == Kind::Agent {
        return "Start now";
    }
    // A live run already owns this conversation, so the row deliberately does
    // not resume it. Say which directory you are being handed and why, rather
    // than letting it read as a plain `cd`.
    if d == Default_::Act(Verb::CdThere) && item.kind == Kind::Session
        && !item.get("active_run").is_empty()
    {
        return if surface == Surface::Clipboard {
            "Copy cd to its active project"
        } else {
            "Insert cd to its active project"
        };
    }
    if d == Default_::Act(Verb::CopyResult) && item.kind == Kind::Clip {
        return match item.get("clip_kind") {
            "files" => "Copy files",
            "image" => "Copy image",
            _ => "Copy text",
        };
    }
    if d == Default_::InsertText(Text::Name) && item.kind == Kind::Link {
        return if surface == Surface::Clipboard { "Copy URL" } else { "Insert URL" };
    }
    if d == Default_::Act(Verb::Open)
        && (item.kind == Kind::Dir || item.get("index_kind") == "folder")
    {
        return "Open folder";
    }
    describe_action(d, surface)
}

fn describe_action(d: Default_, surface: Surface) -> &'static str {
    // The verbs that hand over text read differently depending on where the
    // text lands. Everything below them acts, and acting is the same sentence
    // in both surfaces.
    let clip = surface == Surface::Clipboard;
    match d {
        Default_::Insert if clip => "Copy the command",
        Default_::InsertText(Text::AbsolutePath) if clip => "Copy the full path",
        Default_::InsertText(Text::Name) if clip => "Copy its name",
        Default_::Act(Verb::CdThere) if clip => "Copy the cd command",
        Default_::Act(Verb::ResumeSession) if clip => "Copy the resume command",
        Default_::Act(Verb::RunSkill) if clip => "Copy it as an agent command",
        Default_::Act(Verb::Inspect) if clip => "Copy the command that shows it",

        Default_::Insert => "Insert into prompt",
        Default_::InsertText(Text::AbsolutePath) => "Insert the full path",
        Default_::InsertText(Text::Name) => "Insert its name",
        Default_::InsertText(Text::SkillFile) => "Point an agent at its file",
        Default_::Act(Verb::Open) => "Open it",
        Default_::Act(Verb::Answer) => "Answer it",
        Default_::Act(Verb::Launch) => "Launch it",
        Default_::Act(Verb::OpenUrl) => "Open in browser",
        Default_::Act(Verb::CopyResult) => "Copy the result",
        Default_::Act(Verb::RunSkill) => "Hand it to an agent",
        Default_::Act(Verb::ResumeSession) => "Resume this session",
        Default_::Act(Verb::RunHere) => "Run it here and show the output",
        Default_::Act(Verb::RunInShell) => "Run it in the shell",
        Default_::Act(Verb::Inspect) => "Show what is using it",
        Default_::Act(Verb::CdThere) => "Insert the cd command",
        Default_::Act(Verb::EditSetting) => "Change it",
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
    }
}
