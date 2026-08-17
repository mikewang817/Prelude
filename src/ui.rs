//! The interactive surface: fzf invocation, key handling, and what each key
//! does to the selected item.

use crate::ansi::*;
use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::render::{self, SEP};
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

/// Enter does the obvious thing, ^K reaches every alternative, ^P briefly
/// replaces the list with Quick Look, and esc leaves. Preview earns a place
/// in the footer because it is a mode rather than an item action: it does not
/// belong in ^K, and a hidden one-key view is not discoverable.
///
/// Keys are spelled out rather than drawn as glyphs — a row of symbols is
/// only legible to someone who already knows what they mean. The keys occupy
/// one dim row and their meanings one cyan row below it. Fixed column widths
/// keep the two rows paired and prevent the footer from jumping when focus
/// moves between a file and a folder.
const FOOTER_GAP: usize = 4;
const PRIMARY_FOOTER_WIDTH: usize = 16;

struct FooterColumn<'a> {
    key: &'static str,
    action: &'a str,
    width: usize,
}

pub fn footer_for(primary: &str, terminal_width: usize) -> String {
    footer_for_item(primary, None, false, terminal_width)
}

pub fn footer_for_item(
    primary: &str,
    item: Option<&Item>,
    command_enter: bool,
    terminal_width: usize,
) -> String {
    let mut columns = vec![FooterColumn {
        key: "Enter",
        action: primary,
        width: PRIMARY_FOOTER_WIDTH,
    }];
    if let Some(setting) = item.filter(|item| item.kind == Kind::Setting) {
        columns.push(FooterColumn {
            key: "←",
            action: crate::settings::direction_label(setting, "left"),
            width: 16,
        });
        columns.push(FooterColumn {
            key: "→",
            action: crate::settings::direction_label(setting, "right"),
            width: 16,
        });
    }
    if command_enter && item.is_some_and(revealable_path) {
        // Revealing selects the object *inside* its parent, so a folder row is
        // shown one level up while a file is shown where it lives.
        let label = if item.is_some_and(|item| object_of(item).is_some_and(|(_, dir)| dir)) {
            "Reveal in parent"
        } else {
            "Reveal"
        };
        columns.push(FooterColumn { key: "Ctrl+Enter", action: label, width: 16 });
    }
    if command_enter && item.is_some_and(|item| terminal_directory(item).is_some()) {
        let label = item.map(terminal_label).unwrap_or("Terminal in folder");
        // Wide enough for the longest of the three destinations it names. A
        // column narrower than its own label truncates silently, which reads
        // as a typo rather than as a layout running out of room.
        columns.push(FooterColumn { key: "Ctrl+Shift+Enter", action: label, width: 19 });
    }
    if command_enter && item.is_some_and(|item| path_to_copy(item).is_some()) {
        columns.push(FooterColumn { key: "Ctrl+Option+Enter", action: "Copy path", width: 17 });
    }
    columns.push(FooterColumn { key: "Ctrl+K →", action: "Actions", width: 8 });
    if crate::settings::preview_enabled() {
        columns.push(FooterColumn { key: "Ctrl+P", action: "Preview", width: 7 });
    }

    // Borders, pointer and scrollbar consume a handful of the terminal's
    // columns. Settings arrows are the form's primary controls, so discovery
    // aids disappear before them. If an extremely narrow window cannot hold
    // the pair, hide both together rather than showing only Add or only Remove.
    let available = terminal_width.saturating_sub(8);
    for key in [
        "Ctrl+P",
        "Ctrl+K →",
        "Ctrl+Option+Enter",
        "Ctrl+Shift+Enter",
        "Ctrl+Enter",
    ] {
        let used = columns.iter().map(|column| column.width).sum::<usize>()
            + FOOTER_GAP * columns.len().saturating_sub(1);
        if used <= available {
            break;
        }
        if let Some(at) = columns.iter().position(|column| column.key == key) {
            columns.remove(at);
        }
    }
    let used = columns.iter().map(|column| column.width).sum::<usize>()
        + FOOTER_GAP * columns.len().saturating_sub(1);
    if used > available {
        columns.retain(|column| !matches!(column.key, "←" | "→"));
    }

    let gap = " ".repeat(FOOTER_GAP);
    let keys = columns
        .iter()
        .map(|column| crate::width::pad_to(column.key, column.width, false))
        .collect::<Vec<_>>()
        .join(&gap);
    let actions = columns
        .iter()
        .map(|column| {
            let action = crate::width::dtrunc(column.action, column.width);
            crate::width::pad_to(&action, column.width, false)
        })
        .collect::<Vec<_>>()
        .join(&gap);
    format!("{DIM}{keys}{RESET}\n{CYAN}{actions}{RESET}")
}

/// The prefix language, stated once, where it cannot be missed.
///
/// Half of what this launcher can do is behind a prefix — `s:` for past
/// conversations, `r:` for the live fleet, `@` to put a question to an agent
/// — and none of it appeared anywhere in the interface. It was documented,
/// which is not the same as discoverable: a feature you have to read a README
/// to find is a feature most people never have.
///
/// One whole sentence at a time, rotating on the clock.
///
/// Every earlier version of this line was a *table* squeezed onto one row —
/// `c: clipboard   : scopes`, then `c: what you copied   set: settings and
/// search folders`. Both were written in the shape of a reference rather than
/// of a sentence, and that shape has a hard ceiling: the row is one line, so
/// the more it covers the less each entry can say, and every entry collapses
/// towards its bare noun. `scopes` is what that ceiling produces — the
/// internal name of a mechanism, printed to a person who has never heard of
/// it, because there was no room for the verb that would have explained it.
///
/// Rotating removes the ceiling instead of rationing space under it. One tip
/// gets the whole line, so it can be an instruction with a subject and a verb;
/// and the set can then be *large*, because the cost of adding one is no
/// longer paid by the others. Every scope worth knowing is in here, which is
/// what the single-line version had spent three revisions failing to fit.
///
/// It is a clock bucket rather than a counter or a shuffle, and both of those
/// were the alternatives. A counter needs persisted state — a file written on
/// the launch path, for a hint. A shuffle changes on every press, which reads
/// as noise and means you can never go back to the one you half-read. The
/// bucket is stable for `TIP_ROTATION`, so a tip survives being dismissed and
/// re-opened, and the sequence is the same on every machine, which is what
/// makes it something a person can be told about.
///
/// It shows only on an empty query and disappears the moment you type, so it
/// costs a row exactly when there is nothing else to look at and never
/// competes with results.
pub const TIPS: &[&str] = &[
    "Tip · type c: to see everything you have copied, newest first",
    "Tip · type f: to look for a file or folder by name",
    "Tip · type s: to find a conversation you had with an agent",
    "Tip · press Ctrl+K on any row to see what else it can do",
    "Tip · type set: to choose which folders are searched",
    "Tip · type app: to open an application",
    "Tip · type h: to search the commands you have run before",
    "Tip · press Ctrl+P to look inside a row without opening it",
    "Tip · type : to see every way of narrowing what you are looking at",
    "Tip · type 10kg to lb, or any sum, to get the answer right here",
    "Tip · type ql: to see the keywords you saved for yourself",
    "Tip · press Ctrl+Option+Enter to copy the full path of a file",
    "Tip · type r: to see which agents are working and which are waiting",
    "Tip · type skill: to browse every skill on this machine",
];

