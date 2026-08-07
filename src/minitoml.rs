//! A deliberately tiny TOML reader.
//!
//! Only what this tool needs: section headers and `key = "value"` or
//! `key = ["a", "b"]`. Pulling in a full TOML crate would cost compile time
//! and binary size for config files that are a dozen lines long.

use std::collections::BTreeMap;

pub type Table = BTreeMap<String, BTreeMap<String, String>>;

pub fn parse(text: &str) -> Table {
    let mut out = Table::new();
    let mut section = String::new();
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = inner.trim().trim_matches('"').to_string();
            out.entry(section.clone()).or_default();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().trim_matches('"').to_string();
            out.entry(section.clone())
                .or_default()
                .insert(key, unquote(v.trim()));
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(v: &str) -> String {
    if let Some(inner) = v.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        return inner
            .split(',')
            .map(|p| p.trim().trim_matches('"').to_string())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    v.trim_matches('"').trim_matches('\'').to_string()
}
