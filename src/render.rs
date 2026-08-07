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
use std::collections::HashMap;
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

/// Column widths per kind, taken from the items that carry structured fields.
pub fn field_widths(items: &[Item]) -> HashMap<Kind, Vec<usize>> {
    let mut w: HashMap<Kind, Vec<usize>> = HashMap::new();
    for it in items {
        if it.fields.is_empty() {
            continue;
        }
        let cur = w.entry(it.kind).or_default();
        for (i, f) in it.fields.iter().enumerate() {
            let fw = dwidth(&flatten(f));
            if i < cur.len() {
                cur[i] = cur[i].max(fw);
            } else {
                cur.push(fw);
            }
        }
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

pub fn render(items: &[Item], width: usize, widths: Option<&HashMap<Kind, Vec<usize>>>) -> String {
    render_with(items, width, widths, None)
}

pub fn render_with(
    items: &[Item],
    width: usize,
    widths: Option<&HashMap<Kind, Vec<usize>>>,
    tw_override: Option<usize>,
) -> String {
    let owned;
    let widths = match widths {
        Some(w) => w,
        None => {
            owned = field_widths(items);
            &owned
        }
    };
    // Leave room for fzf's border, pointer and scrollbar, or lines wrap.
    let usable = width.max(48) - 8;
    let tw = tw_override.unwrap_or_else(|| title_width(items, width));
    let lw = label_width();

    let mut out = String::with_capacity(items.len() * 96);
    for it in items {
        let (color, label) = it.kind.style();
        let title = dtrunc(&flatten(&it.title), tw);
        let pad = " ".repeat((tw + 2).saturating_sub(dwidth(&title)).max(1));

        // The kind label is pinned to the right edge, as Raycast pins its
        // result type. That gives it a single column on every row for free,
        // and lets the middle stretch to fill everything in between.
        let budget = usable.saturating_sub(tw + 2 + lw + 3);
        let mut middle = String::new();
        if budget >= 8 {
            let tail = if !it.fields.is_empty() {
                let w = widths.get(&it.kind);
                let cols: Vec<String> = it
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let f = flatten(f);
                        // Numbers read best right-aligned so digits stack.
                        let right = f.starts_with(|c: char| c.is_ascii_digit());
                        let cw = w.and_then(|w| w.get(i)).copied().unwrap_or_else(|| dwidth(&f));
                        pad_to(&f, cw, right)
                    })
                    .collect();
                Some(cols.join(" · ").trim_end().to_string())
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