/// How long one tip stays put.
///
/// A minute. It was fifteen, on the reasoning that a tip should survive being
/// dismissed and re-opened — which it should, and a minute still does that,
/// because the thing being guarded against is the tip changing *between two
/// presses seconds apart*, not over a coffee break. Fifteen minutes bought
/// nothing more for that and cost the whole point of rotating: at four tips an
/// hour a fourteen-tip set takes three and a half hours to come round once,
/// which for somebody who opens the launcher in bursts means seeing the same
/// sentence all morning. A minute shows the set in a quarter of an hour of
/// intermittent use, and a tip is still fixed for the entire life of any one
/// panel a person actually reads.
const TIP_ROTATION: u64 = 60;

/// The tip for a given bucket. Pure, so the rotation can be tested without
/// waiting a minute for it.
pub fn tip_at(bucket: u64) -> &'static str {
    TIPS[(bucket % TIPS.len() as u64) as usize]
}

pub fn hints() -> &'static str {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    tip_at(now / TIP_ROTATION)
}

/// The three object shortcuts are Enter chords only in the fingers: the
/// panel's generated Ghostty config translates Ctrl+Enter into private Ctrl+G,
/// Ctrl+Shift+Enter into private Ctrl+], and Ctrl+Option+Enter into private
/// Ctrl+Y, which is what fzf actually receives.
/// It has to be a translation — fzf knows no `ctrl-enter`, because a bare
/// Return carries no modifier a terminal application can read. Both are
/// contextual to rows that carry a filesystem path. Prelude installs the same
/// translations in ordinary Ghostty, so both launcher entry points receive
/// the same private control codes.
///
/// `ctrl-]` is 0x1d. Not 0x1f, which is `render::SEP` — the delimiter every
/// rendered row already carries, and which fzf would therefore read as the
/// start of a field rather than as a keypress. And not Ctrl+[, which is 0x1b:
/// that *is* Escape, the same byte, so binding it would take away the key
/// that means "back" at every level.
///
/// Only the two keys that act on *every* row are `--expect` keys. The three
/// object chords used to be here too, and an `--expect` key cannot be
/// conditional: fzf exits on it whatever the row, so pressing Ctrl+Enter on a
/// history entry — a row with no object, where the footer advertised nothing —
/// tore the whole launcher down and rebuilt it empty, deleting the typed
/// query. They are transforms now, on Tab's precedent: applicable rows print
/// a marker and accept, everything else is inert.
const EXPECT: &str = "ctrl-x,ctrl-k";

/// The three object chords, as (fzf key, `_objkey` name, output marker).
/// The marker rides fzf's output queue exactly as `OPEN_ACTIONS` does, and
/// `run_fzf` folds it back into the key name so `search` reads one vocabulary.
pub const OBJ_CHORDS: &[(&str, &str, &str)] = &[
    ("ctrl-g", "reveal", "prelude:reveal-object"),
    ("ctrl-]", "terminal", "prelude:terminal-there"),
    ("ctrl-y", "copy-path", "prelude:copy-path"),
];

/// How `→` says "open the action panel" without being an `--expect` key.
///
/// It cannot be one: `--expect` keys are not bindings, so they cannot be
/// unbound, and `→` has to give the query line its arrow back the moment
/// there is any text to move through. A binding can, so `→` is one, and
/// `print` puts this on fzf's output queue for `run_fzf` to recognise. The
/// separator is deliberately absent from it — a line carrying `SEP` would be
/// read as an item.
pub const OPEN_ACTIONS: &str = "prelude:open-actions";

/// Modal pickers use arrows as level navigation. The main list routes arrows
/// contextually through `_setting-key`: ordinary queries keep cursor movement,
/// the empty home keeps `→` as Actions, and Settings uses both as controls.
pub const ARROW_BOTH: &str = "left,right";

/// Whether a key name is one the launcher asks fzf to report.
///
/// Exists so a test can state what is *not* claimable — `ctrl-[` above all,
/// which is Escape's own byte.
#[cfg(test)]
pub fn expects(key: &str) -> bool {
    EXPECT.split(',').any(|k| k == key)
}

pub struct FzfOut {
    pub key: String,
    /// The query as it stood when fzf exited, from `--print-query`. Empty
    /// when the flag was not passed — and on abort, where fzf prints nothing
    /// at all, not even the query line.
    pub query: String,
    pub item: Option<Item>,
    /// The selected line exactly as it was fed in, which is the only thing
    /// that can be found again in the feed. A parsed `Item` cannot: rendering
    /// is lossy and two rows can parse alike.
    pub line: Option<String>,
    pub failed: bool,
    pub stderr: String,
}

