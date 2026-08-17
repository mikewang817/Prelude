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
use crate::width::{dtrunc, dtrunc_middle, dwidth, flatten, pad_to};
use std::sync::OnceLock;

/// Unit separator: splits visible text from hidden payload.
pub const SEP: char = '\u{1f}';

pub fn label_width() -> usize {
    static W: OnceLock<usize> = OnceLock::new();
    *W.get_or_init(|| {
        Kind::all()
            .iter()
            .map(|k| dwidth(k.style().1))
            .chain(std::iter::once(dwidth(Kind::QUICKLINK_STYLE.1)))
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

/// General Search is a semantic three-column table, regardless of where a row
/// came from: object name, object type, then context. It previously concatenated
/// the catalogue renderer, the file renderer, and one-off dynamic renderers;
/// each placed the type at a different x-coordinate, so no amount of tuning a
/// single width could align the combined list.
pub fn render_general(items: &[Item], width: usize) -> String {
    let usable = width.max(48) - 8;
    let lw = label_width();
    let wanted_title = (usable * 26 / 100).clamp(24, 42);
    let max_title = usable.saturating_sub(lw + 3 + 12).max(12);
    let tw = wanted_title.min(max_title);
    let detail_width = usable.saturating_sub(tw + 2 + lw + 3).max(1);
    let mut out = String::with_capacity(items.len() * 112);

    for item in items {
        let filesystem = matches!(item.kind, Kind::File | Kind::Find | Kind::Dir)
            && !item.get("path").is_empty();
        let title = if filesystem && !item.is_quicklink() {
            std::path::Path::new(item.get("path"))
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(&item.title)
        } else {
            &item.title
        };
        let title = dtrunc(&flatten(title), tw);
        let (color, label) = item.style();

        let detail = if filesystem && !item.is_quicklink() {
            // The object's own complete path, the same rule `render_files`
            // follows: parent-only context read `App · ~` on the row for
            // `~/App`, which said least exactly where it mattered most.
            let location = crate::paths::tilde(item.get("path"));
            let tags = item
                .get("tags")
                .split('\u{1e}')
                .filter(|tag| !tag.is_empty())
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" ");
            if tags.is_empty() {
                dtrunc_middle(&flatten(&location), detail_width)
            } else {
                let tag_budget = (detail_width * 45 / 100).max(8).min(detail_width);
                let tag_text = format!(" · {}", dtrunc(&tags, tag_budget.saturating_sub(3)));
                let location_width = detail_width.saturating_sub(dwidth(&tag_text));
                format!("{}{}", dtrunc_middle(&flatten(&location), location_width), tag_text)
            }
        } else {
            let context = if item.fields.is_empty() {
                item.subtitle.clone()
            } else {
                item.fields.iter().map(|field| flatten(field)).collect::<Vec<_>>().join(" · ")
            };
            dtrunc(&context, detail_width)
        };

        out.push_str(&title);
        out.push_str(&" ".repeat((tw + 2).saturating_sub(dwidth(&title)).max(1)));
        out.push_str(color);
        out.push_str(&pad_to(label, lw, false));
        out.push_str(RESET);
        if !detail.is_empty() {
            out.push_str(&format!("{DIM} · {detail}{RESET}"));
        }
        out.push(SEP);
        out.push_str(&serde_json::to_string(item).unwrap_or_default());
        out.push('\n');
    }
    out.pop();
    out
}

/// Settings are a form, not search results. Their useful schema is category,
/// control, current value, and effect; repeating the generic `setting` type on
/// every row spent a column while communicating nothing. The category remains
/// on every row so a filtered subset never loses the context that says what
/// kind of control is left on screen.
fn settings_widths(width: usize) -> (usize, usize, usize, usize) {
    let usable = width.max(64) - 8;
    let group_width = 10;
    let title_width = (usable * 20 / 100).clamp(20, 30);
    let value_width = (usable * 28 / 100).clamp(18, 42);
    let detail_width = usable
        .saturating_sub(group_width + title_width + value_width + 9)
        .max(8);
    (group_width, title_width, value_width, detail_width)
}

pub fn settings_header(width: usize) -> String {
    let (group, title, value, _) = settings_widths(width);
    format!(
        "{DIM}{}   {}   {}   Effect{RESET}",
        pad_to("Category", group, false),
        pad_to("Setting", title, false),
        pad_to("Current", value, false),
    )
}

pub fn render_settings(items: &[Item], width: usize) -> String {
    let (group_width, title_width, value_width, detail_width) = settings_widths(width);
    let mut out = String::with_capacity(items.len() * 112);

    for item in items {
        let group = item.get("group");
        let value = item.fields.first().map(String::as_str).unwrap_or_default();
        let detail = item.fields.get(1).map(String::as_str).unwrap_or_default();
        let title = dtrunc(&flatten(&item.title), title_width);
        let value = dtrunc(&flatten(value), value_width);
        let detail = dtrunc(&flatten(detail), detail_width);

        out.push_str(CYAN);
        out.push_str(&pad_to(group, group_width, false));
        out.push_str(RESET);
        out.push_str("   ");
        out.push_str(&pad_to(&title, title_width, false));
        out.push_str("   ");
        out.push_str(YELLOW);
        out.push_str(&pad_to(&value, value_width, false));
        out.push_str(RESET);
        if !detail.is_empty() {
            out.push_str(&format!("{DIM}   {detail}{RESET}"));
        }
        out.push(SEP);
        out.push_str(&serde_json::to_string(item).unwrap_or_default());
        out.push('\n');
    }
    out.pop();
    out
}

/// File search is one stable three-column form: full filename where possible,
/// then kind, then the parent path. The general catalogue uses percentile
/// widths because descriptions and process fields compete for space; applying
/// that compromise to f: left most of a wide panel blank while truncating the
/// one thing a file picker must distinguish.
pub fn render_files(items: &[Item], width: usize) -> String {
    let usable = width.max(48) - 8;
    let lw = label_width();
    let wanted_title = (usable * 38 / 100).clamp(24, 72);
    let max_title = usable.saturating_sub(lw + 3 + 2 + 12).max(12);
    let tw = wanted_title.min(max_title);
    let path_width = usable.saturating_sub(tw + 2 + lw + 3).max(1);
    let mut out = String::with_capacity(items.len() * 128);

    for item in items {
        let path = item.get("path");
        let title = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&item.title);
        let title = dtrunc(&flatten(title), tw);
        // The object's own complete path, not its parent. A parent-only
        // context put `· ~` beside the row for `~/App` — the one folder its
        // context column said least about — and left eight rows all titled
        // `app` telling the reader to join two columns in their head.
        let location = if path.is_empty() {
            item.subtitle.clone()
        } else {
            crate::paths::tilde(path)
        };
        let tag_text = if path_width >= 16 && !item.get("tags").is_empty() {
            let tags = item
                .get("tags")
                .split('\u{1e}')
                .filter(|tag| !tag.is_empty())
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join(" ");
            let budget = (path_width * 45 / 100).max(8);
            format!(" · {}", dtrunc(&tags, budget.saturating_sub(3)))
        } else {
            String::new()
        };
        let location_width = path_width.saturating_sub(dwidth(&tag_text));
        let location = dtrunc_middle(&flatten(&location), location_width);
        let (color, label) = item.style();

        out.push_str(&title);
        out.push_str(&" ".repeat((tw + 2).saturating_sub(dwidth(&title)).max(1)));
        out.push_str(color);
        out.push_str(&pad_to(label, lw, true));
        out.push_str(RESET);
        out.push_str(&format!("{DIM} · {location}{tag_text}{RESET}"));
        out.push(SEP);
        out.push_str(&serde_json::to_string(item).unwrap_or_default());
        out.push('\n');
    }
    out.pop();
    out
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
        let (color, label) = it.style();
        let title = dtrunc(&flatten(&it.title), tw);
        let pad = " ".repeat((tw + 2).saturating_sub(dwidth(&title)).max(1));

        out.push_str(&title);
        out.push_str(&pad);

        if cols_w.len() >= 2 {
            // The type used to be the sixth and final column, after a skill's
            // description. Put it in the fifth slot instead: the first three
            // detail columns remain shared, then comes the stable kind, and
            // the long description gets the flexible right edge.
            let last = cols_w.len() - 1;
            let prefix_w = cols_w[..last].iter().sum::<usize>() + 3 * last.saturating_sub(1);
            let prefix = it
                .fields
                .iter()
                .enumerate()
                .take(last)
                .map(|(i, f)| pad_to(&dtrunc(&flatten(f), cols_w[i]), cols_w[i], false))
                .collect::<Vec<_>>()
                .join(" · ");
            if prefix.is_empty() {
                out.push_str(&" ".repeat(prefix_w + 3));
            } else {
                out.push_str(&format!(
                    "{DIM}· {}{RESET} ",
                    pad_to(&dtrunc(&prefix, prefix_w), prefix_w, false)
                ));
            }

            // Right-aligned within a constant width, so all type labels form
            // one column even when the rows around them have fewer fields.
            out.push_str(color);
            out.push_str(&pad_to(label, lw, true));
            out.push_str(RESET);

            let final_field = it.fields.get(last).map(String::as_str).or_else(|| {
                (it.fields.is_empty() && !it.subtitle.is_empty()).then_some(it.subtitle.as_str())
            });
            if let Some(field) = final_field.filter(|f| !f.is_empty()) {
                let field = dtrunc(&flatten(field), cols_w[last]);
                out.push_str(&format!(
                    "{DIM} · {}{RESET}",
                    pad_to(&field, cols_w[last], false)
                ));
            } else {
                out.push_str(&" ".repeat(cols_w[last] + 3));
            }
        } else {
            // Small one-column lists have no fifth/sixth pair to swap. Keep
            // their compact layout rather than manufacturing empty columns.
            let mut middle = String::new();
            if budget >= 8 {
                let tail = it.fields.first().map(String::as_str).or_else(|| {
                    (!it.subtitle.is_empty()).then_some(it.subtitle.as_str())
                });
                if let Some(t) = tail.filter(|t| !t.is_empty()) {
                    middle = dtrunc(&flatten(t), budget);
                }
            }
            if middle.is_empty() {
                out.push_str(&" ".repeat(budget + 3));
            } else {
                out.push_str(&format!("{DIM}· {}{RESET} ", pad_to(&middle, budget, false)));
            }
            out.push_str(color);
            out.push_str(&pad_to(label, lw, true));
            out.push_str(RESET);
        }

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

#[cfg(test)]
mod tests {
    use super::{render_files, render_general, SEP};
    use crate::item::{Item, Kind};

    #[test]
    fn general_search_has_one_type_column_for_every_result_family() {
        let rows = vec![
            Item::new("set:", Kind::Search)
                .title("Prelude Settings")
                .fields(["set:", "search roots, hotkey, keys and rules"]),
            Item::new("cd /tmp/Prelude", Kind::Find)
                .title("Prelude")
                .put("path", "/tmp/Prelude")
                .put("index_kind", "folder"),
            Item::new("/tmp/Prelude/prelude.png", Kind::Find)
                .title("prelude.png")
                .put("path", "/tmp/Prelude/prelude.png")
                .put("index_kind", "file"),
            Item::new("https://example.com", Kind::Link)
                .title("prelude")
                .fields(["Search Google", "www.google.com"]),
            Item::new("/natural-chinese-writing", Kind::Skill)
                .title("natural-chinese-writing")
                .fields(["pi", "never used", "Chinese writing"]),
        ];
        let rendered = render_general(&rows, 160);
        let mut positions = Vec::new();
        for line in rendered.lines() {
            let visible = line.split(SEP).next().unwrap();
            let item = super::parse_line(line).unwrap();
            let color = item.style().0;
            positions.push(crate::width::dwidth(visible.split(color).next().unwrap()));
        }
        assert!(positions.iter().all(|position| *position == positions[0]), "{positions:?}");
    }

    /// The context column is the object's **complete** path, requested after
    /// the parent-only form put `· ~` beside the row for `~/App` — the one
    /// folder its context said least about. A long path still loses its
    /// middle before either useful end is discarded.
    #[test]
    fn file_search_spends_space_on_the_filename_and_shows_the_complete_path() {
        let filename = "CN115131558A_complete_name.md";
        let path = crate::paths::home()
            .join("App/a-very-long-container-name/another-long-project-name/source/deep/parent")
            .join(filename);
        let item = Item::new(path.to_string_lossy(), Kind::Find)
            .title(filename)
            .put("path", path.to_string_lossy().into_owned());
        let rendered = render_files(&[item], 160);
        let visible = rendered.split(super::SEP).next().unwrap();
        assert!(visible.contains(filename), "the filename had room but was cut: {visible}");
        assert!(visible.contains("~/App/"), "the path root is useful context: {visible}");
        assert!(visible.contains("..."), "a long path should lose its middle: {visible}");
        assert!(
            visible.contains(&format!("/{filename}")),
            "the path column ends in the object itself, so it reads as one complete path: {visible}"
        );

        // A short path arrives whole: the row for `~/App` says `~/App`, not `~`.
        let folder = Item::new("cd /Users/x/App", Kind::Find)
            .title("App")
            .put("path", "/Users/x/App")
            .put("index_kind", "folder");
        let rendered = render_files(&[folder], 120);
        let visible = rendered.split(super::SEP).next().unwrap();
        assert!(visible.contains("/Users/x/App"), "{visible}");
    }
}
