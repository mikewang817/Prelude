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

pub fn actions_for(it: &Item) -> Vec<Act> {
    actions_for_host(it, crate::defaults::Host::Shell)
}

/// The agents a per-agent verb can be pointed at, as the ids `apply` takes.
///
/// One row can stand for several things — a skill merged across four agents
/// is four directories — and the panel used to enumerate them: `Copy it to
/// codex`, `Copy it to pi`, `Copy it to opencode`, `Copy it to all missing`,
/// plus a `Delete …` each. Seven rows that are really three verbs and a
/// choice of agent. Raycast's answer is a submenu, and it is the right one:
/// the verb is the decision, the agent is a parameter of it.
pub fn agent_options(it: &Item, verb: &str) -> Vec<(String, String)> {
    let missing: Vec<&str> = it.get("missing").split(',').filter(|s| !s.is_empty()).collect();
    let has: Vec<&str> = it
        .get("agent")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "shared")
        .collect();
    match verb {
        "run" => has.iter().map(|n| (format!("run:{n}"), (*n).to_string())).collect(),
        "lend" => match it.kind {
            // A skill can only be borrowed by an agent that lacks it.
            Kind::Skill => missing
                .iter()
                .filter(|n| **n != "shared" && crate::lend::can_borrow_skill(n))
                .filter(|n| crate::sources::sessions::installed().contains(*n))
                .map(|n| (format!("lend:{n}"), (*n).to_string()))
                .collect(),
            // An MCP server can go to any other agent that has a flag for it.
            _ => crate::sources::sessions::installed()
                .into_iter()
                .filter(|n| *n != it.get("agent") && crate::lend::can_borrow_mcp(n))
                .map(|n| (format!("lend:{n}"), n.to_string()))
                .collect(),
        },
        "cp" => {
            let mut v: Vec<(String, String)> =
                missing.iter().map(|n| (format!("cp:{n}"), (*n).to_string())).collect();
            if v.len() > 1 {
                v.push(("cp:*".into(), format!("all {} of them", v.len())));
            }
            v
        }
        // Only agents that actually have a copy to delete.
        "rm" => crate::sources::agents::copies_of(it)
            .into_iter()
            .map(|(agent, dir)| (format!("rm:{agent}"), format!("{agent} · {}", crate::paths::tilde(&dir))))
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
        1 => v.push((leak(opts[0].0.clone()), one.replace("{}", &opts[0].1), sub.to_string())),
        n => v.push((
            leak(format!("menu:{verb}")),
            many.to_string(),
            if sub.is_empty() { format!("{n} agents") } else { sub.to_string() },
        )),
    }
}