fn base_args(prompt: &str, label: &str, footer: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = [
        "--ansi", "--layout=reverse", "--info=inline-right", "--border=rounded",
        "--border-label-pos=3", "--pointer=▸", "--marker=✓", "--with-nth=1",
        "--no-multi",
        // index (not `begin`) — our own priority+frecency ordering must win
        // ties, otherwise 1900 $PATH binaries outrank the project's scripts.
        "--tiebreak=index",
        // Long rows must stay left-aligned: fzf's default horizontal scroll
        // slides the line to reveal the match, which eats the title and
        // breaks every column on that row.
        "--no-hscroll", "--ellipsis=…", "--scrollbar=│",
        "--color=border:8,preview-border:6,preview-label:6,label:6,prompt:6,pointer:5,hl:2,hl+:2,info:8,header:8",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(format!("--border-label={label}"));
    a.push(format!("--prompt={prompt}"));
    a.push(format!("--delimiter={SEP}"));
    if let Some(f) = footer {
        a.push("--footer".into());
        a.push(f.into());
        a.push("--footer-border=line".into());
    }
    a
}

pub fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tip_tests {
    use super::*;

    /// The line is one row above the list, and a tip that runs off the end of
    /// a narrow window is advice nobody can act on. 72 leaves room inside an
    /// 80-column terminal for the borders fzf draws around it.
    #[test]
    fn every_tip_fits_a_narrow_window_and_tells_you_to_do_something() {
        for tip in TIPS {
            let width = crate::width::dwidth(tip);
            assert!(width <= 72, "{tip:?} is {width} columns and will be cut");
            // A tip is an instruction. The versions this replaced were bare
            // nouns — `clipboard`, `scopes` — which is exactly the failure,
            // so the shape is worth pinning rather than trusting to taste.
            assert!(
                tip.contains(" type ") || tip.contains(" press "),
                "{tip:?} names something instead of telling you what to do"
            );
        }
    }

    /// Rotation has to reach every tip and be stable inside one bucket,
    /// because a hint that changes while you are reading it is noise.
    #[test]
    fn the_rotation_is_stable_within_a_bucket_and_reaches_every_tip() {
        assert_eq!(tip_at(7), tip_at(7), "the same bucket is the same tip");
        assert_ne!(tip_at(7), tip_at(8), "and the next one moves on");

        let seen: std::collections::BTreeSet<&str> =
            (0..TIPS.len() as u64 * 2).map(tip_at).collect();
        assert_eq!(seen.len(), TIPS.len(), "every tip comes round");

        // The bucket is unbounded — it is a wall clock divided by the
        // rotation — so the index has to wrap rather than panic in 2035.
        assert_eq!(tip_at(u64::MAX), TIPS[(u64::MAX % TIPS.len() as u64) as usize]);
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    /// Backing out of the action panel put the cursor back on the first row,
    /// so `→` on the fifth row and `←` straight back cost you your place —
    /// every time, on a list whose whole point is that you had found something
    /// in it.
    #[test]
    fn coming_back_from_a_modal_lands_on_the_row_you_left() {
        let row = |title: &str, cmd: &str| format!("{DIM}{title}{RESET}{SEP}{{\"cmd\":\"{cmd}\"}}");
        let feed = format!("{}\n{}\n{}\n", row("alpha", "a"), row("beta", "b"), row("gamma", "c"));

        // What fzf hands back, which is *not* what it was fed: `--ansi` means
        // the colour codes are parsed for display and printed stripped. Every
        // row here is coloured, so comparing whole lines found nothing, every
        // time, without saying so.
        let returned = |title: &str, cmd: &str| format!("{title}{SEP}{{\"cmd\":\"{cmd}\"}}");
        assert_eq!(position_in(&feed, &returned("alpha", "a")), Some(1), "pos() is one-based");
        assert_eq!(position_in(&feed, &returned("gamma", "c")), Some(3));
        // And the fed form still resolves, so this does not depend on which
        // of the two shapes reaches it.
        assert_eq!(position_in(&feed, &row("beta", "b")), Some(2));

        // A row that is not in *this* feed — it came from a query that
        // rebuilt the list, and the list we are returning to is the home one.
        // No position is better than a wrong one.
        assert_eq!(position_in(&feed, &returned("delta", "d")), None);
        // A line with no payload at all is not a row.
        assert_eq!(position_in(&feed, "alpha"), None);
        assert_eq!(position_in(&feed, ""), None);

        let args = vec!["--ansi".to_string()];
        // The first launch is untouched: `--sync` would hold the finder until
        // the whole feed is read, and that is the launch the budget is kept
        // against.
        assert_eq!(with_cursor(&args, None, None), args);
        assert_eq!(
            with_cursor(&args, Some(5), None),
            vec!["--ansi", "--sync", "--bind", "start:pos(5)"],
        );
        // A typed query is the other half of that state. It comes back as
        // `--query` plus the same `_bind` transform every keystroke runs, and
        // it wins over `pos`: with a query the visible list was `_dynamic`'s,
        // where a home-feed position would be a spot in the wrong list.
        let resume = ("clau".to_string(), "transform:prelude _bind".to_string());
        assert_eq!(
            with_cursor(&args, Some(5), Some(&resume)),
            vec!["--ansi", "--query=clau", "--bind", "start:transform:prelude _bind"],
        );
    }
}

/// Where a line sits in the feed, one-based, which is what `pos()` counts in.
///
/// Matched on the payload rather than the whole line, because **`--ansi` means
/// fzf does not give back what it was given**: it parses the colour codes out
/// for display and prints the line without them, so a rendered row — and every
/// row here is coloured — can never be found by string equality. That failed
/// silently, which is the worst shape available: no position, no error, and a
/// cursor back at the top exactly as before the fix.
///
/// The payload is the right key anyway. It carries no colour, it is what every
/// binding already addresses the row by (`{2}`), and it is the row's identity
/// rather than its appearance.
fn position_in(feed: &str, line: &str) -> Option<usize> {
    let payload = payload_of(line)?;
    feed.lines()
        .position(|candidate| payload_of(candidate) == Some(payload))
        .map(|at| at + 1)
}

fn payload_of(line: &str) -> Option<&str> {
    line.split_once(SEP).map(|(_, payload)| payload)
}

/// The same arguments, plus a starting cursor when we are coming back to a
/// list somebody had already moved through.
///
/// **`--sync` is what makes `start` mean anything here.** fzf consumes its
/// input asynchronously, so `start` fires before the list exists and `pos()`
/// lands on whatever few rows have arrived — which is how this reads as
/// "sometimes it works". `--sync` holds the finder until the input is complete
/// and initial processing is done, and only then raises `start`.
///
/// It is added *only* on the way back, never to the first launch: `--sync`
/// means rendering nothing until the whole feed is read, and the first launch
/// is the one measured against the 40ms budget. Coming back from a modal, the
/// feed is a string already in memory and there is no budget being kept.
///
/// `load` would be the other candidate and is the wrong one: it fires again on
/// every `reload`, so the cursor would be dragged back here each time a
/// keystroke rebuilt the list.
/// Re-open the list where the person left it: cursor, or query.
///
/// The two restorations are exclusive. With no query, the row is found in the
/// static home feed and the cursor returns to it. With a typed query, the row
/// lives in a list only `_dynamic` can rebuild, so the query is put back and
/// `start` runs the same `_bind` transform every keystroke runs — without it
/// fzf would filter the *home* feed by the restored query, which is the wrong
/// list wearing the right words. `--sync` stays with `pos` alone: it exists
/// to keep `start` from firing before the feed is read, and the reload path
/// replaces the feed anyway.
fn with_cursor(args: &[String], position: Option<usize>, resume: Option<&(String, String)>) -> Vec<String> {
    let mut args = args.to_vec();
    if let Some((query, reload)) = resume {
        args.push(format!("--query={query}"));
        args.push("--bind".into());
        args.push(format!("start:{reload}"));
        return args;
    }
    if let Some(position) = position {
        args.push("--sync".into());
        args.push("--bind".into());
        args.push(format!("start:pos({position})"));
    }
    args
}

pub fn run_fzf(feed: &str, args: Vec<String>, cols: usize) -> FzfOut {
    // One launcher layout in both windows. The shell entry used to be a 90%
    // inline popup while the Quick Terminal filled its surface, which made the
    // same binary read as two products before an action was even chosen.
    let modes: Vec<Vec<String>> = vec![vec![]];
    let _ = cols;

    let mut last = FzfOut { key: String::new(), query: String::new(), item: None, line: None, failed: true, stderr: String::new() };
    // stdout's shape follows the flags, pinned by experiment against fzf
    // 0.74: with `--print-query` the query is the first line, with `--expect`
    // the key is the next, `print(...)` lines follow, then the selection. On
    // abort fzf prints nothing at all — not even the query line.
    let has_query = args.iter().any(|arg| arg == "--print-query");
    let has_expect = args.iter().any(|arg| arg.starts_with("--expect"));
    for mode in modes {
        let mut cmd = Command::new("fzf");
        cmd.args(&args).args(&mode)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let Ok(mut child) = cmd.spawn() else {
            eprintln!("prelude: fzf not found. Install it:  brew install fzf");
            std::process::exit(2);
        };
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(feed.as_bytes());
        }
        let Ok(out) = child.wait_with_output() else { continue };
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

        // Exit codes alone can't distinguish failure from a normal empty
        // result: a refusal to start and an honest no-match both exit 1.
        // stderr is what separates them — anything written there means fzf
        // never ran.
        let failed = !out.status.success() && stdout.trim().is_empty() && !stderr.trim().is_empty();
        if failed {
            if env_flag("PRELUDE_DEBUG") {
                eprintln!("prelude: fzf mode {mode:?} failed: {}", stderr.trim());
            }
            last = FzfOut { key: String::new(), query: String::new(), item: None, line: None, failed: true, stderr };
            continue;
        }
        let mut lines = stdout.split('\n');
        let query = if has_query {
            lines.next().unwrap_or("").to_string()
        } else {
            String::new()
        };
        let key = if has_expect {
            lines.next().unwrap_or("").trim().to_string()
        } else {
            String::new()
        };
        // Everything after the query and key lines: `print` output and the
        // selection, in whatever order fzf chose.
        let rest: Vec<&str> = lines.collect();
        let line = rest.iter().find(|l| l.contains(SEP)).map(|l| l.to_string());
        let item = line.as_deref().and_then(render::parse_line);
        // `→` is a binding rather than an `--expect` key, so it announces
        // itself on the output queue. Scanned for rather than read from a
        // fixed line: `print` and the selection share one stream and their
        // order is fzf's business, not ours. The object chords ride the same
        // queue for the same reason. Scanned in `rest`, never the whole
        // stdout: `--print-query` puts the typed text on the first line, and
        // a person who types a marker's own words must not trigger its key.
        let key = if rest.iter().any(|l| l.trim() == OPEN_ACTIONS) {
            "ctrl-k".to_string()
        } else if let Some((chord, _, _)) = OBJ_CHORDS
            .iter()
            .find(|(_, _, marker)| rest.iter().any(|l| l.trim() == *marker))
        {
            chord.to_string()
        } else {
            key
        };
        return FzfOut { key, query, item, line, failed: false, stderr };
    }
    last
}

pub fn search() -> i32 {
    // The panel has a configured standing directory and a shell has whatever
    // directory its prompt happens to be in. Using both made project scripts,
    // files and Git rows differ between two otherwise identical launchers.
    // One configured directory gives both entry points one catalogue.
    let _ = std::env::set_current_dir(crate::global::launch_directory());
    // A fresh install or changed root prepares its shared index immediately,
    // without making the first filename the person types be the trigger. The
    // previous generation remains usable while this detached builder runs.
    crate::compute::ensure_fileindex();
    crate::clipd::ensure_running();
    let items = crate::cache::gather();
    if items.is_empty() {
        eprintln!("prelude: nothing to search yet — run some commands first");
        return 2;
    }
    let term = crate::term_width();
    let preview = crate::settings::preview_enabled();
    // Quick Look is hidden until requested and replaces the result area
    // rather than taking a permanent right-hand column. The list therefore
    // owns the full width at all times; laying it out for a pane that is not
    // there threw almost half of a wide terminal away.
    let cols = term;

    // Compute the layout once and hand it to the per-keystroke helper. Both
    // sides must agree; if each measured its own, computed rows would land
    // in a different column from the static ones.
    let tw = render::title_width(&items, cols);
    let widths = render::column_widths(&items, render::middle_budget(cols, tw));
    let root = crate::compute::root_items(&items);
    let root_feed = render::render_general(&root, cols);
    let home = crate::compute::home_items(&items);
    let home_feed = render::render_with(&home, cols, Some(&widths), Some(tw));

    // Root Search has two deliberately small surfaces: the agent home before
    // typing, and agent/Quicklink/search commands afterwards. The full
    // gathered catalogue is data for explicit scopes, not an eager fuzzy
    // list. Keep it as JSON so a scope never runs a source on a keystroke.
    let static_path = crate::paths::cache().join("list.txt");
    let home_path = crate::paths::cache().join("home.txt");
    let items_path = crate::paths::cache().join("search-items.json");
    // One gather's answer in three files: staged, then renamed together, and
    // skipped entirely where the bytes have not moved — which for a catalogue
    // of four hundred kilobytes is most launches.
    let mut snapshot = vec![
        (static_path.clone(), root_feed.as_bytes().to_vec()),
        (home_path, home_feed.as_bytes().to_vec()),
    ];
    if let Ok(json) = serde_json::to_vec(&items) {
        snapshot.push((items_path, json));
    }
    crate::cache::write_group(&snapshot);
    let me = std::env::current_exe().unwrap_or_default();
    let me = me.to_string_lossy().into_owned();

    let mut args = base_args("⌕ ", " Prelude ", Some(&footer_for("Select", cols)));
    args.push(format!("--header={}", hints()));
    args.push(format!("--expect={EXPECT}"));
    // The query is part of the state a modal must hand back. Without it, ^K
    // then Esc landed on the home screen with the typed text gone — "back one
    // level" that quietly took two.
    args.push("--print-query".into());
    args.push("--bind".into());
    args.push(format!(
        "change:transform:{} _bind {{q}} {} {cols} {tw}",
        shq(&me),
        shq(&static_path.to_string_lossy())
    ));
    args.push("--bind".into());
    args.push(format!("enter:transform:{} _enter {{2}}", shq(&me)));
    // Say what Enter will do to *this* row, since the answer depends on what
    // kind of thing it is. Every helper uses the same clipboard vocabulary.
    // `focus` fires on cursor movement and on a search-result update — but
    // *not* when `reload` replaces the whole list and the cursor stays where
    // it already was, on the first row. Every scope entry is such a reload, so
    // the first row of `c:`, `s:`, `f:` and the rest arrived describing the
    // row that was focused before them: the footer under a clipboard entry
    // read "Open this search", which is what `c:`'s own scope command says,
    // and the contextual clipboard preview never opened at all because the
    // decision to open it is made in the same helper. One arrow press fixed
    // it, which is why it survived — the state was only ever wrong at the
    // moment nobody had touched anything yet.
    //
    // `load` is the missing half: it fires when a list has finished arriving,
    // including after every reload. Both events run the same helper.
    let on_focus = if preview {
        // One helper updates both the item-specific footer and the contextual
        // c: image preview, so moving the cursor does not pay for two forks.
        format!("transform:{} _focus {{q}} {{2}}", shq(&me))
    } else {
        format!("transform-footer:{} _footer {{2}}", shq(&me))
    };
    args.push("--bind".into());
    args.push(format!("focus:{on_focus}"));
    args.push("--bind".into());
    args.push(format!("load:{on_focus}"));
    // Resizing can change how many complete shortcut columns fit. Re-run the
    // same helper with fzf's exported FZF_COLUMNS rather than letting either
    // footer row be clipped independently.
    args.push("--bind".into());
    args.push(format!("resize:{on_focus}"));
    // Arrow keys are contextual: normal cursor movement outside Settings,
    // row-specific adjustment inside it, and `→` still opens Actions on the
    // empty home. The helper emits one fzf action for the current query/row.
    for direction in ["left", "right"] {
        args.push("--bind".into());
        args.push(format!(
            "{direction}:transform:{} _setting-key {direction} {{q}} {{2}} {} {cols} {tw}",
            shq(&me),
            shq(&static_path.to_string_lossy()),
        ));
    }
    // Escape is "back", one level at a time, and only closes at the outermost
    // one. A typed query is a level: it is backed out of before the launcher
    // is. Ghostty no longer binds Escape, so this is the whole of what it
    // does — see `global::quick_config`.
    args.push("--bind".into());
    args.push("esc:transform:[ -n {q} ] && echo clear-query || echo abort".into());
    args.push("--bind".into());
    args.push(format!("ctrl-o:execute({} _runhere {{2}})", shq(&me)));
    // The object chords act only where the focused row carries the object,
    // and are inert everywhere else — Tab's precedent. As `--expect` keys
    // they exited fzf on every row, which on an objectless one meant the
    // typed query and the cursor were destroyed for nothing.
    for (chord, which, _) in OBJ_CHORDS {
        args.push("--bind".into());
        args.push(format!("{chord}:transform:{} _objkey {which} {{2}}", shq(&me)));
    }
    // The key that used to be history search still is: pressed *inside* the
    // launcher it moves the query into `h:`, carrying whatever was typed, and
    // pressed again it carries the text back out. Ctrl+R twice at a shell is
    // therefore the old incremental history search, which is the muscle
    // memory this launcher took over and owes back.
    args.push("--bind".into());
    args.push(format!("ctrl-r:transform:{} _hist {{q}}", shq(&me)));
    // Tab is the finger's guess for "finish this for me". It completes the
    // focused row where completion is what Enter would do anyway — scope
    // commands and search providers — and does nothing everywhere else,
    // because inventing a Tab behaviour for rows that have no completion
    // would teach the key to mean something different per row.
    args.push("--bind".into());
    args.push(format!("tab:transform:{} _tab {{2}}", shq(&me)));
    if preview {
        args.push("--preview".into());
        args.push(format!("{} _preview {{2}}", shq(&me)));
        args.push("--preview-window".into());
        args.push("down,99%,wrap,border-top,hidden".into());
        args.push("--bind".into());
        args.push("ctrl-p:toggle-preview".into());
    }

    // The panel outlives its list, so the list has to be kept alive too.
    if let Some(socket) = crate::refresh::listen_socket() {
        args.push(format!("--listen={}", socket.display()));
        crate::refresh::keep_current(socket, widths, tw, cols);
    }

    // The same `_bind` transform every keystroke runs, reused verbatim at
    // `start` when a query is being restored, so the rebuilt list cannot
    // disagree with the one typing would have produced.
    let resume_reload = format!(
        "transform:{} _bind {{q}} {} {cols} {tw}",
        shq(&me),
        shq(&static_path.to_string_lossy())
    );
    let mut restore: Option<usize> = None;
    let mut resume: Option<(String, String)> = None;
    loop {
        let out = run_fzf(&home_feed, with_cursor(&args, restore, resume.as_ref()), cols);
        if out.failed {
            let msg = out.stderr.trim().lines().last().unwrap_or("").to_string();
            eprintln!("prelude: fzf could not start{}", if msg.is_empty() { String::new() } else { format!(": {msg}") });
            return 2;
        }
        // Every `continue` below is a return *to this list* from something
        // modal, so the cursor — and the typed query — belong where they
        // were rather than back at a blank home. Recorded here rather than at
        // each of the six sites: they all mean the same thing, and the next
        // one added would otherwise have to remember to say so.
        restore = out.line.as_deref().and_then(|line| position_in(&home_feed, line));
        resume = (!out.query.trim().is_empty())
            .then(|| (out.query.clone(), resume_reload.clone()));
        let Some(item) = out.item else { return 130 };

        match out.key.as_str() {
            "ctrl-k" => {
                let code = crate::actions::panel(&item);
                if code == crate::actions::PANEL_BACK {
                    // ^K is a modal over the list. Esc backs out one level
                    // instead of closing the launcher entirely.
                    continue;
                }
                return code;
            }
            "ctrl-x" => {
                // The legacy force-run key must not turn a URL back into a
                // shell command. Links always go straight to Launch Services.
                if item.kind == Kind::Link {
                    return apply_default(&item);
                }
                crate::frecency::bump(&item.cmd);
                emit("RUN", &item.cmd);
                return 0;
            }
            "ctrl-g" if revealable_path(&item) => {
                crate::frecency::bump(&item.cmd);
                let Some((path, _)) = object_of(&item) else { continue };
                return match crate::openwith::reveal_now(path) {
                    Ok(()) => 0,
                    Err(error) => {
                        note(&error);
                        2
                    }
                };
            }
            // Ctrl+Shift+Enter, translated by the panel's Ghostty config. It
            // opens a *new* Ghostty standing in this row's directory, which
            // is the one thing the launcher cannot hand over as text: a
            // command to cd somewhere is only useful in a shell you already
            // have, and the point here is not having one.
            "ctrl-]" => {
                let Some(directory) = terminal_directory(&item) else { continue };
                crate::frecency::bump(&item.cmd);
                return match crate::global::open_directory(&directory) {
                    Ok(()) => 0,
                    Err(error) => {
                        note(&error);
                        2
                    }
                };
            }
            "ctrl-y" => {
                let Some(path) = path_to_copy(&item) else { continue };
                crate::frecency::bump(&item.cmd);
                emit("INSERT", path);
                return 0;
            }
            "ctrl-g" => continue,
            _ => return apply_default(&item),
        }
    }
}

/// Whether an object chord applies to this row, answered as the marker the
/// `_objkey` transform prints before accepting — or `None`, which the
/// transform turns into fzf's no-op and the person feels as an inert key.
///
/// The same three predicates the footer uses to decide what to advertise, so
/// a chord can never fire on a row whose footer said nothing about it.
pub fn objkey_marker(which: &str, item: &Item) -> Option<&'static str> {
    let (_, _, marker) = OBJ_CHORDS.iter().find(|(_, name, _)| *name == which)?;
    let applies = match which {
        "reveal" => revealable_path(item),
        "terminal" => terminal_directory(item).is_some(),
        "copy-path" => path_to_copy(item).is_some(),
        _ => false,
    };
    applies.then_some(*marker)
}

