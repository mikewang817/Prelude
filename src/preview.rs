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
    let mut kv = |k: &str, v: &str| {
        if !v.is_empty() {
            out.push(format!("{DIM}{k:<9}{RESET}{v}"));
        }
    };

    match it.kind {
        Kind::Skill => {
            kv("agents", it.get("agent"));
            kv("file", &tilde(it.get("file")));
            if !it.get("desc").is_empty() {
                out.push(String::new());
                out.push(it.get("desc").to_string());
            }
        }
        Kind::Mcp => {
            kv("agent", it.get("agent"));
            kv("name", it.get("name"));
            kv("config", &tilde(it.get("config")));
        }
        Kind::Proc => {
            kv("pid", it.get("pid"));
            kv("cpu", &format!("{}%", it.get("cpu")));
            kv("memory", it.get("mem"));
            if !it.get("cmd").is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}command{RESET}"));
                out.push(it.get("cmd").to_string());
            }
        }
        Kind::Port => {
            kv("port", it.get("port"));
            kv("process", it.get("proc"));
            kv("pid", it.get("pid"));
        }
        Kind::File | Kind::Find => {
            let p = it.get("path");
            kv("path", &tilde(p));
            if let Ok(m) = std::fs::metadata(p) {
                kv("size", &format!("{} bytes", group(m.len())));
            }
            if let Ok(text) = std::fs::read_to_string(p) {
                out.push(String::new());
                out.push(format!("{DIM}head{RESET}"));
                out.extend(text.lines().take(20).map(str::to_string));
            }
        }
        Kind::Clip => out.push(it.get("full").to_string()),
        Kind::App => kv("path", &tilde(it.get("path"))),
        Kind::Link => kv("url", it.get("url")),
        _ => {}
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
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
