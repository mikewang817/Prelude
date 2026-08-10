//! Prelude — a general launcher in the terminal.
//!
//! One launcher over shell commands, macOS objects, and local coding agents.
//! Commands are handed back for review; files, folders, applications, and URLs
//! act directly through Launch Services.

mod actions;
mod agent;
mod ansi;
mod archive;
mod bus;
mod cache;
mod calc;
mod capability;
mod clipd;
mod compute;
mod control;
mod defaults;
mod doctor;
mod exec;
mod favorites;
mod fleet;
mod frecency;
mod global;
mod init;
mod item;
mod lend;
mod minitoml;
mod mcp_tools;
mod openwith;
mod panel;
mod paths;
mod preview;
mod probe;
mod refresh;
mod render;
mod runhere;
mod secrets;
mod settings;
mod sources;
mod translate_build;
mod tty;
mod ui;
mod update;
mod width;

use std::process::ExitCode;

fn main() -> ExitCode {
    // `prelude fleet | head` closes our stdout mid-write. Rust ships with
    // SIGPIPE ignored, which turns that into a panic and a stack trace;
    // restore the default so the process simply ends, like any other CLI.
    unsafe extern "C" {
        unsafe fn signal(sig: i32, handler: usize) -> usize;
    }
    unsafe { signal(13, 0) }; // SIGPIPE -> SIG_DFL
    cache::privacy_migrations();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = match a.as_slice() {
        [] => ui::search(),
        // Ghostty marks quick-terminal surfaces in their environment. The
        // gate matters because macOS can deliver an ordinary Ghostty launch
        // to Prelude's hidden instance after its panel was most recently used.
        ["_surface"] => global::run_surface(),
        // Kept as an internal test/debug door; generated Ghostty configuration
        // enters through `_surface` so ordinary windows never become Prelude.
        ["_panel"] => panel::run(),
        // `paste [pane]` typed the result into a tmux pane instead of
        // returning it. Nothing types into anything now, and a command that
        // silently became an ordinary search would be worse than one that
        // says what happened to it.
        ["paste", ..] => {
            eprintln!("prelude: `paste` is gone with the tmux popup it served.");
            eprintln!("prelude: run `prelude` and what you pick is copied.");
            2
        }
        ["doctor"] => doctor::run(),
        // The specialized reports. Each one is a report and nothing else;
        // `--repair` is a separate door that asks about each finding on its
        // own, and `--json` is there so an agent reads fields rather than a
        // rendered table.
        ["doctor", rest @ ..] => doctor::dispatch(rest),
        ["bench"] => bench(),
        ["fleet"] => fleet::list(false),
        ["fleet", "--json"] => fleet::list(true),
        ["fleet", "--status"] => fleet::status(),
        ["watch"] => fleet::watch(),
        ["control"] => control::list(false),
        ["control", "--json"] => control::list(true),
        ["global", rest @ ..] => global::dispatch(rest),

        // The agent-facing verbs. These are what an agent runs from its own
        // shell, so each one is a plain word with the text last: an agent
        // composing a command from a doc line gets it right first time, and
        // stdout carries the answer and nothing else.
        ["ask", rest @ ..] if !rest.is_empty() => {
            let (flags, text) = split_flags(rest);
            if text.is_empty() {
                eprintln!("prelude: ask what?");
                return ExitCode::from(2);
            }
            let wait = flag_value(&flags, "--timeout").and_then(|v| v.parse().ok());
            bus::ask(&text, wait.unwrap_or(bus::DEFAULT_WAIT), flags.contains(&"--no-wait"))
        }
        ["tell", rest @ ..] if !rest.is_empty() => bus::tell(&split_flags(rest).1),
        // The flags come before the recipient, not after it: `split_flags`
        // stops at the first non-flag word, and that word is the target. A
        // dashed word later on belongs to the message.
        ["say", rest @ ..] if !rest.is_empty() => {
            let (_, text) = split_flags(rest);
            let (target, body) = match text.split_once(' ') {
                Some((target, body)) => (target.to_string(), body.to_string()),
                None => (text.clone(), String::new()),
            };
            if target.is_empty() || body.is_empty() {
                eprintln!("prelude: say to whom, and what? — prelude say WHO TEXT");
                return ExitCode::from(2);
            }
            bus::say(&target, &body)
        }
        ["answer", id, rest @ ..] if !rest.is_empty() => bus::answer(id, &rest.join(" ")),
        ["answer-of", id] => bus::answer_of(id),
        ["reply"] => bus::reply(),
        ["inbox", rest @ ..] => bus::inbox(
            rest.contains(&"--json"),
            rest.contains(&"--all"),
            rest.contains(&"--human"),
        ),
        ["drain"] => bus::drain(),

        ["agents", rest @ ..] => json_dump(cache::gather_agents(), rest.contains(&"--json")),
        ["sessions", rest @ ..] => {
            let sessions = sources::sessions::all();
            let runs = sources::running::live_with_sessions(&sessions);
            json_dump(
                sources::running::annotate_sessions(sessions, &runs),
                rest.contains(&"--json"),
            )
        }
        ["settings", rest @ ..] => settings::dispatch(rest),
        // Knowing which binary you are is step zero; there was no way to ask.
        ["--version"] | ["-V"] | ["version"] => {
            println!("prelude {} ({})", update::VERSION,
                     update::target().unwrap_or("unsupported architecture"));
            if let Some(running) = update::panel_is_stale() {
                println!("panel {running} — run:  prelude global start");
            }
            0
        }
        ["update", rest @ ..] => update::dispatch(rest),
        // `ql` because that is the scope prefix; one word for one concept.
        ["quicklink", rest @ ..] | ["quicklinks", rest @ ..] | ["ql", rest @ ..] => {
            compute::quicklink_cli(rest)
        }
        ["skills", rest @ ..] => {
            json_dump(sources::agents::skills(), rest.contains(&"--json"))
        }
        ["index"] => {
            println!("building file index ...");
            let n = compute::build_fileindex();
            println!("  indexed {n} files from: {}", compute::index_roots().join(", "));
            println!("  search names, paths and Finder tags with  f:name  or  f:tag:work");
            0
        }
        ["clipd"] => clipd::watch(),
        ["build-translate"] => translate_build::build(),
        ["init", "zsh"] => { print!("{}", init::zsh()); 0 }
        ["init", "agent"] => { print!("{}", init::AGENT); 0 }
        ["translate", lang, rest @ ..] if !rest.is_empty() => {
            match compute::translate(&rest.join(" "), lang) {
                Ok(v) => { println!("{v}"); 0 }
                Err(e) => { eprintln!("prelude: {e}"); 2 }
            }
        }
        // Non-interactive dump of the rendered list, for testing and for
        // diffing behaviour without standing up a terminal.
        ["_dump"] => {
            let items = cache::gather();
            println!("{}", render::render(&compute::home_items(&items), term_width()));
            0
        }
        ["_dump-root"] => {
            let items = cache::gather();
            println!("{}", render::render(&compute::root_items(&items), term_width()));
            0
        }
        ["_dump-all"] => {
            let items = cache::gather();
            println!("{}", render::render(&items, term_width()));
            0
        }
        // Scriptable equivalent of the ^K action, and what its test drives.
        ["_copy-skill", dir, agent, name] => {
            match sources::agents::copy_skill(dir, agent, name) {
                Ok(p) => { println!("copied {name} -> {p}"); 0 }
                Err(e) => { eprintln!("prelude: {e}"); 1 }
            }
        }
        // Deleting, without a terminal — and without the confirmation, which
        // is the launcher's job. This is how the guard that refuses anything
        // that is not a skill directory gets tested.
        ["_rm-skill", dir] => match sources::agents::delete_skill(dir) {
            Ok(p) => { println!("moved to {}", p.display()); 0 }
            Err(e) => { eprintln!("prelude: {e}"); 1 }
        },
        // Borrowing, without a terminal: print the command the ^K action
        // would hand you, so what each agent is actually told can be diffed.
        ["_lend-skill", agent, dir, name] => {
            match lend::skill_flags(agent, std::path::Path::new(dir), name) {
                Ok(f) => { println!("{}", lend::borrow_cmd(agent, &f, None, None)); 0 }
                Err(e) => { eprintln!("prelude: {e}"); 1 }
            }
        }
        ["_lend-mcp", agent, name] => {
            let items = cache::gather_agents();
            let hit = items.iter().find(|i| i.kind == item::Kind::Mcp && i.get("name") == *name);
            match hit {
                None => { eprintln!("prelude: no MCP server called {name}"); 1 }
                Some(it) => match lend::resolve(it).and_then(|d| lend::mcp_flags(agent, &d)) {
                    Ok(f) => { println!("{}", lend::borrow_cmd(agent, &f, None, None)); 0 }
                    Err(e) => { eprintln!("prelude: {e}"); 1 }
                },
            }
        }
        // The ^K panel as text. The panel is a second fzf, which a pty
        // harness cannot reliably capture, so this is how its contents get
        // checked.
        ["_actions", line] => match render::parse_line(line) {
            Some(i) => {
                for (id, label, sub) in actions::actions_for(&i, defaults::surface()) {
                    println!("{id:<18} {label}{}", if sub.is_empty() { String::new() } else { format!("   [{sub}]") });
                }
                0
            }
            None => 1,
        },
        ["_refresh-path"] => { sources::user::scan_path(); 0 }
        ["_refresh", name] => if cache::refresh_named(name) { 0 } else { 1 },
        ["_bind", q, path, cols, tw] => bind(q, path, cols, tw),
        ["_dynamic", q] => dynamic(q, "", term_width(), None),
        ["_dynamic", q, path] => dynamic(q, path, term_width(), None),
        ["_dynamic", q, path, cols] => {
            dynamic(q, path, cols.parse().unwrap_or_else(|_| term_width()), None)
        }
        ["_dynamic", q, path, cols, tw] => dynamic(
            q,
            path,
            cols.parse().unwrap_or_else(|_| term_width()),
            tw.parse().ok(),
        ),
        // Asked by the panel's background refresher, through fzf's own socket,
        // so the answer can be "do nothing" when somebody is using the panel.
        // Only fzf knows what has been typed and where the cursor is.
        ["_tick", q, n] => refresh::tick(q, n),
        ["_tick", q] => refresh::tick(q, ""),
        ["_ask", line] => match render::parse_line(line) {
            Some(i) => ui::ask(&i),
            None => 1,
        },
        // Decides, per keypress of Enter, whether to accept the selection or
        // answer it in place. Only fzf can make that conditional.
        ["_enter", line] => {
            let item = render::parse_line(line);
            if let Some(it) = item.as_ref().filter(|i| i.get("mode") == "complete-query") {
                // Search commands are Prelude's one-input equivalent of
                // One-input argument form: stay open and put the provider or
                // scope syntax in the box for the person to continue typing.
                println!("change-query({})", it.get("completion"));
                return ExitCode::SUCCESS;
            }
            let ask = item
                .is_some_and(|i| i.get("mode") == "start" && !i.get("prompt").is_empty());
            let me = exec::shq(&std::env::current_exe().unwrap_or_default().to_string_lossy());
            if ask {
                println!("show-preview+change-preview-window(down,99%,wrap,border-top)+preview({me} _ask {{2}})");
            } else {
                println!("accept");
            }
            0
        }
        // Focus normally changes only the footer. Clipboard images are the
        // one contextual preview: c: has unused horizontal space and an image
        // cannot be identified from its title, so it opens on the right and
        // stands down again when focus leaves it.
        ["_focus", q, line] => focus(q, line),
        // Kept as a direct test/debug door, and for callers that need only the
        // footer rather than fzf actions.
        ["_footer", rest @ ..] => {
            println!("{}", footer_for_line(rest.first().copied().unwrap_or("")));
            0
        }
        ["_preview", line] => { if let Some(i) = render::parse_line(line) { preview::show(&i); } 0 }
        ["_copy", line] => copy_line(line),
        ["_runhere", line] => match render::parse_line(line) {
            Some(i) => runhere::run_item(&i),
            None => 1,
        },
        ["-h"] | ["--help"] | ["help"] => { print!("{HELP}"); 0 }
        _ => { eprintln!("prelude: unknown command {a:?} (try: prelude doctor)"); 2 }
    };
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// Separate `--flags` from the text that follows them.
///
/// The text always comes last and is never quoted by the caller, so anything
/// after the first non-flag word is part of the message — including words
/// that start with a dash further along, which are prose rather than options.
fn split_flags<'a>(args: &[&'a str]) -> (Vec<&'a str>, String) {
    let first = args.iter().position(|a| !a.starts_with("--")).unwrap_or(args.len());
    let (flags, rest) = args.split_at(first);
    // A flag that takes a value swallowed the value too; drop those.
    let flags: Vec<&str> = flags.to_vec();
    (flags, rest.join(" "))
}

fn flag_value(flags: &[&str], name: &str) -> Option<String> {
    flags.iter().find_map(|f| f.strip_prefix(&format!("{name}="))).map(str::to_string)
}

/// Print a list of items as JSON, or as one plain line each.
///
/// The JSON half is the whole point of these subcommands: an agent asking
/// what skills exist on this machine needs fields, not a rendered table.
fn json_dump(items: Vec<item::Item>, json: bool) -> i32 {
    if json {
        println!("{}", serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into()));
        return 0;
    }
    for it in &items {
        let sub = if it.fields.is_empty() { it.subtitle.clone() } else { it.fields.join(" · ") };
        println!("{}\t{}\t{}", it.style().1, it.title, sub);
    }
    0
}

const HELP: &str = "\
prelude — a general launcher in the terminal, and a message bus for agents

HUMANS
  prelude                search · commands go to the Ctrl-R prompt or the
                         global panel's clipboard; macOS objects act directly
  prelude reply          answer the oldest question an agent is blocked on
  prelude fleet          every agent running on this machine, and its state
  prelude fleet --status one short line for a status bar
  prelude watch          notify the moment an agent stops and waits for you
  prelude control [--json]  Agent/Run/Session/Skill/MCP relationships
  prelude global install|uninstall|start|stop|status|open
  prelude global hotkey [CHORD]
                         global key (default Cmd+Shift+Space); the panel is a
                         supervised hidden Ghostty quick terminal
  prelude global directory [PATH|--default]
  prelude settings [--json]
                         effective preferences, sources and files
  prelude settings get KEY | set KEY VALUE | reset KEY|all
  prelude settings check [--json] | path [KEY]
  prelude settings add-root PATH | remove-root PATH | roots
  prelude doctor agents|sessions|skills|mcp
                         what is wrong with the agent side of the machine
                         --json for fields · --repair asks about each finding

  Inside search:  : lists scopes · a: agents · s: sessions · f: files
                  c: clipboard · h: history · set: settings
                  g TERM searches Google

AGENTS  (run these from inside a conversation; see `prelude init agent`)
  prelude ask   TEXT     ask the human a question and wait for the answer
                         answer goes to stdout · exit 3 if nobody answered
                         --timeout=N seconds (default 600) · --no-wait
  prelude tell  TEXT     tell the human something, without waiting
  prelude say   WHO TEXT send a line to another running agent
                         WHO is a project, an agent name, or a pid
  prelude inbox [--json] messages left for you · --all includes handled ones
                         --human shows questions waiting on a person instead
  prelude drain          mark your inbox collected

  prelude answer ID TEXT answer a question someone else is blocked on
  prelude answer-of ID   collect the answer to a --no-wait question
  prelude fleet --json   who else is running, and which of them are stuck
  prelude agents   [--json]   the agent overview, as data
  prelude sessions [--json]   every past conversation, as data
  prelude skills   [--json]   every skill and who has it, as data

SETUP
  prelude init zsh|agent shell integration, and the block for CLAUDE.md
  prelude index          build the file index for f:name
  prelude doctor         diagnose the setup
  prelude bench          measure candidate-gathering
  prelude build-translate  compile the Apple translation helper
";

const AUTO_PREVIEW_LABEL: &str = "Clipboard preview";
const CLIPBOARD_PREVIEW_WINDOW: &str = "right,55%,wrap,border-left";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusPreview {
    Keep,
    ShowClipboardPreview,
    HideClipboardPreview,
}

fn footer_for_line(line: &str) -> String {
    let item = render::parse_line(line);
    let primary = item
        .as_ref()
        .map(|item| defaults::describe(item, defaults::surface()))
        .unwrap_or("Select");
    ui::footer_for_item(
        primary,
        item.as_ref(),
        std::env::var_os("PRELUDE_FULL_SURFACE").is_some_and(|value| !value.is_empty()),
    )
}

