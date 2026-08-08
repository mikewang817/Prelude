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
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && c == '\\' {
            escaped = true;
            continue;
        }
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if q == c => quote = None,
            (None, '#') => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(v: &str) -> String {
    if let Some(inner) = v.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        return inner
            .split(',')
            .map(|p| unquote(p.trim()))
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
    }
    if let Some(inner) = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        }
        return out;
    }
    v.strip_prefix('\'')
        .and_then(|x| x.strip_suffix('\''))
        .unwrap_or(v)
        .to_string()
}
