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
            vec![
                a("insert", &format!("Start {n} here"), &it.cmd),
                a("run", &format!("Start {n} now"), ""),
                (leak(format!("askagent:{n}")), format!("Ask {n} something"), String::new()),
                a("copy", "Copy its name", &n),
            ]
        }
        Kind::Config => vec![
            a("open", "Open in editor", it.get("path")),
            a("reveal", "cd to its folder", parent_of(it.get("path"))),
            a("copyabs", "Copy its path", it.get("path")),
        ],
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
            v
        }
        Kind::Mcp => {
            let target = first_nonempty(it, &["file", "dir", "config"]);
            let mut v = vec![a("insert", "Insert its name", &it.cmd)];
            v.push(a("open", "Open in editor", &target));
            v.push(a("reveal", "cd to its folder", parent_of(&target)));
            v.push(a("copy", "Copy name", ""));
            v
        }
        Kind::File | Kind::Find => vec![
            a("insert", "Insert the path", it.get("path")),
            a("open", "Open in editor", &editor),
            a("copyabs", "Copy absolute path", it.get("path")),
            a("reveal", "cd to its folder", parent_of(it.get("path"))),
        ],
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
    if let Some(label) = crate::defaults::describe_secondary(it, host) {
        acts.insert(1, a("secondary", label, "Option+Enter"));
    }
    // Everything the removed shortcuts used to do stays reachable here.
    if !acts.iter().any(|(id, ..)| *id == "runhere") {
        acts.push(a("runhere", "Run here, inside this window", ""));
    }
    if !acts.iter().any(|(id, ..)| *id == "run") {
        acts.push(a("run", "Run in the shell below", ""));
    }
    if !acts.iter().any(|(id, ..)| *id == "copy") {
        acts.push(a("copy", "Copy to clipboard", ""));
    }
    acts.push(a("ask", "Ask an agent about this", "hands it to claude"));
    if let Some(cwd) = &it.cwd {
        acts.push(a("cd", "Go to project folder", cwd.clone()));
    }
    if it.kind == Kind::Dir {
        acts.push(a("here", "Insert path without cd", ""));
    }
    acts
}

fn first_nonempty(it: &Item, keys: &[&str]) -> String {
    keys.iter().map(|k| it.get(k)).find(|v| !v.is_empty()).unwrap_or("").to_string()
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
                    eprintln!("prelude: {e}");
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
                eprintln!("prelude: nothing to copy from");
                return 2;
            }
            for agent in targets {
                match crate::sources::agents::copy_skill(dir, &agent, name) {
                    Ok(p) => eprintln!("copied {name} -> {p}"),
                    Err(e) => eprintln!("prelude: {e}"),
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