pub fn actions_for_host(it: &Item, host: crate::defaults::Host) -> Vec<Act> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let mut acts = match it.kind {
        Kind::Session => {
            // No "Resume this session": that is Enter, word for word.
            let mut v = vec![a("run", "Resume it now", it.get("agent"))];
            if !it.get("cwd").is_empty() {
                v.push(a("cdsession", "cd to where it ran", crate::paths::tilde(it.get("cwd"))));
                v.push(a("newsession", "Start a fresh session there", it.get("agent")));
            }
            v.push(a("copy", "Copy the session id", it.get("id")));
            v
        }
        // An agent CLI. Enter puts its name on the prompt, because that is
        // where you add `--resume`, a model, or an opening question. So the
        // panel is the three of those you reach for by name rather than by
        // typing — and nothing else. "Copy its name" is two letters you can
        // type faster than you can open this panel, and "Ask an agent about
        // this" would hand claude the word "pi".
        Kind::Agent => {
            let n = it.get("agent").to_string();
            let mut v = Vec::new();
            // The single most common thing anyone does with an agent they
            // have used before, and the row already says how many sessions
            // there are. Finding the newest by hand means `s:`, reading
            // dates, and copying a uuid.
            if let Some(s) = crate::sources::sessions::latest_for(&n) {
                v.push((
                    leak(format!("resume:{n}")),
                    "Resume its most recent session".to_string(),
                    format!("{} · {}", crate::width::dtrunc(&s.title, 40), s.fields.get(2).cloned().unwrap_or_default()),
                ));
            }
            v.push((leak(format!("askagent:{n}")), format!("Ask {n} something"), "without leaving here".into()));
            if let Some(p) = crate::sources::agents::config_for(&n) {
                v.push((
                    leak(format!("agentcfg:{n}")),
                    "Open its settings".to_string(),
                    crate::paths::tilde(&p),
                ));
            }
            v
        }
        // A running agent. At eighty of them the panel is the difference
        // between a fleet and a mess: go to it, answer it without leaving
        // here, or end it.
        Kind::Run => {
            let mut v = Vec::new();
            let addr = it.get("addr");
            // No "go to it" row: Enter already is that, and the panel
            // states Enter's action at the top.
            if !it.get("pane").is_empty() {
                v.push(a("say", "Send it a line, without going there", "typed into its pane"));
                v.push(a("zoom", "Go to it, zoomed full-screen", addr));
            }
            v.push(a("cdrun", "cd to its project", crate::paths::tilde(it.get("cwd"))));
            v.push(a("killrun", "End it", format!("{} · pid {}", it.get("agent"), it.get("pid"))));
            v.push(a("copy", "Copy its address", addr));
            v
        }
        // A question an agent is blocked on. Enter already answers it, so
        // this is everything else you might want first: see the conversation
        // it came out of, go there, or decline so it stops waiting.
        Kind::Msg => {
            let mut v = Vec::new();
            if !it.get("pane").is_empty() {
                v.push(a("zoom", "Go to it, zoomed full-screen", it.get("pane")));
            }
            v.push(a("msg:no", "Answer \"no\"", "unblocks it immediately"));
            v.push(a("msg:go", "Answer \"go ahead\"", "unblocks it immediately"));
            v.push(a("cdrun", "cd to its project", crate::paths::tilde(it.get("cwd"))));
            v.push(a("copy", "Copy the question", ""));
            v
        }
        Kind::Config => open_actions(it.get("path"), &editor),
        Kind::Port => vec![
            a("run", "Kill it now", format!("{} · pid {}", it.get("proc"), it.get("pid"))),
            a("copy", "Copy the pid", it.get("pid")),
        ],
        Kind::Proc => vec![
            a("run", "Kill it now", format!("{} · {}% CPU", it.get("name"), it.get("cpu"))),
            a("copy", "Copy the pid", it.get("pid")),
        ],
        Kind::Container => vec![
            a("logs", "Follow its logs", format!("docker logs -f {}", it.get("name"))),
            a("stop", "Stop it", format!("docker stop {}", it.get("name"))),
            a("restart", "Restart it", format!("docker restart {}", it.get("name"))),
            a("copy", "Copy name", it.get("name")),
        ],
        Kind::Skill => {
            let target = first_nonempty(it, &["file", "dir"]);
            let mut v = Vec::new();
            with_options(&mut v, it, "run", "Run it with {}", "Run it with…", "");
            // Borrowing comes before copying: it is the lighter of the two,
            // and the one that is nearly always what was meant. Copying puts
            // a second copy of the skill on disk, to be maintained forever;
            // borrowing lasts exactly one run and leaves nothing behind.
            with_options(&mut v, it, "lend", "Use it in {}, just this run",
                         "Use it in…, just this run", "nothing is installed");
            with_options(&mut v, it, "cp", "Copy it to {}", "Copy it to…", "");
            if !it.get("desc").is_empty() {
                v.push(a("desc", "Show full description", ""));
            }
            v.push(a("open", "Open in editor", &target));
            v.push(a("reveal", "cd to its folder", parent_of(&target)));
            // Last, and one entry per agent that actually has a copy. A
            // skill merged across four agents is four separate decisions —
            // "delete it" would otherwise mean something different depending
            // on a number the row only hints at.
            with_options(&mut v, it, "rm", "Delete {}'s copy…", "Delete a copy…",
                         "to the Trash, after confirming");
            v
        }
        Kind::Mcp => {
            let target = first_nonempty(it, &["file", "dir", "config"]);
            let owner = it.get("agent");
            let mut v = Vec::new();
            // Only offered to agents that can take one for a single run, and
            // never back to the one that already has it. Whether *this*
            // particular server can be lent at all takes a subprocess to
            // find out, so that answer arrives when the action runs rather
            // than being guessed at here.
            with_options(&mut v, it, "lend", "Lend it to {} for one run",
                         "Lend it for one run…", &format!("from {owner}"));
            v.push(a("insert", "Insert the lookup command", &it.cmd));
            v.push(a("open", "Open in editor", &target));
            v.push(a("reveal", "cd to its folder", parent_of(&target)));
            v.push(a("copy", "Copy name", ""));
            v
        }
        Kind::File | Kind::Find => open_actions(it.get("path"), &editor),
        // Enter pastes it and the secondary puts it back on the clipboard,
        // both stated above. What is left is the thing you cannot get any
        // other way: reading it in a language you have.
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
        Kind::Ssh => vec![
            a("editssh", "Edit ~/.ssh/config", ""),
            a("copy", "Copy host", it.get("host")),
        ],
        // No "Launch it now": Enter is that, and the panel states it above.
        Kind::App => vec![
            a("insert", "Insert the open command", &it.cmd),
            a("reveal", "cd to its folder", it.get("path")),
            a("copy", "Copy its path", it.get("path")),
        ],
        Kind::Sys => vec![
            a("copy", "Copy the command", ""),
        ],
        // No "Open in browser": that is Enter, stated above. The insert is
        // the `open …` command, which the secondary's bare URL is not.
        Kind::Link => vec![
            a("insert", "Insert the open command", &it.cmd),
            a("copy", "Copy the URL", it.get("url")),
        ],
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
    // Enter's action leads, and names its key — Raycast's convention, and
    // for Raycast's reason: the panel is searchable, fuzzy matching flattens
    // it into one list, and a primary that is not in the list cannot be
    // found by typing its name. The panel has to be the complete inventory
    // of what is possible here, not the inventory-minus-one.
    //
    // The secondary follows, and names no key because it has none — that is
    // the whole reason this row exists. It says what it does rather than
    // announcing itself as "the other one", which was an internal word for
    // it leaking onto the screen.
    acts.insert(0, a("default", crate::defaults::describe(it, host), "Enter"));
    if let Some(label) = crate::defaults::describe_secondary(it, host) {
        acts.insert(1, a("secondary", label, ""));
    }
    // Everything the removed shortcuts used to do stays reachable here — but
    // only where it means anything. Offering to run a translation, or to
    // paint an agent's TUI into a preview pane, is not a fallback.
    // `docs/ACTIONS.md`, R3 and R5.
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
        crate::defaults::on_enter(it, host) == Default_::Act(v)
            || crate::defaults::on_secondary(it, host) == Some(Default_::Act(v))
    };
    // Three verbs all end in `emit("RUN", cmd)`, so any of them above means
    // the generic runner is the same keystroke with a duller label.
    let runs_it = already(Verb::RunInShell) || already(Verb::Launch) || already(Verb::OpenUrl);
    if it.kind.is_command_line()
        && !it.kind.is_interactive()
        && !generic_run_would_kill
        && !already(Verb::RunHere)
        && !acts.iter().any(|(id, ..)| *id == "runhere")
    {
        acts.push(a("runhere", "Run here, inside this window", ""));
    }
    if it.kind.is_command_line() && !runs_it && !acts.iter().any(|(id, ..)| *id == "run") {
        acts.push(a("run", "Run in the shell below", ""));
    }
    // `copyabs` already copies the path, and for a file that is exactly what
    // `copy` copies too — the same action twice, worded differently. Nor is
    // there anything to copy off an agent row: `pi` is two letters you can
    // type faster than you can open this panel.
    if it.kind != Kind::Agent
        && !already(Verb::CopyResult)
        && !acts.iter().any(|(id, ..)| *id == "copy" || *id == "copyabs")
    {
        acts.push(a("copy", "Copy to clipboard", ""));
    }
    if it.kind.worth_asking_about() {
        acts.push(a("ask", "Ask an agent about this", "hands it to claude"));
    }
    if let Some(cwd) = &it.cwd {
        acts.push(a("cd", "Go to project folder", cwd.clone()));
    }
    // No "Insert path without cd" for a folder: that is precisely what the
    // secondary hands you, and it is stated two rows above.
    // The order is the five questions, in the order a person asks them.
    // Stable, so each kind's own sequencing survives inside its group.
    acts.sort_by_key(|(id, ..)| group(it.kind, id));
    acts
}

