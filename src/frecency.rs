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

/// The most a record of use is worth in the ordering.
///
/// It cannot cross a kind — `cache::by_rank` settles the band first — so the
/// ceiling is no longer there to stop it vaulting. It is there so that past
/// a point, what the source itself knows (this session is the newest, this
/// run is the one that is stuck) still counts for something against one more
/// use.
pub const MAX_BONUS: f64 = 60.0;

/// What a record of `n` uses, last at `last`, is worth in the ordering.
///
/// One definition, used for both kinds of evidence there are: the times you
/// picked a row in the launcher, and the times a skill was actually invoked
/// in a conversation. Eight of one should weigh the same as eight of the
/// other, and they only do if the arithmetic lives in one place.
pub fn bonus(n: u64, last: f64) -> f64 {
    (score(n, last) * 12.0).min(MAX_BONUS)
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
    // Read, change, write — so two of these at once lose one of the two
    // counts. Which is not hypothetical here: this runs on *every* Enter, and
    // a person with a shell and the panel open is two Preludes. The lock is
    // held across the whole cycle, and if it cannot be had in a quarter of a
    // second the bump is taken anyway: a launcher may lose a use count, but
    // it may never make somebody wait to press a key.
    let _lock = crate::cache::lock_for_write(&file());
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
    let _ = crate::cache::write_state(&file(), body.as_bytes());
}