/// Whether Enter should hand this row to Finder rather than to the
/// application that owns it. Deliberately narrower than `object_of` below:
/// this decides what Enter *does*, and widening it would change defaults.
fn is_folder(item: &Item) -> bool {
    item.kind == Kind::Dir || item.get("index_kind") == "folder"
}

/// The filesystem object a row **is**, and whether that object is a directory.
///
/// All three Enter chords ask this one question, so they cannot disagree about
/// which rows they apply to. They used to ask it separately and each answered
/// `File | Find | Dir` — which was narrower than the data by eight kinds. An
/// application, a config, a conversation, a live run, a skill, an MCP server,
/// a settings file and a clipped image all carry a real path, and every one of
/// them already offered *the same verbs by name* in `^K`: `Reveal in Finder`,
/// `Copy path`, `Open terminal in containing folder`. So the keys were dead on
/// exactly the rows whose action panel proved the key had something to do.
///
/// A row is included only where the object is unambiguous. Nothing here
/// guesses: a history entry, an agent CLI and a `$PATH` binary have no
/// filesystem object that is *theirs*, and inventing one would make the chords
/// mean something different from row to row.
///
/// A Setting is the deliberate exclusion, and it does carry a path. Two
/// reasons, both already written down. Its backing file is storage rather than
/// intent, and belongs in Details and `^K` where the rest of the storage
/// vocabulary lives. And `set:` is a form, not a list of objects: its two
/// controls are `←` and `→`, and three more footer columns push the key that
/// says so off the end of a narrow window.
pub fn object_of(item: &Item) -> Option<(&str, bool)> {
    let field = |key: &'static str| {
        let value = item.get(key);
        (!value.is_empty()).then_some(value)
    };
    Some(match item.kind {
        Kind::Dir => (field("path")?, true),
        Kind::Find => (field("path")?, item.get("index_kind") == "folder"),
        // An `.app` is a bundle, so the kernel calls it a directory and every
        // person calls it the application. Finder agrees with the person:
        // revealing it selects the bundle, and a terminal belongs beside it in
        // `/Applications` rather than inside its `Contents`.
        Kind::File | Kind::Config | Kind::App => (field("path")?, false),
        // A conversation's object is the native file the agent wrote, which is
        // what `^K`'s `Reveal native session file` already points at.
        Kind::Session => (field("file")?, false),
        // A skill is a directory with an entry point. Prefer the entry point:
        // `SKILL.md` is the thing you open, and its folder is one Ctrl+Enter
        // away from it.
        Kind::Skill => match field("file") {
            Some(file) => (file, false),
            None => (field("dir")?, true),
        },
        // A live agent's object is the project it is working in.
        Kind::Run => (field("cwd").or_else(|| field("path"))?, true),
        // Only the clips that put a payload on disk. A text clip has none.
        Kind::Clip => (field("path")?, false),
        _ => return None,
    })
}

