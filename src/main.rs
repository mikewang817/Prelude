//! Prelude — a Raycast-style launcher for the terminal.
//!
//! One hotkey, fuzzy search across everything you might want to run, and it
//! *types the command for you* rather than running it.

mod actions;
mod ansi;
mod bus;
mod cache;
mod calc;
mod clipd;
mod compute;
mod defaults;
mod doctor;
mod exec;
mod fleet;
mod frecency;
mod init;
mod item;
mod lend;
mod minitoml;
mod openwith;
mod paths;
mod preview;
mod probe;
mod render;
mod runhere;
mod secrets;
mod sources;
mod translate_build;
mod tty;
mod ui;
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

    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = match a.as_slice() {
        [] => ui::search(None),
        ["paste"] => ui::search(ui::resolve_pane(None)),
        ["paste", pane] => ui::search(ui::resolve_pane(Some(pane))),
        ["doctor"] => doctor::run(),
        ["bench"] => bench(),
        ["fleet"] => fleet::list(false),
        ["fleet", "--json"] => fleet::list(true),
        ["fleet", "--status"] => fleet::status(),
        ["watch"] => fleet::watch(),

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
        ["tell", rest @ ..] if !rest.is_empty() => bus::tell(&rest.join(" ")),
        ["say", target, rest @ ..] if !rest.is_empty() => bus::say(target, &rest.join(" ")),
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
            json_dump(sources::sessions::all(), rest.contains(&"--json"))
        }
        ["skills", rest @ ..] => {
            json_dump(sources::agents::skills(), rest.contains(&"--json"))
        }
        ["index"] => {
            println!("building file index ...");
            let n = compute::build_fileindex();
            println!("  indexed {n} files from: {}", compute::index_roots().join(", "));
            println!("  search them with  f:name  in the launcher");
            0
        }
        ["clipd"] => clipd::watch(),
        ["build-translate"] => translate_build::build(),
        ["init", "zsh"] => { print!("{}", init::ZSH); 0 }
        ["init", "tmux"] => { print!("{}", init::TMUX); 0 }
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
                for (id, label, sub) in actions::actions_for(&i) {
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
        ["_ask", line] => match render::parse_line(line) {
            Some(i) => ui::ask(&i),
            None => 1,
        },
        // Decides, per keypress of Enter, whether to accept the selection or
        // answer it in place. Only fzf can make that conditional.
        ["_enter", line] => {
            let ask = render::parse_line(line)
                .is_some_and(|i| i.get("mode") == "start" && !i.get("prompt").is_empty());
            let me = exec::shq(&std::env::current_exe().unwrap_or_default().to_string_lossy());
            if ask {
                println!("change-preview-window(right,55%,wrap,border-left)+preview({me} _ask {{2}})");
            } else {
                println!("accept");
            }
            0
        }
        ["_footer", rest @ ..] => {
            let agent = rest.contains(&"--agent");
            let host = if agent { defaults::Host::Agent } else { defaults::Host::Shell };
            // Which agent, when the pane could say — `search` resolved it
            // once and passed it down, exactly as it does the column widths.
            if let Some(i) = rest.iter().position(|a| *a == "--agent") {
                defaults::set_host_agent(rest.get(i + 1).map(|s| s.to_string()));
            }
            let item = rest.first().and_then(|l| render::parse_line(l));
            let primary = item
                .as_ref()
                .map(|i| defaults::describe(i, host))
                .unwrap_or("Select")
                .to_string();
            println!("{}", ui::footer_for(&primary));
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
        println!("{}\t{}\t{}", it.kind.style().1, it.title, sub);
    }
    0
}

const HELP: &str = "\
prelude — a launcher for terminals, and a message bus for the agents in them

HUMANS
  prelude                search (what the Ctrl-R widget calls)
  prelude paste [pane]   type the result into a tmux pane instead
  prelude reply          answer the oldest question an agent is blocked on
  prelude fleet          every agent running on this machine, and its state
  prelude fleet --status one short line for a tmux status bar
  prelude watch          notify the moment an agent stops and waits for you

AGENTS  (run these from inside a conversation; see `prelude init agent`)
  prelude ask   TEXT     ask the human a question and wait for the answer
                         answer goes to stdout · exit 3 if nobody answered
                         --timeout=N seconds (default 600) · --no-wait
  prelude tell  TEXT     tell the human something, without waiting
  prelude say   WHO TEXT send a line to another running agent
                         WHO is a project, an agent name, or a pane address
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
  prelude init zsh|tmux|agent   shell, tmux, and the block for CLAUDE.md
  prelude index          build the file index for f:name
  prelude doctor         diagnose the setup
  prelude bench          measure candidate-gathering
  prelude build-translate  compile the Apple translation helper
";

