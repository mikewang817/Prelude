//! Shell history.

use crate::item::{Item, Kind};
use crate::paths;
use std::sync::OnceLock;

const META: u8 = 0x83;

/// Undo zsh's metafication of the history file.
///
/// zsh does not store raw bytes: any byte in 0x80-0x9f is written as 0x83
/// followed by (byte ^ 32). Decoding the file as UTF-8 without undoing that
/// mangles every multi-byte character — 基 comes back as 僿 plus a
/// replacement char. Beyond being unreadable, replacement characters have the
/// wrong display width, which knocks column alignment out and leaves redraw
/// artifacts in the list.
pub fn unmetafy(data: &[u8]) -> Vec<u8> {
    if !data.contains(&META) {
        return data.to_vec();
    }
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == META && i + 1 < data.len() {
            out.push(data[i + 1] ^ 32);
            i += 2;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// Parsed once per process; several sources consume it.
pub fn raw() -> &'static [String] {
    static CACHE: OnceLock<Vec<String>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut candidates = Vec::new();
        if let Some(hf) = std::env::var_os("HISTFILE") {
            candidates.push(std::path::PathBuf::from(hf));
        }
        candidates.push(paths::home().join(".zsh_history"));
        candidates.push(paths::home().join(".bash_history"));

        let Some(path) = candidates.into_iter().find(|p| p.exists()) else {
            return Vec::new();
        };
        // A history file is append-only and nobody prunes it, so the bound is
        // taken from the *end*. Reading the first 32MB of a 40MB history
        // returns what somebody ran years ago and drops what they ran this
        // morning — and the list would look full while being full of the
        // wrong things, which is the kind of wrong nobody reports.
        let Some(bytes) = paths::read_tail_bounded(&path, paths::LOG_FILE) else {
            return Vec::new();
        };
        let text = String::from_utf8_lossy(&unmetafy(&bytes)).into_owned();

        let mut cmds = Vec::new();
        let mut buf = String::new();
        for line in text.lines() {
            // zsh EXTENDED_HISTORY:  ": 1691234567:0;git status"
            let line = strip_timestamp(line);
            let mut line = if buf.is_empty() {
                line.to_string()
            } else {
                let joined = format!("{buf}\n{line}");
                buf.clear();
                joined
            };
            if line.ends_with('\\') {
                line.pop();
                buf = line;
                continue;
            }
            let line = line.trim().to_string();
            if !line.is_empty() {
                cmds.push(line);
            }
        }
        cmds
    })
}

fn strip_timestamp(line: &str) -> &str {
    let Some(rest) = line.strip_prefix(": ") else {
        return line;
    };
    let Some((stamp, cmd)) = rest.split_once(';') else {
        return line;
    };
    let ok = stamp
        .split_once(':')
        .is_some_and(|(a, b)| !a.is_empty() && a.bytes().all(|c| c.is_ascii_digit())
            && !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()));
    if ok { cmd } else { line }
}

const NOISE: &[&str] = &["cd", "ls", "clear", "exit", "prelude", "trx"];

pub fn source() -> Vec<Item> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for c in raw().iter().rev() {
        if c.len() < 2 || seen.contains(c.as_str()) {
            continue;
        }
        let first = c.split_whitespace().next().unwrap_or("");
        if NOISE.contains(&first) || crate::secrets::looks_secret(c) {
            continue;
        }
        seen.insert(c.as_str());
        out.push(Item::new(c.clone(), Kind::History));
        if out.len() >= 3000 {
            break;
        }
    }
    out
}

/// `cd` targets mined from history — the fallback when zoxide has no database.
pub fn dirs_from_history() -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in raw().iter().rev() {
        let mut parts = line.split_whitespace();
        let Some(verb) = parts.next() else { continue };
        if !matches!(verb, "cd" | "z" | "pushd") {
            continue;
        }
        let rest = line[verb.len()..].trim().trim_start_matches("-- ").trim();
        let target = rest.trim_matches(['\'', '"']).trim();
        if target.is_empty()
            || matches!(target, "-" | ".." | "~" | ".")
            || target.starts_with('$')
        {
            continue;
        }
        let expanded = if let Some(r) = target.strip_prefix("~/") {
            paths::home().join(r).to_string_lossy().into_owned()
        } else {
            target.to_string()
        };
        if !expanded.starts_with('/') || !seen.insert(expanded.clone()) {
            continue;
        }
        out.push(expanded);
    }
    out
}
