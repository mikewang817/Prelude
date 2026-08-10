//! The action panel: the verbs available for a thing, by what kind of thing
//! it is. This is the difference between a command picker and a launcher —
//! a port isn't text to insert, it's something you kill or inspect.

use crate::ansi::*;
use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::render::SEP;
use crate::ui;

type Act = (&'static str, String, String);

/// Action ids are &'static; the per-agent ones are built at runtime.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn a(id: &'static str, label: &str, sub: impl Into<String>) -> Act {
    (id, label.to_string(), sub.into())
}


/// The agents a per-agent verb can be pointed at, as the ids `apply` takes.
///
/// One row can stand for several things — a skill merged across four agents
/// is four directories — and the panel used to enumerate them: `Copy it to
/// codex`, `Copy it to pi`, `Copy it to opencode`, `Copy it to all missing`,
/// plus a `Delete …` each. Seven rows that are really three verbs and a
/// choice of agent. Raycast's answer is a submenu, and it is the right one:
/// the verb is the decision, the agent is a parameter of it.
pub fn agent_options(it: &Item, verb: &str) -> Vec<(String, String, String)> {
    let missing: Vec<&str> = it.get("missing").split(',').filter(|s| !s.is_empty()).collect();
    let has: Vec<&str> = it
        .get("agent")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "shared")
        .collect();
    match verb {
        "run" => has.iter().map(|n| (format!("run:{n}"), (*n).to_string(), String::new())).collect(),
        "lend" => match it.kind {
            // A skill can only be borrowed by an agent that lacks it.
            Kind::Skill if it.get("source_sensitive") == "true" => Vec::new(),
            Kind::Skill => missing
                .iter()
                .filter(|n| **n != "shared" && crate::lend::can_borrow_skill(n))
                .filter(|n| crate::agent::installed().contains(*n))
                .map(|n| (format!("lend:{n}"), (*n).to_string(), String::new()))
                .collect(),
            // An MCP server can go to any other agent that has a flag for it.
            _ if it.get("portable") == "false" => Vec::new(),
            _ => {
                let mut owners: Vec<String> = serde_json::from_str(it.get("owners")).unwrap_or_default();
                if owners.is_empty() { owners.push(it.get("agent").to_string()); }
                crate::agent::installed()
                    .into_iter()
                    .filter(|n| !owners.iter().any(|owner| owner == n) && crate::lend::can_borrow_mcp(n))
                    .map(|n| (format!("lend:{n}"), n.to_string(), String::new()))
                    .collect()
            }
        },
        "cp" if it.kind == Kind::Skill && it.get("source_sensitive") == "true" => Vec::new(),
        "cp" => {
            let mut v: Vec<(String, String, String)> = missing
                .iter()
                .filter(|name| {
                    crate::agent::get(name).is_some_and(|spec| spec.capabilities.install_skill)
                })
                .map(|n| (format!("cp:{n}"), (*n).to_string(), String::new()))
                .collect();
            if v.len() > 1 {
                v.push(("cp:*".into(), "all of them".into(), format!("{} agents", v.len())));
            }
            v
        }
        // Every other installed agent that has an `mcp add` of its own.
        "install" if it.kind == Kind::Mcp
            && (it.get("portable") == "false" || it.get("sensitive") == "true") => Vec::new(),
        "install" => {
            let mut owners: Vec<String> = serde_json::from_str(it.get("owners")).unwrap_or_default();
            if owners.is_empty() { owners.push(it.get("agent").to_string()); }
            crate::agent::installed()
                .into_iter()
                .filter(|name| {
                    !owners.iter().any(|owner| owner == name)
                        && crate::agent::get(name).is_some_and(|spec| spec.capabilities.install_mcp)
                })
                .map(|n| (format!("install:{n}"), n.to_string(), String::new()))
                .collect()
        }
        "mcpsync" if it.get("portable") == "false" || it.get("sensitive") == "true" => Vec::new(),
        "mcpsync" => {
            let variants = crate::capability::mcp_variants(it);
            let owner = it.get("agent");
            let source_hash = it.get("definition_hash");
            variants.into_iter()
                .filter(|variant| variant.agent != owner && variant.fingerprint != source_hash)
                .map(|variant| (
                    format!("mcp-sync:{}", variant.agent),
                    variant.agent.clone(),
                    format!("replace {} with {owner}'s definition", variant.agent),
                ))
                .collect()
        }
        "diff" | "sync" => {
            let copies = crate::capability::copies(it);
            let mut options = Vec::new();
            for (left, source) in copies.iter().enumerate() {
                for (right, target) in copies.iter().enumerate() {
                    if left == right || source.fingerprint == target.fingerprint {
                        continue;
                    }
                    if verb == "diff" && right < left {
                        continue;
                    }
                    let option = if verb == "diff" {
                        (
                            format!("diff:{}:{}", source.agent, target.agent),
                            format!("{} ↔ {}", source.agent, target.agent),
                            "read-only recursive diff".to_string(),
                        )
                    } else {
                        (
                            format!("sync:{}:{}", source.agent, target.agent),
                            format!("replace {} from {}", target.agent, source.agent),
                            crate::paths::tilde(&target.dir),
                        )
                    };
                    options.push(option);
                }
            }
            options
        }
        // Only agents that actually have a copy to delete.
        "rm" => crate::sources::agents::copies_of(it)
            .into_iter()
            .map(|(agent, dir)| (format!("rm:{agent}"), agent, crate::paths::tilde(&dir)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Add a verb to the panel: as one row when there is a choice to make, or as
/// the choice itself when there is only one.
///
/// A submenu over a single option is a keystroke that asks a question with
/// one answer.
fn with_options(v: &mut Vec<Act>, it: &Item, verb: &'static str, one: &str, many: &str, sub: &str) {
    let opts = agent_options(it, verb);
    match opts.len() {
        0 => {}
        // The name substitutes into the sentence; the detail belongs to the
        // submenu row, where there is a column for it. Splicing a path into
        // "Delete {}'s copy" produced "Delete claude · /tmp/a's copy".
        1 => v.push((leak(opts[0].0.clone()), one.replace("{}", &opts[0].1), sub.to_string())),
        n => v.push((
            leak(format!("menu:{verb}")),
            many.to_string(),
            if sub.is_empty() { format!("{n} agents") } else { sub.to_string() },
        )),
    }
}

pub fn actions_for(it: &Item, surface: crate::defaults::Surface) -> Vec<Act> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let mut acts = match it.kind {
        Kind::Session => {
            let mut v = Vec::new();
            if it.get("active_run").is_empty() {
                v.push(a("run", "Resume now", it.get("agent")));
            }
            if crate::sources::sessions::fork_cmd(it.get("agent"), it.get("id")).is_some() {
                v.push(a("session-fork", "Fork conversation", "new Session with this context"));
            }
            // Resuming with something the Agent does not own, for one run.
            // Absent for the agents with no one-run syntax, on exactly the
            // rule `fork_cmd` follows above: a command assembled for those
            // would look right on the prompt and fail after the launcher had
            // closed. And never against a live Run, for the same reason
            // `Resume now` is not offered there — it would start a competitor.
            if it.get("active_run").is_empty() && !it.get("id").is_empty() {
                if crate::lend::can_borrow_skill(it.get("agent")) {
                    v.push(a("session-skill", "Resume with a skill…", "for this run only"));
                }
                if crate::lend::can_borrow_mcp(it.get("agent")) {
                    v.push(a("session-mcp", "Resume with an MCP server…", "for this run only"));
                }
            }
            if it.get("pinned") == "true" {
                v.push(a("session-unpin", "Unpin conversation", "return to recency order"));
            } else {
                v.push(a("session-pin", "Pin conversation", "keep above recent sessions"));
            }
            v.push(a("session-rename", "Rename conversation…", it.title.clone()));
            if !it.get("native_title").is_empty() {
                v.push(a("session-reset-name", "Restore native name", it.get("native_title")));
            }
            if it.get("archived") == "true" {
                v.push(a("session-unarchive", "Restore from archive", "return to normal session search"));
            } else if it.get("active_run").is_empty() {
                v.push(a("session-archive", "Archive conversation", "find later with s:is:archived"));
            }
            v.push(a("details", "Show conversation details", it.get("opening")));
            if !it.get("cwd").is_empty() {
                v.push(a("newsession", "Start fresh in this project", it.get("agent")));
                v.push(a("cdsession", "Insert cd command", crate::paths::tilde(it.get("cwd"))));
            }
            if !it.get("file").is_empty() {
                v.push(a("session-export", "Export raw conversation", "private Prelude exports folder"));
                // The other half: the raw JSONL is what you hand back to the
                // agent that wrote it, and this is what you send to a person.
                v.push(a("session-export-md", "Export portable transcript", "readable Markdown · redacted"));
                v.push(a("reveal-finder", "Reveal native session file", it.get("file")));
            }
            v.push(a("copy", "Copy session ID", it.get("id")));
            if it.get("active_run").is_empty() && !it.get("file").is_empty() {
                v.push(a("session-trash", "Move native conversation to Trash…", "recoverable · removes it from the agent"));
            }
            v
        }
        // An agent CLI. Enter puts its name on the prompt, because that is
        // where you add `--resume`, a model, or an opening question.
        Kind::Agent => {
            let n = it.get("agent").to_string();
            let mut v = Vec::new();
            if it.get("run_count").parse::<usize>().unwrap_or(0) > 0 {
                v.push(a("agent-runs", "Running instances…", it.get("run_count")));
            }
            // The single most common thing anyone does with an agent they
            // have used before, and the row already says how many sessions
            // there are. Finding the newest by hand means `s:`, reading
            // dates, and copying a uuid.
            if let Some(s) = crate::sources::sessions::latest_for(&n) {
                v.push((
                    leak(format!("resume:{n}")),
                    "Resume latest session".to_string(),
                    format!("{} · {}", crate::width::dtrunc(&s.title, 40), s.fields.get(2).cloned().unwrap_or_default()),
                ));
                v.push(a("agent-sessions", "Browse conversations…", it.fields.get(2).cloned().unwrap_or_default()));
            }
            if crate::agent::get(&n).is_some_and(|spec| spec.capabilities.ask) {
                v.push((leak(format!("askagent:{n}")), format!("Ask {n} a one-off question"), "answer here".into()));
            }
            if let Some(p) = crate::sources::agents::config_for(&n) {
                v.push((
                    leak(format!("agentcfg:{n}")),
                    "Open settings".to_string(),
                    crate::paths::tilde(&p),
                ));
            }
            v.push(a("agent-doctor", "Diagnose Agents", "versions, login, config and relationships"));
            v
        }
        // A running agent. Nothing here reaches into its terminal — Prelude
        // cannot put your cursor in somebody else's window, and typing into
        // one behind your back was worse than not offering it. What is left
        // is what a pid and a conversation file can honestly answer: what it
        // last said, where it is working, and ending it.
        Kind::Run => {
            let mut v = Vec::new();
            v.push(a("details", "Show last response", it.get("subject")));
            // A message goes to its inbox, for it to collect. That is slower
            // than typing into a pane and it is the only delivery that does
            // not depend on where the agent happens to be running.
            v.push(a("say", "Leave it a message…", "waits in its inbox"));
            if !it.get("cwd").is_empty() {
                v.push(a("cdrun", "Insert cd command", crate::paths::tilde(it.get("cwd"))));
            }
            if !it.get("pid").is_empty() {
                v.push(a("copy", "Copy PID", it.get("pid")));
            }
            v.push(a("killrun", "End agent…", format!("{} · pid {}", it.get("agent"), it.get("pid"))));
            v
        }
        // A question an agent is blocked on. Enter already answers it, so
        // this is everything else you might want first: answer it in one
        // keystroke either way, or read what it came out of.
        Kind::Msg => vec![
            a("msg:go", "Answer “go ahead”", "unblocks it immediately"),
            a("msg:no", "Answer “no”", "unblocks it immediately"),
            a("details", "Show conversation context", ""),
            a("copy", "Copy question", ""),
        ],
        // One of Prelude's own preferences. Enter changes it, and this is
        // everything else — never Enter again under the same words, which is
        // the one thing this panel is not for.
        Kind::Setting => {
            let mut v = Vec::new();
            let enter = it.get("edit");
            match it.get("setting") {
                "roots" => {
                    // Enter is "Add a folder…", so it is absent here.
                    v.push(a("set-root-remove", "Remove a folder…", "leaves the folder alone"));
                    v.push(a("details", "Show every root", ""));
                }
                "index" => {
                    v.push(a("details", "Show index status", ""));
                }
                "hotkey" => {
                    v.push(a("set-default", "Reset to Cmd+Space", "still checks for conflicts"));
                    v.push(a("set-panel-open", "Start the panel", "if it is not running"));
                    v.push(a("set-panel-restart", "Restart the panel", "picks up a rebuilt binary"));
                }
                "paneldir" => v.push(a("set-default", "Reset to $HOME", "")),
                "preview" | "enter" | "key" | "height" => {
                    v.push(a("set-default", "Reset to the default", it.get("default")));
                    v.push(a("details", "What this changes", ""));
                }
                _ => {}
            }
            let path = it.get("path");
            if !path.is_empty() {
                let short = crate::paths::tilde(path);
                let exists = std::path::Path::new(path).exists();
                let can_create = matches!(
                    it.get("setting"),
                    "roots" | "openwith" | "snippets" | "quicklinks" | "favorites"
                        | "key" | "height" | "preview" | "enter"
                );
                // `set-open-file` is Enter for list-shaped settings. It asks
                // the owning module to materialise a missing file first, so
                // "none yet" never turns into a Launch Services error.
                if enter != crate::settings::EDIT_OPEN && (exists || can_create) {
                    let label = if exists { "Open the file" } else { "Create and open the file" };
                    v.push(a("set-open-file", label, short.clone()));
                }
                if exists {
                    v.push(a("open", "Open it in your editor", format!("{editor} …")));
                    v.push(a("reveal-finder", "Reveal in Finder", short.clone()));
                }
                v.push(a("copyabs", "Copy the file path", short));
            }
            v
        }
        Kind::Config => open_actions(it.kind, it.get("path"), &editor),
        Kind::Port => vec![
            a("copy", "Copy PID", it.get("pid")),
            a("run", "Kill process…", format!("{} · pid {}", it.get("proc"), it.get("pid"))),
        ],
        Kind::Proc => vec![
            a("copy", "Copy PID", it.get("pid")),
            a("run", "Kill process…", format!("{} · {}% CPU", it.get("name"), it.get("cpu"))),
        ],
        Kind::Container => vec![
            a("logs", "Insert follow-logs command", format!("docker logs -f {}", it.get("name"))),
            a("restart", "Insert restart command", format!("docker restart {}", it.get("name"))),
            a("copy", "Copy container name", it.get("name")),
            a("stop", "Insert stop command", format!("docker stop {}", it.get("name"))),
        ],
        Kind::Skill => {
            let target = first_nonempty(it, &["file", "dir"]);
            let mut v = Vec::new();
            with_options(&mut v, it, "run", "Run with {}", "Run with…", "");
            // The two bare forms of the same skill, for a conversation that
            // is already open somewhere else. `/name` works only for an agent
            // that has it — to any other it is a line of prose that means
            // nothing, silently — so the file pointer sits beside it rather
            // than behind a guess about who you are talking to. Prelude used
            // to make that guess by asking tmux what the pane underneath was
            // running; with no pane underneath, the choice is yours.
            if !target.is_empty() {
                v.push(a("skillcmd", "Insert the slash command", &it.cmd));
                v.push(a("skillfile", "Point an agent at its file", &target));
            }
            // Borrowing comes before copying: it is the lighter of the two,
            // and the one that is nearly always what was meant. Copying puts
            // a second copy of the skill on disk, to be maintained forever;
            // borrowing lasts exactly one run and leaves nothing behind.
            with_options(&mut v, it, "lend", "Prepare one-off run with {}",
                         "Prepare one-off run with…", "inserts command · nothing installed");
            with_options(&mut v, it, "cp", "Install in {}", "Install into…", "");
            if it.get("integrity") == "divergent" {
                with_options(&mut v, it, "diff", "Compare {}", "Compare copies…",
                             "before replacing either copy");
                with_options(&mut v, it, "sync", "Replace {}", "Replace a divergent copy…",
                             "shows diff · old copy goes to Trash");
            }
            if !it.get("desc").is_empty() {
                v.push(a("desc", "Read instructions", ""));
            }
            if !target.is_empty() {
                v.push(a("open", "Open SKILL.md in editor", &target));
            }
            // …and where there is more than one copy, all of them. With one
            // copy `Open` above already is this, and a row that opens the
            // same directory twice under two names teaches you not to read
            // the panel.
            let copies = crate::sources::agents::copy_paths(it);
            if copies.len() > 1 {
                v.push(a("open-copies", "Open all copies", format!("{} directories", copies.len())));
            }
            // Last, and one entry per agent that actually has a copy. A
            // skill merged across four agents is four separate decisions —
            // "delete it" would otherwise mean something different depending
            // on a number the row only hints at.
            with_options(&mut v, it, "rm", "Delete {} copy…", "Delete a copy…",
                         "moves it to the Trash");
            v
        }
        Kind::Mcp => {
            let target = first_nonempty(it, &["file", "dir", "config"]);
            let owner = it.get("agent");
            let mut v = Vec::new();
            if !it.get("tools").is_empty() && it.get("tools") != "[]" {
                let count = serde_json::from_str::<Vec<crate::mcp_tools::Tool>>(it.get("tools"))
                    .map(|tools| tools.len()).unwrap_or(0);
                v.push(a("mcp-tools", "Show cached tools", format!("{count} tools")));
            }
            v.push(a("mcprefresh", if owner == "claude" {
                "Test connection now"
            } else {
                "Refresh status now"
            }, owner));
            if it.get("transport") == "stdio" && it.get("health") != "disabled" {
                v.push(a("mcp-tools-refresh", "Refresh tool inventory", "explicit protocol handshake"));
            }
            // Only offered to agents that can take one for a single run, and
            // never back to the one that already has it. Whether *this*
            // particular server can be lent at all takes a subprocess to
            // find out, so that answer arrives when the action runs rather
            // than being guessed at here.
            with_options(&mut v, it, "lend", "Prepare one-off use with {}",
                         "Prepare one-off use with…", &format!("inserts command · from {owner}"));
            // Slot 7. Lending lasts one run; this is the other half, and it
            // uses the agent's own `mcp add` rather than editing anyone's
            // config file — the CLI knows the format and we do not have to.
            with_options(&mut v, it, "install", "Insert install command for {}",
                         "Insert install command…", "review before running");
            if crate::capability::mcp_variants(it).len() > 1 {
                v.push(a("mcpcompare", "Compare Agent definitions", "redacted capability matrix"));
            }
            if it.get("comparison") == "divergent" {
                with_options(&mut v, it, "mcpsync", "Prepare replacement for {}",
                             "Prepare definition replacement…", "shows redacted comparison first");
            }
            // Inspection is Enter at a shell and is stated in the header, so
            // it is not repeated as a selectable action here.
            // A row that says `⚠ not logged in` must offer a route forward.
            if matches!(it.get("health"), "auth" | "needsauth" | "failed") {
                v.push(a("mcplogin", "Insert login command", format!("{owner} mcp login")));
            }
            // Configuration changes are inserted for review rather than run.
            if !target.is_empty() {
                v.push(a("open", "Open owner configuration", &target));
            }
            v.push(a("copy", "Copy server name", ""));
            v.push(a("mcpremove", "Insert remove command…", format!("{owner} mcp remove")));
            v
        }
        Kind::File | Kind::Find => open_actions(it.kind, it.get("path"), &editor),
        // Enter inserts the payload path(s); the secondary restores the
        // original pasteboard object. Object clips also get Finder verbs,
        // while only actual text can be translated.
        Kind::Clip if it.get("clip_kind") == "files" => vec![
            a("openit", "Open first file", it.get("path")),
            a("reveal-finder", "Reveal first file in Finder", it.get("path")),
            a("copyabs", "Copy paths as text", it.get("full")),
        ],
        Kind::Clip if it.get("clip_kind") == "image" => vec![
            a("openit", "Open image", it.get("path")),
            a("reveal-finder", "Reveal image in Finder", it.get("path")),
            a("copyabs", "Copy image path", it.get("path")),
        ],
        Kind::Clip => vec![
            a("tr_en", "Translate to English", ""),
            a("tr_zh", "Translate to Chinese", ""),
        ],
        Kind::Snippet => vec![
            a("editsnips", "Edit snippets file", crate::paths::config().join("snippets.toml").to_string_lossy()),
            a("copy", "Copy raw", ""),
        ],
        // Enter copies the translation and the secondary inserts it, so
        // only the third row is new.
        Kind::Translate => vec![a("tr_src", "Copy the original", it.get("source"))],
        // There are exactly two things to do with a number, and Enter and
        // its counterpart already are both of them. The panel listed them
        // again underneath: four rows, two actions.
        Kind::Calc => vec![],
        Kind::Search if !it.get("provider").is_empty() => vec![
            a("quicklink-edit", "Edit Search Providers", crate::compute::quicklinks_file().to_string_lossy()),
        ],
        Kind::Search => vec![],
        Kind::Ssh => vec![
            a("editssh", "Edit ~/.ssh/config", ""),
            a("copy", "Copy host", it.get("host")),
        ],
        // No "Launch it now": Enter is that, and the panel states it above.
        // Slot 5 had no answer here while a file two rows away had two, and
        // slot 9 none at all — yet dragging an .app to the Trash is how you
        // uninstall on this platform.
        Kind::App => vec![
            a("reveal-finder", "Reveal in Finder", it.get("path")),
            a("copy-file", "Copy application", it.get("path")),
            a("copy", "Copy application path", it.get("path")),
            a("insert", "Insert open command", &it.cmd),
            a("trash", "Move to Trash…", "uninstalls it, recoverably"),
        ],
        Kind::Dir => vec![
            a("copy-file", "Copy folder", it.get("path")),
            a("insert", "Insert cd command", &it.cmd),
            a("copy", "Copy path", it.get("path")),
        ],
        Kind::Sys => vec![
            a("copy", "Copy the command", ""),
        ],
        // Enter opens the browser directly through Launch Services. The two
        // useful alternatives are text: hand it over, or copy it.
        Kind::Link => vec![a("copy", "Copy URL", it.get("url"))],
        // History, scripts, $PATH, branches, folders. Enter inserts them and
        // the secondary runs them, which is the whole of what they are — so
        // this arm adds nothing and the generic tail below fills in `run`,
        // `runhere` and `copy` where each still means something.
        //
        // It used to open with `Insert into prompt`, which is what Enter
        // already does and was even labelled identically. A panel whose
        // third row repeats its first is teaching you not to read it.
        _ => vec![],
    };
    // A quicklink names a stable object without changing what kind of object
    // it is. Its configuration belongs before any action that removes the
    // target itself.
    if crate::compute::quicklinkable(it.kind) {
        let mut qacts = Vec::new();
        if it.get("quicklink").is_empty() {
            qacts.push(a("quicklink-create", "Create Quicklink…", "give this object a keyword"));
        } else {
            qacts.push(a("quicklink-edit", "Edit Quicklink Definition", it.get("quicklink")));
            if it.get("quicklink_managed") == "true" {
                qacts.push(a("quicklink-remove", "Remove Quicklink…", "the target is untouched"));
            }
        }
        let at = acts.iter().position(|(id, ..)| is_destructive(it.kind, id)).unwrap_or(acts.len());
        acts.splice(at..at, qacts);
    }

    // Archive is a reversible Prelude overlay. It never moves a Skill copy or
    // edits/disables an MCP definition. Restoring is first on an archived row;
    // putting away a visible row sits beside the other management actions.
    if crate::archive::key(it).is_some() {
        let action = if it.get("archived") == "true" {
            a("capability-unarchive", "Restore from archive", "return to normal inventory")
        } else {
            let scope = if it.kind == Kind::Skill { "skill:is:archived" } else { "mcp:is:archived" };
            a("capability-archive", "Archive in Prelude", format!("find later with {scope}"))
        };
        if it.get("archived") == "true" {
            acts.insert(0, action);
        } else {
            let at = acts.iter().position(|(id, ..)| is_destructive(it.kind, id)).unwrap_or(acts.len());
            acts.insert(at, action);
        }
    }

    // Favourites are Prelude's launcher preference, not native Agent
    // metadata. They are available on stable inventory objects only and sit
    // before destructive management actions.
    if crate::favorites::key(it).is_some() {
        let action = if it.get("favorite") == "true" {
            a("unfavorite", "Remove from Favorites", "keeps the Agent object unchanged")
        } else {
            a("favorite", "Add to Favorites", "promotes it inside this category")
        };
        let at = acts.iter().position(|(id, ..)| is_destructive(it.kind, id)).unwrap_or(acts.len());
        acts.insert(at, action);
    }

    // Enter is already stated in the main footer and again in this panel's
    // non-selectable header. Listing it as the first action made ^K start by
    // repeating the key the person had just declined. The secondary remains
    // here because it has no reliable terminal key of its own.
    //
    // Some rich agent kinds already expose that same behaviour under a more
    // useful, specific label, so they do not get a generic secondary row.
    let specific_alternative = matches!(
        it.kind,
        Kind::Run | Kind::Msg | Kind::Session | Kind::App | Kind::Skill | Kind::Mcp | Kind::Setting
    );
    if !specific_alternative {
        if let Some(label) = crate::defaults::describe_secondary(it, surface) {
            acts.insert(0, a("secondary", label, ""));
        }
    }
    // Output-in-Prelude is useful for small non-interactive commands, not as
    // a generic fallback for every row that happens to carry command-shaped
    // text.
    // A port's and a process's command line *is* the kill, and both kinds
    // already offer it, named, at the bottom. Adding "Run here, inside this
    // window" is a second route to the same kill wearing a harmless label —
    // in the third row, where the destructive one was moved out of.
    let generic_run_would_kill = matches!(it.kind, Kind::Port | Kind::Proc);
    // …and it must not repeat what the first two rows already offer. On an
    // agent the secondary *is* "Run it in the shell", so the tail added
    // "Run in the shell below" underneath it: the same action, twice, in one
    // six-line panel. Checking ids could never catch that — the duplicate is
    // in the behaviour, not the name.
    use crate::defaults::{Default_, Verb};
    let already = |v: Verb| {
        crate::defaults::on_enter(it) == Default_::Act(v)
            || crate::defaults::on_secondary(it, surface) == Some(Default_::Act(v))
    };
    // Three verbs all end in `emit("RUN", cmd)`, so any of them above means
    // the generic runner is the same keystroke with a duller label.
    let runs_it = already(Verb::RunInShell) || already(Verb::Launch) || already(Verb::OpenUrl);
    let useful_in_preview = matches!(
        it.kind,
        Kind::History | Kind::Script | Kind::Path | Kind::Snippet | Kind::Sys | Kind::Git
    );
    if useful_in_preview
        && !generic_run_would_kill
        && !already(Verb::RunHere)
        && !acts.iter().any(|(id, ..)| *id == "runhere")
    {
        acts.push(a("runhere", "Run and show output", "inside Prelude"));
    }
    // "Run now" means "we hand it over already submitted", which needs a
    // shell on the other end of the handover. From the panel there is only
    // the clipboard, where a submitted command and an unsubmitted one are the
    // same bytes — so the row would be Enter again under a bolder name.
    // `runhere` above is the honest way to run something from that surface,
    // and it stays.
    let runnable = surface != crate::defaults::Surface::Clipboard
        && (matches!(
            it.kind,
            Kind::History | Kind::Script | Kind::Path | Kind::Snippet | Kind::Ssh
                | Kind::Container | Kind::Git | Kind::Sys | Kind::Agent
        ) || (it.kind == Kind::Session && it.get("active_run").is_empty()));
    if runnable && !runs_it && !acts.iter().any(|(id, ..)| *id == "run") {
        acts.push(a("run", "Run now", ""));
    }
    // `copyabs` already copies the path, and for a file that is exactly what
    // `copy` copies too — the same action twice, worded differently. Nor is
    // there anything to copy off an agent row: `pi` is two letters you can
    // type faster than you can open this panel.
    let generic_copy_is_useful = matches!(
        it.kind,
        Kind::History | Kind::Script | Kind::Path | Kind::Snippet | Kind::Ssh
            | Kind::Container | Kind::Git | Kind::Sys | Kind::Clip | Kind::Translate
            | Kind::Calc | Kind::Dir
    );
    if generic_copy_is_useful
        && !already(Verb::CopyResult)
        && !acts.iter().any(|(id, ..)| *id == "copy" || *id == "copyabs")
    {
        acts.push(a("copy", "Copy to clipboard", ""));
    }
    // Do not append generic "Ask an agent" or "Go to project" rows. They
    // made every panel look complete while usually being unrelated to why
    // this particular item was selected. Kinds that genuinely need project
    // navigation name it explicitly above.
    //
    // Each kind above is written in the order a person actually reaches for
    // its actions. Preserve that order; the only global rule is that danger
    // stays at the bottom, away from a fast Enter.
    acts.sort_by_key(|(id, ..)| is_destructive(it.kind, id));
    acts
}

/// Is this entry one you cannot take back with another keystroke?
///
/// Two consequences, both Raycast's: it is drawn in red, and — where the
/// thing genuinely cannot be reverted — it asks first. Stopping a container
/// is red but not confirmed, because `docker start` exists; killing a
/// process is both, because nothing brings it back.
pub fn is_destructive(kind: Kind, id: &str) -> bool {
    matches!(
        id,
        "killrun" | "stop" | "trash" | "session-trash" | "mcpremove" | "quicklink-remove"
    )
        || id.starts_with("rm:")
        || id.starts_with("sync:")
        || matches!(id, "menu:rm" | "menu:sync")
        || (id == "run" && matches!(kind, Kind::Port | Kind::Proc))
}

/// …and of those, the ones with no way back at all.
pub fn needs_confirming(kind: Kind, id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "killrun" => Some(("End it", "the conversation in it is lost")),
        "run" if matches!(kind, Kind::Port | Kind::Proc) => {
            Some(("Kill it", "the process does not come back"))
        }
        // `rm:` asks its own question, naming the agent and the path.
        _ => None,
    }
}

/// The short list of useful alternatives for a file.
///
/// This is the half of the launcher that behaves like Finder rather than a
/// shell: open it another way, locate it, extract its path, or change what
/// application owns its extension next time.
fn open_actions(kind: Kind, path: &str, editor: &str) -> Vec<Act> {
    let chosen = crate::openwith::chosen_for(path);
    let ext = crate::openwith::ext_of(path);
    let scope = if ext.is_empty() { "files like this".to_string() } else { format!(".{ext} files") };
    let mut v = vec![
        (leak("openwith".into()), "Open with…".into(), match &chosen {
            Some(app) => format!("currently {app}"),
            None => "currently the system default".into(),
        }),
        a("open", "Open in editor", editor),
        a("reveal-finder", "Reveal in Finder", parent_of(path)),
        a("copy-file", "Copy file", path),
        a("copyabs", "Copy path", path),
        (leak("openalways".into()), format!("Change default app for {scope}…"), "used next time".into()),
    ];
    // Slot 9. A skill could be deleted and the kind the launcher spends most
    // of its rows on could not. Not offered for a config: deleting the file
    // your agent is configured by, out of a fuzzy list, is a foot-gun with
    // very little on the other side of it.
    if kind != Kind::Config {
        v.push(a("trash", "Move it to the Trash…", "recoverable from Finder"));
    }
    v
}

fn mcp_tool_lines(it: &Item) -> Vec<String> {
    let tools: Vec<crate::mcp_tools::Tool> = serde_json::from_str(it.get("tools")).unwrap_or_default();
    if tools.is_empty() {
        return vec![format!("tool inventory: {}", it.get("tools_status"))];
    }
    tools.into_iter().map(|tool| {
        if tool.description.is_empty() { tool.name } else { format!("{} · {}", tool.name, tool.description) }
    }).collect()
}

fn mcp_matrix_lines(it: &Item) -> Vec<String> {
    let mut lines: Vec<String> = crate::capability::mcp_variants(it).into_iter().map(|variant| {
        let hash = variant.fingerprint.strip_prefix("fnv1a64-v1:").unwrap_or(&variant.fingerprint);
        format!(
            "{:<10} {:<10} {} · {}{}",
            variant.agent,
            variant.health,
            variant.summary,
            hash,
            if !variant.portable {
                " · owner-account only"
            } else if variant.sensitive {
                " · private fields omitted"
            } else {
                ""
            },
        )
    }).collect();
    lines.push(String::new());
    lines.push("public definition diff".into());
    lines.extend(crate::capability::mcp_definition_diff(it));
    lines
}

/// One choice out of a list, as its own picker rather than a `menu:` submenu.
///
/// The `menu:` mechanism builds its options while the panel is being drawn,
/// which is right when the options are already on the row and wrong when
/// finding them means reading every skill directory on the machine. These are
/// built only once somebody has chosen the verb.
///
/// `(key, name, detail)`, and the key comes back. Padded by display width,
/// because a skill named in CJK is twice as wide as its character count.
/// The Skills a Session's Agent could be handed for one run — which is every
/// Skill it does not already have.
///
/// Borrowing is defined as taking a capability the Agent does not own, and the
/// picker used to filter on nothing but "has a directory and a name": a claude
/// Session was offered a one-run borrow of claude's own nine Skills, which is
/// a nine-row question with no answer in it. The row it would have chosen does
/// nothing that typing `/name` into the conversation would not.
pub(crate) fn borrowable_skills(skills: &[Item], agent: &str) -> Vec<(String, String, String)> {
    skills
        .iter()
        .filter(|skill| crate::archive::visible(skill))
        .filter(|skill| !skill.get("dir").is_empty() && !skill.get("name").is_empty())
        .filter(|skill| !owned_by(skill.get("agent"), agent))
        .map(|skill| {
            (
                skill.get("dir").to_string(),
                skill.get("name").to_string(),
                skill.get("agent").to_string(),
            )
        })
        .collect()
}

/// The same rule for MCP servers, plus the one it already had: a server with
/// no transferable local definition — an account-hosted one — has nothing to
/// lend, so it is not a choice either.
pub(crate) fn borrowable_servers(servers: &[Item], agent: &str) -> Vec<(String, String, String)> {
    servers
        .iter()
        .filter(|server| crate::archive::visible(server))
        .filter(|server| !server.get("name").is_empty() && server.get("portable") != "false")
        .filter(|server| !owned_by(server.get("agent"), agent))
        .map(|server| {
            (
                format!("{}\u{1}{}", server.get("agent"), server.get("name")),
                server.get("name").to_string(),
                server.get("agent").to_string(),
            )
        })
        .collect()
}

/// Does `agent` already own a capability whose owner list is `owners`?
///
/// `owners` is the comma-joined `agent` field a Skill or MCP row carries.
/// `shared` is not an answer to this: `~/.agents/skills` is a location rather
/// than an agent, and `missing_agents` says as much by reporting a skill that
/// lives only there as missing from every one of them — so a shared skill is
/// still something claude may be handed for one run.
fn owned_by(owners: &str, agent: &str) -> bool {
    !agent.is_empty() && owners.split(',').map(str::trim).any(|owner| owner == agent)
}

pub fn pick_one(title: &str, choices: &[(String, String, String)]) -> Option<String> {
    if choices.is_empty() {
        return None;
    }
    let feed: String = choices
        .iter()
        .map(|(key, name, detail)| {
            let name = crate::width::pad_to(&crate::width::dtrunc(name, 28), 28, false);
            let tail = if detail.is_empty() { String::new() } else { format!("{DIM}· {detail}{RESET}") };
            format!("{name}{tail}{SEP}{key}\n")
        })
        .collect();
    ui::pick_raw(feed.trim_end(), title, "Choose › ", "Choose  Enter   ·   Back  Esc", "")
}

fn first_nonempty(it: &Item, keys: &[&str]) -> String {
    keys.iter().map(|k| it.get(k)).find(|v| !v.is_empty()).unwrap_or("").to_string()
}

/// Just the file name, for the picker's title bar.
fn short_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn parent_of(p: impl AsRef<str>) -> String {
    let p = p.as_ref();
    if std::path::Path::new(p).is_dir() {
        return p.to_string();
    }
    p.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
}

/// The action panel is a modal over the main search. Esc means "back", not
/// "close Prelude"; the caller uses this distinct code to reopen the list.
pub const PANEL_BACK: i32 = 131;

pub fn panel(it: &Item) -> i32 {
    let surface = crate::defaults::surface();
    let acts = actions_for(it, surface);
    let feed: String = acts
        .iter()
        .map(|(id, label, sub)| {
            let tail = if sub.is_empty() { String::new() } else { format!("{DIM}· {sub}{RESET}") };
            let padded = crate::width::pad_to(
                &crate::width::dtrunc(&crate::width::flatten(label), 28), 28, false
            );
            let label = if is_destructive(it.kind, id) {
                format!("{RED}{padded}{RESET}")
            } else {
                padded
            };
            format!("{label}{tail}{SEP}{id}\n")
        })
        .collect();

    let title = crate::width::dtrunc(&crate::width::flatten(&it.title), 48);
    let kind = it.kind.style().1;
    let default = crate::defaults::describe(it, surface);
    let header = format!("{DIM}Default: {default} · Enter{RESET}");

    loop {
        let Some(mut id) = ui::pick_raw(
            feed.trim_end(),
            &format!(" {title} · {kind} "),
            "Action › ",
            "Choose  Enter   ·   Back  Esc",
            &header,
        ) else {
            return PANEL_BACK;
        };

        // A submenu is a parameter picker, not a second modal. Esc from it
        // returns to this list instead of throwing the user back to the shell.
        if let Some(verb) = id.strip_prefix("menu:") {
            let opts = agent_options(it, verb);
            let choices: String = opts
                .iter()
                .map(|(oid, name, detail)| {
                    let name = crate::width::pad_to(name, 28, false);
                    let tail = if detail.is_empty() {
                        String::new()
                    } else {
                        format!("{DIM}· {detail}{RESET}")
                    };
                    format!("{name}{tail}{SEP}{oid}\n")
                })
                .collect();
            let Some(chosen) = ui::pick_raw(
                choices.trim_end(),
                &format!(" {title} "),
                "Choose › ",
                "Choose  Enter   ·   Back  Esc",
                "",
            ) else {
                continue;
            };
            id = chosen;
        }

        let code = apply(&id, it);
        if code == 130 {
            // A canceled confirmation returns to the actions too.
            continue;
        }
        if code != 0 || !stays_in_panel(&id) {
            return code;
        }
    }
}

fn stays_in_panel(id: &str) -> bool {
    matches!(
        id,
        "copy" | "copyabs" | "copy-file" | "desc" | "details" | "mcptools" | "mcp-tools"
            | "mcpcompare"
    ) || id.starts_with("diff:")
}

pub fn apply(id: &str, it: &Item) -> i32 {
    // Anything with no way back says so before it happens, naming what is
    // lost. Cancel is the default, so a stray Enter cancels.
    if let Some((verb, loss)) = needs_confirming(it.kind, id) {
        let what = crate::width::dtrunc(&crate::width::flatten(&it.title), 40);
        if !ui::confirm(&format!("{} {what}?", verb.to_lowercase()), verb, loss) {
            return 130;
        }
    }
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let target = first_nonempty(it, &["file", "dir", "config", "path"]);
    match id {
        "insert" => {
            crate::frecency::bump(&it.cmd);
            let cmd = if it.kind == Kind::Snippet { ui::fill_placeholders(&it.cmd) } else { it.cmd.clone() };
            ui::emit("INSERT", &cmd);
        }
        "run" => {
            crate::frecency::bump(&it.cmd);
            ui::emit("RUN", &it.cmd);
        }
        "runhere" => return crate::runhere::run_item(it),
        "details" => show_details(it),
        "copy" => ui::copy(&copy_text(it)),
        "copyabs" => ui::copy(if it.kind == Kind::Clip { it.get("full") } else { it.get("path") }),
        "copy-file" => {
            let path = first_nonempty(it, &["path", "file", "dir"]);
            match crate::clipd::copy_files(&[path]) {
                Ok(()) => ui::note("copied as a Finder object"),
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            }
        }
        "cd" => ui::emit("INSERT", &format!("cd {}", shq(it.cwd.as_deref().unwrap_or("")))),
        "here" => ui::emit("INSERT", it.cmd.split_once(' ').map(|(_, r)| r).unwrap_or(&it.cmd)),
        "inspect" => {
            let c = if it.kind == Kind::Proc {
                format!("ps -p {} -o command=", it.get("pid"))
            } else {
                format!("lsof -nP -iTCP:{} -sTCP:LISTEN", it.get("port"))
            };
            ui::emit("INSERT", &c);
        }
        "logs" => ui::emit("INSERT", &format!("docker logs -f {}", shq(it.get("name")))),
        "stop" => ui::emit("INSERT", &format!("docker stop {}", shq(it.get("name")))),
        "restart" => ui::emit("INSERT", &format!("docker restart {}", shq(it.get("name")))),
        "open" if !target.is_empty() => run_or_emit(&format!("{editor} {}", shq(&target))),
        "reveal" if !target.is_empty() => ui::emit("INSERT", &format!("cd {}", shq(&parent_of(&target)))),
        "desc" => show_description(it),
        // Prelude's own preferences. Each one is written by the code that
        // owns its file, so a chord goes through the same validation and the
        // same panel restart the CLI performs.
        "set-root-add" => return crate::settings::add_root_interactively(),
        "set-root-remove" => return crate::settings::remove_root_interactively(),
        "set-value" => return crate::settings::edit(it),
        "set-default" => return crate::settings::reset_item(it),
        "set-open-file" => return crate::settings::open_file(it),
        "set-panel-open" => match crate::global::open_panel() {
            Ok(()) => return 0,
            Err(e) => {
                ui::note(&e);
                return 2;
            }
        },
        "set-panel-restart" => match crate::global::restart_panel() {
            Ok(message) => ui::note(&message),
            Err(e) => {
                ui::note(&e);
                return 2;
            }
        },
        // The two bare forms of a skill, for a conversation open somewhere
        // else. `/name` works only for an agent that already has it; the file
        // pointer works for every agent, including the ones whose CLI cannot
        // load a borrowed skill at all.
        "skillcmd" => ui::emit("INSERT", &it.cmd),
        "skillfile" => ui::emit(
            "INSERT",
            &crate::defaults::text_for(it, crate::defaults::Text::SkillFile),
        ),
        "editsnips" => ui::emit("INSERT", &format!("{editor} {}", shq(&crate::paths::config().join("snippets.toml").to_string_lossy()))),
        "editssh" => ui::emit("INSERT", &format!("{editor} ~/.ssh/config")),
        "quicklink-create" => {
            let suggestion = crate::compute::quicklink_suggestion(it);
            let Some(raw_key) = ui::prompt_line_initial(" quicklink keyword ", &suggestion) else {
                return 130;
            };
            let key = match crate::compute::normalize_quicklink_key(&raw_key) {
                Ok(key) => key,
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            };
            if crate::compute::quicklinks().contains_key(&key) {
                ui::note(&format!("a quicklink called {key} already exists"));
                return 2;
            }
            let draft = match crate::compute::quicklink_draft(it) {
                Ok(Some(d)) => d,
                Ok(None) => {
                    ui::note("that kind cannot be a quicklink");
                    return 2;
                }
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            };
            if !ui::confirm(
                &format!("create quicklink “{key}”?"),
                "Create Quicklink",
                &format!("{} · {}", draft.kind, draft.target),
            ) {
                return 130;
            }
            match crate::compute::create_quicklink(&key, it) {
                Ok(_) => ui::note(&format!("created quicklink {key}")),
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            }
        }
        "quicklink-edit" => {
            // Reading once also creates the default file on a fresh install.
            let _ = crate::compute::quicklinks();
            run_or_emit(
                &format!("{editor} {}", shq(&crate::compute::quicklinks_file().to_string_lossy())),
            );
        }
        "quicklink-remove" => {
            let key = it.get("quicklink");
            if key.is_empty() {
                return 2;
            }
            if !ui::confirm(
                &format!("remove quicklink “{key}”?"),
                "Remove Quicklink",
                "the target is untouched",
            ) {
                return 130;
            }
            match crate::compute::remove_quicklink(key) {
                Ok(()) => ui::note(&format!("removed quicklink {key}")),
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            }
        }
        "tr_en" | "tr_zh" => {
            let text = if it.get("full").is_empty() { it.cmd.clone() } else { it.get("full").to_string() };
            let lang = if id == "tr_en" { "en" } else { "zh-Hans" };
            match crate::compute::translate(&text, lang) {
                Ok(v) => {
                    ui::copy(&v);
                    ui::emit("INSERT", &v);
                }
                Err(e) => {
                    ui::note(&e.to_string());
                    return 2;
                }
            }
        }
        "tr_src" => ui::copy(it.get("source")),
        "default" => return ui::apply_default(it),
        "secondary" => {
            if let Some(d) = crate::defaults::on_secondary(it, crate::defaults::surface()) {
                return ui::perform(it, d);
            }
        }
        // The two answers worth a keystroke of their own. Everything an agent
        // stops to ask is, at bottom, "may I" — and being able to say yes or
        // no without typing is what makes answering ten of them bearable.
        _ if id.starts_with("msg:") => {
            let text = if &id[4..] == "no" { "no" } else { "go ahead" };
            return crate::bus::answer(it.get("id"), text);
        }
        "cdsession" => ui::emit("INSERT", &format!("cd {}", shq(it.get("cwd")))),
        "session-fork" => match crate::sources::sessions::fork_cmd(it.get("agent"), it.get("id")) {
            Some(command) => ui::emit("RUN", &command),
            None => { ui::note("that agent has no known fork command"); return 2; }
        },
        "session-pin" | "session-unpin" => {
            let pinned = id == "session-pin";
            match crate::sources::sessions::set_pinned(it.get("session_id"), pinned) {
                Ok(()) => ui::note(if pinned { "pinned conversation" } else { "unpinned conversation" }),
                Err(e) => { ui::note(&e); return 2; }
            }
        }
        "session-rename" => {
            let Some(title) = ui::prompt_line_initial(" conversation name ", &it.title) else {
                return 130;
            };
            match crate::sources::sessions::rename(it.get("session_id"), &title) {
                Ok(()) => ui::note("renamed conversation"),
                Err(e) => { ui::note(&e); return 2; }
            }
        }
        "session-reset-name" => match crate::sources::sessions::rename(it.get("session_id"), "") {
            Ok(()) => ui::note("restored native conversation name"),
            Err(e) => { ui::note(&e); return 2; }
        },
        "session-archive" | "session-unarchive" => {
            let archived = id == "session-archive";
            match crate::sources::sessions::set_archived(it.get("session_id"), archived) {
                Ok(()) => ui::note(
                    if archived { "archived · find it with s:is:archived" } else { "restored conversation" },
                ),
                Err(e) => { ui::note(&e); return 2; }
            }
        }
        // One-run borrowing, applied to a conversation instead of a fresh
        // start. The picker is here rather than in `agent_options` because
        // building it means reading every skill directory or the MCP cache,
        // and the panel must not pay that to draw a row nobody chose.
        "session-skill" => {
            let skills = crate::sources::agents::skills();
            let choices = borrowable_skills(&skills, it.get("agent"));
            if choices.is_empty() {
                ui::note(&format!("{} already has every skill on this machine", it.get("agent")));
                return 0;
            }
            let Some(dir) = pick_one(" resume with which skill ", &choices) else { return 130 };
            let Some((_, name, _)) = choices.iter().find(|(d, ..)| *d == dir) else { return 2 };
            match crate::sources::sessions::resume_with_skill_cmd(
                it.get("agent"), it.get("id"), std::path::Path::new(&dir), name,
            ) {
                Ok(command) => ui::emit("RUN", &command),
                Err(e) => { ui::note(&e); return 2; }
            }
        }
        "session-mcp" => {
            let mut servers = crate::cache::read_cached("mcp");
            crate::archive::decorate(&mut servers);
            let choices = borrowable_servers(&servers, it.get("agent"));
            if choices.is_empty() {
                ui::note(
                    &format!("{} already owns every portable MCP server here", it.get("agent")),
                );
                return 0;
            }
            let Some(key) = pick_one(" resume with which MCP server ", &choices) else { return 130 };
            let Some(server) = servers.iter().find(|server| {
                format!("{}\u{1}{}", server.get("agent"), server.get("name")) == key
            }) else {
                return 2;
            };
            match crate::sources::sessions::resume_with_mcp_cmd(it.get("agent"), it.get("id"), server) {
                Ok(command) => ui::emit("RUN", &command),
                Err(e) => { ui::note(&e); return 2; }
            }
        }
        // The readable half of the export pair. The raw JSONL beside it is
        // the authoritative one and is what goes back to an agent; this is
        // what goes to a person.
        "session-export-md" => match crate::sources::sessions::export_transcript(it) {
            Ok(path) => {
                let shown = crate::paths::tilde(&path.to_string_lossy());
                match crate::openwith::reveal_now(&path.to_string_lossy()) {
                    Ok(()) => ui::note(&format!("exported to {shown}")),
                    Err(e) => ui::note(&format!("exported to {shown} ({e})")),
                }
            }
            Err(e) => { ui::note(&e); return 2; }
        },
        "session-export" => match crate::sources::sessions::export_raw(it) {
            Ok(path) => {
                if let Err(e) = crate::openwith::reveal_now(&path.to_string_lossy()) {
                    ui::note(&format!("exported to {} ({e})", crate::paths::tilde(&path.to_string_lossy())));
                } else {
                    ui::note(&format!("exported to {}", crate::paths::tilde(&path.to_string_lossy())));
                }
            }
            Err(e) => { ui::note(&e); return 2; }
        },
        "session-trash" => {
            if !ui::confirm(
                &format!("move {} to the Trash?", crate::width::dtrunc(&it.title, 40)),
                "Move conversation",
                "recoverable from Finder · the agent will no longer list it",
            ) {
                return 130;
            }
            match crate::sources::sessions::trash_session(it) {
                Ok(path) => ui::note(&format!("moved to {}", crate::paths::tilde(&path.to_string_lossy()))),
                Err(e) => { ui::note(&e); return 2; }
            }
        }
        "newsession" => ui::emit("RUN",
            &crate::sources::sessions::start_cmd(it.get("agent"),
                Some(it.get("cwd")).filter(|s| !s.is_empty()), None)),
        "ask" => {
            // Whatever is selected becomes the subject of a question.
            let subject = if it.get("path").is_empty() { it.cmd.clone() } else { it.get("path").into() };
            ui::emit("INSERT", &format!("claude {}", shq(&format!("about this: {subject}"))));
        }
        "agent-runs" => {
            let runs: Vec<Item> = crate::sources::running::live()
                .into_iter()
                .filter(|run| run.get("agent") == it.get("agent"))
                .collect();
            let choices: Vec<(String, String, String)> = runs.iter().map(|run| (
                run.get("run_id").to_string(),
                run.get("project").to_string(),
                format!("{} · {}", run.get("state"), run.get("addr")),
            )).collect();
            let Some(id) = pick_one(" running instances ", &choices) else { return 130 };
            let Some(run) = runs.iter().find(|run| run.get("run_id") == id) else { return 2 };
            if run.get("cwd").is_empty() {
                ui::note("that run has no readable project directory");
                return 2;
            }
            ui::emit("INSERT", &format!("cd {}", shq(run.get("cwd"))));
        }
        "agent-sessions" => {
            let mut sessions = crate::cache::read_cached("sessions-linked");
            if sessions.is_empty() {
                sessions = crate::cache::read_cached("sessions");
            }
            sessions.retain(|session| session.get("agent") == it.get("agent"));
            sessions.sort_by(|a, b| {
                b.get("ts").parse::<f64>().unwrap_or(0.0)
                    .partial_cmp(&a.get("ts").parse::<f64>().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let choices: Vec<(String, String, String)> = sessions.into_iter().take(100).map(|session| (
                session.cmd,
                session.title,
                session.fields.get(2).cloned().unwrap_or_default(),
            )).collect();
            let Some(command) = pick_one(" conversations ", &choices) else { return 130 };
            ui::emit("INSERT", &command);
        }
        "agent-doctor" => {
            let exe = std::env::current_exe().unwrap_or_else(|_| "prelude".into());
            return crate::runhere::run_cmd(&format!("{} doctor agents", shq(&exe.to_string_lossy())));
        }
        "favorite" | "unfavorite" => {
            let wanted = id == "favorite";
            match crate::favorites::set(it, wanted) {
                Ok(()) => ui::note(if wanted { "added to Favorites" } else { "removed from Favorites" }),
                Err(error) => { ui::note(&error); return 2; }
            }
        }
        "capability-archive" | "capability-unarchive" => {
            let archived = id == "capability-archive";
            match crate::archive::set(it, archived) {
                Ok(()) => ui::note(if archived {
                    if it.kind == Kind::Skill {
                        "archived · find it with skill:is:archived"
                    } else {
                        "archived · find it with mcp:is:archived"
                    }
                } else {
                    "restored to normal inventory"
                }),
                Err(error) => { ui::note(&error); return 2; }
            }
        }
        _ if id.starts_with("askagent:") => {
            let agent = &id[9..];
            let Some(prompt) = ui::prompt_line(&format!(" ask {agent} ")) else {
                return 130;
            };
            let Some(args) = crate::sources::sessions::ask_cmd(agent, &prompt) else {
                ui::note(&format!("don't know how to ask {agent}"));
                return 2;
            };
            let cmd = args.iter().map(|s| shq(s)).collect::<Vec<_>>().join(" ");
            return crate::runhere::run_cmd(&cmd);
        }
        _ if id.starts_with("resume:") => {
            match crate::sources::sessions::latest_for(&id[7..]) {
                Some(s) => ui::emit("RUN", &s.cmd),
                None => ui::note("no sessions recorded for that agent yet"),
            }
        }
        _ if id.starts_with("agentcfg:") => {
            match crate::sources::agents::config_for(&id[9..]) {
                Some(p) => {
                    if let Err(e) = crate::openwith::open_default_now(&p) {
                        ui::note(&e);
                        return 2;
                    }
                }
                None => ui::note("that agent has no settings file here"),
            }
        }
        _ if id.starts_with("run:") => {
            let agent = &id[4..];
            ui::emit("RUN", &format!("{agent} {}", shq(&it.cmd)));
        }
        // A skill merged across four agents is four directories, and an
        // "open all" that quietly opened the first would look like a failure.
        "open-copies" => {
            let copies = crate::sources::agents::copy_paths(it);
            let mut failed = Vec::new();
            for (agent, dir) in &copies {
                if let Err(e) = crate::openwith::open_now(dir, None) {
                    failed.push(format!("{agent}: {e}"));
                }
            }
            if !failed.is_empty() {
                ui::note(&failed.join(" · "));
                return 2;
            }
            ui::note(&format!("opened {} copies", copies.len()));
        }
        "cdrun" => ui::emit("INSERT", &format!("cd {}", shq(it.get("cwd")))),
        // The pid, and only the pid. This used to kill the run's pane instead
        // when it had one, on the reasoning that killing the process leaves a
        // dead pane behind — which was true, and was also this launcher
        // reaching into a terminal it does not own.
        "killrun" => {
            let pid = it.get("pid");
            if pid.is_empty() {
                ui::note("that run has no pid to end");
                return 2;
            }
            crate::exec::run(&["kill", pid], std::time::Duration::from_secs(2));
        }
        // Leave a line in its inbox, for `prelude inbox` to collect. Typing
        // straight into the agent's terminal was faster and is gone with the
        // pane that made it addressable; what remains works wherever the run
        // happens to be, including nowhere in particular.
        "say" => {
            let Some(line) = ui::prompt_line(&format!(" message {} ", it.get("agent"))) else {
                return 130;
            };
            match crate::bus::leave(it, &line) {
                Ok(to) => ui::note(&format!("left in {to}'s inbox")),
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            }
        }
        // The application half. `openit` is what Enter does, repeated here so
        // the panel states it; the other two are how you change it.
        "openit" => {
            if let Err(e) = crate::openwith::open_now(&target, None) {
                ui::note(&e);
                return 2;
            }
        }
        "reveal-finder" => {
            if let Err(e) = crate::openwith::reveal_now(&target) {
                ui::note(&e);
                return 2;
            }
        }
        "openwith" | "openalways" => {
            let Some(app) = crate::openwith::pick_app(short_name(&target)) else { return 130 };
            if id == "openalways" {
                let ext = crate::openwith::ext_of(&target);
                if let Err(e) = crate::openwith::remember(&ext, &app) {
                    ui::note(&format!("could not remember that: {e}"));
                    return 2;
                }
                let scope = if ext.is_empty() { "files like that".into() } else { format!(".{ext} files") };
                ui::note(&format!("{scope} now open in {app}"));
            }
            if let Err(e) = crate::openwith::open_now(&target, Some(&app)) {
                ui::note(&e);
                return 2;
            }
        }
        "mcpcompare" => return crate::runhere::show_text(
            &format!("MCP definitions · {}", it.get("name")),
            &mcp_matrix_lines(it),
        ),
        _ if id.starts_with("mcp-sync:") => {
            let target_agent = &id[9..];
            let _ = crate::runhere::show_text(
                &format!("MCP definitions · {}", it.get("name")),
                &mcp_matrix_lines(it),
            );
            if !ui::confirm(
                &format!("prepare replacement for {target_agent}?"),
                "Prepare command",
                "the command is inserted for review, not run",
            ) {
                return 130;
            }
            let definition = match crate::lend::resolve(it) {
                Ok(definition) => definition,
                Err(error) => { ui::note(&error); return 2; }
            };
            let install = match crate::lend::install_cmd(target_agent, &definition) {
                Ok(command) => command,
                Err(error) => { ui::note(&error); return 2; }
            };
            let remove = format!("{target_agent} mcp remove {}", shq(it.get("name")));
            ui::emit("INSERT", &format!("{remove} && {install}"));
        }
        _ if id.starts_with("diff:") || id.starts_with("sync:") => {
            let mut parts = id.split(':');
            let verb = parts.next().unwrap_or("");
            let from = parts.next().unwrap_or("");
            let to = parts.next().unwrap_or("");
            let copies = crate::capability::copies(it);
            let source = copies.iter().find(|copy| copy.agent == from);
            let target = copies.iter().find(|copy| copy.agent == to);
            let (Some(source), Some(target)) = (source, target) else {
                ui::note("those Skill copies are no longer present");
                return 2;
            };
            let expected = if verb == "sync" {
                let source_now = crate::capability::hash_skill(&source.agent, std::path::Path::new(&source.dir));
                let target_now = crate::capability::hash_skill(&target.agent, std::path::Path::new(&target.dir));
                if source_now.fingerprint.is_empty() || target_now.fingerprint.is_empty() {
                    ui::note("one of those Skill copies cannot be read completely");
                    return 2;
                }
                if source_now.sensitive_files > 0 {
                    ui::note("the source contains credential-like material; refusing to copy it");
                    return 2;
                }
                Some((source_now.fingerprint, target_now.fingerprint))
            } else {
                None
            };
            let command = format!("diff -ru {} {}", shq(&source.dir), shq(&target.dir));
            let _ = crate::runhere::run_cmd(&command);
            if verb == "diff" {
                return 0;
            }
            if !ui::confirm(
                &format!("replace {to}'s copy of {}?", it.get("name")),
                &format!("Replace {to} from {from}"),
                "the old copy moves to the Trash",
            ) {
                return 130;
            }
            let (source_hash, target_hash) = expected.unwrap_or_default();
            match crate::sources::agents::sync_skill(
                &source.dir, &target.dir, &source_hash, &target_hash,
            ) {
                Ok(trashed) => {
                    let _ = crate::cache::refresh_named("skill-hashes");
                    ui::note(
                        &format!("replaced {to}; old copy at {}", crate::paths::tilde(&trashed.to_string_lossy())),
                    );
                }
                Err(error) => { ui::note(&error); return 2; }
            }
        }
        // Borrow: build the one command that starts `agent` with someone
        // else's capability attached, and hand it over unrun. Nothing is
        // installed, nothing is written to either agent's directories, and
        // the loan ends when that process does.
        _ if id.starts_with("lend:") => {
            let agent = &id[5..];
            let cmd = match it.kind {
                Kind::Skill => {
                    let dir = it.get("dir");
                    let name = it.get("name");
                    if dir.is_empty() || name.is_empty() {
                        ui::note("that skill has no directory to lend");
                        return 2;
                    }
                    match crate::lend::skill_flags(agent, std::path::Path::new(dir), name) {
                        // No `/skill-name` prefilled: claude's synopsis takes
                        // a single `[prompt]`, so anything typed after the
                        // quoted one becomes a second positional argument and
                        // is silently dropped. Invoking the skill inside the
                        // agent, where the slash command has completion, is
                        // both safer and one keystroke away.
                        Ok(f) => crate::lend::borrow_cmd(agent, &f, None, None),
                        Err(e) => {
                            ui::note(&e);
                            return 2;
                        }
                    }
                }
                Kind::Mcp => {
                    let def = match crate::lend::resolve(it) {
                        Ok(d) => d,
                        Err(e) => {
                            ui::note(&e);
                            return 2;
                        }
                    };
                    match crate::lend::mcp_flags(agent, &def) {
                        Ok(f) => crate::lend::borrow_cmd(agent, &f, None, None),
                        Err(e) => {
                            ui::note(&e);
                            return 2;
                        }
                    }
                }
                _ => return 2,
            };
            ui::emit("INSERT", &cmd);
        }
        // Slot 9 for anything on disk. The path is named in the
        // confirmation, it goes to the Trash rather than being unlinked, and
        // `paths::trash` refuses $HOME, the root and the system directories
        // however the row got here.
        "trash" => {
            let p = first_nonempty(it, &["path", "file", "dir"]);
            if p.is_empty() {
                ui::note("nothing to delete on that row");
                return 2;
            }
            if !ui::confirm(
                &format!("move {} to the Trash?", short_name(&p)),
                &format!("Move {}", crate::paths::tilde(&p)),
                "recoverable from Finder",
            ) {
                return 130;
            }
            match crate::paths::trash(std::path::Path::new(&p)) {
                Ok(d) => ui::note(
                    &format!("moved to {}", crate::paths::tilde(&d.to_string_lossy())),
                ),
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            }
        }
        // The MCP verbs are the agent's own CLI, handed over rather than run.
        // Each of these edits the agent's configuration or opens a browser
        // for OAuth, and both are things to read before agreeing to.
        "mcp-tools" => return crate::runhere::show_text(
            &format!("MCP tools · {} · {}", it.get("agent"), it.get("name")),
            &mcp_tool_lines(it),
        ),
        "mcprefresh" => {
            if !crate::cache::refresh_named("mcp") {
                ui::note("could not refresh MCP status");
                return 2;
            }
            let refreshed = crate::cache::read_cached("mcp");
            let current = refreshed.iter().find(|server| {
                server.get("agent") == it.get("agent") && server.get("name") == it.get("name")
            });
            match current {
                Some(server) => ui::note(
                    &format!("{} status: {}", server.get("agent"), server.get("health")),
                ),
                None => ui::note("the owner no longer reports that MCP server"),
            }
        }
        "mcp-tools-refresh" => {
            if !crate::cache::refresh_named("mcp-tools") {
                ui::note("could not refresh MCP tools");
                return 2;
            }
            let refreshed = crate::cache::read_cached("mcp-tools");
            let current = refreshed.iter().find(|server| {
                server.get("agent") == it.get("agent") && server.get("name") == it.get("name")
            });
            match current {
                Some(server) => {
                    let count = serde_json::from_str::<Vec<crate::mcp_tools::Tool>>(server.get("tools"))
                        .map(|tools| tools.len()).unwrap_or(0);
                    ui::note(&format!("tool inventory: {} · {count} tools", server.get("status")));
                }
                None => ui::note("no tool inventory was produced for that server"),
            }
        }
        "mcptools" => {
            let c = format!("{} mcp get {}", it.get("agent"), shq(it.get("name")));
            return crate::runhere::run_cmd(&c);
        }
        "mcplogin" => ui::emit(
            "INSERT",
            &format!("{} mcp login {}", it.get("agent"), shq(it.get("name"))),
        ),
        "mcpremove" => ui::emit(
            "INSERT",
            &format!("{} mcp remove {}", it.get("agent"), shq(it.get("name"))),
        ),
        _ if id.starts_with("install:") => {
            let target = &id[8..];
            match crate::lend::resolve(it).and_then(|d| crate::lend::install_cmd(target, &d)) {
                Ok(c) => ui::emit("INSERT", c.as_str()),
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            }
        }
        // The only destructive thing here. It names the agent and the path
        // before asking, moves the directory to the Trash rather than
        // removing it, and says where it went — so the answer to "that was
        // the wrong one" is Finder, not a backup.
        _ if id.starts_with("rm:") => {
            let agent = &id[3..];
            let copies = crate::sources::agents::copies_of(it);
            let Some((_, dir)) = copies.iter().find(|(a, _)| a == agent) else {
                ui::note(&format!("{agent} has no copy of that"));
                return 2;
            };
            if !ui::confirm(
                &format!("delete {} from {agent}?", it.get("name")),
                &format!("Delete {}", crate::paths::tilde(dir)),
                "recoverable from the Trash",
            ) {
                return 130;
            }
            match crate::sources::agents::delete_skill(dir) {
                Ok(p) => {
                    let _ = crate::cache::refresh_named("skill-hashes");
                    ui::note(
                        &format!("{} deleted — now in {}", it.get("name"), crate::paths::tilde(&p.to_string_lossy())),
                    )
                },
                Err(e) => {
                    ui::note(&e);
                    return 2;
                }
            }
        }
        _ if id.starts_with("cp:") => {
            let want = &id[3..];
            let targets: Vec<String> = if want == "*" {
                it.get("missing").split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()
            } else {
                vec![want.to_string()]
            };
            let name = it.get("name");
            let dir = it.get("dir");
            if dir.is_empty() || name.is_empty() {
                ui::note("nothing to copy from");
                return 2;
            }
            let mut changed = false;
            for agent in targets {
                match crate::sources::agents::copy_skill(dir, &agent, name) {
                    Ok(p) => { changed = true; eprintln!("copied {name} -> {p}"); }
                    Err(e) => ui::note(&e.to_string()),
                }
            }
            if changed {
                let _ = crate::cache::refresh_named("skill-hashes");
            }
        }
        _ => return 130,
    }
    0
}

/// A RUN travels back to whatever started us, which is the only route there
/// has ever needed to be. It used to fork here instead when the launcher was
/// a popup over somebody else's conversation, because emitting would have
/// typed the command into the chat; there is no such surface now.
fn run_or_emit(cmd: &str) {
    ui::emit("RUN", cmd);
}

fn copy_text(it: &Item) -> String {
    if it.kind == Kind::Dir {
        return crate::defaults::text_for(it, crate::defaults::Text::AbsolutePath);
    }
    let by_kind = match it.kind {
        Kind::Port | Kind::Proc => it.get("pid"),
        Kind::Ssh => it.get("host"),
        Kind::Container => it.get("name"),
        Kind::Mcp => it.get("name"),
        Kind::Link => it.get("url"),
        Kind::File | Kind::Find | Kind::App => it.get("path"),
        _ => "",
    };
    if by_kind.is_empty() { it.cmd.clone() } else { by_kind.to_string() }
}

fn page(text: &str) {
    let mut cmd = std::process::Command::new("less");
    cmd.arg("-R").stdin(std::process::Stdio::piped());
    if let Ok(mut child) = cmd.spawn() {
        if let Some(mut si) = child.stdin.take() {
            use std::io::Write;
            let _ = si.write_all(text.as_bytes());
        }
        let _ = child.wait();
    } else {
        print!("{text}");
    }
}

/// Details are a view, not a terminal action: page them and return to ^K.
fn show_details(it: &Item) {
    page(&crate::preview::text(it));
}

/// Skill instructions are long; page them rather than truncating.
fn show_description(it: &Item) {
    page(&format!(
        "{}  [{}]\n\n{}\n\n{}\n",
        it.cmd,
        it.get("agent"),
        it.get("desc"),
        it.get("file")
    ));
}