/// Decide, per keystroke, what fzf should do.
///
/// fzf matches against the *displayed* text, so a computed row (a sum, a
/// translation) can never fuzzy-match the query that produced it — you'd type
/// `en:...` and watch your own answer get filtered out. For those queries we
/// turn fzf's own filtering off and let our row stand at the top; for
/// ordinary queries we leave fuzzy search exactly as it was.
fn bind(q: &str, path: &str, cols: &str, tw: &str) -> i32 {
    let me = std::env::current_exe().unwrap_or_default();
    let me = exec::shq(&me.to_string_lossy());
    let search = if compute::is_special(q) { "disable-search" } else { "enable-search" };
    // The prefix hints belong to the empty query and nothing else: once there
    // is a query there are results to read, and a row of syntax above them is
    // in the way rather than helpful.
    let header = if q.is_empty() { ui::HINTS } else { "" };
    println!("{search}+change-header({header})+reload({me} _dynamic {} {} {} {})",
             exec::shq(q), exec::shq(path), exec::shq(cols), exec::shq(tw));
    0
}

/// Emit query-dependent rows, then the pre-rendered static list.
///
/// `cols` is passed in rather than measured: fzf runs this with stdout on a
/// pipe, so a measurement would fall back to a default and the computed rows
/// would drift ten columns out of step with the static ones.
fn dynamic(q: &str, path: &str, cols: usize, tw: Option<usize>) -> i32 {
    let rows = compute::dynamic_rows(q);
    if !rows.is_empty() {
        println!("{}", render::render_with(&rows, cols, None, tw));
    }
    // Once the query has clearly declared an intent — a sum, a translation,
    // an agent to start — the 2000-odd unrelated rows underneath are noise.
    // Show only what was asked for.
    if compute::is_special(q) {
        return 0;
    }
    if !path.is_empty() {
        if let Ok(t) = std::fs::read_to_string(path) {
            print!("{t}");
            if !t.ends_with('\n') {
                println!();
            }
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
/// blank — and, because the detail pane is gated on the same number, turned
/// the pane off as well. Ask the other standard descriptors, then the
/// controlling terminal itself, before giving up.
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
        use crate::defaults::{on_enter, on_secondary, Default_, Host};
        use crate::item::{Item, Kind::*};
        // Commands are never acted on, in either host.
        for k in [History, Script, Path, Snippet, Port, Proc, Sys] {
            let it = Item::new("x", k);
            for h in [Host::Shell, Host::Agent] {
                assert_eq!(on_enter(&it, h), Default_::Insert, "{k:?} in {h:?}");
            }
        }
        // Objects act at a shell but hand over text to an agent.
        for k in [File, App, Link] {
            let it = Item::new("x", k);
            assert!(matches!(on_enter(&it, Host::Shell), Default_::Act(_)), "{k:?}");
            assert!(matches!(on_enter(&it, Host::Agent), Default_::InsertText(_)), "{k:?}");
        }
        // The secondary sits directly under Enter's own entry in the ^K
        // panel, so it must differ from it rather than repeat it — two rows
        // saying the same thing is worse than one.
        for k in [History, Script, File, App, Calc, Clip, Session] {
            let it = Item::new("x", k);
            let p = on_enter(&it, Host::Shell);
            if let Some(s) = on_secondary(&it, Host::Shell) {
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

    /// Removing the secondary's key is only defensible because the action
    /// itself did not go anywhere. Pin that: the panel's first two entries
    /// are what Enter does and its opposite, in that order.
    #[test]
    fn the_panel_still_carries_the_secondary() {
        use crate::defaults::{describe, describe_secondary, Host};
        use crate::item::{Item, Kind};
        let it = Item::new("/tmp/x.txt", Kind::File).title("x.txt").put("path", "/tmp/x.txt");
        let acts = crate::actions::actions_for(&it);
        assert_eq!(acts[0].0, "default");
        assert_eq!(acts[0].1, describe(&it, Host::Shell), "first entry states what Enter does");
        assert_eq!(acts[1].0, "secondary");
        assert_eq!(acts[1].1, describe_secondary(&it, Host::Shell).unwrap());
        assert_ne!(acts[0].1, acts[1].1, "two rows saying the same thing is worse than one");
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
        let ids: Vec<&str> = crate::actions::actions_for(&it).iter().map(|(i, ..)| *i).collect();
        // Never back to the agent that already has it.
        assert!(!ids.contains(&"lend:codex"), "{ids:?}");
        // pi and opencode have no way to take one, so they are not offered
        // however many of them are installed.
        assert!(!ids.contains(&"lend:pi") && !ids.contains(&"lend:opencode"), "{ids:?}");
        if crate::sources::sessions::installed().contains(&"claude") {
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
        use crate::defaults::{on_enter, Default_, Host, Verb};
        use crate::item::{Item, Kind::*};
        // Anything that becomes a command line, including an agent.
        for k in [Agent, History, Script, Snippet, Port, Proc, Dir, Clip] {
            assert_eq!(on_enter(&Item::new("x", k), Host::Shell), Default_::Insert, "{k:?}");
        }
        // Objects, where there is no command worth reading.
        for k in [File, Find, Config] {
            assert_eq!(on_enter(&Item::new("x", k), Host::Shell), Default_::Act(Verb::Open), "{k:?}");
        }
        // Mid-conversation there is no prompt to paste onto, so "start it"
        // can only mean a window beside the one you are in.
        assert_eq!(
            on_enter(&Item::new("pi", Agent), Host::Agent),
            Default_::Act(Verb::SplitPane)
        );
    }

    /// Opening is always by application and never by executing the file, and
    /// an application with a space in its name is one argument.
    #[test]
    fn open_commands_are_well_formed() {
        use crate::openwith::{ext_of, open_cmd};
        assert_eq!(open_cmd("/tmp/x.json", None), "open /tmp/x.json");
        assert_eq!(open_cmd("/tmp/a b.json", None), "open '/tmp/a b.json'");
        assert_eq!(
            open_cmd("/tmp/x.json", Some("Visual Studio Code")),
            "open -a 'Visual Studio Code' /tmp/x.json"
        );
        // An empty remembered app must fall back, not produce `open -a ''`.
        assert_eq!(open_cmd("/tmp/x.json", Some("  ")), "open /tmp/x.json");
        assert_eq!(ext_of("/a/b/.claude.json"), "json");
        assert_eq!(ext_of("/a/b/Makefile"), "");
        assert_eq!(ext_of("/a/b/X.JSON"), "json", "extensions are matched case-insensitively");
    }

    /// The one signal that makes a fleet usable: an agent that is working
    /// prints — tokens, tool output, a spinner — so tmux's activity clock
    /// keeps moving. Silence is what a question looks like from outside the
    /// process, and those are the runs holding you up, so they sort first.
    #[test]
    fn a_quiet_agent_is_one_that_is_waiting_for_you() {
        use crate::sources::running::{classify, State, Turn};
        // With no conversation to read, silence is all there is to go on.
        assert!(matches!(classify(false, 0, None), State::Working));
        assert!(matches!(classify(false, 29, None), State::Working));
        assert!(matches!(classify(false, 30, None), State::Waiting));
        assert!(matches!(classify(false, 9000, None), State::Waiting));
        // A dead pane is not waiting for anything, whatever its clock says.
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
        let mk = |agent: &str, project: &str, addr: &str, pane: &str| {
            Item::new("x", Kind::Run)
                .put("agent", agent)
                .put("project", project)
                .put("addr", addr)
                .put("pane", pane)
                .put("cwd", format!("/Users/x/{project}"))
        };
        let runs = vec![
            mk("claude", "api-gateway", "work:2.1", "%4"),
            mk("claude", "api-gateway-tests", "work:2.2", "%5"),
            mk("codex", "docs", "fleet:0.1", "%6"),
        ];
        // An exact project name wins outright, even though it is also a
        // prefix of another project's.
        let hit = resolve(&runs, "api-gateway");
        assert_eq!(hit.len(), 1, "exact match must not widen: {:?}", hit.len());
        assert_eq!(hit[0].get("addr"), "work:2.1");
        // Addressable the ways an agent might plausibly try.
        assert_eq!(resolve(&runs, "work:2.2").len(), 1);
        assert_eq!(resolve(&runs, "%6").len(), 1);
        assert_eq!(resolve(&runs, "docs")[0].get("agent"), "codex");
        // Two claudes and a bare agent name: caller must be told, not guessed
        // at. `say` refuses on anything but exactly one hit.
        assert_eq!(resolve(&runs, "claude").len(), 2);
        // And a substring that spans both projects is likewise ambiguous.
        assert_eq!(resolve(&runs, "api").len(), 2);
        assert!(resolve(&runs, "nothing-like-this").is_empty());
    }

    /// A question is answered, gone to, or left alone — never run. It arrives
    /// as an English sentence, and "Run in the shell below" on a sentence is
    /// the launcher offering to execute prose.
    #[test]
    fn a_question_offers_answers_and_never_execution() {
        use crate::defaults::{describe, Host};
        use crate::item::{Item, Kind};
        let it = Item::new("Proceed with the migration?", Kind::Msg)
            .title("claude · api asks")
            .put("id", "123-4")
            .put("agent", "claude")
            .put("pane", "%4");
        assert_eq!(describe(&it, Host::Shell), "Answer it");
        // The same wherever you are standing: there is one thing to do with
        // a question, and the host does not change it.
        assert_eq!(describe(&it, Host::Agent), "Answer it");
        let acts = crate::actions::actions_for(&it);
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

    /// A question an agent is blocked on outranks everything, and it has to
    /// outrank it by more than the learned ranking can make up.
    ///
    /// This was a real bug: Msg sat ten points above Agent, frecency adds up
    /// to sixty, and so the claude row you pick every day floated above a
    /// question waiting on you. The band has to clear the whole bonus.
    #[test]
    fn a_question_outranks_everything_frecency_can_lift() {
        use crate::item::Kind;
        let msg = Kind::Msg.priority();
        // 60 is the cap `finish` applies to the frecency bonus.
        for k in Kind::all() {
            if *k == Kind::Msg {
                continue;
            }
            assert!(
                k.priority() + 60 < msg,
                "{k:?} at {} + frecency can reach a blocked question at {msg}",
                k.priority()
            );
        }
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
}