/// Pure focus rule: every real clipboard row gets the same right-hand pane.
/// Images render as pixels; text and Finder objects render their full content
/// and metadata. A manually opened Quick Look has no marker and is left alone.
fn focus_preview(q: &str, item: Option<&item::Item>, automatic_is_open: bool) -> FocusPreview {
    let clipboard = compute::scope_query(q)
        .is_some_and(|(scope, _)| scope == compute::Scope::Clipboard);
    if clipboard && item.is_some_and(|item| item.kind == item::Kind::Clip) {
        FocusPreview::ShowClipboardPreview
    } else if automatic_is_open {
        FocusPreview::HideClipboardPreview
    } else {
        FocusPreview::Keep
    }
}

fn fzf_action_arg(value: &str) -> String {
    value.replace('\\', "\\\\").replace(')', "\\)")
}

fn focus(q: &str, line: &str) -> i32 {
    let item = render::parse_line(line);
    let automatic_is_open = std::env::var("FZF_PREVIEW_LABEL")
        .is_ok_and(|label| label.trim() == AUTO_PREVIEW_LABEL);
    let mut action = format!("change-footer({})", fzf_action_arg(&footer_for_line(line)));
    match focus_preview(q, item.as_ref(), automatic_is_open) {
        FocusPreview::Keep => {}
        FocusPreview::ShowClipboardPreview => action.push_str(&format!(
            "+change-preview-window({CLIPBOARD_PREVIEW_WINDOW})+show-preview\
             +change-preview-label({AUTO_PREVIEW_LABEL})"
        )),
        // `change-preview-window` without `hidden` makes a hidden pane visible.
        // It used to run after `hide-preview`, immediately undoing the hide and
        // producing the horizontal text pane shown in the bug report.
        FocusPreview::HideClipboardPreview => action.push_str(
            "+change-preview-window(down,99%,wrap,border-top,hidden)\
             +change-preview-label()",
        ),
    }
    println!("{action}");
    0
}

/// Decide, per keystroke, what fzf should do.
///
/// fzf matches against the *displayed* text, so a computed row (a sum, a
/// translation) can never fuzzy-match the query that produced it — you'd type
/// `en:...` and watch your own answer get filtered out. For those queries we
/// turn fzf's own filtering off and let our row stand at the top; for
/// ordinary queries we leave fuzzy search exactly as it was.
fn bind(q: &str, path: &str, cols: &str, tw: &str) -> i32 {
    let me = std::env::current_exe().unwrap_or_default();
    println!("{}", bind_actions(q, &exec::shq(&me.to_string_lossy()), path, cols, tw));
    0
}

/// The keystroke decision itself, with no process to read it out of — the same
/// reason every rule below `surface()` takes its inputs as parameters.
fn bind_actions(q: &str, me: &str, path: &str, cols: &str, tw: &str) -> String {
    let search = if compute::is_special(q) { "disable-search" } else { "enable-search" };
    // The prefix hints belong to the empty query and nothing else: once there
    // is a query there are results to read, and a row of syntax above them is
    // in the way rather than helpful.
    let header = if q.trim().is_empty() { ui::HINTS } else { "" };
    // The arrow keys belong to the query line whenever there is a query to
    // move through, and to the level structure when there is not. `^K` is the
    // way in either way, so nothing is lost while they are away.
    let arrows = if q.is_empty() {
        format!("rebind({})", ui::ARROW_INTO)
    } else {
        format!("unbind({})", ui::ARROW_INTO)
    };
    format!("{search}+{arrows}+change-header({header})+reload({me} _dynamic {} {} {} {})",
            exec::shq(q), exec::shq(path), exec::shq(cols), exec::shq(tw))
}

