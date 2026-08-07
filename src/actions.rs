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

pub fn actions_for_host(it: &Item, host: crate::defaults::Host) -> Vec<Act> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let mut acts = match it.kind {
        Kind::Session => {
            let mut v = vec![
                a("insert", "Resume this session", &it.cmd),
                a("run", "Resume it now", it.get("agent")),
            ];
            if !it.get("cwd").is_empty() {
                v.push(a("cdsession", "cd to where it ran", crate::paths::tilde(it.get("cwd"))));
                v.push(a("newsession", "Start a fresh session there", it.get("agent")));
            }
            v.push(a("copy", "Copy the session id", it.get("id")));
            v
        }
        Kind::Agent => {
            let n = it.get("agent").to_string();
            // Enter already starts it, so this is the "not quite like that"
            // list: put the command on the prompt so a flag or a model or an
            // opening prompt can be added before it runs.
            vec![
                (leak(format!("askagent:{n}")), format!("Ask {n} something"), String::new()),
                a("copy", "Copy its name", &n),
            ]
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
            a("insert", &format!("Insert: kill whatever is on :{}", it.get("port")), it.get("proc")),
            a("run", "Kill it now", format!("{} · pid {}", it.get("proc"), it.get("pid"))),
            a("inspect", "Show what's using it", format!("lsof -nP -iTCP:{}", it.get("port"))),
            a("copy", "Copy the pid", it.get("pid")),
        ],
        Kind::Proc => vec![
            a("insert", &format!("Insert: kill {}", it.get("name")), format!("pid {}", it.get("pid"))),
            a("run", "Kill it now", format!("{} · {}% CPU", it.get("name"), it.get("cpu"))),
            a("inspect", "Show its full command", it.get("cmd").chars().take(50).collect::<String>()),
            a("copy", "Copy the pid", it.get("pid")),
        ],
        Kind::Container => vec![
            a("insert", "Shell into it", format!("docker exec -it {} sh", it.get("name"))),
            a("logs", "Follow its logs", format!("docker logs -f {}", it.get("name"))),
            a("stop", "Stop it", format!("docker stop {}", it.get("name"))),
            a("restart", "Restart it", format!("docker restart {}", it.get("name"))),
            a("copy", "Copy name", it.get("name")),
        ],
        Kind::Skill => {
            let target = first_nonempty(it, &["file", "dir"]);
            let mut v = Vec::new();
            for agent in it.get("agent").split(',').map(str::trim).filter(|s| !s.is_empty() && *s != "shared") {
                v.push((leak(format!("run:{agent}")), format!("Run it with {agent}"), it.cmd.clone()));
            }
            let missing: Vec<&str> = it.get("missing").split(',').filter(|s| !s.is_empty()).collect();
            // Borrowing comes before copying: it is the lighter of the two,
            // and the one that is nearly always what was meant. Copying puts
            // a second copy of the skill on disk, to be maintained forever;
            // borrowing lasts exactly one run and leaves nothing behind.
            for agent in &missing {
                if *agent != "shared"
                    && crate::lend::can_borrow_skill(agent)
                    && crate::sources::sessions::installed().contains(agent)
                {
                    v.push((
                        leak(format!("lend:{agent}")),
                        format!("Use it in {agent}, just this run"),
                        "nothing is installed".into(),
                    ));
                }
            }
            for agent in &missing {
                v.push((leak(format!("cp:{agent}")), format!("Copy it to {agent}"), String::new()));
            }
            if missing.len() > 1 {
                v.push(a("cp:*", "Copy it to all missing agents", missing.join(", ")));
            }
            v.push(a("insert", "Insert its name", &it.cmd));
            if !it.get("desc").is_empty() {
                v.push(a("desc", "Show full description", ""));
            }
            v.push(a("open", "Open in editor", &target));
            v.push(a("reveal", "cd to its folder", parent_of(&target)));
            // Last, and one entry per agent that actually has a copy. A
            // skill merged across four agents is four separate decisions —
            // "delete it" would otherwise mean something different depending
            // on a number the row only hints at.
            let copies = crate::sources::agents::copies_of(it);
            for (agent, _) in &copies {
                v.push((
                    leak(format!("rm:{agent}")),
                    format!("Delete {agent}'s copy…"),
                    "to the Trash, after confirming".into(),
                ));
            }
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
            for agent in crate::sources::sessions::installed() {
                if agent != owner && crate::lend::can_borrow_mcp(agent) {
                    v.push((
                        leak(format!("lend:{agent}")),
                        format!("Lend it to {agent} for one run"),
                        format!("from {owner}"),
                    ));
                }
            }
            v.push(a("insert", "Insert its name", &it.cmd));
            v.push(a("open", "Open in editor", &target));
            v.push(a("reveal", "cd to its folder", parent_of(&target)));
            v.push(a("copy", "Copy name", ""));
            v
        }
        Kind::File | Kind::Find => open_actions(it.get("path"), &editor),
        Kind::Clip => vec![
            a("insert", "Paste it", ""),
            a("copy", "Copy it again", ""),
            a("tr_en", "Translate to English", ""),
            a("tr_zh", "Translate to Chinese", ""),
        ],
        Kind::Snippet => vec![
            a("insert", "Insert and fill in the blanks", ""),
            a("editsnips", "Edit snippets file", crate::paths::config().join("snippets.toml").to_string_lossy()),
            a("copy", "Copy raw", ""),
        ],
        Kind::Translate => vec![
            a("insert", "Insert the translation", ""),
            a("copy", "Copy the translation", ""),
            a("tr_src", "Copy the original", it.get("source")),
        ],
        Kind::Calc => vec![
            a("copy", "Copy the result", &it.cmd),
            a("insert", "Insert the result", ""),
        ],
        Kind::Ssh => vec![
            a("insert", "Connect", &it.cmd),
            a("editssh", "Edit ~/.ssh/config", ""),
            a("copy", "Copy host", it.get("host")),
        ],
        Kind::App => vec![
            a("insert", "Insert the open command", &it.cmd),
            a("run", "Launch it now", ""),
            a("reveal", "cd to its folder", it.get("path")),
            a("copy", "Copy its path", it.get("path")),
        ],
        Kind::Sys => vec![
            a("insert", "Insert the command", &it.cmd),
            a("run", "Run it now", "⚠ no review step"),
            a("copy", "Copy the command", ""),
        ],
        Kind::Link => vec![
            a("run", "Open in browser", it.get("url")),
            a("insert", "Insert the open command", &it.cmd),
            a("copy", "Copy the URL", it.get("url")),
        ],
        _ => vec![
            a("insert", "Insert into prompt", "you press Enter to run it"),
            a("runhere", "Run here, inside this window", ""),
            a("run", "Run in the shell below", "execute immediately"),
            a("copy", "Copy to clipboard", ""),
        ],
    };
    // Say plainly what Enter does here, so the behaviour is never a mystery.
    acts.insert(0, a("default", crate::defaults::describe(it, host), "Enter"));
    // The secondary action has no key of its own, so this row is the whole
    // of it. It sits directly under Enter's because it is Enter's opposite,
    // and it is the answer to "that is not quite what I wanted".
    if let Some(label) = crate::defaults::describe_secondary(it, host) {
        acts.insert(1, a("secondary", label, "the other one"));
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
    if it.kind.is_command_line()
        && !it.kind.is_interactive()
        && !generic_run_would_kill
        && !acts.iter().any(|(id, ..)| *id == "runhere")
    {
        acts.push(a("runhere", "Run here, inside this window", ""));
    }
    if it.kind.is_command_line() && !acts.iter().any(|(id, ..)| *id == "run") {
        acts.push(a("run", "Run in the shell below", ""));
    }
    // `copyabs` already copies the path, and for a file that is exactly what
    // `copy` copies too — the same action twice, worded differently.
    if !acts.iter().any(|(id, ..)| *id == "copy" || *id == "copyabs") {
        acts.push(a("copy", "Copy to clipboard", ""));
    }
    // "Ask an agent about this" makes no sense for a question an agent has
    // already asked you — the whole row is a request for *your* answer.
    if it.kind != Kind::Msg {
        acts.push(a("ask", "Ask an agent about this", "hands it to claude"));
    }
    if let Some(cwd) = &it.cwd {
        acts.push(a("cd", "Go to project folder", cwd.clone()));
    }
    if it.kind == Kind::Dir {
        acts.push(a("here", "Insert path without cd", ""));
    }
    // The order is the five questions, in the order a person asks them.
    // Stable, so each kind's own sequencing survives inside its group.
    acts.sort_by_key(|(id, ..)| group(it.kind, id));
    acts
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
        _ if id.starts_with("rm:") => DESTROY,
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
        a("insert", "Insert the path", path),
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
    let acts = actions_for(it);
    let feed: String = acts
        .iter()
        .map(|(id, label, sub)| {
            let tail = if sub.is_empty() { String::new() } else { format!("{DIM}· {sub}{RESET}") };
            format!("{:<28}{tail}{SEP}{id}\n", label)
        })
        .collect();

    let short = crate::width::dtrunc(&crate::width::flatten(&it.cmd), 56);
    // The panel's payload is a bare action id, not JSON, so take the raw
    // selection rather than trying to parse an Item out of it.
    match ui::pick_raw(feed.trim_end(), &format!(" {short} "), "⌘ ", "Run  Enter   ·   Back  Esc") {
        Some(id) => apply(&id, it, &paste_target),
        None => 130,
    }
}

pub fn apply(id: &str, it: &Item, paste: &Option<String>) -> i32 {
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
