//! What you actually pick floats up. A plain text file you can read and edit.

use crate::paths;
use std::collections::HashMap;

pub type Freq = HashMap<String, (u64, f64)>;

fn file() -> std::path::PathBuf {
    paths::data().join("frecency.tsv")
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn load() -> Freq {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(file()) else { return out };
    for line in text.lines() {
        let mut p = line.splitn(3, '\t');
        let (Some(n), Some(t), Some(cmd)) = (p.next(), p.next(), p.next()) else { continue };
        let (Ok(n), Ok(t)) = (n.parse::<u64>(), t.parse::<f64>()) else { continue };
        out.insert(cmd.to_string(), (n, t));
    }
    out
}

/// zoxide-style decay: recent use is worth much more than old use.
pub fn score(n: u64, last: f64) -> f64 {
    let hours = (now() - last).max(0.0) / 3600.0;
    let mult = if hours < 1.0 {
        4.0
    } else if hours < 24.0 {
        2.0
    } else if hours < 24.0 * 7.0 {
        0.5
    } else {
        0.25
    };
    n as f64 * mult
}

pub fn bump(cmd: &str) {
    if cmd.is_empty() {
        return;
    }
    let mut freq = load();
    let entry = freq.entry(cmd.to_string()).or_insert((0, 0.0));
    entry.0 += 1;
    entry.1 = now();

    let mut ranked: Vec<_> = freq.into_iter().collect();
    ranked.sort_by(|a, b| {
        score(b.1 .0, b.1 .1)
            .partial_cmp(&score(a.1 .0, a.1 .1))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(5000);
    let body: String = ranked
        .iter()
        .map(|(c, (n, t))| format!("{n}\t{t:.0}\t{c}\n"))
        .collect();
    let _ = crate::cache::write_atomic(&file(), body.as_bytes());
}