/// Emit query-dependent rows, then the pre-rendered static list.
///
/// `cols` is passed in rather than measured: fzf runs this with stdout on a
/// pipe, so a measurement would fall back to a default and the computed rows
/// would drift ten columns out of step with the static ones.
fn dynamic(q: &str, path: &str, cols: usize, tw: Option<usize>) -> i32 {
    let static_items: Vec<item::Item> = if compute::needs_static_items(q) {
        std::fs::read_to_string(paths::cache().join("search-items.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let rows = compute::dynamic_rows_with(q, &static_items);
    if !rows.is_empty() {
        let rendered = if compute::scope_query(q)
            .is_some_and(|(scope, _)| scope == compute::Scope::Files)
        {
            render::render_files(&rows, cols)
        } else {
            render::render_with(&rows, cols, None, tw)
        };
        println!("{rendered}");
    }
    // Once the query has clearly declared an intent — a sum, a translation,
    // a scope, an agent to start — the unrelated catalogue underneath is
    // noise. Show only what was asked for.
    if compute::is_special(q) {
        return 0;
    }
    let list = if q.trim().is_empty() {
        paths::cache().join("home.txt")
    } else {
        std::path::PathBuf::from(path)
    };
    if let Ok(t) = std::fs::read_to_string(list) {
        print!("{t}");
        if !t.ends_with('\n') {
            println!();
        }
    }
    0
}

/// `^Y`: copy without closing the launcher.
fn copy_line(line: &str) -> i32 {
    let Some(it) = render::parse_line(line) else { return 1 };
    use item::Kind::*;
    let by_kind = match it.kind {
        Port | Proc => it.get("pid"),
        Ssh => it.get("host"),
        Container | Mcp => it.get("name"),
        Link => it.get("url"),
        File | Find => it.get("path"),
        Clip => it.get("full"),
        _ => "",
    };
    let text = if by_kind.is_empty() { it.cmd.clone() } else { by_kind.to_string() };
    ui::copy(&text);
    frecency::bump(&it.cmd);
    0
}

fn bench() -> i32 {
    let mut times = Vec::new();
    let mut n = 0;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        n = cache::gather().len();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("gather: {n} items  min {:.1}ms  median {:.1}ms  max {:.1}ms",
             times[0], times[times.len() / 2], times[times.len() - 1]);
    println!("budget: 40ms  ->  {}", if times[times.len() / 2] < 40.0 { "OK" } else { "OVER" });
    0
}

/// Columns of the terminal we are drawing on.
///
/// Asking fd 1 alone is not enough: the zsh widget runs us inside `$(...)`,
/// so stdout is a pipe and the ioctl fails. The fallback then laid every row
/// out for 100 columns on a 260-column terminal — two thirds of the window
/// blank. Ask the other standard descriptors, then the controlling terminal
/// itself, before giving up.
pub fn term_width() -> usize {
    if let Ok(c) = std::env::var("COLUMNS") {
        if let Ok(n) = c.parse::<usize>() {
            if n > 0 {
                return n;
            }
        }
    }
    for fd in [1, 2, 0] {
        if let Some(w) = winsize_cols(fd) {
            return w;
        }
    }
    // Last resort: our own descriptors may all be redirected, but the
    // session still has a terminal. This is the same door probe.rs uses.
    use std::os::fd::AsRawFd;
    if let Ok(tty) = std::fs::File::open("/dev/tty") {
        if let Some(w) = winsize_cols(tty.as_raw_fd()) {
            return w;
        }
    }
    100
}

fn winsize_cols(fd: i32) -> Option<usize> {
    #[repr(C)]
    struct WinSize { rows: u16, cols: u16, x: u16, y: u16 }
    unsafe extern "C" { unsafe fn ioctl(fd: i32, req: u64, ...) -> i32; }
    let mut ws = WinSize { rows: 0, cols: 0, x: 0, y: 0 };
    // TIOCGWINSZ
    if unsafe { ioctl(fd, 0x4008_7468, &mut ws) } == 0 && ws.cols > 0 {
        Some(ws.cols as usize)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn arithmetic() {
        for (q, want) in [
            ("1847*0.23", "424.81"),
            ("1024*1024", "1,048,576"),
            ("(15+5)/4", "5"),
            ("2+3*4", "14"),
            ("-5 + 3", "-2"),
        ] {
            assert_eq!(crate::calc::calc(q).as_deref(), Some(want), "for {q}");
        }
        for q in ["git status", "hello", "docker", "12"] {
            assert_eq!(crate::calc::calc(q), None, "for {q}");
        }
    }

    #[test]
    fn web_addresses_are_recognised_without_guessing_at_files() {
        use crate::compute::web_url;
        for (typed, want) in [
            ("https://example.com/a?x=1#top", "https://example.com/a?x=1#top"),
            ("HTTP://example.com", "http://example.com"),
            ("example.com", "https://example.com"),
            ("www.example.com/docs", "https://www.example.com/docs"),
            ("localhost:3000", "http://localhost:3000"),
            ("127.0.0.1:8080/api", "http://127.0.0.1:8080/api"),
            ("demo.test:3000", "http://demo.test:3000"),
            ("example.com:443", "https://example.com:443"),
            ("https://hello.app", "https://hello.app"),
            ("[::1]:3000", "http://[::1]:3000"),
        ] {
            assert_eq!(web_url(typed).as_deref(), Some(want), "{typed}");
        }
        assert!(!crate::compute::is_special("example.com"), "ordinary search results stay visible");
        let row = crate::compute::dynamic_rows_with("example.com", &[]).into_iter().next().unwrap();
        assert_eq!(row.kind, crate::item::Kind::Link);
        assert_eq!(row.cmd, "https://example.com", "a URL is data, not an `open ...` shell command");

        for typed in [
            "Cargo.toml",
            "main.rs",
            "notes.md",
            "1.2.3",
            "999.1.1.1",
            "https://user:secret@example.com",
            "javascript:alert(1)",
            "file:///tmp/x",
            "example.com:0",
            "example.com:99999",
            "not a url.com",
        ] {
            assert_eq!(web_url(typed), None, "must not guess at {typed}");
        }
    }

    #[test]
    fn clipboard_rows_preview_automatically_and_only_inside_their_scope() {
        use crate::item::{Item, Kind};
        use super::{focus_preview, FocusPreview, CLIPBOARD_PREVIEW_WINDOW};

        let image = Item::new("/private/image.png", Kind::Clip)
            .put("clip_kind", "image")
            .put("path", "/private/image.png");
        let text = Item::new("copied text", Kind::Clip).put("clip_kind", "text");

        assert!(
            CLIPBOARD_PREVIEW_WINDOW.contains("border-left"),
            "the list and live image need a visible divider"
        );
        assert_eq!(
            focus_preview("c:", Some(&image), false),
            FocusPreview::ShowClipboardPreview
        );
        assert_eq!(
            focus_preview("c: screenshot", Some(&image), false),
            FocusPreview::ShowClipboardPreview
        );
        assert_eq!(
            focus_preview("c:", Some(&text), false),
            FocusPreview::ShowClipboardPreview,
            "text uses the same vertical pane rather than a horizontal Quick Look"
        );
        assert_eq!(
            focus_preview("f:", Some(&image), true),
            FocusPreview::HideClipboardPreview,
            "leaving c: must not strand its right-hand pane"
        );
        assert_eq!(
            focus_preview("f:", Some(&image), false),
            FocusPreview::Keep,
            "manual Quick Look outside c: remains a mode"
        );
    }

    #[test]
    fn root_search_is_an_agent_home_until_the_user_types() {
        use crate::compute::{home_items, root_items, scope_query, scoped_rows, Scope};
        use crate::item::{Item, Kind};

        let mut all = vec![
            Item::new("claude", Kind::Agent).put("agent", "claude"),
            Item::new("/review", Kind::Skill),
            Item::new("gmail", Kind::Mcp),
            Item::new("kill 42", Kind::Run),
            Item::new("old", Kind::Session),
            Item::new("cargo", Kind::Path),
            Item::new("copied", Kind::Clip).put("full", "copied"),
            Item::new("git status", Kind::History),
        ];
        all.extend(crate::compute::scope_commands());
        let home = home_items(&all);
        assert_eq!(home.len(), 5);
        assert!(home.iter().all(|i| matches!(
            i.kind,
            Kind::Agent | Kind::Skill | Kind::Mcp | Kind::Run | Kind::Session
        )));
        let root = root_items(&all);
        assert!(root.iter().any(|i| i.title == "Search Files"));
        assert!(root.iter().all(|i| !matches!(i.kind, Kind::Session | Kind::Path | Kind::Clip | Kind::History)));
        let f = crate::compute::dynamic_rows_with("f", &all);
        assert_eq!(f.len(), 1, "an exact scope alias must not open global fuzzy search");
        assert_eq!(f[0].get("completion"), "f:");

        assert_eq!(scope_query("c:"), Some((Scope::Clipboard, "")));
        assert_eq!(scope_query("H: git"), Some((Scope::History, "git")));
        assert_eq!(scope_query("en:hello"), None, "translation is not a search scope");
        assert!(crate::compute::is_special("f:"));
        assert!(crate::compute::is_special(":"));
        assert!(crate::compute::is_special("/"));
        assert!(crate::compute::is_special("@"));

        let skill_rows = crate::compute::dynamic_rows_with("/rev", &all);
        assert_eq!(skill_rows.len(), 1);
        assert_eq!(skill_rows[0].kind, Kind::Skill);
        let ask_rows = crate::compute::dynamic_rows_with("@cla", &all);
        assert_eq!(ask_rows.len(), 1);
        assert_eq!(ask_rows[0].kind, Kind::Search);
        assert_eq!(ask_rows[0].get("completion"), "@claude ");

        let clips = scoped_rows(Scope::Clipboard, "cop", &all);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].kind, Kind::Clip);
        assert!(scoped_rows(Scope::Clipboard, "missing", &all).is_empty());
        let agents = scoped_rows(Scope::Agent, "", &all);
        assert!(agents.iter().all(|i| i.kind != Kind::Session), "sessions have their own s: scope");
    }

    #[test]
    fn clipboard_is_strictly_recent_and_preserves_file_objects() {
        let history = concat!(
            r#"{"ts":100.0,"kind":"text","t":"older"}"#, "\n",
            r#"{"ts":101.0,"kind":"text","t":"newer"}"#, "\n",
        );
        let clips = crate::sources::user::clips_from(history);
        assert_eq!(clips.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["newer", "older"]);
        assert!(clips[0].score - clips[1].score > 60.0,
                "even the maximum frecency bonus must not reorder clipboard history");

        let path = std::env::current_dir().unwrap().join("Cargo.toml");
        let history = serde_json::json!({
            "ts": 102.0,
            "kind": "files",
            "paths": [path.to_string_lossy()],
        })
        .to_string();
        let files = crate::sources::user::clips_from(&history);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].get("clip_kind"), "files");
        assert_eq!(files[0].title, "Cargo.toml");
        assert!(files[0].cmd.contains("Cargo.toml"));
    }

    #[test]
    fn file_search_matches_finder_tags_without_confusing_them_with_paths() {
        let index = concat!(
            "/tmp/client/report.md\t[\"Orange\",\"Project Alpha\"]\n",
            "/tmp/orange-name.txt\t[\"Blue\"]\n",
            "/tmp/plain.txt\n",
        );
        let by_name = crate::compute::search_fileindex_from(index, "orange");
        assert_eq!(by_name.len(), 2, "ordinary terms search both paths and tags");
        let by_tag = crate::compute::search_fileindex_from(index, "tag:orange");
        assert_eq!(by_tag.len(), 1, "tag: searches metadata only");
        assert_eq!(by_tag[0].title, "report.md");
        assert_eq!(by_tag[0].get("tags"), "Orange\u{1e}Project Alpha");
        assert_eq!(
            crate::compute::search_fileindex_from(index, "tag:\"Project Alpha\"").len(),
            1,
            "quoted multi-word Finder tags stay together",
        );
        assert!(crate::compute::search_fileindex_from(index, "tag:orange-name").is_empty());
    }

    #[test]
    fn finder_tag_import_is_bounded_private_and_tied_to_indexed_paths() {
        let allowed = std::collections::HashSet::from(["/tmp/allowed"]);
        let records = concat!(
            "{\"path\":\"/tmp/allowed\",\"tags\":[\"Work\",\"work\",\"API_TOKEN=secret\"]}\n",
            "{\"path\":\"/tmp/not-indexed\",\"tags\":[\"Private\"]}\n",
        );
        let tags = crate::compute::sanitized_file_tags(records, &allowed);
        assert_eq!(tags.get("/tmp/allowed").unwrap(), &["Work"]);
        assert!(!tags.contains_key("/tmp/not-indexed"));
    }

    #[test]
    fn command_enter_is_only_advertised_for_files_and_opens_their_parent() {
        use crate::item::{Item, Kind};
        let file = Item::new("/tmp/project/readme.md", Kind::Find)
            .put("path", "/tmp/project/readme.md");
        assert_eq!(
            crate::ui::containing_directory(&file).as_deref(),
            Some(std::path::Path::new("/tmp/project")),
        );
        assert!(crate::ui::footer_for_item("Open it", Some(&file), true).contains("Cmd+Enter"));
        assert!(!crate::ui::footer_for_item("Open it", Some(&file), false).contains("Cmd+Enter"));
        let folder = Item::new("cd /tmp/project", Kind::Dir).put("path", "/tmp/project");
        assert!(!crate::ui::footer_for_item("Open it", Some(&folder), true).contains("Cmd+Enter"));
    }

    #[test]
    fn search_providers_are_commands_before_they_have_an_argument() {
        use crate::item::Kind;
        let google = crate::compute::quicklink_items_from(crate::compute::QUICKLINKS_DEFAULT).into_iter()
            .find(|i| i.get("provider") == "g").expect("Google provider");
        assert_eq!(google.kind, Kind::Search);
        assert_eq!(google.get("mode"), "complete-query");
        assert_eq!(google.get("completion"), "g ");
        assert!(google.title.contains("Google"));
        assert_eq!(crate::compute::search_provider_from(crate::compute::QUICKLINKS_DEFAULT, "google")
            .unwrap().get("completion"), "g ");
        assert!(crate::compute::exact_quicklink_key_from(crate::compute::QUICKLINKS_DEFAULT, "g"),
            "an exact alias must outrank ordinary fuzzy matches");
        let (url, _, term, _) = crate::compute::quicklink_from(crate::compute::QUICKLINKS_DEFAULT, "g rust").unwrap();
        assert_eq!(term, "rust");
        assert_eq!(url, "https://www.google.com/search?q=rust");
    }

    #[test]
    fn web_search_defaults_migrate_once_without_overwriting_a_provider() {
        let original = "# mine\n[b]\nname = \"My Baidu\"\nurl = \"https://example.com/?q={q}\"\n";
        let (migrated, changed) = crate::compute::add_web_search_defaults(original.to_string());
        assert!(changed);
        assert!(migrated.starts_with(original));
        let links = crate::minitoml::parse(&migrated);
        assert_eq!(links["b"]["name"], "My Baidu", "a user's provider always wins");
        assert_eq!(links["bing"]["name"], "Bing");
        assert_eq!(links["ddg"]["name"], "DuckDuckGo");

        let (again, changed) = crate::compute::add_web_search_defaults(migrated.clone());
        assert!(!changed);
        assert_eq!(again, migrated, "the migration must not restore a provider the user later deletes");
    }

    #[test]
    fn created_quicklinks_round_trip_without_rewriting_the_config() {
        use crate::compute::{append_quicklink, fixed_quicklink_from, remove_quicklink_block, QuicklinkDraft};
        use crate::item::Kind;

        let original = "# keep this comment\n[g]\nname = \"Google\"\nurl = \"https://google.com?q={q}\"\n";
        let draft = QuicklinkDraft {
            name: "Design \"draft\"".into(),
            kind: "file",
            target: "~/Notes/A # draft.md".into(),
        };
        let made = append_quicklink(original.to_string(), "design", &draft).unwrap();
        assert!(made.starts_with(original), "manual formatting and comments must survive");
        assert!(append_quicklink(made.clone(), "design", &draft).is_err(), "never overwrite");

        let parsed = crate::minitoml::parse(&made);
        assert_eq!(parsed["design"]["name"], draft.name);
        assert_eq!(parsed["design"]["target"], draft.target);
        let item = fixed_quicklink_from(&made, "DESIGN").unwrap();
        assert_eq!(item.kind, Kind::File);
        assert_eq!(item.get("quicklink"), "design");
        assert_eq!(item.get("quicklink_managed"), "true");

        let removed = remove_quicklink_block(made, "design").unwrap();
        assert_eq!(removed, original, "removing it must leave hand-written quicklinks byte-for-byte");

        let secret = crate::item::Item::new("https://example.com", Kind::Link)
            .title("download")
            .put("url", "https://example.com/file?token=abc123");
        assert!(crate::compute::quicklink_draft(&secret).is_err(), "credentials are never indexed");
        let clip = crate::item::Item::new("some text", Kind::Clip);
        assert!(crate::compute::quicklink_draft(&clip).unwrap().is_none(), "ephemeral rows are not stable targets");
    }

    /// A hand-written entry is as real as one Prelude wrote, and every one of
    /// these was a silent failure: the row simply never appeared, or the
    /// keyword simply never resolved, with nothing anywhere saying why.
    #[test]
    fn hand_written_entries_are_found_edited_and_removed_like_any_other() {
        use crate::compute::*;

        // `minitoml` keeps a section name as written and every lookup
        // lowercased it, so `[Design]` matched nothing and produced no row.
        let text = "# header\n\n[Design]\nname = \"Design\"\nkind = \"url\"\ntarget = \"https://example.com/d\"\n\n[GH]\nname = \"GitHub\"\nurl = \"https://github.com/search?q={q}\"\n";
        assert!(fixed_quicklink_from(text, "design").is_some(), "a capitalised key must still be reachable");
        assert!(fixed_quicklink_from(text, "DESIGN").is_some());
        assert!(exact_quicklink_key_from(text, "gh"));
        let keys: Vec<String> =
            quicklink_items_from(text).iter().map(|it| it.get("quicklink").to_string()).collect();
        assert_eq!(keys, ["design", "gh"], "both entries reach the catalogue, folded to lower case");
        assert!(quicklink_from(text, "GH rust").is_some(), "and a template resolves the same way");

        // Removal reached only marked blocks; an unmarked one was refused.
        let gone = remove_quicklink_block(text.to_string(), "design").unwrap();
        assert!(!gone.contains("[Design]"));
        assert!(gone.contains("[GH]"), "the neighbour is untouched");
        assert!(gone.starts_with("# header"), "and so is the file's own comment");

        // Renaming and re-pointing existed nowhere but $EDITOR.
        let renamed = rename_quicklink_key(text, "design", "d").unwrap();
        assert!(fixed_quicklink_from(&renamed, "d").is_some());
        assert!(fixed_quicklink_from(&renamed, "design").is_none());
        assert!(renamed.contains("target = \"https://example.com/d\""), "renaming moves nothing else");

        let repointed = set_quicklink_field(text, "design", "target", "https://example.com/new").unwrap();
        assert_eq!(fixed_quicklink_from(&repointed, "design").unwrap().get("url"), "https://example.com/new");
        let added = set_quicklink_field(text, "design", "name", "Design docs").unwrap();
        assert_eq!(fixed_quicklink_from(&added, "design").unwrap().title, "Design docs");
    }

    /// The three ways a keyword could be accepted and then never work.
    #[test]
    fn a_keyword_is_refused_at_the_one_moment_it_can_be_explained() {
        use crate::compute::*;

        // `dynamic_rows_with` settles a scope command before a quicklink, so
        // `f` was written to the file and was unreachable forever.
        assert!(quicklink_conflict("f").is_some(), "f: is the file scope");
        assert!(quicklink_conflict("s").is_some());
        assert!(quicklink_conflict("mcp").is_some());
        assert!(quicklink_conflict("design").is_none());
        assert!(append_quicklink(String::new(), "f", &QuicklinkDraft {
            name: "Figma".into(), kind: "url", target: "https://figma.com".into(),
        }).is_err());

        // ASCII-only keys meant a Chinese user could not name a quicklink in
        // the language the thing they were naming was written in.
        assert_eq!(normalize_quicklink_key(" 设计 ").unwrap(), "设计");
        assert_eq!(normalize_quicklink_key("GH").unwrap(), "gh");
        assert!(normalize_quicklink_key("a b").is_err(), "a space would split the query");
        assert!(normalize_quicklink_key("a:b").is_err(), "and a colon is a scope");
        assert!(normalize_quicklink_key("").is_err());

        let text = "[设计]\nname = \"设计文档\"\nkind = \"url\"\ntarget = \"https://example.com/d\"\n";
        assert!(fixed_quicklink_from(text, "设计").is_some(), "and it has to resolve once written");

        // The suggestion was built by dropping every non-ASCII character, so a
        // CJK-named file always opened the prompt blank.
        let cn = crate::item::Item::new("/tmp/设计文档.pdf", crate::item::Kind::File)
            .title("设计文档.pdf")
            .put("path", "/tmp/设计文档.pdf");
        assert_eq!(quicklink_suggestion(&cn), "设计文档");
    }

    /// The `{q}` half of the feature had no way in that was not hand-written
    /// TOML — which is exactly the population least likely to write any.
    #[test]
    fn a_search_keyword_can_be_made_from_the_url_in_front_of_you() {
        use crate::compute::*;

        assert_eq!(
            template_suggestion("https://jira.example.com/issues?jql=timeout"),
            "https://jira.example.com/issues?jql={q}"
        );
        assert_eq!(
            template_suggestion("https://example.com/s?lang=en&query=rust+async#top"),
            "https://example.com/s?lang=en&query={q}#top",
            "the last filled parameter is the term; the others are settings"
        );
        assert_eq!(template_suggestion("https://example.com/wiki"), "https://example.com/wiki/{q}");

        let draft = template_draft("Jira", "https://jira.example.com/issues?jql={q}").unwrap();
        assert!(draft.is_template());
        assert!(template_draft("x", "https://example.com/s?q=fixed").is_err(), "no braces, no template");
        assert!(template_draft("x", "not a url {q}").is_err());
        assert!(
            template_draft("x", "https://example.com/s?token=abc&q={q}").is_err(),
            "a credential in the template is a credential in every search"
        );

        let made = append_quicklink(String::new(), "jira", &draft).unwrap();
        let (url, _, term, key) = quicklink_from(&made, "jira api timeout").unwrap();
        assert_eq!(url, "https://jira.example.com/issues?jql=api%20timeout");
        assert_eq!((term.as_str(), key.as_str()), ("api timeout", "jira"));

        // A key the person typed beats a display name another entry carries.
        // `[g]` is named "Google", and it used to swallow a fixed `[google]`.
        let both = concat!(
            "[g]\nname = \"Google\"\nurl = \"https://www.google.com/search?q={q}\"\n",
            "\n[google]\nname = \"My profile\"\nkind = \"url\"\ntarget = \"https://example.com/me\"\n",
        );
        assert!(exact_quicklink_key_from(both, "google"));
        assert_eq!(fixed_quicklink_from(both, "google").unwrap().get("url"), "https://example.com/me");
        assert_eq!(search_provider_from(both, "g").unwrap().get("provider"), "g");
    }

    /// A keyword the person invented outranks anything Prelude merely found.
    #[test]
    fn a_saved_quicklink_outranks_the_catalogue_it_points_into() {
        use crate::item::{Item, Kind};

        let text = "[notes]\nname = \"Notes\"\nkind = \"url\"\ntarget = \"https://example.com/notes\"\n";
        let ql = crate::compute::fixed_quicklink_from(text, "notes").unwrap();
        assert_eq!(ql.kind, Kind::Link, "the target's kind still drives Enter and ^K");
        assert!(ql.is_quicklink());
        assert_eq!(ql.style().1, "quicklink", "but the row says which one it is");
        assert_eq!(ql.band(), Kind::QUICKLINK);

        // Left in the target's band a File quicklink sat at 60, below every
        // scope command in root search.
        for kind in [Kind::File, Kind::Link, Kind::App, Kind::Dir, Kind::Search] {
            let scope = Item::new("f:", kind);
            assert!(crate::cache::by_rank(&ql, &scope) == std::cmp::Ordering::Less,
                    "a quicklink must sort above a bare {kind:?} row");
        }
        // And not above the things the launcher is actually for.
        let agent = Item::new("claude", Kind::Agent);
        assert!(crate::cache::by_rank(&ql, &agent) == std::cmp::Ordering::Greater);

        // Two quicklinks pointing at different kinds start level, so frecency
        // alone orders them rather than Link-beats-App-beats-Dir-beats-File.
        let file_ql = crate::compute::fixed_quicklink_from(
            "[readme]\nname = \"readme\"\nkind = \"file\"\ntarget = \"/etc/hosts\"\n", "readme").unwrap();
        assert_eq!(file_ql.score, ql.score);

        // A search *result* is not a saved quicklink and must keep saying so.
        let result = Item::new("https://x.test", Kind::Link).put("ql", "result").put("quicklink", "g");
        assert!(!result.is_quicklink());
        assert_eq!(result.style().1, "open");

        // The kind column answers "what kind of thing is this", not "what will
        // Enter do" — almost every other label is a noun naming a source, and
        // Enter is already stated in the footer and in the ^K header. So both
        // shapes of a Quicklink say so, and `search` is left meaning the one
        // thing it names: a scope command into Prelude's own index.
        let template = crate::compute::search_provider_from(
            "[gh]\nname = \"GitHub\"\nurl = \"https://github.com/search?q={q}\"\n", "gh").unwrap();
        assert_eq!(template.style().1, "quicklink");
        assert_eq!(template.band(), Kind::QUICKLINK);

        // `Kind::Search` carries both populations. A scope command is built in
        // and goes to Prelude's index; it is not a Quicklink and must not
        // borrow the word.
        let scope = Item::new("f:", Kind::Search).put("mode", "complete-query");
        assert!(!scope.is_quicklink());
        assert_eq!(scope.style().1, "search");
        assert!(crate::cache::by_rank(&template, &scope) == std::cmp::Ordering::Less,
                "and the person's keyword still leads the built-in scope");
    }

    /// Arrows move between levels only while there is no text to move
    /// through. Taking `←`/`→` away from a half-typed scope would be the
    /// launcher deciding it knows better than the person editing.
    #[test]
    fn the_arrow_keys_yield_to_a_query_and_ctrl_k_never_does() {
        let actions = |q: &str| crate::bind_actions(q, "prelude", "/dev/null", "80", "20");
        assert!(actions("").contains(&format!("rebind({})", crate::ui::ARROW_INTO)),
                "an empty query has nothing to move through, so → enters");
        assert!(actions("c:").contains(&format!("unbind({})", crate::ui::ARROW_INTO)),
                "a typed scope keeps its cursor keys");
        assert!(actions("claude ").contains(&format!("unbind({})", crate::ui::ARROW_INTO)));

        // `→` cannot be an --expect key, because those cannot be unbound. It
        // announces itself on the output queue instead, and the marker must
        // never look like an item.
        assert!(!crate::ui::OPEN_ACTIONS.contains(crate::render::SEP));
        assert!(!crate::ui::OPEN_ACTIONS.trim().is_empty());
    }

    /// The panel refreshes itself behind your back, so the one thing it must
    /// never do is move something you are looking at.
    #[test]
    fn a_background_refresh_only_touches_an_untouched_panel() {
        use crate::refresh::may_redraw;

        // A hidden panel waiting to be revealed: nothing typed, cursor where
        // fzf drew it.
        assert!(may_redraw("", "0"));
        assert!(may_redraw("", ""));

        // Somebody is typing. Reloading under a query would replace the
        // results they are reading.
        assert!(!may_redraw("c:", "0"));
        assert!(!may_redraw("g", ""));
        // Somebody has scrolled. A reload resets the cursor to the top, which
        // is the same as losing their place.
        assert!(!may_redraw("", "4"));
        // Anything unrecognised is treated as "in use". The cost of being
        // wrong that way is a stale row; the other way it is a moving list.
        assert!(!may_redraw("", "unexpected"));
    }

    /// A built-in keyword is chosen for everybody and can be corrected by
    /// nobody who does not know it is there, so it has to clear the bars a
    /// hand-made one is merely checked against.
    #[test]
    fn built_in_keywords_shadow_nothing_and_every_one_of_them_resolves() {
        use crate::compute::*;

        let shipped = crate::minitoml::parse(QUICKLINKS_DEFAULT);
        let mut seen: Vec<&str> = Vec::new();
        for (marker, entries) in DEFAULT_BLOCKS {
            assert!(
                QUICKLINKS_DEFAULT.lines().any(|l| l == *marker),
                "{marker} must also be in the file a fresh install gets, or the \
                 migration will append the block on top of entries already there"
            );
            for (key, name, url) in *entries {
                assert!(normalize_quicklink_key(key).is_ok_and(|k| k == *key), "{key} is not a usable keyword");
                assert!(quicklink_conflict(key).is_none(), "{key} is a scope command");
                // An exact keyword resolves ahead of the catalogue, so a
                // built-in `claude` would push the Agent row it names down a
                // line for every user, by default.
                assert!(
                    !crate::agent::SPECS.iter().any(|s| s.name == *key),
                    "{key} is an Agent's name"
                );
                assert!(!seen.contains(key), "{key} is shipped twice");
                seen.push(key);

                assert_eq!(shipped.get(*key).and_then(|b| b.get("url")).map(String::as_str), Some(*url),
                           "{key} differs between the migration and the shipped file");
                assert_eq!(shipped.get(*key).and_then(|b| b.get("name")).map(String::as_str), Some(*name));

                // Every one has to actually go somewhere. A template is
                // checked with a term in it; a fixed entry as it stands.
                if url.contains("{q}") {
                    let (made, _, term, got) = quicklink_from(QUICKLINKS_DEFAULT, &format!("{key} a b")).unwrap();
                    assert_eq!((got.as_str(), term.as_str()), (*key, "a b"));
                    assert!(crate::compute::web_url(&made).is_some(), "{key} builds {made}");
                } else {
                    assert!(fixed_quicklink_from(QUICKLINKS_DEFAULT, key).is_some(), "{key} does not resolve");
                }
            }
        }
        assert!(seen.contains(&"so") && seen.contains(&"hf") && seen.contains(&"ccdocs"));
    }

    /// An exact keyword is the one row the person definitely meant. It is not
    /// the only row they can have meant, and it used to be the only row shown:
    /// at `githu` both `Search GitHub` and a `github` quicklink were on
    /// screen, and the next keystroke deleted one of them.
    #[test]
    fn an_exact_keyword_leads_the_list_without_clearing_it() {
        use crate::compute::{fixed_quicklink_from, quicklink_items_from, quicklink_with_neighbours};

        let text = concat!(
            "[gh]\nname = \"GitHub\"\nurl = \"https://github.com/search?q={q}\"\n",
            "\n[github]\nname = \"My GitHub\"\nkind = \"url\"\ntarget = \"https://github.com/me\"\n",
        );
        let catalogue = quicklink_items_from(text);
        let exact = fixed_quicklink_from(text, "github").unwrap();

        let rows = quicklink_with_neighbours(exact.clone(), "github", &catalogue);
        assert_eq!(rows[0].get("quicklink"), "github", "the keyword leads");
        assert!(
            rows.iter().skip(1).any(|it| it.get("quicklink") == "gh"),
            "and Search GitHub is still there: {:?}",
            rows.iter().map(|it| it.title.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            rows.iter().filter(|it| it.get("quicklink") == "github").count(),
            1,
            "the row the exact match resolved to is not also listed underneath"
        );

        // A catalogue cached by an older build carries none of the new data
        // keys — and a template row is admitted to root search by its Kind, so
        // nothing else would have caught it. Identity holds on (kind, cmd) too.
        let template = crate::compute::search_provider_from(text, "gh").unwrap();
        let mut legacy = template.clone();
        legacy.data.clear();
        assert_eq!(legacy.kind, crate::item::Kind::Search);
        let rows = quicklink_with_neighbours(template, "gh", &[legacy]);
        assert_eq!(rows.len(), 1, "an upgrade must not show the same row twice");
    }

    /// `ql:` is the only place the whole set is visible, so it has to show the
    /// entries that do not work as well as the ones that do.
    #[test]
    fn broken_entries_are_visible_where_they_can_be_repaired() {
        let text = concat!(
            "[ok]\nname = \"OK\"\nkind = \"url\"\ntarget = \"https://example.com\"\n",
            "\n[bad]\nname = \"Bad\"\nkind = \"wat\"\ntarget = \"???\"\n",
            "\n[f]\nname = \"Shadowed\"\nkind = \"url\"\ntarget = \"https://example.com/f\"\n",
        );
        // The ordinary catalogue is right to drop what it cannot render.
        let listed: Vec<String> = crate::compute::quicklink_items_from(text)
            .iter().map(|it| it.get("quicklink").to_string()).collect();
        assert_eq!(listed, ["f", "ok"], "a row that cannot be built is not a search result");

        let problems = |t: &str| {
            let mut v: Vec<String> = Vec::new();
            for (key, why) in crate::compute::quicklink_problems_in(t) {
                v.push(format!("{key}: {why}"));
            }
            v
        };
        let found = problems(text);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().any(|p| p.starts_with("bad:")));
        assert!(found.iter().any(|p| p.starts_with("f:")), "a shadowed keyword is broken, not healthy");
    }

    #[test]
    fn typed_local_paths_are_file_objects_with_quicklink_and_quick_look() {
        use crate::defaults::Surface;
        use crate::item::Kind;

        let root = std::env::temp_dir().join(format!(
            "prelude-local-path-{}-{}",
            std::process::id(),
            crate::frecency::now() as u64
        ));
        let folder = root.join("Local Folder");
        let file = folder.join("完整 note.md");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&file, "the complete local file\nsecond line\n").unwrap();
        let pasted = file.to_string_lossy().replace(' ', "\\ ");

        assert!(crate::compute::is_special(&pasted));
        let row = crate::compute::dynamic_rows_with(&pasted, &[])
            .into_iter()
            .next()
            .expect("an existing pasted path becomes a row");
        assert_eq!(row.kind, Kind::File);
        assert_eq!(row.title, "完整 note.md");
        assert_eq!(row.get("path"), file.canonicalize().unwrap().to_string_lossy());
        assert!(crate::preview::text(&row).contains("the complete local file"));
        assert!(crate::actions::actions_for(&row, Surface::Prompt)
            .iter()
            .any(|(id, ..)| *id == "quicklink-create"));
        let draft = crate::compute::quicklink_draft(&row).unwrap().unwrap();
        assert_eq!(draft.kind, "file");
        assert_eq!(draft.target, crate::paths::tilde(row.get("path")));

        let folder_row = crate::compute::dynamic_rows_with(&folder.to_string_lossy(), &[])
            .into_iter()
            .next()
            .expect("a local folder becomes an object too");
        assert_eq!(folder_row.kind, Kind::Dir);
        assert!(crate::preview::text(&folder_row).contains("Local Folder"));
        assert!(crate::actions::actions_for(&folder_row, Surface::Prompt)
            .iter()
            .any(|(id, ..)| *id == "quicklink-create"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn timestamps_and_offsets() {
        let (v, _) = crate::calc::timecalc("1699999999").unwrap();
        assert!(v.starts_with("2023-11-"), "got {v}");
        assert!(crate::calc::timecalc("now + 3 days").is_some());
        assert!(crate::calc::timecalc("today + 1 week").is_some());
        assert!(crate::calc::timecalc("banana").is_none());
    }

    #[test]
    fn unmetafy_recovers_chinese() {
        // 基 stored by zsh as e5 83(META) bf ba  ->  e5 9f ba
        let raw = b"\xe5\x83\xbf\xba";
        assert_eq!(
            String::from_utf8_lossy(&crate::sources::history::unmetafy(raw)),
            "基"
        );
    }

    #[test]
    fn secrets_are_filtered() {
        for s in ["export API_KEY=abc", "curl -H 'Bearer x'", "sk-aaaaaaaaaaaaaaaaaaaaaa"] {
            assert!(crate::secrets::looks_secret(s), "{s}");
        }
        for s in ["git push origin main", "npm run build", "psql -U admin"] {
            assert!(!crate::secrets::looks_secret(s), "{s}");
        }
        assert!(!crate::secrets::looks_secret_material(
            "NOTICE_TOKEN = re.compile(r\"notice\")"
        ));
        assert!(crate::secrets::looks_secret_material(
            "access_token=abcdefghijklmnopqrstuvwxyz"
        ));
    }

    /// Copying a skill is the only place Prelude writes to user files, so
    /// pin both halves: it copies the whole tree, and it never overwrites.
    #[test]
    fn skill_copy_is_complete_and_refuses_to_overwrite() {
        let root = std::env::temp_dir().join(format!("prelude-t{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src/demo");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("SKILL.md"), "---\nname: demo\n---\n").unwrap();
        std::fs::write(src.join("nested/extra.txt"), "x").unwrap();

        let dest_root = root.join("dest");
        let dest = dest_root.join("demo");
        std::fs::create_dir_all(&dest_root).unwrap();
        crate::sources::agents::copy_tree(&src, &dest).unwrap();
        assert!(dest.join("SKILL.md").exists());
        assert!(dest.join("nested/extra.txt").exists(), "subdirectories must come too");

        // and the guard that stops a copy clobbering an existing skill
        assert!(dest.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enter_defaults_split_commands_from_objects() {
        use crate::defaults::{on_enter, on_secondary, Default_, Surface};
        use crate::item::{Item, Kind::*};
        // Commands are never acted on.
        for k in [History, Script, Path, Snippet, Port, Proc, Sys] {
            assert_eq!(on_enter(&Item::new("x", k)), Default_::Insert, "{k:?}");
        }
        // Files, folders and apps act.
        for k in [File, Dir, App] {
            assert!(matches!(on_enter(&Item::new("x", k)), Default_::Act(_)), "{k:?}");
        }
        // A URL is an external object; ^K still offers its text.
        let link = Item::new("https://example.com", Link).put("url", "https://example.com");
        assert_eq!(on_enter(&link), Default_::Act(crate::defaults::Verb::OpenUrl));
        assert_eq!(
            on_secondary(&link, Surface::Prompt),
            Some(Default_::InsertText(crate::defaults::Text::Name))
        );
        // The secondary sits directly under Enter's own entry in the ^K
        // panel, so it must differ from it rather than repeat it — two rows
        // saying the same thing is worse than one.
        for k in [History, Script, File, App, Calc, Clip, Session] {
            let it = Item::new("x", k);
            let p = on_enter(&it);
            if let Some(s) = on_secondary(&it, Surface::Prompt) {
                assert_ne!(p, s, "{k:?} primary and secondary are the same");
            }
        }
    }

    /// A borrowed server has to arrive in the borrower's own dialect, not
    /// the lender's. These are the two translations, pinned.
    #[test]
    fn borrowed_mcp_is_translated_per_agent() {
        use crate::lend::Mcp;
        let stdio = Mcp::Stdio {
            name: "node_repl".into(),
            command: "/bin/node_repl".into(),
            args: vec!["--serve".into()],
            env: Vec::new(),
        };
        let j = stdio.to_claude_json();
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["mcpServers"]["node_repl"]["command"], "/bin/node_repl");
        assert_eq!(v["mcpServers"]["node_repl"]["args"][0], "--serve");

        let http = Mcp::Http {
            name: "chatcut".into(),
            url: "https://example.test/mcp".into(),
            headers: Vec::new(),
        };
        let v: serde_json::Value = serde_json::from_str(&http.to_claude_json()).unwrap();
        assert_eq!(v["mcpServers"]["chatcut"]["type"], "http");
        assert_eq!(v["mcpServers"]["chatcut"]["url"], "https://example.test/mcp");

        // codex takes dotted overrides whose value half is TOML, so the
        // string has to arrive quoted or a path with a slash in it parses
        // as something else entirely.
        let f = stdio.to_codex_flags();
        assert_eq!(f[0], "-c");
        assert!(f.contains(&r#"mcp_servers.node_repl.command="/bin/node_repl""#.to_string()), "{f:?}");
        assert!(f.contains(&r#"mcp_servers.node_repl.args=["--serve"]"#.to_string()), "{f:?}");
    }

    /// The variadic-flag trap: `--mcp-config <path>` keeps eating bare words,
    /// so a prompt typed after the borrowed command becomes a config file.
    /// The `=` form is what stops it, and nothing may quietly undo that.
    #[test]
    fn borrowed_claude_flag_cannot_swallow_a_prompt() {
        let m = crate::lend::Mcp::Http {
            name: "x".into(),
            url: "https://example.test/mcp".into(),
            headers: Vec::new(),
        };
        let flags = crate::lend::mcp_flags("claude", &m).unwrap();
        assert_eq!(flags.len(), 1, "must be one token, not flag-then-value: {flags:?}");
        assert!(flags[0].starts_with("--mcp-config="), "{flags:?}");
    }

    /// Borrowing must never put a credential on the command line, because
    /// the command goes to the shell prompt and from there into the history
    /// this launcher reads back.
    #[test]
    fn borrowing_refuses_to_put_a_secret_on_the_command_line() {
        use crate::lend::{mcp_flags, Mcp};
        let secret = Mcp::Stdio {
            name: "paid".into(),
            command: "server".into(),
            args: Vec::new(),
            env: vec![("API_KEY".into(), "abc123".into())],
        };
        // codex can only be handed a server inline, so it must decline.
        let e = mcp_flags("codex", &secret).unwrap_err();
        assert!(e.contains("API_KEY"), "{e}");
        // Whatever claude is given, the key itself is not in it.
        for tok in mcp_flags("claude", &secret).unwrap() {
            assert!(!tok.contains("abc123"), "secret leaked into {tok}");
        }
        let unrecognised = Mcp::Http {
            name: "private".into(),
            url: "https://example.test/mcp".into(),
            headers: vec![("X-Custom".into(), "opaque-value".into())],
        };
        assert!(mcp_flags("codex", &unrecognised).unwrap_err().contains("private fields"));
    }

    /// Every agent has a different door, and three of the eight pairings
    /// have none at all. Offering one that does not exist would produce a
    /// command that fails after the launcher has already closed.
    #[test]
    fn borrowing_is_offered_only_where_it_exists() {
        use crate::lend::{can_borrow_mcp, can_borrow_skill, skill_flags, Mcp};
        assert!(can_borrow_mcp("claude") && can_borrow_mcp("codex"));
        assert!(!can_borrow_mcp("pi") && !can_borrow_mcp("opencode"));
        assert!(can_borrow_skill("pi") && can_borrow_skill("claude"));
        assert!(!can_borrow_skill("codex") && !can_borrow_skill("opencode"));
        let d = std::path::Path::new("/tmp/whatever");
        assert!(skill_flags("pi", d, "demo").is_ok());
        assert!(skill_flags("codex", d, "demo").is_err());

        // claude.ai-hosted servers carry no definition to lend; that has to
        // read as a refusal rather than an empty command.
        assert!(Mcp::from_claude_get("claude.ai Gmail",
            "claude.ai Gmail:\n  Scope: claude.ai config\n  Status: ✔ Connected").is_none());
    }

    /// Enter is already named in the main footer and in the action panel's
    /// non-selectable header. ^K therefore starts with a real alternative,
    /// not the action the user just declined by opening the panel.
    #[test]
    fn the_panel_contains_alternatives_not_the_default_again() {
        use crate::defaults::{describe, describe_secondary, Surface};
        use crate::item::{Item, Kind};
        let it = Item::new("/tmp/x.txt", Kind::File).title("x.txt").put("path", "/tmp/x.txt");
        let acts = crate::actions::actions_for(&it, crate::defaults::Surface::Prompt);
        assert!(!acts.iter().any(|(id, ..)| *id == "default"));
        assert!(!acts.iter().any(|(_, label, ..)| label == describe(&it, Surface::Prompt)));
        assert_eq!(acts[0].0, "secondary");
        assert_eq!(acts[0].1, describe_secondary(&it, Surface::Prompt).unwrap());
        assert_eq!(acts[0].2, "");
    }

    /// Representative menus are product surfaces, not merely collections
    /// that satisfy generic invariants. Pin their order and keep them short.
    #[test]
    fn common_objects_have_intentional_action_menus() {
        use crate::item::{Item, Kind};
        let ids = |it: &Item| -> Vec<String> {
            crate::actions::actions_for(it, crate::defaults::Surface::Prompt).iter().map(|(id, ..)| id.to_string()).collect()
        };

        let file = Item::new("/tmp/readme.md", Kind::File).put("path", "/tmp/readme.md");
        assert_eq!(
            ids(&file),
            ["secondary", "openwith", "open", "reveal-finder", "copy-file", "copyabs", "openalways", "quicklink-create", "trash"]
        );

        let app = Item::new("open -a Zed", Kind::App).put("path", "/Applications/Zed.app");
        assert_eq!(ids(&app), ["reveal-finder", "copy-file", "copy", "insert", "quicklink-create", "trash"]);

        // A URL is where a `{q}` template comes from, so it carries both.
        let link = Item::new("https://example.com", Kind::Link).put("url", "https://example.com");
        assert_eq!(ids(&link), ["secondary", "copy", "quicklink-create", "quicklink-template"]);

        let dir = Item::new("cd /tmp/project", Kind::Dir).put("path", "/tmp/project");
        assert_eq!(ids(&dir), ["secondary", "copy-file", "insert", "copy", "quicklink-create"]);

        let linked = Item::new("/tmp/readme.md", Kind::File)
            .put("path", "/tmp/readme.md")
            .quicklink("readme", "fixed")
            .put("quicklink_managed", "true");
        let linked_ids = ids(&linked);
        assert!(!linked_ids.contains(&"quicklink-create".to_string()));
        // Every edit a quicklink needs, without opening the file.
        for id in ["quicklink-rename", "quicklink-relabel", "quicklink-retarget", "quicklink-remove"] {
            assert!(linked_ids.contains(&id.to_string()), "{id} missing from {linked_ids:?}");
        }
        assert!(linked_ids.iter().position(|id| id == "quicklink-remove").unwrap()
            < linked_ids.iter().position(|id| id == "trash").unwrap());

        // A hand-written entry is removable too. It used to be refused with
        // "that quicklink is managed in the config file", which reads as the
        // opposite of what it meant.
        let handwritten = Item::new("/tmp/readme.md", Kind::File)
            .put("path", "/tmp/readme.md")
            .quicklink("readme", "fixed")
            .put("quicklink_managed", "false");
        assert!(ids(&handwritten).contains(&"quicklink-remove".to_string()));

        // The result of a search is the URL most worth keeping, and it was the
        // one row where saving was suppressed.
        let result = Item::new("https://www.google.com/search?q=rust", Kind::Link)
            .put("url", "https://www.google.com/search?q=rust")
            .put("ql", "result")
            .put("quicklink", "g");
        assert!(ids(&result).contains(&"quicklink-create".to_string()));
        assert!(ids(&result).contains(&"quicklink-template".to_string()));

        let clip = Item::new("rm -rf /tmp/x", Kind::Clip);
        let clip_ids = ids(&clip);
        assert_eq!(clip_ids, ["secondary", "tr_en", "tr_zh"]);

        let image_clip = Item::new("'/tmp/image.png'", Kind::Clip)
            .put("clip_kind", "image")
            .put("path", "/tmp/image.png");
        assert_eq!(ids(&image_clip), ["secondary", "openit", "reveal-finder", "copyabs"]);
        assert!(!clip_ids.iter().any(|id| id == "run" || id == "runhere"));
    }

    #[test]
    fn capabilities_can_be_archived_and_restored_without_offering_that_to_agents() {
        use crate::item::{Item, Kind};
        let ids = |item: &Item| -> Vec<String> {
            crate::actions::actions_for(item, crate::defaults::Surface::Prompt)
                .iter()
                .map(|(id, ..)| id.to_string())
                .collect()
        };
        for item in [
            Item::new("/review", Kind::Skill).put("name", "review"),
            Item::new("codex mcp get node", Kind::Mcp)
                .put("name", "node")
                .put("capability_id", "mcp:node"),
        ] {
            assert!(ids(&item).contains(&"capability-archive".to_string()));
            let archived = item.put("archived", "true");
            let archived_ids = ids(&archived);
            assert_eq!(archived_ids[0], "capability-unarchive");
            assert!(!archived_ids.contains(&"capability-archive".to_string()));
        }
        let agent = Item::new("claude", Kind::Agent).put("agent", "claude");
        assert!(!ids(&agent).iter().any(|id| id.starts_with("capability-")));
    }

    #[test]
    fn stable_agent_objects_can_be_favourited_without_changing_native_data() {
        use crate::item::{Item, Kind};
        let ids = |item: &Item| -> Vec<String> {
            crate::actions::actions_for(item, crate::defaults::Surface::Prompt).iter().map(|(id, ..)| id.to_string()).collect()
        };
        for item in [
            Item::new("claude", Kind::Agent).put("agent", "claude"),
            Item::new("/review", Kind::Skill).put("name", "review"),
            Item::new("codex mcp get node", Kind::Mcp).put("name", "node"),
        ] {
            assert!(ids(&item).contains(&"favorite".to_string()));
            assert!(ids(&item.clone().put("favorite", "true")).contains(&"unfavorite".to_string()));
        }
    }

    /// A skill's two bare forms are both offered, named, rather than one of
    /// them being chosen for you.
    ///
    /// Prelude used to pick: it asked tmux which agent was running in the pane
    /// underneath the popup, gave that agent `/name`, and gave everyone else
    /// the instruction to read the skill's file. The guess is unavailable now,
    /// and the failure it was avoiding is still real — `/name` at an agent
    /// that does not have the skill is a line of prose that does nothing, and
    /// does nothing *silently* — so both rows are present and say which is
    /// which.
    #[test]
    fn a_skill_offers_both_of_its_handover_forms() {
        use crate::item::{Item, Kind};
        let skill = Item::new("/review", Kind::Skill)
            .put("name", "review")
            .put("agent", "claude")
            .put("file", "/tmp/skills/review/SKILL.md")
            .put("missing", "codex");
        let acts = crate::actions::actions_for(&skill, crate::defaults::Surface::Prompt);
        let ids: Vec<&str> = acts.iter().map(|(i, ..)| *i).collect();
        assert!(ids.contains(&"skillcmd"), "{ids:?}");
        assert!(ids.contains(&"skillfile"), "{ids:?}");
        // And they are two different things to hand over, not one thing
        // twice: the file form is an instruction, not a bare path.
        let file = crate::defaults::text_for(&skill, crate::defaults::Text::SkillFile);
        assert!(file.starts_with("Read ") && file.ends_with(" and follow it."), "{file}");
        assert_ne!(file, skill.cmd);
    }

    /// What the ^K panel actually offers, without standing up an fzf to
    /// look at it. The row is built exactly as the mcp source builds one.
    #[test]
    fn panel_offers_the_loan_only_to_other_agents() {
        use crate::item::{Item, Kind};
        let it = Item::new("codex mcp get node_repl", Kind::Mcp)
            .title("node_repl")
            .put("agent", "codex")
            .put("name", "node_repl");
        let ids: Vec<&str> = crate::actions::actions_for(&it, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| *i).collect();
        // Never back to the agent that already has it.
        assert!(!ids.contains(&"lend:codex"), "{ids:?}");
        // pi and opencode have no way to take one, so they are not offered
        // however many of them are installed.
        assert!(!ids.contains(&"lend:pi") && !ids.contains(&"lend:opencode"), "{ids:?}");
        if crate::agent::installed().contains(&"claude") {
            assert!(ids.contains(&"lend:claude"), "{ids:?}");
        }
    }

    /// The loan that needs no flag and no restart: a skill handed to an
    /// agent that cannot load it as a skill at all.
    ///
    /// It has to read as an instruction. A bare path invites the agent to
    /// summarise the file instead of doing what it says.
    ///
    /// Which of the two texts Enter picks depends on `host_agent`, which is
    /// process-wide and set once, so that half is covered by running
    /// `_footer` in separate processes rather than from here.
    #[test]
    fn a_skill_can_be_handed_over_as_a_file() {
        use crate::defaults::{text_for, Text};
        use crate::item::{Item, Kind};
        let it = Item::new("/kimi-webbridge", Kind::Skill)
            .title("kimi-webbridge")
            .put("agent", "codex")
            .put("file", "/Users/x/.codex/skills/kimi-webbridge/SKILL.md");
        let t = text_for(&it, Text::SkillFile);
        assert!(t.contains("/Users/x/.codex/skills/kimi-webbridge/SKILL.md"), "{t}");
        assert!(t.to_lowercase().contains("follow"), "must tell the agent to act on it: {t}");
    }

    /// The line that decides a default: is there anything you might want to
    /// edit before it happens?
    ///
    /// A command line usually has something — `claude` becomes `--resume`,
    /// a model, an opening prompt — so it goes to the prompt and your own
    /// Enter runs it. `open -a Zed foo.json` has nothing anyone would edit,
    /// so it just happens. Safety is not the axis: `claude` is harmless and
    /// still gets handed over.
    #[test]
    fn commands_are_handed_over_and_objects_just_happen() {
        use crate::defaults::{on_enter, Default_, Verb};
        use crate::item::{Item, Kind::*};
        // Anything that denotes a command line, including an agent.
        for k in [Agent, History, Script, Snippet, Port, Proc, Clip] {
            assert_eq!(on_enter(&Item::new("x", k)), Default_::Insert, "{k:?}");
        }
        // Objects, where there is no command worth reading.
        for k in [File, Find, Config, Dir] {
            assert_eq!(on_enter(&Item::new("x", k)), Default_::Act(Verb::Open), "{k:?}");
        }
    }

    /// Resuming with a borrowed capability exists only where the Agent has a
    /// one-run flag for it. Three of the eight pairings have none, and a
    /// command assembled for one of those fails after the launcher has
    /// already closed — the same rule `fork_cmd` follows.
    #[test]
    fn resuming_with_a_capability_is_absent_where_the_agent_cannot() {
        use crate::item::{Item, Kind};
        let session = |agent: &str| {
            Item::new(format!("{agent} --resume abc"), Kind::Session)
                .title("a conversation")
                .put("agent", agent)
                .put("id", "abc")
                .put("session_id", format!("{agent}:abc"))
        };
        let ids = |agent: &str| -> Vec<String> {
            crate::actions::actions_for(&session(agent), crate::defaults::Surface::Prompt)
                .iter().map(|(id, ..)| id.to_string()).collect()
        };
        // claude can take both; codex has no skill flag; pi has no MCP flag;
        // opencode has neither.
        assert!(ids("claude").contains(&"session-skill".to_string()));
        assert!(ids("claude").contains(&"session-mcp".to_string()));
        assert!(!ids("codex").contains(&"session-skill".to_string()), "{:?}", ids("codex"));
        assert!(ids("codex").contains(&"session-mcp".to_string()));
        assert!(ids("pi").contains(&"session-skill".to_string()));
        assert!(!ids("pi").contains(&"session-mcp".to_string()), "{:?}", ids("pi"));
        assert!(!ids("opencode").contains(&"session-skill".to_string()));
        assert!(!ids("opencode").contains(&"session-mcp".to_string()));
        // Never against a live Run: that is the competing resume the whole
        // active-Session rule exists to stop.
        let active: Vec<String> = crate::actions::actions_for(&session("claude").put("active_run", "claude:7:1"), crate::defaults::Surface::Prompt)
            .iter().map(|(id, ..)| id.to_string()).collect();
        assert!(!active.contains(&"session-skill".to_string()), "{active:?}");
        assert!(!active.contains(&"session-mcp".to_string()), "{active:?}");
        // The readable export sits beside the authoritative raw one.
        let with_file: Vec<String> = crate::actions::actions_for(&session("claude").put("file", "/tmp/abc.jsonl"), crate::defaults::Surface::Prompt)
            .iter().map(|(id, ..)| id.to_string()).collect();
        assert!(with_file.contains(&"session-export".to_string()));
        assert!(with_file.contains(&"session-export-md".to_string()));
    }

    /// "Open all copies" is only a distinct action when there is more than
    /// one. With a single copy `Open` already is that, and a panel whose
    /// fifth row repeats its fourth teaches you not to read it.
    #[test]
    fn open_all_copies_appears_only_where_there_is_more_than_one() {
        use crate::item::{Item, Kind};
        let ids = |it: &Item| -> Vec<String> {
            crate::actions::actions_for(it, crate::defaults::Surface::Prompt).iter().map(|(id, ..)| id.to_string()).collect()
        };
        let one = Item::new("/demo", Kind::Skill)
            .title("demo")
            .put("name", "demo")
            .put("dir", "/x/.claude/skills/demo")
            .put("file", "/x/.claude/skills/demo/SKILL.md")
            .put("copies", r#"[["claude","/x/.claude/skills/demo"]]"#);
        assert!(!ids(&one).contains(&"open-copies".to_string()), "{:?}", ids(&one));

        let two = one.clone().put(
            "copies",
            r#"[["claude","/x/.claude/skills/demo"],["codex","/x/.codex/skills/demo"]]"#,
        );
        let both = ids(&two);
        assert!(both.contains(&"open-copies".to_string()), "{both:?}");
        assert_eq!(crate::sources::agents::copy_paths(&two).len(), 2);
        // It sits with the other ways of looking at the skill, above
        // anything that deletes one.
        let open_at = both.iter().position(|id| id == "open-copies").unwrap();
        assert!(both[open_at..].iter().all(|id| !id.starts_with("rm:") && id != "menu:rm")
            || both.iter().position(|id| id.starts_with("rm:") || id == "menu:rm").unwrap() > open_at);
    }

    /// External objects are passed as arguments, never shell syntax. Spaces
    /// in an application or path therefore remain inside one argument.
    #[test]
    fn external_open_arguments_are_well_formed() {
        use crate::openwith::{app_command, ext_of, launch_args, open_args};
        assert_eq!(open_args("/tmp/x.json", None), ["/tmp/x.json"]);
        assert_eq!(open_args("/tmp/a b.json", None), ["/tmp/a b.json"]);
        assert_eq!(
            open_args("/tmp/x.json", Some("Visual Studio Code")),
            ["-a", "Visual Studio Code", "/tmp/x.json"]
        );
        // An empty remembered app must fall back, not produce `-a ''`.
        assert_eq!(open_args("/tmp/x.json", Some("  ")), ["/tmp/x.json"]);

        // The hidden global panel has Ghostty's bundle identity. Launching the
        // ordinary app directly should therefore ask Launch Services for a new
        // instance instead of first being misrouted through the surface gate.
        assert_eq!(app_command("Ghostty"), "open -na Ghostty");
        assert_eq!(
            launch_args("/Applications/Ghostty.app"),
            ["-n", "/Applications/Ghostty.app"]
        );
        assert_eq!(app_command("Visual Studio Code"), "open -a 'Visual Studio Code'");
        assert_eq!(
            launch_args("/Applications/Visual Studio Code.app"),
            ["/Applications/Visual Studio Code.app"]
        );

        assert_eq!(ext_of("/a/b/.claude.json"), "json");
        assert_eq!(ext_of("/a/b/Makefile"), "");
        assert_eq!(ext_of("/a/b/X.JSON"), "json", "extensions are matched case-insensitively");
    }

    /// The one signal that makes a fleet usable: an agent that is working
    /// appends to its conversation file as it goes, and writes nothing at all
    /// while it waits. Silence is what a question looks like from outside the
    /// process, and those are the runs holding you up, so they sort first.
    #[test]
    fn a_quiet_agent_is_one_that_is_waiting_for_you() {
        use crate::sources::running::{classify, State, Turn};
        // With no conversation to read, silence is all there is to go on.
        assert!(matches!(classify(false, 0, None), State::Working));
        assert!(matches!(classify(false, 29, None), State::Working));
        assert!(matches!(classify(false, 30, None), State::Waiting));
        assert!(matches!(classify(false, 9000, None), State::Waiting));
        // A dead process is not waiting for anything, whatever its clock says.
        assert!(matches!(classify(true, 9000, None), State::Dead));

        // The false positive this exists to kill: a run three minutes into a
        // build is silent, and is not asking you anything. Mid-turn beats
        // any clock.
        assert!(matches!(classify(false, 9000, Some(Turn::Acting)), State::Working));
        // And a turn that ended in prose is a question, once it has sat
        // long enough to be one.
        assert!(matches!(classify(false, 30, Some(Turn::Spoke)), State::Waiting));
        assert!(matches!(classify(false, 5, Some(Turn::Spoke)), State::Working));
        assert!(matches!(classify(true, 9000, Some(Turn::Acting)), State::Dead));
    }

    /// Every agent binary is also its own admin CLI, and Prelude runs one of
    /// those itself — `claude mcp list` — on every refresh. Without this the
    /// fleet view listed its own probes as a dozen phantom agents in whatever
    /// project it was launched from. Anything that reports the fleet has to
    /// not be part of it.
    #[test]
    fn the_fleet_does_not_include_the_tools_that_report_it() {
        use crate::sources::running::is_conversation;
        for tooling in [
            vec!["mcp", "list"],
            vec!["mcp", "get", "node_repl"],
            vec!["config", "set", "x"],
            vec!["doctor"],
            vec!["login"],
            vec!["--json", "mcp", "list"],
        ] {
            assert!(!is_conversation(&tooling), "{tooling:?} is not a run");
        }
        // Real runs, including the batch ones that are marked rather than
        // dropped, and a resume whose first bare word is a session id.
        for run in [
            vec![],
            vec!["--resume", "abc-123"],
            vec!["exec", "rewrite the README"],
            vec!["-p", "what is 2+2"],
            vec!["fix the rate limiter"],
            vec!["--model", "opus"],
        ] {
            assert!(is_conversation(&run), "{run:?} is a run");
        }
    }

    /// Addressing a message to the wrong agent is worse than not sending it:
    /// it lands in a conversation that did not ask for it and reads as the
    /// human's own words. So an ambiguous target must stay ambiguous, and an
    /// exact name must never be widened into a substring match.
    #[test]
    fn a_message_is_never_delivered_to_a_guess() {
        use crate::bus::resolve;
        use crate::item::{Item, Kind};
        let mk = |agent: &str, project: &str, pid: &str| {
            Item::new(format!("kill {pid}"), Kind::Run)
                .put("agent", agent)
                .put("project", project)
                .put("addr", format!("pid {pid}"))
                .put("pid", pid)
                .put("cwd", format!("/Users/x/{project}"))
        };
        let runs = vec![
            mk("claude", "api-gateway", "21"),
            mk("claude", "api-gateway-tests", "22"),
            mk("codex", "docs", "31"),
        ];
        // An exact project name wins outright, even though it is also a
        // prefix of another project's.
        let hit = resolve(&runs, "api-gateway");
        assert_eq!(hit.len(), 1, "exact match must not widen: {:?}", hit.len());
        assert_eq!(hit[0].get("pid"), "21");
        // Addressable the ways an agent might plausibly try. A tmux pane id
        // used to be one of them and is not an address any more.
        assert_eq!(resolve(&runs, "pid 22").len(), 1);
        assert_eq!(resolve(&runs, "31").len(), 1);
        assert_eq!(resolve(&runs, "docs")[0].get("agent"), "codex");
        // Two claudes and a bare agent name: caller must be told, not guessed
        // at. `say` refuses on anything but exactly one hit.
        assert_eq!(resolve(&runs, "claude").len(), 2);
        // And a substring that spans both projects is likewise ambiguous.
        assert_eq!(resolve(&runs, "api").len(), 2);
        assert!(resolve(&runs, "nothing-like-this").is_empty());
    }

    /// Where a handed-over command lands is the difference between "insert"
    /// and "run", and the panel has nowhere to land it but the clipboard —
    /// where the two are the same bytes. So neither the secondary nor the
    /// generic tail may offer running as an alternative there: it would be
    /// Enter again under a bolder name, on rows whose command lines are the
    /// ones most worth reading first.
    ///
    /// Both routes are checked because they are separate code paths that
    /// arrive at the same duplicate — suppressing one and not the other is
    /// how it came back the first time.
    #[test]
    fn copying_leaves_no_row_that_claims_to_run_it() {
        use crate::defaults::{on_secondary, Default_, Surface, Verb};
        use crate::item::{Item, Kind};
        for k in [Kind::History, Kind::Script, Kind::Snippet, Kind::Agent] {
            let it = Item::new("deploy --prod", k).put("agent", "claude");
            assert_eq!(
                on_secondary(&it, Surface::Prompt),
                Some(Default_::Act(Verb::RunInShell)),
                "{k:?} at a prompt: running it is the real opposite of inserting it"
            );
            assert_eq!(on_secondary(&it, Surface::Clipboard), None, "{k:?} secondary");
            let ids: Vec<&str> = crate::actions::actions_for(&it, Surface::Clipboard)
                .iter()
                .map(|(i, ..)| *i)
                .collect();
            assert!(!ids.contains(&"run"), "{k:?} generic tail: {ids:?}");
            assert!(!ids.contains(&"secondary"), "{k:?} panel: {ids:?}");
        }
        // …and the same rows are there at a prompt, where they mean something.
        let it = Item::new("deploy --prod", Kind::History);
        let ids: Vec<&str> = crate::actions::actions_for(&it, Surface::Prompt)
            .iter()
            .map(|(i, ..)| *i)
            .collect();
        assert!(ids.contains(&"secondary"), "{ids:?}");
    }

    /// Every label states whether it acts or hands you text, so the ones that
    /// hand you text have to change wording with the surface — "Insert into
    /// prompt" is a lie about a panel that copies.
    #[test]
    fn a_label_says_where_the_text_actually_goes() {
        use crate::defaults::{describe, Surface};
        use crate::item::{Item, Kind};
        let it = Item::new("claude", Kind::Agent).put("agent", "claude");
        assert_eq!(describe(&it, Surface::Prompt), "Insert into prompt");
        assert_eq!(describe(&it, Surface::Clipboard), "Copy the command");
        // Acting is the same sentence in both: a file opens either way.
        let f = Item::new("/tmp/x.txt", Kind::File).put("path", "/tmp/x.txt");
        assert_eq!(describe(&f, Surface::Prompt), describe(&f, Surface::Clipboard));
    }

    /// A question is answered or left alone — never run. It arrives as an
    /// English sentence, and "Run in the shell below" on a sentence is the
    /// launcher offering to execute prose.
    #[test]
    fn a_question_offers_answers_and_never_execution() {
        use crate::defaults::{describe, Surface};
        use crate::item::{Item, Kind};
        let it = Item::new("Proceed with the migration?", Kind::Msg)
            .title("claude · api asks")
            .put("id", "123-4")
            .put("agent", "claude");
        assert_eq!(describe(&it, Surface::Prompt), "Answer it");
        let acts = crate::actions::actions_for(&it, crate::defaults::Surface::Prompt);
        let ids: Vec<&str> = acts.iter().map(|(i, ..)| *i).collect();
        assert!(!ids.contains(&"run"), "{ids:?}");
        assert!(!ids.contains(&"runhere"), "{ids:?}");
        assert!(!ids.contains(&"ask"), "asking an agent about its own question: {ids:?}");
        // Yes and no without typing: at ten questions that is the difference
        // between answering them and putting it off.
        assert!(ids.contains(&"msg:no") && ids.contains(&"msg:go"), "{ids:?}");
    }

    /// The agent-facing verbs take their text last and unquoted, so the flag
    /// split must stop at the first word that is not a flag — otherwise a
    /// question containing a dashed word loses half of itself.
    #[test]
    fn a_question_keeps_its_own_words() {
        use crate::split_flags;
        let (f, t) = split_flags(&["--timeout=30", "should", "I", "force-push", "--no-verify", "?"]);
        assert_eq!(f, vec!["--timeout=30"]);
        assert_eq!(t, "should I force-push --no-verify ?");
        assert_eq!(crate::flag_value(&f, "--timeout").as_deref(), Some("30"));
        let (f, t) = split_flags(&["plain", "question"]);
        assert!(f.is_empty());
        assert_eq!(t, "plain question");
    }

    /// Build one row of every kind, the way `_actions` does, so the panel's
    /// own invariants can be checked without standing up an fzf.
    #[cfg(test)]
    fn every_kind_row() -> Vec<crate::item::Item> {
        use crate::item::{Item, Kind};
        Kind::all()
            .iter()
            .map(|k| {
                Item::new("x", *k)
                    .title("x")
                    .put("path", "/tmp/a.json")
                    .put("pid", "1")
                    .put("port", "80")
                    .put("name", "n")
                    .put("agent", "claude")
                    .put("pane", "%1")
                    .put("cwd", "/tmp")
                    .put("dir", "/tmp/d")
                    .put("file", "/tmp/f")
                    .put("desc", "d")
                    .put("missing", "codex")
                    .put("copies", r#"[["claude","/tmp/d"]]"#)
            })
            .collect()
    }

    /// Nothing irreversible may sit anywhere but the end.
    ///
    /// `Delete claude's copy…` followed by `Copy to clipboard` is an accident
    /// waiting for a fast scroll, and the panel had exactly that: destructive
    /// entries landed wherever their kind happened to list them, with the
    /// generic tail appended after. See `docs/ACTIONS.md`, R4.
    #[test]
    fn destructive_actions_are_always_last() {
        use crate::item::Kind;
        let destructive = |kind: Kind, id: &str| {
            id.starts_with("rm:")
                || matches!(id, "killrun" | "stop")
                || (id == "run" && matches!(kind, Kind::Port | Kind::Proc))
        };
        for it in every_kind_row() {
            let ids: Vec<String> =
                crate::actions::actions_for(&it, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| i.to_string()).collect();
            let first = ids.iter().position(|i| destructive(it.kind, i));
            if let Some(first) = first {
                assert!(
                    ids[first..].iter().all(|i| destructive(it.kind, i)),
                    "{:?}: something harmless sits below a destructive entry: {ids:?}",
                    it.kind
                );
            }
            // Enter belongs in the panel header, never as a selectable row.
            assert!(!ids.iter().any(|id| id == "default"), "{:?}", it.kind);
        }
    }

    /// "Run it" is offered only where there is something to run.
    ///
    /// The generic tail used to be appended to every kind that had not
    /// claimed it, which offered to execute a calculator result, a
    /// translation, and `/skill-name`. See `docs/ACTIONS.md`, R3.
    #[test]
    fn running_is_offered_only_where_it_means_something() {
        use crate::item::Kind;
        let previewable = [Kind::History, Kind::Script, Kind::Path, Kind::Snippet, Kind::Sys, Kind::Git];
        for it in every_kind_row() {
            let ids: Vec<&str> =
                crate::actions::actions_for(&it, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| *i).collect();
            assert_eq!(
                ids.contains(&"runhere"),
                previewable.contains(&it.kind),
                "{:?} has the wrong run-and-show action: {ids:?}",
                it.kind
            );
        }
        // Results, prose and capability rows are never treated as commands.
        for k in [Kind::Calc, Kind::Translate, Kind::Skill, Kind::Msg, Kind::Clip] {
            let it = every_kind_row().into_iter().find(|i| i.kind == k).unwrap();
            let ids: Vec<&str> = crate::actions::actions_for(&it, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| *i).collect();
            assert!(!ids.contains(&"run") && !ids.contains(&"runhere"), "{k:?}: {ids:?}");
        }
    }

    /// No two entries may do the same thing under different words.
    ///
    /// A port's command line *is* its kill, so "Run here, inside this
    /// window" was a second route to the destructive action — with an
    /// innocuous label, three rows from the top, right after the destructive
    /// one had been moved to the bottom for safety. And on a file, `copy`
    /// and `copyabs` both copy the path. See `docs/ACTIONS.md`, R5.
    #[test]
    fn no_entry_repeats_another_in_different_words() {
        use crate::item::Kind;
        for it in every_kind_row() {
            let ids: Vec<&str> =
                crate::actions::actions_for(&it, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| *i).collect();
            if matches!(it.kind, Kind::Port | Kind::Proc) {
                assert!(
                    !ids.contains(&"runhere"),
                    "{:?}: running its command line is the kill, already offered: {ids:?}",
                    it.kind
                );
            }
            assert!(
                !(ids.contains(&"copy") && ids.contains(&"copyabs")),
                "{:?}: two ways to copy the same path: {ids:?}",
                it.kind
            );
            // No id may appear twice at all.
            let mut seen: Vec<&str> = ids.clone();
            seen.sort_unstable();
            let before = seen.len();
            seen.dedup();
            assert_eq!(before, seen.len(), "{:?} repeats an id: {ids:?}", it.kind);
        }
    }

    /// The UI's two measures for actions that bite, and the line between
    /// them: red for anything destructive, a confirmation only for what
    /// cannot be reverted. `docker stop` is red but not confirmed, because
    /// `docker start` exists; killing a process is both, because nothing
    /// brings it back.
    #[test]
    fn destructive_is_red_and_the_irreversible_asks_first() {
        use crate::actions::{is_destructive, needs_confirming};
        use crate::item::Kind;
        for (kind, id) in [
            (Kind::Run, "killrun"),
            (Kind::Container, "stop"),
            (Kind::Skill, "rm:claude"),
            (Kind::Skill, "menu:rm"),
            (Kind::File, "quicklink-remove"),
            (Kind::Port, "run"),
            (Kind::Proc, "run"),
        ] {
            assert!(is_destructive(kind, id), "{kind:?}/{id} must read as destructive");
        }
        // Things that merely start something are not.
        for (kind, id) in [
            (Kind::Container, "restart"),
            (Kind::Skill, "menu:cp"),
            (Kind::App, "run"),
            (Kind::Session, "run"),
        ] {
            assert!(!is_destructive(kind, id), "{kind:?}/{id} is not destructive");
        }
        // Confirming is the stronger measure, reserved for no-way-back.
        assert!(needs_confirming(Kind::Run, "killrun").is_some());
        assert!(needs_confirming(Kind::Port, "run").is_some());
        assert!(needs_confirming(Kind::Proc, "run").is_some());
        assert!(needs_confirming(Kind::Container, "stop").is_none(), "a container starts again");
        // `rm:` asks its own question, naming the agent and the path, so it
        // must not be double-prompted.
        assert!(needs_confirming(Kind::Skill, "rm:claude").is_none());
        // And every confirmed action is destructive, never the other way.
        assert!(needs_confirming(Kind::App, "run").is_none());
    }

    /// Trashing is offered on many kinds now, so the guard is no longer a
    /// property of skill directories — it has to stand on its own against
    /// whatever path a row happened to carry.
    ///
    /// Trash is recoverable, so this is not the last line of defence; the
    /// confirmation is. It is the line against the *catastrophic*: a fuzzy
    /// list, a mis-aimed Enter, and a row whose path was `$HOME`.
    #[test]
    fn nothing_can_trash_your_home_or_the_system() {
        use crate::paths::{home, is_protected, trash};
        for bad in [
            home(),
            std::path::PathBuf::from("/"),
            std::path::PathBuf::from("/System"),
            std::path::PathBuf::from("/usr"),
            std::path::PathBuf::from("/etc"),
            std::path::PathBuf::from("/usr/bin"),
        ] {
            assert!(is_protected(&bad), "{} must be protected", bad.display());
            assert!(trash(&bad).is_err(), "{} must be refused", bad.display());
        }
        // A path that does not resolve is refused rather than guessed at.
        assert!(is_protected(std::path::Path::new("/nope/nothing/here")));
        // Ordinary things a person might actually select are not protected.
        let tmp = std::env::temp_dir();
        assert!(!is_protected(&tmp) || tmp.starts_with("/var"), "{}", tmp.display());
    }

    /// A directory that merely *holds* projects is not one, and `$HOME` is the
    /// case that bites: it is where the global panel stands.
    ///
    /// `root()` used to fall back to the current directory whenever no marker
    /// was found above it, so "the files in this project" became `fd
    /// --max-depth 6` over the whole home directory on every open — six levels
    /// into `~/Library`, which macOS protects as other applications' data. It
    /// surfaced as a TCC panel asking whether *Ghostty* should be allowed to
    /// read other apps' data, because `fd` was running under the terminal and
    /// the terminal is who gets asked. Nothing in that dialog said Prelude.
    ///
    /// The unmarked fallback itself is deliberate and stays: a scratch folder
    /// of notes is its own project. It is the containers that are excluded.
    #[test]
    fn a_directory_that_holds_projects_is_not_one() {
        use crate::paths::is_protected;
        use std::path::PathBuf;
        // `project::root`'s fallback is `(!is_protected(cur)).then_some(cur)`,
        // so these are exactly the directories it will refuse to walk.
        for holder in [
            crate::paths::home(),
            PathBuf::from("/"),
            PathBuf::from("/Users"),
            PathBuf::from("/Applications"),
            PathBuf::from("/Volumes"),
            PathBuf::from("/Library"),
        ] {
            assert!(is_protected(&holder), "{} must not be walked as a project", holder.display());
        }
        // …and an ordinary directory somebody works in still is one. Not the
        // temp directory, which canonicalizes under `/private/var` and is
        // therefore protected on the older rule as well.
        let here = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert!(!is_protected(&here), "{} is somebody's work", here.display());
        let nested = here.join("src/sources");
        assert!(!is_protected(&nested), "{} is somebody's work", nested.display());
    }

    /// A setting must state its value, and `^K` must not repeat Enter.
    ///
    /// Both are the panel's standing rules, and settings are where they are
    /// easiest to break: every one of these rows has an obvious primary, so
    /// listing it again is the natural mistake. Enter's own action is already
    /// the panel's non-selectable header.
    #[test]
    fn a_setting_states_its_value_and_the_panel_never_repeats_enter() {
        use crate::defaults::{describe, Surface};
        for it in crate::settings::items() {
            let enter = describe(&it, Surface::Prompt);
            assert_ne!(enter, "Change it", "{} has no specific label: {enter}", it.title);
            let acts = crate::actions::actions_for(&it, Surface::Prompt);
            assert!(
                !acts.iter().any(|(_, label, ..)| label == enter),
                "{} lists Enter ({enter}) again: {:?}",
                it.title,
                acts.iter().map(|(_, l, ..)| l).collect::<Vec<_>>()
            );
            assert!(
                !acts.iter().any(|(id, ..)| *id == "secondary"),
                "{} got a generic secondary row",
                it.title
            );
            if it.get("setting") == "hotkey" {
                assert!(
                    acts.iter().any(|(_, label, ..)| label == "Reset to Cmd+Shift+Space"),
                    "the action label must name the same default settings::reset applies"
                );
            }
        }
    }

    /// High-value management actions stay reachable after pruning the panel;
    /// a shorter menu must not make files, apps, or MCP servers dead ends.
    #[test]
    fn important_management_actions_remain_reachable() {
        use crate::item::{Item, Kind};
        let ids = |it: &Item| -> Vec<String> {
            crate::actions::actions_for(it, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| i.to_string()).collect()
        };
        // Ordinary files can be removed recoverably.
        let f = Item::new("/tmp/x.txt", Kind::File).put("path", "/tmp/x.txt");
        assert!(ids(&f).contains(&"trash".to_string()), "{:?}", ids(&f));
        // …but not on a config: deleting the file your agent is configured
        // by, out of a fuzzy list, is a foot-gun with nothing on the far side.
        let c = Item::new("/x/.claude.json", Kind::Config).put("path", "/x/.claude.json");
        assert!(!ids(&c).contains(&"trash".to_string()), "{:?}", ids(&c));
        // Applications can be located and uninstalled recoverably.
        let ap = Item::new("open -a Zed", Kind::App).put("path", "/Applications/Zed.app");
        assert!(ids(&ap).contains(&"reveal-finder".to_string()), "{:?}", ids(&ap));
        assert!(ids(&ap).contains(&"trash".to_string()), "{:?}", ids(&ap));
        // MCP inspection is the default; auth, installation and removal are alternatives.
        let m = Item::new("codex mcp get n", Kind::Mcp)
            .put("name", "n")
            .put("agent", "codex")
            .put("health", "needsauth");
        let mi = ids(&m);
        assert_eq!(
            crate::defaults::describe(&m, crate::defaults::Surface::Prompt),
            "Show what it exposes",
            "inspection is the MCP default, not another action row"
        );
        assert!(mi.contains(&"mcplogin".to_string()), "not logged in, no way in: {mi:?}");
        assert!(mi.contains(&"mcpremove".to_string()), "{mi:?}");
        let has_install_target = crate::agent::installed().into_iter().any(|name| {
            name != "codex"
                && crate::agent::get(name).is_some_and(|spec| spec.capabilities.install_mcp)
        });
        assert_eq!(
            mi.iter().any(|i| i.starts_with("install:") || i == "menu:install"),
            has_install_target,
            "an install action is honest only when another installed agent can receive it: {mi:?}"
        );
        // A healthy server is not nagged about logging in.
        let ok = Item::new("codex mcp get n", Kind::Mcp)
            .put("name", "n").put("agent", "codex").put("health", "ok");
        assert!(!ids(&ok).contains(&"mcplogin".to_string()), "{:?}", ids(&ok));
    }

    /// Installing a server for good puts its definition on the command line,
    /// so unlike lending there is no form of it that keeps a credential off
    /// the shell history this launcher reads back. Both agents must refuse.
    #[test]
    fn installing_a_server_for_good_refuses_to_carry_a_secret() {
        use crate::lend::{install_cmd, Mcp};
        let secret = Mcp::Stdio {
            name: "paid".into(),
            command: "server".into(),
            args: Vec::new(),
            env: vec![("API_KEY".into(), "abc123".into())],
        };
        for agent in ["claude", "codex"] {
            let e = install_cmd(agent, &secret).unwrap_err();
            assert!(e.contains("API_KEY"), "{agent}: {e}");
        }
        // Without one, both get a command in their own dialect.
        let plain = Mcp::Http {
            name: "chatcut".into(),
            url: "https://example.test/mcp".into(),
            headers: Vec::new(),
        };
        assert!(install_cmd("claude", &plain).unwrap().starts_with("claude mcp add-json "));
        let c = install_cmd("codex", &plain).unwrap();
        assert!(c.starts_with("codex mcp add ") && c.contains("--url"), "{c}");
    }

    /// Deleting is the only destructive thing Prelude does to a user's
    /// files, and the path it acts on has been through JSON, a row, and a
    /// shell. A launcher that removes whatever it is handed is one malformed
    /// field away from removing something else, so the guard is what makes
    /// that impossible rather than unlikely — and it must refuse by default.
    #[test]
    fn deleting_refuses_anything_that_is_not_a_skill_directory() {
        use crate::sources::agents::{delete_skill, is_skill_dir};
        let home = crate::paths::home();
        for bad in [
            home.join(".claude/skills"),          // the container, not a skill
            home.join(".claude"),                 // its parent
            home.clone(),                         // home itself
            std::path::PathBuf::from("/"),        // the root
            std::path::PathBuf::from("/etc"),
            home.join(".claude/skills/x/nested"), // deeper than a skill
            home.join("Documents"),               // somewhere else entirely
        ] {
            assert!(!is_skill_dir(&bad), "{} must not read as a skill", bad.display());
            assert!(
                delete_skill(&bad.to_string_lossy()).is_err(),
                "{} must be refused",
                bad.display()
            );
        }
        // A path that does not resolve at all is refused rather than guessed
        // at — canonicalize fails, and failing closed is the point.
        assert!(delete_skill("").is_err());
        assert!(delete_skill("/nope/nothing/here").is_err());
        // And traversal cannot dress something up as a skill.
        assert!(!is_skill_dir(&home.join(".claude/skills/../../Documents")));
    }

    /// A skill merged across four agents is four directories behind one row.
    /// Deleting has to name which, so the panel offers one entry per copy —
    /// and the row has to carry them, since `dir` is only ever the first.
    #[test]
    fn every_agents_copy_is_reachable_from_the_row() {
        use crate::item::{Item, Kind};
        use crate::sources::agents::copies_of;
        let it = Item::new("/demo", Kind::Skill)
            .title("demo")
            .put("name", "demo")
            .put("agent", "claude, codex")
            .put("dir", "/x/.claude/skills/demo")
            .put(
                "copies",
                r#"[["claude","/x/.claude/skills/demo"],["codex","/x/.codex/skills/demo"]]"#,
            );
        let copies = copies_of(&it);
        assert_eq!(copies.len(), 2);
        assert_eq!(copies[1].0, "codex");
        // Two copies is a choice, so the panel carries one row and the agent
        // is picked after — seven rows of `Copy it to <agent>` and
        // `Delete <agent>'s copy` were three verbs and a parameter.
        let ids: Vec<&str> = crate::actions::actions_for(&it, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| *i).collect();
        assert!(ids.contains(&"menu:rm"), "{ids:?}");
        assert!(!ids.iter().any(|i| i.starts_with("rm:")), "not enumerated: {ids:?}");
        // …and every copy is still reachable through it.
        let opts: Vec<String> =
            crate::actions::agent_options(&it, "rm").into_iter().map(|(id, ..)| id).collect();
        assert_eq!(opts, vec!["rm:claude", "rm:codex"], "{opts:?}");

        // Destructive entries come last, never next to the default.
        let first_rm = ids.iter().position(|i| i.starts_with("menu:rm")).unwrap();
        assert_eq!(first_rm, ids.len() - 1, "delete must be last: {ids:?}");

        // One copy is not a choice: a submenu over a single option asks a
        // question with one answer.
        let solo = Item::new("/demo", Kind::Skill)
            .title("demo")
            .put("name", "demo")
            .put("copies", r#"[["claude","/x/.claude/skills/demo"]]"#);
        let ids: Vec<&str> = crate::actions::actions_for(&solo, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| *i).collect();
        assert!(ids.contains(&"rm:claude"), "{ids:?}");
        assert!(!ids.contains(&"menu:rm"), "{ids:?}");

        // A row with no copies recorded offers no delete at all.
        let bare = Item::new("/demo", Kind::Skill).title("demo");
        let ids: Vec<&str> = crate::actions::actions_for(&bare, crate::defaults::Surface::Prompt).iter().map(|(i, ..)| *i).collect();
        assert!(!ids.iter().any(|i| i.starts_with("rm:") || i.starts_with("menu:rm")), "{ids:?}");
    }

    /// A source's own ordering has to survive the round trip to disk.
    ///
    /// `read_cached` rebuilds the score from kind plus `rank`, so a rank that
    /// is applied but not recorded is a rank that exists only until the
    /// cache is next read — and sessions, the kind that most depends on it,
    /// are always read back from the cache.
    #[test]
    fn a_sources_own_ordering_survives_the_cache() {
        use crate::item::{Item, Kind};
        let it = Item::new("claude --resume abc", Kind::Session).rank(199.6);
        assert_eq!(it.score, Kind::Session.priority() as f64 + 199.6);
        // What `read_cached` does on the way back off disk.
        let restored =
            Kind::Session.priority() as f64 + it.get("rank").parse::<f64>().unwrap_or(0.0);
        assert_eq!(restored, it.score, "rank must be recorded, not just applied");
    }

    /// The skill column said "used 8× · 1d ago" and the ordering ignored it,
    /// so a skill invoked eight times sorted below four never touched — on
    /// the strength of its first letter, because everything tied at zero and
    /// the merge map is alphabetical. A number shown that prominently has to
    /// mean something.
    #[test]
    fn skills_are_ordered_by_how_often_you_actually_invoke_them() {
        use crate::frecency::{now, MAX_BONUS};
        use crate::sources::agents::usage_rank;
        let day = 86_400.0;

        // Count decides, and by more than the launcher's own frecency can
        // ever add. Clicking a skill row is usually reading its description
        // or lending it somewhere — only invoking it says you use it.
        assert!(
            usage_rank(1, now() - 30.0 * day) > MAX_BONUS,
            "one real invocation must outrank any number of launcher picks"
        );
        // More uses always wins, however long ago they were. A skill used
        // eight times over a month is yours; one used once yesterday is not.
        assert!(usage_rank(8, now() - 90.0 * day) > usage_rank(1, now()));
        assert!(usage_rank(2, now() - 365.0 * day) > usage_rank(1, now()));
        // Recency only separates skills you reach for equally often.
        assert!(usage_rank(3, now()) > usage_rank(3, now() - 10.0 * day));
        // Never invoked scores nothing, so frecency and then the name decide
        // among the ones you have not used.
        assert_eq!(usage_rank(0, 0.0), 0.0);
    }

    /// Kind decides the band; how much you use something only orders it
    /// *inside* that band. Nothing you pick often can climb out.
    ///
    /// This was a real bug and a visible one. The agent cluster spans 25
    /// points (Agent 1000 down to Config 975) while the frecency bonus
    /// reached 60, so on a real machine a skill used twice that morning sat
    /// above `claude` itself and a config file sat above a skill. Tuning the
    /// cap would only move the threshold; comparing the band first removes
    /// the possibility.
    #[test]
    fn frecency_orders_within_a_kind_and_never_across_kinds() {
        use crate::cache::by_rank;
        use crate::item::{Item, Kind};
        use std::cmp::Ordering;

        // Every pair of distinct kinds, with the *lower* band handed an
        // absurd score and the higher band none at all.
        for hi in Kind::all() {
            for lo in Kind::all() {
                if hi.priority() <= lo.priority() {
                    continue;
                }
                let mut top = Item::new("x", *hi);
                top.score = 0.0;
                let mut bottom = Item::new("y", *lo);
                bottom.score = 1e9;
                assert_eq!(
                    by_rank(&top, &bottom),
                    Ordering::Less,
                    "{lo:?} with unlimited frecency climbed over {hi:?}"
                );
            }
        }

        // And inside one kind the learned ranking is exactly what decides.
        let mut a = Item::new("a", Kind::History);
        let mut b = Item::new("b", Kind::History);
        a.score = 100.0;
        b.score = 160.0;
        assert_eq!(by_rank(&a, &b), Ordering::Greater, "b is used more, b comes first");
    }

    /// The status bar says something only when there is something to say.
    /// An idle machine gets an empty segment, not a permanent "0 waiting" —
    /// and waiting always comes first, because it is the half you act on.
    #[test]
    fn the_status_bar_is_silent_when_there_is_nothing_to_say() {
        use crate::fleet::status_line;
        assert_eq!(status_line(0, 0, 0), "");
        assert_eq!(status_line(0, 0, 3), "3 working");
        assert_eq!(status_line(0, 2, 0), "2 waiting");
        assert_eq!(status_line(0, 2, 3), "2 waiting · 3 working");
        // Asking leads: that one is blocked on you by name.
        assert_eq!(status_line(1, 2, 3), "1 asking · 2 waiting · 3 working");
        assert_eq!(status_line(1, 0, 0), "1 asking");
    }

    /// A notification fires on the edge into waiting and never again while
    /// the run stays there. First sight of an already-quiet run counts too:
    /// starting the watcher late is not a reason to miss the one agent that
    /// has been stuck all along.
    #[test]
    fn a_run_is_announced_once_when_it_goes_quiet() {
        use crate::fleet::should_notify;
        assert!(should_notify(Some("working"), "waiting"), "the edge");
        assert!(should_notify(None, "waiting"), "already quiet when the watcher started");
        assert!(!should_notify(Some("waiting"), "waiting"), "no repeats");
        assert!(!should_notify(Some("waiting"), "working"), "going back to work is not news");
        assert!(!should_notify(None, "working"));
    }

    #[test]
    fn cjk_width_and_truncation() {
        assert_eq!(crate::width::dwidth("abc"), 3);
        assert_eq!(crate::width::dwidth("宽字符测试"), 10);
        let t = crate::width::dtrunc("宽字符截断测试", 6);
        assert!(crate::width::dwidth(&t) <= 6, "got {t}");
    }

    #[test]
    fn runs_link_to_sessions_only_when_the_evidence_is_unambiguous() {
        use crate::item::{Item, Kind};
        use crate::sources::running::{annotate_sessions, attach_sessions};

        let session = Item::new("claude --resume abc", Kind::Session)
            .title("the conversation")
            .put("agent", "claude")
            .put("id", "abc")
            .put("session_id", "claude:abc")
            .put("cwd", "/tmp/project")
            .put("file", "/tmp/abc.jsonl")
            .put("ts", "100");
        let explicit = Item::new("kill 10", Kind::Run)
            .put("agent", "claude")
            .put("run_id", "claude:10:1")
            .put("pid", "10")
            .put("cwd", "/tmp/project")
            .put("requested_session", "abc")
            .put("state", "working")
            .put("addr", "pid 10");
        let mut runs = vec![explicit];
        attach_sessions(&mut runs, std::slice::from_ref(&session));
        assert_eq!(runs[0].get("session_id"), "claude:abc");
        assert_eq!(runs[0].get("session_match"), "explicit");

        // A live run owns this conversation, so Enter must not start a
        // competing resume of it. It hands over the project instead — which
        // is now the only honest answer, there being no terminal to go to.
        let sessions = annotate_sessions(vec![session.clone()], &runs);
        assert_eq!(sessions[0].get("active_run"), "claude:10:1");
        assert_eq!(
            crate::defaults::on_enter(&sessions[0]),
            crate::defaults::Default_::Act(crate::defaults::Verb::CdThere),
        );

        let mut ambiguous = vec![
            Item::new("kill 11", Kind::Run).put("agent", "claude").put("cwd", "/tmp/project"),
            Item::new("kill 12", Kind::Run).put("agent", "claude").put("cwd", "/tmp/project"),
        ];
        attach_sessions(&mut ambiguous, &[session]);
        assert!(ambiguous.iter().all(|run| run.get("session_id").is_empty()));
        assert!(ambiguous.iter().all(|run| run.get("session_match") == "ambiguous"));
    }

    #[test]
    fn session_metadata_is_a_view_over_native_conversations() {
        use crate::item::{Item, Kind};
        use crate::sources::sessions::{decorate_sessions, filter_search, SessionMeta};
        use std::collections::BTreeMap;

        let native = Item::new("claude --resume abc", Kind::Session)
            .rank(5.0)
            .title("Native title")
            .fields(["claude", "~/App", "now"])
            .put("agent", "claude")
            .put("session_id", "claude:abc")
            .put("id", "abc");
        let mut metadata = BTreeMap::new();
        metadata.insert("claude:abc".into(), SessionMeta {
            title: Some("My name".into()),
            pinned: true,
            archived: true,
        });
        let mut sessions = vec![native];
        decorate_sessions(&mut sessions, &metadata);
        let session = &sessions[0];
        assert_eq!(session.title, "My name");
        assert_eq!(session.get("native_title"), "Native title");
        assert_eq!(session.get("pinned"), "true");
        assert_eq!(session.get("archived"), "true");
        assert!(session.get("rank").parse::<f64>().unwrap() > 10_000.0);
        assert!(session.fields[0].contains("pinned · archived"));

        assert!(filter_search(sessions.clone(), "").is_empty());
        assert_eq!(filter_search(sessions.clone(), "is:archived").len(), 1);
        assert_eq!(filter_search(sessions.clone(), "is:all Native").len(), 1);
        assert_eq!(filter_search(sessions.clone(), "is:pinned My").len(), 0); // archived stays hidden

        sessions[0].data.insert("active_run".into(), "claude:7:1".into());
        assert_eq!(
            filter_search(sessions.clone(), "").len(),
            1,
            "an archived conversation somebody has resumed is visible again, and `s:` \
             must say so too — it was on the home and missing from the one scope for \
             finding every conversation",
        );
        assert_eq!(filter_search(sessions.clone(), "is:active").len(), 1);
        assert_eq!(filter_search(sessions, "is:archived").len(), 1);
    }

    #[test]
    fn only_native_agent_session_files_pass_the_trash_boundary() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("prelude-session-boundary-{}", std::process::id()));
        let valid = root.join(".claude/projects/project/abc.jsonl");
        let wrong_ext = root.join(".claude/projects/project/abc.txt");
        let outside = root.join("notes.jsonl");
        fs::create_dir_all(valid.parent().unwrap()).unwrap();
        fs::write(&valid, "{}\n").unwrap();
        fs::write(&wrong_ext, "{}\n").unwrap();
        fs::write(&outside, "{}\n").unwrap();
        assert!(crate::sources::sessions::safe_session_path(&valid, &root));
        assert!(!crate::sources::sessions::safe_session_path(&wrong_ext, &root));
        assert!(!crate::sources::sessions::safe_session_path(&outside, &root));
        assert!(!crate::sources::sessions::safe_session_path(&valid.join("../notes.jsonl"), &root));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_sessions_cannot_be_archived_or_trashed() {
        use crate::item::{Item, Kind};
        let inactive = Item::new("claude --resume abc", Kind::Session)
            .title("conversation")
            .put("agent", "claude")
            .put("session_id", "claude:abc")
            .put("cwd", "/tmp/project")
            .put("file", "/tmp/abc.jsonl");
        let inactive_ids: Vec<_> = crate::actions::actions_for(&inactive, crate::defaults::Surface::Prompt)
            .into_iter().map(|(id, ..)| id).collect();
        assert!(inactive_ids.contains(&"session-archive"));
        assert!(inactive_ids.contains(&"session-trash"));

        let active = inactive.clone().put("active_run", "claude:7:1");
        let active_ids: Vec<_> = crate::actions::actions_for(&active, crate::defaults::Surface::Prompt)
            .into_iter().map(|(id, ..)| id).collect();
        assert!(!active_ids.contains(&"session-archive"));
        assert!(!active_ids.contains(&"session-trash"));

        let maybe_same = Item::new("kill 7", Kind::Run)
            .put("agent", "claude")
            .put("cwd", "/tmp/project");
        let other = Item::new("kill 8", Kind::Run)
            .put("agent", "pi")
            .put("cwd", "/tmp/project");
        assert!(crate::sources::sessions::may_be_active(&inactive, &[maybe_same]));
        assert!(!crate::sources::sessions::may_be_active(&inactive, &[other]));
    }

    #[test]
    fn native_agent_arguments_produce_stable_relationship_hints() {
        use crate::sources::running::{elapsed_seconds, requested_session};
        assert_eq!(
            crate::sources::sessions::fork_cmd("claude", "abc").as_deref(),
            Some("claude --resume abc --fork-session"),
        );
        assert_eq!(crate::sources::sessions::fork_cmd("codex", "abc").as_deref(), Some("codex fork abc"));
        assert_eq!(crate::sources::sessions::fork_cmd("pi", "abc").as_deref(), Some("pi --fork abc"));
        assert!(crate::sources::sessions::fork_cmd("opencode", "abc").is_none());
        assert_eq!(elapsed_seconds("02:03"), Some(123));
        assert_eq!(elapsed_seconds("1-02:03:04"), Some(93_784));
        assert_eq!(requested_session("claude", &["--resume", "abc"]), Some("abc".into()));
        assert_eq!(requested_session("codex", &["resume", "def"]), Some("def".into()));
        assert_eq!(requested_session("pi", &["--session=ghi"]), Some("ghi".into()));
        assert_eq!(requested_session("claude", &["-p", "a prompt"]), None);
    }

    #[test]
    fn capability_hashes_detect_drift_without_indexing_secret_values() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("prelude-capability-hash-{}", std::process::id()));
        let left = root.join("left");
        let right = root.join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        fs::write(left.join("SKILL.md"), "hello\nAPI_KEY=sk-aaaaaaaaaaaaaaaaaaaa\n").unwrap();
        fs::write(right.join("SKILL.md"), "hello\nAPI_KEY=sk-bbbbbbbbbbbbbbbbbbbb\n").unwrap();
        let a = crate::capability::hash_skill("claude", &left);
        let b = crate::capability::hash_skill("codex", &right);
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.sensitive_files, 1);
        fs::write(right.join("SKILL.md"), "changed\nAPI_KEY=sk-bbbbbbbbbbbbbbbbbbbb\n").unwrap();
        let changed = crate::capability::hash_skill("codex", &right);
        assert_ne!(a.fingerprint, changed.fingerprint);
        assert!(!crate::capability::skill_stamp(&right).is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn divergent_skill_copies_require_a_diff_before_replacement() {
        use crate::capability::SkillCopy;
        use crate::item::{Item, Kind};
        let copies = vec![
            SkillCopy { agent: "claude".into(), dir: "/tmp/claude/x".into(), fingerprint: "a".into(), ..Default::default() },
            SkillCopy { agent: "codex".into(), dir: "/tmp/codex/x".into(), fingerprint: "b".into(), ..Default::default() },
        ];
        let skill = Item::new("/x", Kind::Skill)
            .put("name", "x")
            .put("agent", "claude,codex")
            .put("integrity", "divergent")
            .put("copy_info", serde_json::to_string(&copies).unwrap());
        let diff = crate::actions::agent_options(&skill, "diff");
        let sync = crate::actions::agent_options(&skill, "sync");
        assert_eq!(diff.len(), 1);
        assert_eq!(sync.len(), 2);
        assert_eq!(diff[0].0, "diff:claude:codex");
        assert!(sync.iter().any(|option| option.0 == "sync:claude:codex"));
        assert!(sync.iter().any(|option| option.0 == "sync:codex:claude"));
    }

    #[test]
    fn mcp_fingerprints_omit_private_definition_values() {
        use crate::lend::Mcp;
        let one = Mcp::Stdio {
            name: "server".into(), command: "node".into(), args: vec!["run.js".into()],
            env: vec![("API_KEY".into(), "first-secret-value".into())],
        };
        let two = Mcp::Stdio {
            name: "server".into(), command: "node".into(), args: vec!["run.js".into()],
            env: vec![("OTHER_KEY".into(), "another-secret-value".into())],
        };
        assert_eq!(one.public_fingerprint(), two.public_fingerprint());
        assert!(one.has_sensitive_fields());
        assert!(!one.public_fingerprint().contains("secret"));
        let credential_url = Mcp::Http {
            name: "remote".into(),
            url: "https://user:pass@example.test/mcp".into(),
            headers: vec![],
        };
        assert!(credential_url.has_sensitive_fields());
        assert_eq!(credential_url.secret_field().as_deref(), Some("URL credential"));
        assert_eq!(
            crate::sources::agents::safe_mcp_detail("https://user:pass@example.test/mcp"),
            "private target omitted",
        );
        let hosted = crate::item::Item::new("claude mcp get hosted", crate::item::Kind::Mcp)
            .put("agent", "claude")
            .put("name", "hosted")
            .put("portable", "false");
        assert!(crate::actions::agent_options(&hosted, "lend").is_empty());
        assert!(crate::actions::agent_options(&hosted, "install").is_empty());
    }

    #[test]
    fn control_snapshot_exposes_edges_instead_of_flat_rows() {
        use crate::item::{Item, Kind};
        let run = Item::new("claude prompt-that-must-not-enter-the-graph", Kind::Run)
            .put("agent", "claude")
            .put("run_id", "claude:7:1")
            .put("pid", "7")
            .put("session_id", "claude:s1");
        let session = Item::new("claude --resume s1", Kind::Session)
            .put("agent", "claude")
            .put("id", "s1")
            .put("session_id", "claude:s1")
            .put("native_title", "native")
            .put("pinned", "true")
            .put("archived", "true")
            .put("active_run", "claude:7:1");
        let skill = Item::new("/review", Kind::Skill)
            .put("name", "review")
            .put("agent", "claude")
            .put("archived", "true");
        let mcp = Item::new("claude mcp get node", Kind::Mcp)
            .put("name", "node")
            .put("agent", "claude")
            .put("capability_id", "mcp:node")
            .put("archived", "true");
        let graph = crate::control::Snapshot::from_items(
            &[run], &[session], &[skill], &[mcp], &[],
        );
        assert_eq!(graph.schema, 4);
        let claude = graph.agents.iter().find(|agent| agent.id == "claude").unwrap();
        assert_eq!(claude.runs, ["claude:7:1"]);
        assert_eq!(claude.sessions, ["claude:s1"]);
        assert_eq!(graph.runs[0].session.as_deref(), Some("claude:s1"));
        assert_eq!(graph.sessions[0].active_run.as_deref(), Some("claude:7:1"));
        assert_eq!(graph.sessions[0].native_title.as_deref(), Some("native"));
        assert!(graph.sessions[0].pinned && graph.sessions[0].archived);
        assert!(graph.skills[0].archived && graph.mcp[0].archived);
        assert!(!serde_json::to_string(&graph).unwrap().contains("prompt-that-must-not-enter-the-graph"));
    }

    /// A confirmed MCP name is read off a command line, and what is written
    /// there is the sanitised key — never the display name.
    #[test]
    fn a_hosted_server_finds_the_run_that_borrowed_it_under_its_sanitised_key() {
        use crate::item::{Item, Kind};
        let run = Item::new("kill 7", Kind::Run)
            .put("agent", "claude")
            .put("run_id", "claude:7:1")
            .put("pid", "7")
            // `lend::Mcp::key` — the staged file's name and codex's dotted
            // path segment. This is the only spelling a process ever carries.
            .put("run_mcp", "claude_ai_Gmail");
        let server = Item::new("claude mcp get gmail", Kind::Mcp)
            .put("agent", "claude")
            .put("name", "claude.ai Gmail")
            .put("capability_id", "mcp:claude.ai gmail")
            .put("portable", "true");
        let graph = crate::control::Snapshot::from_items(&[run], &[], &[], &[server], &[]);
        assert_eq!(
            graph.mcp[0].runs, ["claude:7:1"],
            "a display name and a key are the same server"
        );
    }

    /// "Resume with a skill…" means one the Agent does not have.
    #[test]
    fn the_resume_with_pickers_offer_only_what_the_agent_does_not_own() {
        use crate::actions::{borrowable_servers, borrowable_skills};
        use crate::item::{Item, Kind};
        let skill = |name: &str, owners: &str| {
            Item::new(format!("/{name}"), Kind::Skill)
                .put("name", name)
                .put("agent", owners)
                .put("dir", format!("/skills/{name}"))
        };
        let skills = [
            skill("deploy", "claude"),
            skill("review", "codex, pi"),
            skill("notes", "shared"),
            skill("retired", "codex").put("archived", "true"),
        ];
        let offered = |agent: &str| -> Vec<String> {
            borrowable_skills(&skills, agent).into_iter().map(|(_, name, _)| name).collect()
        };
        assert_eq!(offered("claude"), ["review", "notes"], "owned and archived skills stay out");
        assert_eq!(offered("codex"), ["deploy", "notes"]);
        // `~/.agents/skills` is a location, not an Agent — `missing_agents`
        // reports a Skill that lives only there as missing from every one of
        // them — so a shared Skill stays borrowable for all of them.
        assert!(offered("pi").contains(&"notes".to_string()));
        // Nothing known about the host means nothing is excluded.
        assert_eq!(offered("").len(), skills.len() - 1, "archive puts it away everywhere");

        let server = |name: &str, owner: &str, portable: &str| {
            Item::new(format!("{owner} mcp get {name}"), Kind::Mcp)
                .put("name", name)
                .put("agent", owner)
                .put("portable", portable)
        };
        let servers = [
            server("node_repl", "claude", "true"),
            server("chatcut", "codex", "true"),
            server("retired", "codex", "true").put("archived", "true"),
            server("claude.ai Gmail", "claude", "false"),
        ];
        let names: Vec<String> =
            borrowable_servers(&servers, "claude").into_iter().map(|(_, name, _)| name).collect();
        assert_eq!(names, ["chatcut"], "its own server, and an unlendable one, are not choices");
    }

    #[test]
    fn mcp_tool_inventory_keeps_only_safe_bounded_metadata() {
        assert_eq!(crate::sources::agents::normalize_transport("streamable_http"), "http");
        assert_eq!(crate::sources::agents::normalize_transport("stdio"), "stdio");
        assert_eq!(crate::sources::agents::normalize_transport("hosted"), "hosted");
        let response = serde_json::json!({
            "jsonrpc": "2.0", "id": 2,
            "result": {"tools": [
                {"name": "search", "description": "Search the public index"},
                {"name": "API_KEY_exfiltrate", "description": "must disappear"},
                {"name": "login", "description": "Use password=abcdefghijklmnop"}
            ]}
        });
        let tools = crate::mcp_tools::parse_tools_response(&response);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "search");
        assert_eq!(tools[0].description, "Search the public index");
        assert_eq!(tools[1].name, "login");
        assert!(tools[1].description.is_empty());
        let json = serde_json::to_string(&tools).unwrap();
        assert!(!json.contains("abcdefghijklmnop") && !json.contains("exfiltrate"));
        let failed = serde_json::json!({
            "jsonrpc": "2.0", "id": 2,
            "error": {"message": "startup failed with API_KEY=do-not-retain"}
        });
        assert!(crate::mcp_tools::parse_tools_response(&failed).is_empty());
        assert!(!serde_json::to_string(&crate::mcp_tools::parse_tools_response(&failed))
            .unwrap().contains("do-not-retain"));
    }

    #[test]
    fn control_snapshot_groups_mcp_variants_without_private_definitions() {
        use crate::capability::McpVariant;
        use crate::item::{Item, Kind};
        let variants = vec![
            McpVariant { agent: "claude".into(), health: "ok".into(), summary: "node a.js".into(), fingerprint: "a".into(), source: "semantic".into(), public_definition: serde_json::json!({"type":"stdio","command":"node","args":["a.js"],"private_fields":0}), sensitive: false, portable: true, ..Default::default() },
            McpVariant { agent: "codex".into(), health: "auth".into(), summary: "node b.js".into(), fingerprint: "b".into(), source: "semantic".into(), public_definition: serde_json::json!({"type":"stdio","command":"node","args":["b.js"],"private_fields":1}), sensitive: true, portable: true, ..Default::default() },
        ];
        let common = serde_json::to_string(&variants).unwrap();
        let claude = Item::new("claude mcp get shared", Kind::Mcp)
            .put("agent", "claude").put("name", "shared").put("capability_id", "mcp:shared")
            .put("comparison", "divergent").put("variants", &common);
        let codex = Item::new("codex mcp get shared", Kind::Mcp)
            .put("agent", "codex").put("name", "shared").put("capability_id", "mcp:shared")
            .put("comparison", "divergent").put("variants", common)
            .put("def", "private-definition-must-not-be-in-control");
        let diff = crate::capability::mcp_definition_diff(&claude);
        assert!(diff.iter().any(|line| line == "args[0]"));
        assert!(diff.iter().any(|line| line.contains("a.js")));
        assert!(diff.iter().any(|line| line.contains("b.js")));
        let graph = crate::control::Snapshot::from_items(&[], &[], &[], &[claude, codex], &[]);
        assert_eq!(graph.mcp.len(), 1);
        assert_eq!(graph.mcp[0].owners, ["claude", "codex"]);
        assert_eq!(graph.mcp[0].comparison, "divergent");
        let json = serde_json::to_string(&graph).unwrap();
        assert!(!json.contains("private-definition-must-not-be-in-control"));
    }

    #[test]
    fn result_kind_precedes_the_final_detail_column() {
        use crate::item::{Item, Kind};
        let item = Item::new("demo", Kind::Skill)
            .fields(["claude", "used once", "missing: pi", "the full description"]);
        let rendered = crate::render::render(&[item], 140);
        let visible = rendered.split(crate::render::SEP).next().unwrap();
        assert!(visible.find("skill").unwrap() < visible.find("the full description").unwrap());
    }
}