/// Is this entry one you cannot take back with another keystroke?
///
/// Two consequences, both Raycast's: it is drawn in red, and — where the
/// thing genuinely cannot be reverted — it asks first. Stopping a container
/// is red but not confirmed, because `docker start` exists; killing a
/// process is both, because nothing brings it back.
pub fn is_destructive(kind: Kind, id: &str) -> bool {
    group(kind, id) == 5
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

/// Which of the five questions in `docs/ACTIONS.md` an entry answers.
///
/// Kept as one function over ids rather than as structure in each kind's
/// list, so the ordering is a property of the panel rather than of
/// twenty-five independent decisions that agreed for a while.
///
/// It needs the kind because one id is genuinely two verbs: `run` means
/// "start it" nearly everywhere and "kill it now" on a port or a process.
fn group(kind: Kind, id: &str) -> u8 {
    const ACT: u8 = 2;
    const TAKE: u8 = 3;
    const GO: u8 = 4;
    const DESTROY: u8 = 5;
    match id {
        "default" => 0,
        "secondary" => 1,
        // Irreversible, so last however the kind happened to list it.
        "killrun" | "stop" => DESTROY,
        _ if id.starts_with("rm:") || id == "menu:rm" => DESTROY,
        "run" if matches!(kind, Kind::Port | Kind::Proc) => DESTROY,
        // Text out of the row.
        "insert" | "copy" | "copyabs" | "here" | "desc" | "inspect" | "tr_src" => TAKE,
        // Where it lives.
        "reveal" | "reveal-finder" | "cd" | "cdrun" | "cdsession" | "zoom" | "jump" | "editsnips"
        | "editssh" => GO,
        // Everything else is another way to act on it, including the two
        // tenses of "not like that": `openwith` and `openalways`.
        _ => ACT,
    }
}

/// Everything you might want to do with a file, in the order you want it.
///
/// This is the half of the launcher that behaves like a Finder rather than a
/// shell: which application opens this, and should that stick. The chosen
/// application is named rather than implied, so the first row says what
/// Enter already does instead of leaving you to find out.
fn open_actions(path: &str, editor: &str) -> Vec<Act> {
    let chosen = crate::openwith::chosen_for(path);
    let ext = crate::openwith::ext_of(path);
    let scope = if ext.is_empty() { "files like this".to_string() } else { format!(".{ext} files") };
    vec![
        (leak("openwith".into()), "Open with…".into(), match &chosen {
            Some(app) => format!("currently {app}"),
            None => "currently the system default".into(),
        }),
        (leak("openalways".into()), format!("Always open {scope} with…"), "makes it stick".into()),
        a("reveal-finder", "Reveal in Finder", parent_of(path)),
        a("open", "Open in $EDITOR", editor),
        a("copyabs", "Copy absolute path", path),
        a("reveal", "cd to its folder", parent_of(path)),
    ]
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

pub fn panel(it: &Item, paste_target: Option<String>) -> i32 {
    // The panel reflects where you opened it from, exactly as the footer
    // does — in a popup over an agent, Enter means something else and the
    // header has to say so.
    let host = if paste_target.is_some() {
        crate::defaults::Host::Agent
    } else {
        crate::defaults::Host::Shell
    };
    let acts = actions_for_host(it, host);
    let feed: String = acts
        .iter()
        .map(|(id, label, sub)| {
            let tail = if sub.is_empty() { String::new() } else { format!("{DIM}· {sub}{RESET}") };
            // Red is the whole of what Raycast's "Danger zone" section title
            // achieves that we can have: fzf has no unselectable separator
            // row, but it does render colour, and the point of the title was
            // never the word — it was that these rows look different from
            // the ones above them.
            let label = if is_destructive(it.kind, id) {
                format!("{RED}{label:<28}{RESET}")
            } else {
                format!("{label:<28}")
            };
            format!("{label}{tail}{SEP}{id}\n")
        })
        .collect();

    let short = crate::width::dtrunc(&crate::width::flatten(&it.cmd), 56);
    // The panel's payload is a bare action id, not JSON, so take the raw
    // selection rather than trying to parse an Item out of it.
    match ui::pick_raw(
        feed.trim_end(),
        &format!(" {short} "),
        "⌘ ",
        "Run  Enter   ·   Back  Esc",
        "",
    ) {
        Some(id) => apply(&id, it, &paste_target),
        None => 130,
    }
}

pub fn apply(id: &str, it: &Item, paste: &Option<String>) -> i32 {
    // A verb that needed an agent: ask which, then carry on as if that row
    // had been chosen directly.
    if let Some(verb) = id.strip_prefix("menu:") {
        let opts = agent_options(it, verb);
        let feed: String = opts
            .iter()
            .map(|(oid, label)| format!("{:<28}{SEP}{oid}\n", label))
            .collect();
        let Some(chosen) = ui::pick_raw(
            feed.trim_end(),
            &format!(" {} ", it.get("name")),
            "⌘ ",
            "Choose  Enter   ·   Back  Esc",
            "",
        ) else {
            return 130;
        };
        return apply(&chosen, it, paste);
    }
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
            ui::emit("INSERT", &cmd, paste);
        }
        "run" => {
            crate::frecency::bump(&it.cmd);
            ui::emit("RUN", &it.cmd, paste);
        }
        "runhere" => return crate::runhere::run_item(it),
        "copy" => ui::copy(&copy_text(it)),
        "copyabs" => ui::copy(it.get("path")),
        "cd" => ui::emit("INSERT", &format!("cd {}", shq(it.cwd.as_deref().unwrap_or(""))), paste),
        "here" => ui::emit("INSERT", it.cmd.split_once(' ').map(|(_, r)| r).unwrap_or(&it.cmd), paste),
        "inspect" => {
            let c = if it.kind == Kind::Proc {
                format!("ps -p {} -o command=", it.get("pid"))
            } else {
                format!("lsof -nP -iTCP:{} -sTCP:LISTEN", it.get("port"))
            };
            ui::emit("INSERT", &c, paste);
        }
        "logs" => ui::emit("INSERT", &format!("docker logs -f {}", shq(it.get("name"))), paste),
        "stop" => ui::emit("INSERT", &format!("docker stop {}", shq(it.get("name"))), paste),
        "restart" => ui::emit("INSERT", &format!("docker restart {}", shq(it.get("name"))), paste),
        "open" if !target.is_empty() => ui::emit("INSERT", &format!("{editor} {}", shq(&target)), paste),
        "reveal" if !target.is_empty() => ui::emit("INSERT", &format!("cd {}", shq(&parent_of(&target))), paste),
        "desc" => show_description(it),
        "editsnips" => ui::emit("INSERT", &format!("{editor} {}", shq(&crate::paths::config().join("snippets.toml").to_string_lossy())), paste),
        "editssh" => ui::emit("INSERT", &format!("{editor} ~/.ssh/config"), paste),
        "tr_en" | "tr_zh" => {
            let text = if it.get("full").is_empty() { it.cmd.clone() } else { it.get("full").to_string() };
            let lang = if id == "tr_en" { "en" } else { "zh-Hans" };
            match crate::compute::translate(&text, lang) {
                Ok(v) => {
                    ui::copy(&v);
                    ui::emit("INSERT", &v, paste);
                }
                Err(e) => {
                    ui::note(&e.to_string(), paste);
                    return 2;
                }
            }
        }
        "tr_src" => ui::copy(it.get("source")),
        "default" => return ui::apply_default(it, paste),
        "secondary" => {
            let host = if paste.is_some() { crate::defaults::Host::Agent }
                       else { crate::defaults::Host::Shell };
            if let Some(d) = crate::defaults::on_secondary(it, host) {
                return ui::perform(it, d, paste);
            }
        }
        // The two answers worth a keystroke of their own. Everything an agent
        // stops to ask is, at bottom, "may I" — and being able to say yes or
        // no without typing is what makes answering ten of them bearable.
        _ if id.starts_with("msg:") => {
            let text = if &id[4..] == "no" { "no" } else { "go ahead" };
            return crate::bus::answer(it.get("id"), text);
        }
        "cdsession" => ui::emit("INSERT", &format!("cd {}", shq(it.get("cwd"))), paste),
        "newsession" => ui::emit("INSERT",
            &crate::sources::sessions::start_cmd(it.get("agent"),
                Some(it.get("cwd")).filter(|s| !s.is_empty()), None), paste),
        "ask" => {
            // Whatever is selected becomes the subject of a question.
            let subject = if it.get("path").is_empty() { it.cmd.clone() } else { it.get("path").into() };
            ui::emit("INSERT", &format!("claude {}", shq(&format!("about this: {subject}"))), paste);
        }
        _ if id.starts_with("askagent:") => {
            ui::emit("INSERT", &format!("@{} ", &id[9..]), paste);
        }
        _ if id.starts_with("resume:") => {
            match crate::sources::sessions::latest_for(&id[7..]) {
                Some(s) => ui::emit("INSERT", &s.cmd, paste),
                None => ui::note("no sessions recorded for that agent yet", paste),
            }
        }
        _ if id.starts_with("agentcfg:") => {
            match crate::sources::agents::config_for(&id[9..]) {
                Some(p) => ui::emit("RUN", &crate::openwith::open_default(&p), paste),
                None => ui::note("that agent has no settings file here", paste),
            }
        }
        _ if id.starts_with("run:") => {
            let agent = &id[4..];
            ui::emit("INSERT", &format!("{agent} {}", shq(&it.cmd)), paste);
        }
        "jump" => return ui::act_jump(it, paste, false),
        "zoom" => return ui::act_jump(it, paste, true),
        "cdrun" => ui::emit("INSERT", &format!("cd {}", shq(it.get("cwd"))), paste),
        // Kill the pane rather than the pid when there is one: killing the
        // process leaves a dead pane behind, which is the mess this is
        // meant to clear up.
        "killrun" => {
            let d = std::time::Duration::from_secs(2);
            let pane = it.get("pane");
            if pane.is_empty() {
                crate::exec::run(&["kill", it.get("pid")], d);
            } else {
                crate::exec::run(&["tmux", "kill-pane", "-t", pane], d);
            }
        }
        // Answering a stuck agent is the single most common thing you want
        // at scale, and switching to it to type one line is most of the
        // cost. `-l` sends the text literally; the separate Enter submits.
        "say" => {
            let pane = it.get("pane");
            if pane.is_empty() {
                ui::note("nothing to type into — that one is not in tmux", paste);
                return 2;
            }
            let Some(line) = ui::prompt_line(&format!(" say to {} ", it.get("agent"))) else {
                return 130;
            };
            let d = std::time::Duration::from_secs(2);
            crate::exec::run(&["tmux", "send-keys", "-t", pane, "-l", &line], d);
            crate::exec::run(&["tmux", "send-keys", "-t", pane, "Enter"], d);
            ui::note(&format!("sent to {}", it.get("addr")), paste);
        }
        // The application half. `openit` is what Enter does, repeated here so
        // the panel states it; the other two are how you change it.
        "openit" => ui::emit("RUN", &crate::openwith::open_default(&target), paste),
        "reveal-finder" => ui::emit("RUN", &format!("open -R {}", shq(&target)), paste),
        "openwith" | "openalways" => {
            let Some(app) = crate::openwith::pick_app(short_name(&target)) else { return 130 };
            if id == "openalways" {
                let ext = crate::openwith::ext_of(&target);
                if let Err(e) = crate::openwith::remember(&ext, &app) {
                    ui::note(&format!("could not remember that: {e}"), paste);
                    return 2;
                }
                let scope = if ext.is_empty() { "files like that".into() } else { format!(".{ext} files") };
                ui::note(&format!("{scope} now open in {app}"), paste);
            }
            ui::emit("RUN", &crate::openwith::open_cmd(&target, Some(&app)), paste);
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
                        ui::note("that skill has no directory to lend", paste);
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
                            ui::note(&e, paste);
                            return 2;
                        }
                    }
                }
                Kind::Mcp => {
                    let def = match crate::lend::resolve(it) {
                        Ok(d) => d,
                        Err(e) => {
                            ui::note(&e, paste);
                            return 2;
                        }
                    };
                    match crate::lend::mcp_flags(agent, &def) {
                        Ok(f) => crate::lend::borrow_cmd(agent, &f, None, None),
                        Err(e) => {
                            ui::note(&e, paste);
                            return 2;
                        }
                    }
                }
                _ => return 2,
            };
            ui::emit("INSERT", &cmd, paste);
        }
        // The only destructive thing here. It names the agent and the path
        // before asking, moves the directory to the Trash rather than
        // removing it, and says where it went — so the answer to "that was
        // the wrong one" is Finder, not a backup.
        _ if id.starts_with("rm:") => {
            let agent = &id[3..];
            let copies = crate::sources::agents::copies_of(it);
            let Some((_, dir)) = copies.iter().find(|(a, _)| a == agent) else {
                ui::note(&format!("{agent} has no copy of that"), paste);
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
                Ok(p) => ui::note(
                    &format!("{} deleted — now in {}", it.get("name"), crate::paths::tilde(&p.to_string_lossy())),
                    paste,
                ),
                Err(e) => {
                    ui::note(&e, paste);
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
                ui::note("nothing to copy from", paste);
                return 2;
            }
            for agent in targets {
                match crate::sources::agents::copy_skill(dir, &agent, name) {
                    Ok(p) => eprintln!("copied {name} -> {p}"),
                    Err(e) => ui::note(&e.to_string(), paste),
                }
            }
        }
        _ => return 130,
    }
    0
}

fn copy_text(it: &Item) -> String {
    let by_kind = match it.kind {
        Kind::Port | Kind::Proc => it.get("pid"),
        Kind::Ssh => it.get("host"),
        Kind::Container => it.get("name"),
        Kind::Mcp => it.get("name"),
        Kind::Link => it.get("url"),
        Kind::File | Kind::Find => it.get("path"),
        _ => "",
    };
    if by_kind.is_empty() { it.cmd.clone() } else { by_kind.to_string() }
}

/// Skill descriptions are long; page them rather than truncating.
fn show_description(it: &Item) {
    let text = format!(
        "{}  [{}]\n\n{}\n\n{}\n",
        it.cmd,
        it.get("agent"),
        it.get("desc"),
        it.get("file")
    );
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
