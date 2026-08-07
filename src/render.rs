//! Turning items into the lines fzf displays.
//!
//! Layout rules learned the hard way:
//!   * The kind label is padded to a CONSTANT width. Not measured — the
//!     per-keystroke helper renders in a separate process, and if each side
//!     measured its own the two would drift apart and every computed row
//!     would sit ten columns off.
//!   * Sub-fields get per-kind column widths so pids and memory stack.
//!   * The payload rides after a unit separator; fzf is told to display only
//!     field 1, and bindings read field 2.

use crate::ansi::*;
use crate::item::{Item, Kind};
use crate::width::{dtrunc, dwidth, flatten, pad_to};
use std::sync::OnceLock;

/// Unit separator: splits visible text from hidden payload.
pub const SEP: char = '\u{1f}';

pub fn label_width() -> usize {
    static W: OnceLock<usize> = OnceLock::new();
    *W.get_or_init(|| {
        Kind::all()
            .iter()
            .map(|k| dwidth(k.style().1))
            .max()
            .unwrap_or(9)
    })
}

/// Width available to the middle columns, given the title column.
pub fn middle_budget(width: usize, tw: usize) -> usize {
    let usable = width.max(48) - 8;
    usable.saturating_sub(tw + 2 + label_width() + 3)
}

/// One set of column widths for the whole list, not one per kind.
///
/// Per-kind widths line up within a kind and nowhere else, so the separators
/// scatter: agents put theirs at 20/31/39, skills at 20/37/57/100. Sharing
/// the widths turns the dots into continuous vertical rules down the list,
/// which is what makes it read as a table rather than as rows that happen to
/// be near each other.
pub fn column_widths(items: &[Item], budget: usize) -> Vec<usize> {
    // Gather every value per column, then take a high percentile rather than
    // the maximum. One session recorded in a deep iCloud folder is 127
    // columns wide; letting it set the column left a hundred blanks after
    // "0 mcp" on every other row.
    let mut per: Vec<Vec<usize>> = Vec::new();
    for it in items {
        for (i, f) in it.fields.iter().enumerate() {
            let fw = dwidth(&flatten(f));
            if i >= per.len() {
                per.push(Vec::new());
            }
            per[i].push(fw);
        }
    }
    let mut w: Vec<usize> = per
        .into_iter()
        .map(|mut v| {
            v.sort_unstable();
            v[(v.len() * 90 / 100).min(v.len() - 1)]
        })
        .collect();
    if w.is_empty() {
        return w;
    }
    // Trim from the right until it fits: the last column is a description,
    // which tolerates truncation far better than a pid or a timestamp.
    loop {
        let total: usize = w.iter().sum::<usize>() + 3 * (w.len() - 1);
        if total <= budget {
            break;
        }
        let over = total - budget;
        let last = w.len() - 1;
        if w[last] > over + 8 {
            w[last] -= over;
            break;
        }
        if w.len() == 1 {
            w[0] = w[0].min(budget);
            break;
        }
        w.pop();
    }
    // Anything left over goes to the final column so the row reaches the
    // right-hand edge instead of stopping short.
    let total: usize = w.iter().sum::<usize>() + 3 * (w.len() - 1);
    if let Some(last) = w.last_mut() {
        *last += budget.saturating_sub(total);
    }
    w
}

/// Title column, sized to the titles themselves.
///
/// Not a fixed fraction, and not "whatever the tail leaves over" either. The
/// tail is dominated by one kind — a skill's description runs for hundreds of
/// columns — so sizing against it squeezed titles while most rows still left
/// the right-hand third blank. Sizing against the titles keeps them intact
/// and hands the slack to the middle column, which is the part with
/// something to say.
pub fn title_width(items: &[Item], width: usize) -> usize {
    let usable = width.max(48) - 8;
    let mut w: Vec<usize> = items
        .iter()
        .map(|i| dwidth(&flatten(&i.title)))
        .collect();
    if w.is_empty() {
        return usable / 3;
    }
    w.sort_unstable();
    // Cover most titles rather than the longest: a handful of very long
    // history entries should not set the column for two thousand rows.
    let p85 = w[w.len() * 85 / 100];
    p85.clamp(18, usable * 45 / 100)
}

pub fn render(items: &[Item], width: usize) -> String {
    render_with(items, width, None, None)
}

pub fn render_with(
    items: &[Item],
    width: usize,
    widths: Option<&[usize]>,
    tw_override: Option<usize>,
) -> String {
    // The 8 columns fzf takes for border, pointer and scrollbar are already
    // subtracted inside title_width and middle_budget.
    let tw = tw_override.unwrap_or_else(|| title_width(items, width));
    let lw = label_width();
    let budget = middle_budget(width, tw);
    let owned;
    let cols_w: &[usize] = match widths {
        Some(w) => w,
        None => {
            owned = column_widths(items, budget);
            &owned
        }
    };

    let mut out = String::with_capacity(items.len() * 96);
    for it in items {
        let (color, label) = it.kind.style();
        let title = dtrunc(&flatten(&it.title), tw);
        let pad = " ".repeat((tw + 2).saturating_sub(dwidth(&title)).max(1));

        // The kind label is pinned to the right edge, as Raycast pins its
        // result type. That gives it a single column on every row for free,
        // and lets the middle stretch to fill everything in between.
        let mut middle = String::new();
        if budget >= 8 {
            let tail = if !it.fields.is_empty() {
                let cols: Vec<String> = it
                    .fields
                    .iter()
                    .enumerate()
                    .take(cols_w.len())
                    .map(|(i, f)| {
                        // Everything left-aligned, hard against its
                        // separator. Right-aligning numbers stacks digits
                        // neatly within one kind, but the columns are shared
                        // across all of them — so "2 skills" got shoved to
                        // the far end of a column widened by "claude,
                        // shared", metres away from the dot it belongs to.
                        pad_to(&dtrunc(&flatten(f), cols_w[i]), cols_w[i], false)
                    })
                    .collect();
                Some(cols.join(" · "))
            } else if !it.subtitle.is_empty() {
                Some(flatten(&it.subtitle))
            } else {
                None
            };
            if let Some(t) = tail {
                if !t.is_empty() {
                    middle = dtrunc(&t, budget);
                }
            }
        }

        out.push_str(&title);
        out.push_str(&pad);
        if middle.is_empty() {
            out.push_str(&" ".repeat(budget + 3));
        } else {
            // "· " + budget + one space = budget + 3, matching the empty
            // branch exactly so every row ends in the same column.
            out.push_str(&format!("{DIM}· {}{RESET} ", pad_to(&middle, budget, false)));
        }
        // Right-aligned within a constant width, so the labels share an
        // edge and every row ends in the same column.
        out.push_str(color);
        out.push_str(&pad_to(label, lw, true));
        out.push_str(RESET);

        out.push(SEP);
        out.push_str(&serde_json::to_string(it).unwrap_or_default());
        out.push('\n');
    }
    out.pop();
    out
}

/// Recover an item from what an fzf binding handed us.
///
/// Bindings use `{2}` (the payload field), not `{}` — with `--with-nth` in
/// play, `{}` is the *transformed* display text and the payload isn't in it
/// at all. Both forms are accepted so either works.
pub fn parse_line(line: &str) -> Option<Item> {
    let s = line.trim();
    let json = match s.split_once(SEP) {
        Some((_, rest)) => rest,
        None => s,
    };
    serde_json::from_str(json).ok()
}
