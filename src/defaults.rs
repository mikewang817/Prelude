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
//!
//! The `enter` preference moves that line, and only along the command side of
//! it: `copy commands` makes every row that would *act* hand its text over
//! instead, except the three verbs that end at Launch Services. It has nothing
//! to say to a row that already hands text over — there the question is
//! settled, and the only thing left would be changing *which* text. A
//! preference that could take opening a file away from a launcher, or take a
//! Skill's portable invocation away from it, is not a preference; it is a
//! fault report waiting to be filed.

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
    /// This is what Enter on a Skill hands over, because it is the only form
    /// that is unconditional. `/name` needs the Agent in front of you to own
    /// the Skill, and `claude /name` needs you not to have an Agent open
    /// already; both are in `^K`, named for what they are.
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
    ResumeSession,
    /// Type an answer to a question an agent is blocked on.
    Answer,
    /// Run it right here in the launcher and show the output.
    RunHere,
    /// Send the row's text to be rewritten as a clear English prompt, and
    /// leave the result on the clipboard.
    Rewrite,
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
    enter_with(item, crate::settings::classic_enter())
}

/// The rule, with the preference handed in.
///
/// Process state stays at the edge for the reason `surface()` gives: a rule
/// that reads a file to decide cannot be walked kind by kind in a test, and
/// this one has two exceptions that are only visible when it is.
pub fn enter_with(item: &Item, classic: bool) -> Default_ {
    let chosen = by_kind(item);
    // Settings are controls, not payload. Making `copy commands` copy
    // `set:roots` instead of opening its manager made the very screen that can
    // turn the preference off unusable. Control-plane rows always act.
    if item.kind == Kind::Setting {
        return chosen;
    }
    // A rewrite already ends with text on the clipboard, which is the whole of
    // what this preference asks for. Converted to `Insert` the row would hand
    // back the text you asked to have rewritten — not a different way of doing
    // the same thing, but the row refusing to do it at all, while still saying
    // it would. It is the Skill exception below, arrived at from the other
    // side: there, Enter is text already; here, Enter's *product* is.
    if item.kind == Kind::Rewrite {
        return chosen;
    }
    // The preference is about **commands**: the rows whose being handed over
    // is the point, because a command is worth reading before it runs.
    //
    // It used to convert every row, and neither half of that argument
    // survives the trip to an object. There is nothing to review — the text a
    // File row would hand you is a path, and an App row's is `open -na
    // /Applications/Safari.app`, which nobody proofreads. And nothing is
    // averted: opening a file is not `kill $(lsof -ti tcp:3000)`. What
    // actually happened is that the launcher stopped opening files, folders,
    // applications and links, and was reported as broken rather than as a
    // preference doing what it said — which is the correct reading. Whoever
    // turns this on is thinking about commands.
    // …and it has nothing to say to a row that already hands text over. The
    // preference chooses between *acting* and *being given the text*, so on a
    // row where Enter is already text the question is settled; all it could do
    // is change **which** text, which is not what the row says it does.
    //
    // A Skill is the case that makes this concrete. Its Enter is the portable
    // invocation — the name and the absolute path to its `SKILL.md` — which is
    // the one form that works in an Agent you already have open. Converted to
    // a bare `Insert` it becomes `/review`, which does nothing at all in an
    // Agent that does not own the Skill. Turning on `copy commands` would then
    // silently take away the ability to use a Skill outside the Agent it is
    // installed in, and the row would still read `copy commands`.
    if classic && matches!(chosen, Default_::Act(_)) && !goes_to_launch_services(chosen) {
        return Default_::Insert;
    }
    chosen
}

/// The verbs that end at Launch Services, and the reason two very different
/// rules can share one predicate.
///
/// `Open`, `Launch` and `OpenUrl` hand an object to macOS and can do nothing
/// else: no clipboard is written, no agent starts, no question is answered and
/// no command runs. `link.rs` needs exactly that property because a web page
/// can navigate to `prelude://`; the Enter preference needs exactly that
/// property because it is the one class of row where "hand it over instead"
/// answers nothing. Anything added here is a claim about the verb, not about
/// either caller — and `may_be_linked` is a security boundary, so the claim
/// has to be true before it is made.
pub fn goes_to_launch_services(d: Default_) -> bool {
    matches!(d, Default_::Act(Verb::Open | Verb::Launch | Verb::OpenUrl))
}