fn revealable_path(item: &Item) -> bool {
    object_of(item).is_some()
}

pub fn path_to_copy(item: &Item) -> Option<&str> {
    object_of(item).map(|(path, _)| path)
}

pub(crate) fn containing_directory(item: &Item) -> Option<std::path::PathBuf> {
    let (path, directory) = object_of(item)?;
    if directory {
        return None;
    }
    std::path::Path::new(path).parent().map(std::path::Path::to_path_buf)
}

/// The folder Ctrl+Shift+Enter should stand a terminal in.
///
/// One meaning, applied consistently: *the directory this row's work happens
/// in*. For a folder that is the folder itself, because a folder row is
/// already at the place you would want the prompt to be, and sending a
/// terminal to its parent instead would be the launcher being clever. For a
/// file it is the file's parent, the same place Ctrl+Enter reveals.
///
/// And for a row that records its own working directory, it is that: a run and
/// a conversation have both *said* where their work happens, which beats
/// deriving a directory from where a file is stored. A conversation's `.jsonl`
/// lives in the agent's private storage, so the parent rule would stand a
/// terminal in `~/.claude/projects/…` — an answer to a question nobody asked,
/// about the one row where the right answer is written down.
///
/// This is the chord's whole reason for existing, and the live-run row is the
/// case the argument was written for: a `cd` is only useful in a shell you
/// already have, and the point of the panel is not having one.
///
/// Nothing else answers. A history entry or an agent CLI has no directory that
/// is *its*, and a key that guesses one for them would stop meaning one thing.
pub fn terminal_directory(item: &Item) -> Option<std::path::PathBuf> {
    let recorded = item.get("cwd");
    if !recorded.is_empty() {
        return Some(std::path::PathBuf::from(recorded));
    }
    let (path, directory) = object_of(item)?;
    if directory {
        return Some(std::path::PathBuf::from(path));
    }
    containing_directory(item)
}

