//! Display width in terminal columns.
//!
//! Two traps live here. CJK characters occupy two columns, so counting
//! `char`s misaligns every row containing Chinese. And East Asian *Ambiguous*
//! characters — `·` `—` `“”` `→` — are one column in most Western terminals
//! and two in CJK-configured ones. `·` happens to be the separator this UI
//! uses on every single row, so guessing wrong shifts everything.
//!
//! Rather than infer it from `$LANG`, `prelude doctor` prints one and asks the
//! terminal where the cursor landed; the answer is cached.

use std::sync::OnceLock;
use unicode_width::UnicodeWidthChar;

static AMBIG: OnceLock<usize> = OnceLock::new();

pub fn ambiguous_width() -> usize {
    *AMBIG.get_or_init(|| {
        std::fs::read_to_string(crate::paths::cache().join("ambiguous_width"))
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|w| *w == 1 || *w == 2)
            .unwrap_or(1)
    })
}

/// True for characters unicode-width reports as ambiguous.
fn is_ambiguous(c: char) -> bool {
    // unicode-width exposes this via the CJK variant differing from the base.
    UnicodeWidthChar::width_cjk(c) != UnicodeWidthChar::width(c)
}

pub fn cw(c: char) -> usize {
    if is_ambiguous(c) {
        ambiguous_width()
    } else {
        UnicodeWidthChar::width(c).unwrap_or(0)
    }
}

pub fn dwidth(s: &str) -> usize {
    s.chars().map(cw).sum()
}

/// Truncate to a column budget, not a character count.
pub fn dtrunc(s: &str, limit: usize) -> String {
    if dwidth(s) <= limit {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for c in s.chars() {
        let width = cw(c);
        if w + width > limit.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += width;
    }
    out.push('…');
    out
}

/// Keep both ends of a long value and replace its middle with three visible
/// dots. Paths need this rather than `dtrunc`: the root says where it starts,
/// while the final components say which folder actually contains the file.
pub fn dtrunc_middle(s: &str, limit: usize) -> String {
    if dwidth(s) <= limit {
        return s.to_string();
    }
    if limit <= 3 {
        return dtrunc(s, limit);
    }
    let available = limit - 3;
    let left_limit = available * 2 / 5;
    let right_limit = available - left_limit;

    let mut left = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let columns = cw(ch);
        if width + columns > left_limit {
            break;
        }
        left.push(ch);
        width += columns;
    }

    let mut right = Vec::new();
    width = 0;
    for ch in s.chars().rev() {
        let columns = cw(ch);
        if width + columns > right_limit {
            break;
        }
        right.push(ch);
        width += columns;
    }
    right.reverse();
    format!("{left}...{}", right.into_iter().collect::<String>())
}

pub fn pad_to(s: &str, w: usize, right: bool) -> String {
    let gap = " ".repeat(w.saturating_sub(dwidth(s)));
    if right {
        format!("{gap}{s}")
    } else {
        format!("{s}{gap}")
    }
}

/// Collapse all whitespace runs to single spaces; newlines would break the
/// one-row-per-line contract with fzf.
pub fn flatten(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
