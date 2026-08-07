//! The detail pane.
//!
//! 90% of titles are under 17 columns, so a wide terminal left most of the
//! panel empty while Chinese skill descriptions were cut at eight characters.
//! Narrowing the title column is not the answer — fzf matches against
//! *displayed* text, so truncating titles would stop you finding a long
//! command by its tail. The space goes to detail instead.

use crate::ansi::*;
use crate::item::{Item, Kind};
use crate::paths::tilde;

pub fn show(it: &Item) {
    let (color, label) = it.kind.style();
    let mut out = vec![format!("{color}{label}{RESET}"), String::new()];
    fn kv(out: &mut Vec<String>, k: &str, v: &str) {
        if !v.is_empty() {
            out.push(format!("{DIM}{k:<9}{RESET}{v}"));
        }
    }

    match it.kind {
        // A question, in full, plus the conversation it came out of. The row
        // has to fit the question on one line; this does not, and a decision
        // you are being asked to make deserves the whole sentence and the
        // few exchanges that led to it.
        Kind::Msg => {
            kv(&mut out, "from", it.get("agent"));
            kv(&mut out, "project", &tilde(it.get("cwd")));
            kv(&mut out, "at", it.get("pane"));
            out.push(String::new());
            out.extend(it.get("text").lines().map(str::to_string));
            let pane = it.get("pane");
            if !pane.is_empty() {
                let screen = crate::exec::run(
                    &["tmux", "capture-pane", "-p", "-t", pane, "-S", "-30"],
                    std::time::Duration::from_secs(2),
                );
                let screen = screen.trim_end();
                if !screen.is_empty() {
                    out.push(String::new());
                    out.push(format!("{DIM}what it was doing{RESET}"));
                    out.extend(screen.lines().map(str::to_string));
                }
            }
        }
        // The one preview that answers the question you actually had. A row
        // saying "waiting 12m" tells you something is stuck; only its screen
        // tells you what it asked. One capture for the selected pane, not
        // for all eighty of them.
        Kind::Run => {
            kv(&mut out, "agent", it.get("agent"));
            kv(&mut out, "project", &tilde(it.get("cwd")));
            kv(&mut out, "at", it.get("addr"));
            kv(&mut out, "state", it.get("state"));
            kv(&mut out, "pid", it.get("pid"));
            // What it last said, from its conversation file — which exists
            // whether or not it is in a terminal Prelude can see into. The
            // pane's screen is better when there is one, since it shows the
            // half-finished line too.
            let pane = it.get("pane");
            let screen = if pane.is_empty() {
                String::new()
            } else {
                crate::exec::run(
                    &["tmux", "capture-pane", "-p", "-t", pane, "-S", "-40"],
                    std::time::Duration::from_secs(2),
                )
            };
            let screen = screen.trim_end();
            if !screen.is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}its screen{RESET}"));
                out.extend(screen.lines().map(str::to_string));
            } else {
                let tail = crate::sources::running::transcript_tail(it.get("session"), 8);
                if !tail.is_empty() {
                    out.push(String::new());
                    out.push(format!("{DIM}what it last said{RESET}"));
                    out.extend(tail);
                }
            }
        }
        Kind::Skill => {
            kv(&mut out, "agents", it.get("agent"));
            kv(&mut out, "file", &tilde(it.get("file")));
            if !it.get("desc").is_empty() {
                out.push(String::new());
                out.push(it.get("desc").to_string());
            }
        }
        Kind::Mcp => {
            kv(&mut out, "agent", it.get("agent"));
            kv(&mut out, "name", it.get("name"));
            kv(&mut out, "config", &tilde(it.get("config")));
        }
        Kind::Proc => {
            kv(&mut out, "pid", it.get("pid"));
            kv(&mut out, "cpu", &format!("{}%", it.get("cpu")));
            kv(&mut out, "memory", it.get("mem"));
            if !it.get("cmd").is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}command{RESET}"));
                out.push(it.get("cmd").to_string());
            }
        }
        Kind::Port => {
            kv(&mut out, "port", it.get("port"));
            kv(&mut out, "process", it.get("proc"));
            kv(&mut out, "pid", it.get("pid"));
        }
        Kind::File | Kind::Find => {
            let p = it.get("path");
            kv(&mut out, "path", &tilde(p));
            if let Ok(m) = std::fs::metadata(p) {
                kv(&mut out, "size", &format!("{} bytes", group(m.len())));
            }
            if let Ok(text) = std::fs::read_to_string(p) {
                out.push(String::new());
                out.push(format!("{DIM}head{RESET}"));
                out.extend(text.lines().take(20).map(str::to_string));
            }
        }
        Kind::Clip => out.push(it.get("full").to_string()),
        Kind::App => kv(&mut out, "path", &tilde(it.get("path"))),
        Kind::Link => kv(&mut out, "url", it.get("url")),
        Kind::Agent => {
            for (k, v) in [("skills", 0), ("mcp", 1), ("sessions", 2)] {
                kv(&mut out, k, it.fields.get(v).map(String::as_str).unwrap_or(""));
            }
            out.push(String::new());
            out.push(format!("{DIM}start it{RESET}"));
            out.push(it.get("agent").to_string());
        }
        Kind::Session => {
            kv(&mut out, "agent", it.get("agent"));
            kv(&mut out, "id", it.get("id"));
            kv(&mut out, "where", &tilde(it.get("cwd")));
            if !it.get("opening").is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}opened with{RESET}"));
                out.push(it.get("opening").to_string());
            }
        }
        Kind::Config => {
            kv(&mut out, "agent", it.get("agent"));
            let p = it.get("path");
            kv(&mut out, "path", &tilde(p));
            if let Ok(text) = std::fs::read_to_string(p) {
                out.push(String::new());
                out.push(format!("{DIM}head{RESET}"));
                out.extend(text.lines().take(24).map(str::to_string));
            }
        }
        Kind::Snippet => {
            kv(&mut out, "name", it.get("name"));
            out.push(String::new());
            out.push(crate::ui::fill_placeholders(&it.cmd));
        }
        Kind::Container => {
            kv(&mut out, "name", it.get("name"));
            kv(&mut out, "image", it.get("image"));
        }
        Kind::Ssh => kv(&mut out, "host", it.get("host")),
        Kind::Dir | Kind::Script | Kind::Git | Kind::History | Kind::Path
        | Kind::Sys | Kind::Calc | Kind::Translate => {
            if !it.subtitle.is_empty() {
                out.push(it.subtitle.clone());
                out.push(String::new());
            }
            if let Some(cwd) = &it.cwd {
                kv(&mut out, "in", &tilde(cwd));
            }
        }
    }
    if it.kind != Kind::Clip {
        out.push(String::new());
        out.push(format!("{DIM}runs{RESET}"));
        out.push(it.cmd.clone());
    }
    println!("{}", out.join("\n"));
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