/// What the Ctrl+Shift+Enter footer column should say, which is wherever
/// `terminal_directory` is actually about to send the terminal.
fn terminal_label(item: &Item) -> &'static str {
    if !item.get("cwd").is_empty() {
        "Terminal in project"
    } else if object_of(item).is_some_and(|(_, directory)| directory) {
        "Terminal here"
    } else {
        "Terminal in folder"
    }
}

/// Carry out whatever Enter means for this item.
pub fn apply_default(item: &Item) -> i32 {
    crate::frecency::bump(&item.cmd);
    perform(item, crate::defaults::on_enter(item))
}

pub fn perform(item: &Item, what: crate::defaults::Default_) -> i32 {
    use crate::defaults::{text_for, Default_};
    match what {
        Default_::Insert => {
            let cmd = if item.kind == Kind::Snippet {
                fill_placeholders(&item.cmd)
            } else {
                item.cmd.clone()
            };
            emit("INSERT", &cmd);
        }
        Default_::InsertText(what) => emit("INSERT", &text_for(item, what)),
        Default_::Act(verb) => return act(item, verb),
    }
    0
}

fn act(item: &Item, verb: crate::defaults::Verb) -> i32 {
    use crate::defaults::Verb::*;
    match verb {
        // External objects go straight to macOS Launch Services. No `open`
        // command touches the terminal buffer or shell history. Files honour
        // Prelude's remembered application; folders always use Finder.
        Open => {
            let p = first_of(item, &["path", "file", "config"]);
            let p = if p.is_empty() { item.cmd.clone() } else { p };
            let result = if is_folder(item) {
                crate::openwith::open_finder_now(&p)
            } else {
                crate::openwith::open_default_now(&p)
            };
            if let Err(e) = result {
                note(&e);
                return 2;
            }
        }
        // An agent is blocked on this. Ask for the line here, in the surface
        // already in front of you, and the answer unblocks it where it stands
        // — no going to its window, no finding the question again.
        Answer => {
            let id = item.get("id");
            if id.is_empty() {
                return 2;
            }
            let Some(line) = prompt_line(&format!(" answer {} ", item.get("agent"))) else {
                return 130;
            };
            return crate::bus::answer(id, &line);
        }
        Launch => {
            if let Err(e) = crate::openwith::launch_now(item.get("path")) {
                note(&e);
                return 2;
            }
        }
        OpenUrl => {
            let url = if item.get("url").is_empty() { &item.cmd } else { item.get("url") };
            if let Err(e) = crate::openwith::open_now(url, None) {
                note(&e);
                return 2;
            }
        }
        CopyResult => {
            if item.kind == Kind::Clip && item.get("clip_kind") != "text" {
                if let Err(e) = crate::clipd::restore(item) {
                    note(&e);
                    return 2;
                }
                eprintln!("restored to clipboard: {}", item.title);
            } else {
                copy(if item.kind == Kind::Clip { item.get("full") } else { &item.cmd });
                eprintln!("copied: {}", item.title);
            }
        }
        ResumeSession => emit("INSERT", &item.cmd),
        RunHere => {
            let code = crate::runhere::run_item(item);
            // An update replaces the binary this panel is running, and it
            // cannot restart the panel from inside it — stopping the panel
            // kills the process tree the update is running in. `update`
            // already asked the newly installed binary to regenerate and
            // reload Ghostty's config in place; closing this old surface is
            // the remaining step, so the next press starts the new `_surface`.
            // The zsh widget ignores a verb it has no case for.
            if code == 0 && item.get("update") == "available" {
                emit("CLOSE", "press the chord again to come back on the new version");
            }
            return code;
        }
        RunInShell => {
            let cmd = if item.kind == Kind::Snippet {
                fill_placeholders(&item.cmd)
            } else {
                item.cmd.clone()
            };
            emit("RUN", &cmd);
        }
        Inspect => {
            let c = if item.kind == Kind::Proc {
                format!("ps -p {} -o command=", item.get("pid"))
            } else {
                format!("lsof -nP -iTCP:{} -sTCP:LISTEN", item.get("port"))
            };
            emit("INSERT", &c);
        }
        CdThere => {
            let d = if item.get("cwd").is_empty() { item.get("path") } else { item.get("cwd") };
            emit("INSERT", &format!("cd {}", shq(d)));
        }
        EditSetting => return crate::settings::edit(item),
        // The one action here that waits on a model, so it says it is working:
        // a launcher that goes still for four seconds has, as far as anyone
        // watching can tell, done nothing.
        Rewrite => {
            let (source, cfg) = crate::rewrite::for_item(item);
            eprintln!("rewriting with {} …", cfg.model);
            match crate::rewrite::rewrite(&source, &cfg) {
                // Copy-and-close, the same contract every other row ends on.
                Ok(out) if out.warnings.is_empty() => emit("INSERT", &out.text),
                // Both halves matter, and `MSG` is terminal, so the clipboard
                // is written here rather than by the parent. A rewrite that
                // dropped `Sources/App.swift` reads perfectly well and there is
                // nothing left to compare it against once the panel has gone —
                // this is the only moment the suspicion is worth anything.
                Ok(out) => {
                    copy(&out.text);
                    note(&format!("copied · check it: {}", out.warnings.join("; ")));
                }
                Err(e) => {
                    note(&e);
                    return 2;
                }
            }
        }
    }
    0
}