/// What this row is, before anybody's preference about Enter is applied.
///
/// `classic_enter` says what the *launcher's* Enter does, and a caller with no
/// launcher and no clipboard in front of it must not be governed by it.
/// `prelude://` is the case: asking `on_enter` there meant that turning on
/// copy-everything silently made every deeplink refuse, because every row's
/// answer had become "hand it over" — a feature dead for a whole class of
/// people, and quietly. The preference no longer reaches those three verbs, so
/// the two answers now agree for everything a link may name; `link.rs` still
/// asks here, because agreeing today is not the same as being governed by it.
///
/// Split out rather than restated, so there is still one table.
pub fn by_kind(item: &Item) -> Default_ {
    use Default_::*;
    use Kind::*;
    if item.kind == Setting {
        return Act(Verb::EditSetting);
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

        // The work has not happened yet: the row describes a rewrite, and
        // Enter is what pays for it. Nothing is computed while you type, for
        // the reason `rewrite.rs` opens with.
        Rewrite => Act(Verb::Rewrite),


        // The one form of a Skill that works in an Agent you already have
        // open, whichever Agent it is and wherever the Skill is installed.
        //
        // Enter used to hand over `claude /name` — a shell command that
        // *starts* an Agent. That answers a question nobody with a
        // conversation already running is asking, and it is the common case:
        // you are in Claude Code, you want a Skill that lives in
        // `~/.agents/skills`, and `/name` does nothing there because that root
        // is not Claude's. The old answer to that was to install a copy or
        // arrange a one-run loan, which is a lot of machinery for "read this
        // file".
        //
        // So Enter is the portable invocation: the Skill's name and the
        // absolute path to its `SKILL.md`, as an instruction. It needs no
        // installation, works in all four Agents identically, and — because
        // the path is what identifies it — two Skills sharing a name are
        // simply two different sentences. `^K` keeps the narrower forms: the
        // slash command for an Agent that does own it, and the shell command
        // for starting a new one.
        Skill => InsertText(Text::SkillFile),

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
        Setting => unreachable!("settings return above, before the payload policy"),

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
        App | Skill => InsertText(Text::Name),
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
        // Enter rewrites it. The alternatives — the original text, the same
        // text through a different profile — are named in the panel, because
        // "the other profile" is not one thing to compress into an unlabelled
        // opposite when there are three of them.
        Rewrite => return None,
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
            "fallbacks" => "Choose the providers…",
            "enter" => {
                if item.fields.first().map(String::as_str) == Some("per kind") {
                    "Switch to copy-commands"
                } else {
                    "Switch to per-kind"
                }
            }
            "rewrite" => "Choose the service…",
            "rewrite_url" => "Change the endpoint…",
            "rewrite_model" => "Choose a model…",
            "rewrite_key" => "Set the API key…",
            "rewrite_profile" => "Choose the style…",
            "rewrite_review" => {
                if item.fields.first().map(String::as_str) == Some("on") {
                    "Turn the review pass off"
                } else {
                    "Turn the review pass on"
                }
            }
            "openwith" => "Manage open-with rules",
            "snippets" => "Manage snippets",
            "quicklinks" => "Manage Quicklinks",
            "favorites" => "Manage Favorites",
            "aliases" => "Manage aliases",
            // Anything not named here falls back to the backing file, which is
            // right for a row that has no manager and a lie for one that does:
            // Enter on a collection opens `manage_collection`, so a missing arm
            // makes the footer describe an action the key does not perform.
            _ => "Open the file",
        };
    }
    if d == Default_::Act(Verb::Inspect) && item.kind == Kind::Proc {
        return "Show its full command";
    }
    if d == Default_::Act(Verb::RunHere) && item.get("update") == "available" {
        return "Update now";
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
        Default_::InsertText(Text::SkillFile) if clip => "Copy it for any agent",
        Default_::Act(Verb::CdThere) if clip => "Copy the cd command",
        Default_::Act(Verb::ResumeSession) if clip => "Copy the resume command",
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
        Default_::Act(Verb::ResumeSession) => "Resume this session",
        Default_::Act(Verb::RunHere) => "Run it here and show the output",
        Default_::Act(Verb::RunInShell) => "Run it in the shell",
        Default_::Act(Verb::Inspect) => "Show what is using it",
        Default_::Act(Verb::CdThere) => "Insert the cd command",
        Default_::Act(Verb::EditSetting) => "Change it",
        // The same sentence on either surface: the result lands on the
        // clipboard because that is the only place this launcher delivers to,
        // not because of a preference about commands.
        Default_::Act(Verb::Rewrite) => "Rewrite it and copy",
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
            _ => it.cmd.clone(),
        },
        // An instruction rather than a bare path: the point is for the
        // agent to follow the skill, and a path on its own invites it to
        // summarise the file instead.
        //
        // The name is in it as well as the path, and both are load-bearing.
        // The path is what makes this work at all — it is the whole reason
        // nothing has to be installed, and it is what tells two Skills sharing
        // a name apart. The name is what makes the pasted line readable: an
        // absolute path to somebody's home directory says nothing about what
        // is about to happen, either to the person who pasted it or in the
        // transcript they read back later.
        Text::SkillFile => {
            let p = it.get("file");
            let p = if p.is_empty() { it.get("dir") } else { p };
            if p.is_empty() {
                return it.cmd.clone();
            }
            let name = it.get("name");
            let name = if name.is_empty() { it.title.as_str() } else { name };
            format!("Use the skill \"{name}\": read {p} and follow it.")
        }
    }
}