fn first_of(it: &Item, keys: &[&str]) -> String {
    keys.iter().map(|k| it.get(k)).find(|v| !v.is_empty()).unwrap_or("").to_string()
}

/// Turn `{{port}}` into `<port>` — visible blanks you tab over and replace.
pub fn fill_placeholders(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut rest = cmd;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                out.push('<');
                out.push_str(after[..end].trim());
                out.push('>');
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Hand text to the launcher parent.
///
/// The dedicated panel parent receives its original one-line protocol because
/// it owns copy-and-close. An ordinary terminal invocation has no such parent,
/// so it copies here and emits only a `COPIED` notification for the zsh widget.
pub fn emit(verb: &str, cmd: &str) {
    // The dedicated panel parent already owns copy-and-close, so preserve its
    // protocol. At a shell there may be either the current zsh widget or one
    // loaded before this release; copy here and return a notification verb
    // both can safely ignore rather than letting an old widget insert text.
    if matches!(verb, "INSERT" | "RUN") && !env_flag("PRELUDE_FULL_SURFACE") {
        copy(cmd);
        if std::io::stdout().is_terminal() {
            eprintln!("copied: {}", crate::width::flatten(cmd));
        } else {
            println!("COPIED\t{}", crate::width::flatten(cmd));
        }
        return;
    }
    println!("{verb}\t{cmd}");
}

/// Say something went wrong, somewhere the person will actually see it.
///
/// The launcher closes the instant an action finishes, and the zsh widget
/// captures our stdout inside `$(...)` with stderr sent to /dev/null — so
/// every refusal, every "that agent has no way to borrow a skill", every
/// failed open, arrived as *nothing at all*. The window shut, the prompt was
/// unchanged, and the only thing the user learned was that the launcher is
/// unreliable. All the care taken over when to refuse was invisible.
///
/// So refusals travel the same road as results: a third verb the widget
/// knows, rendered with `zle -M`, which prints below the prompt without
/// touching what is on it and clears itself at the next keystroke. The panel
/// reads the same verb and shows it before standing down.
///
/// `MSG` is terminal — an action either did something or explains why it did
/// not, never both — which keeps the one-line contract intact.
pub fn note(text: &str) {
    // Newlines would break the widget's one-line contract.
    println!("MSG\t{}", crate::width::flatten(text));
}

pub fn copy(text: &str) {
    for tool in [vec!["pbcopy"], vec!["wl-copy"], vec!["xclip", "-selection", "clipboard"]] {
        if crate::exec::which(tool[0]).is_none() {
            continue;
        }
        let mut cmd = Command::new(tool[0]);
        cmd.args(&tool[1..]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Ok(mut child) = cmd.spawn() {
            if let Some(mut si) = child.stdin.take() {
                let _ = si.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return;
        }
    }
}

/// Run fzf over a simple list whose payload is a plain string, returning the
/// selected payload. Used by the action panel, where the payload is an id.
/// Ask for a line of free text, using the surface already in front of you.
///
/// fzf with `--print-query` over an empty list: whatever you type is the
/// answer. A second binary for a one-line prompt would be a dependency, and
/// this way the input box looks exactly like the one you just came from.
pub fn prompt_line(label: &str) -> Option<String> {
    prompt_line_initial(label, "")
}

/// The same one-line prompt, with an editable suggestion already present.
pub fn prompt_line_initial(label: &str, initial: &str) -> Option<String> {
    let accept = if initial.is_empty() { "Send" } else { "Continue" };
    let mut args = base_args("› ", label, Some(&format!("{DIM}{accept}  Enter   ·   Cancel  Esc{RESET}")));
    args.push("--print-query".into());
    args.push("--no-info".into());
    if !initial.is_empty() {
        args.push(format!("--query={initial}"));
    }
    let out = run_fzf("", args, 0);
    if out.failed {
        return None;
    }
    let line = out.query.trim().to_string();
    if line.is_empty() { None } else { Some(line) }
}

/// Ask before doing something that cannot be taken back with a keystroke.
///
/// Cancel is the *first* entry, so it is what the cursor starts on and what a
/// stray Enter chooses. A confirmation whose default is "yes" only adds a
/// keystroke to the accident it was supposed to prevent.
///
/// The question is the border label and the consequence is spelled out on the
/// row rather than implied: "Delete claude's copy" is a decision, "Are you
/// sure?" is a reflex.
pub fn confirm(question: &str, go_ahead: &str, detail: &str) -> bool {
    let tail = if detail.is_empty() { String::new() } else { format!("{DIM}· {detail}{RESET}") };
    let feed = format!(
        "{:<34}{SEP}no\n{:<34}{tail}{SEP}yes",
        "Cancel", go_ahead
    );
    pick_raw(&feed, &format!(" {question} "), "? ", "Choose  Enter →   ·   Cancel  Esc ←", "")
        .as_deref()
        == Some("yes")
}

/// `footer` names the keys; `header` is a line above the list that cannot be
/// selected — which is what makes it the right home for a statement rather
/// than an action.
pub fn pick_raw(
    feed: &str,
    label: &str,
    prompt: &str,
    footer: &str,
    header: &str,
) -> Option<String> {
    let mut args = base_args(prompt, label, Some(&format!("{DIM}{footer}{RESET}")));
    if !header.is_empty() {
        args.push(format!("--header={header}"));
    }
    // Every level of the panel answers to the same two keys. `←` and Escape
    // both back out — abort returns `None`, which each caller already reads as
    // "the person did not choose" and turns into its own kind of back — and
    // `→` chooses, which for a row with a submenu behind it means entering it.
    // Escape is not conditioned on the query: backing out is what it means
    // everywhere, and a filter typed into a modal is part of the modal.
    args.push("--bind".into());
    args.push("left:abort".into());
    args.push("--bind".into());
    args.push("right:accept".into());
    args.push("--bind".into());
    args.push(format!(
        "change:transform:[ -n {{q}} ] && echo 'unbind({ARROW_BOTH})' || echo 'rebind({ARROW_BOTH})'"
    ));
    let modes: Vec<Vec<String>> = vec![vec![]];
    for mode in modes {
        let mut cmd = Command::new("fzf");
        cmd.args(&args).args(&mode)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let Ok(mut child) = cmd.spawn() else { return None };
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(feed.as_bytes());
        }
        let Ok(out) = child.wait_with_output() else { return None };
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() && stdout.trim().is_empty() && !stderr.trim().is_empty() {
            continue;
        }
        return stdout
            .split('\n')
            .find(|l| l.contains(SEP))
            .and_then(|l| l.split_once(SEP))
            .map(|(_, id)| id.trim().to_string());
    }
    None
}

/// Ask an agent and stream the answer into the preview pane.
///
/// The point is that you asked a question, so you should get an answer where
/// you are looking — not a full-screen TUI that takes over the terminal, and
/// not a command to press Enter on again. Uses each agent's non-interactive
/// mode so the reply is plain text on stdout.
pub fn ask(item: &Item) -> i32 {
    use std::io::{BufRead, BufReader, Write};
    let agent = item.get("agent");
    let prompt = item.get("prompt");
    if agent.is_empty() || prompt.is_empty() {
        println!("nothing to ask");
        return 1;
    }
    let Some(args) = crate::sources::sessions::ask_cmd(agent, prompt) else {
        println!("{YELLOW}don't know how to run {agent} non-interactively{RESET}");
        return 1;
    };

    println!("{DIM}asking {agent}…{RESET}\n");
    let _ = std::io::stdout().flush();

    let mut cmd = Command::new(&args[0]);
    // stderr is kept: discarding it turned every failure into a silent,
    // permanent "asking…" with no clue why. Agents also refuse for ordinary
    // reasons — not a git repo, not logged in — and you need to see that.
    cmd.args(&args[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = item.data.get("cwd").filter(|d| std::path::Path::new(d).is_dir()) {
        cmd.current_dir(dir);
    }
    let Ok(mut child) = cmd.spawn() else {
        println!("{YELLOW}could not run {agent}{RESET}");
        return 1;
    };
    // Drain stderr on a helper thread; a child that fills its stderr pipe
    // while we read stdout would deadlock.
    let errs = child.stderr.take().map(|e| {
        std::thread::spawn(move || {
            BufReader::new(e).lines().map_while(Result::ok).collect::<Vec<_>>()
        })
    });

    // Stream stdout, so a slow answer appears as it arrives rather than all
    // at the end.
    let mut any = false;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            any = true;
            println!("{line}");
            let _ = std::io::stdout().flush();
        }
    }
    let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
    let errs: Vec<String> = errs.and_then(|h| h.join().ok()).unwrap_or_default();

    if !any {
        // Say what went wrong rather than leaving "asking…" on screen.
        println!("{YELLOW}{agent} returned nothing (exit {code}){RESET}");
        for l in errs.iter().rev().take(6).rev() {
            println!("{DIM}{l}{RESET}");
        }
    }
    0
}
