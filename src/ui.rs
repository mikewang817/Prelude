//! The interactive surface: fzf invocation, key handling, and what each key
//! does to the selected item.

use crate::ansi::*;
use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::render::{self, SEP};
use std::io::Write;
use std::process::{Command, Stdio};

/// Three keys, deliberately. Enter does the obvious thing, ^K reaches
/// everything else, esc leaves. The old ^O/^X/^Y/^P still work for anyone
/// who learned them, they are simply no longer advertised — a launcher whose
/// header is a row of shortcuts has already failed to have an obvious
/// default.
pub fn header_for(label: &str) -> String {
    format!("{DIM}⏎ {label}   ^K actions   esc close{RESET}")
}

const EXPECT: &str = "ctrl-x,ctrl-k,alt-enter";

pub struct FzfOut {
    pub key: String,
    pub item: Option<Item>,
    pub failed: bool,
    pub stderr: String,
}

fn base_args(prompt: &str, label: &str, header: Option<&str>) -> Vec<String> {
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
        "--color=border:8,label:6,prompt:6,pointer:5,hl:2,hl+:2,info:8,header:8",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(format!("--border-label={label}"));
    a.push(format!("--prompt={prompt}"));
    a.push(format!("--delimiter={SEP}"));
    if let Some(h) = header {
        a.push("--header".into());
        a.push(h.into());
    }
    a
}

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

pub fn run_fzf(feed: &str, args: Vec<String>, cols: usize) -> FzfOut {
    // Three surfaces, in order of preference:
    //   PRELUDE_IN_POPUP  we ARE the popup, so fill it completely — passing
    //                     --height would use a fraction of an already-small
    //                     window and waste the rest.
    //   tmux popup        nicest, but needs tmux >= 3.2 AND an attached
    //                     client; try it and fall back rather than predicting.
    //   inline            N lines under the prompt, fzf-style.
    let mut modes: Vec<Vec<String>> = Vec::new();
    if env_flag("PRELUDE_IN_POPUP") {
        modes.push(vec![]);
    } else {
        if std::env::var_os("TMUX").is_some() && !env_flag("PRELUDE_NO_POPUP") {
            modes.push(vec!["--tmux".into(), "center,80%,80%".into()]);
        }
        let h = std::env::var("PRELUDE_HEIGHT").unwrap_or_else(|_| "60%".into());
        modes.push(vec![format!("--height={h}")]);
    }
    let _ = cols;

    let mut last = FzfOut { key: String::new(), item: None, failed: true, stderr: String::new() };
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
        // result: `--tmux` with no attached client exits 1 ("no current
        // client"), and so does an honest no-match. stderr is what separates
        // them — anything written there means fzf refused to run.
        let failed = !out.status.success() && stdout.trim().is_empty() && !stderr.trim().is_empty();
        if failed {
            if env_flag("PRELUDE_DEBUG") {
                eprintln!("prelude: fzf mode {mode:?} failed: {}", stderr.trim());
            }
            last = FzfOut { key: String::new(), item: None, failed: true, stderr };
            continue;
        }
        let mut lines = stdout.split('\n');
        let key = lines.next().unwrap_or("").trim().to_string();
        let item = lines.find(|l| l.contains(SEP)).and_then(render::parse_line);
        return FzfOut { key, item, failed: false, stderr };
    }
    last
}

pub fn search(paste_target: Option<String>) -> i32 {
    crate::clipd::ensure_running();
    let items = crate::cache::gather();
    if items.is_empty() {
        eprintln!("prelude: nothing to search yet — run some commands first");
        return 2;
    }
    let cols = crate::term_width();
    let feed = render::render(&items, cols, None);

    // Park the static list on disk so the per-keystroke reload only has to
    // cat it, rather than re-gathering every source.
    let static_path = crate::paths::cache().join("list.txt");
    let _ = crate::cache::write_atomic(&static_path, feed.as_bytes());
    let me = std::env::current_exe().unwrap_or_default();
    let me = me.to_string_lossy().into_owned();

    let mut args = base_args("⌕ ", " Prelude ", Some(&header_for("select")));
    args.push(format!("--expect={EXPECT}"));
    args.push("--bind".into());
    args.push(format!(
        "change:transform:{} _bind {{q}} {} {cols}",
        shq(&me),
        shq(&static_path.to_string_lossy())
    ));
    // These two act *inside* the launcher and deliberately do not exit it:
    // execute() hands the terminal over and comes back; execute-silent()
    // doesn't even repaint. This is what lets you do several things per open.
    args.push("--bind".into());
    args.push(format!("enter:transform:{} _enter {{2}}", shq(&me)));
    // Say what Enter will do to *this* row, since the answer depends on what
    // kind of thing it is and on where the launcher was opened from.
    args.push("--bind".into());
    args.push(format!(
        "focus:transform-header:{} _header {{2}}{}",
        shq(&me),
        if paste_target.is_some() { " --agent" } else { "" }
    ));
    args.push("--bind".into());
    args.push(format!("ctrl-o:execute({} _runhere {{2}})", shq(&me)));
    args.push("--bind".into());
    args.push(format!("ctrl-y:execute-silent({} _copy {{2}})", shq(&me)));
    // Only on terminals wide enough that the list doesn't get cramped.
    let min: usize = std::env::var("PRELUDE_PREVIEW_MIN").ok()
        .and_then(|v| v.parse().ok()).unwrap_or(150);
    if cols >= min && !env_flag("PRELUDE_NO_PREVIEW") {
        args.push("--preview".into());
        args.push(format!("{} _preview {{2}}", shq(&me)));
        args.push("--preview-window".into());
        args.push("right,45%,wrap,border-left".into());
        args.push("--bind".into());
        args.push("ctrl-p:toggle-preview".into());
    }

    let out = run_fzf(&feed, args, cols);
    if out.failed {
        let msg = out.stderr.trim().lines().last().unwrap_or("").to_string();
        eprintln!("prelude: fzf could not start{}", if msg.is_empty() { String::new() } else { format!(": {msg}") });
        return 2;
    }
    let Some(item) = out.item else { return 130 };

    match out.key.as_str() {
        "ctrl-k" => crate::actions::panel(&item, paste_target),
        "ctrl-x" | "alt-enter" => {
            crate::frecency::bump(&item.cmd);
            emit("RUN", &item.cmd, &paste_target);
            0
        }
        _ => apply_default(&item, &paste_target),
    }
}

/// Carry out whatever Enter means for this item in this host.
pub fn apply_default(item: &Item, paste: &Option<String>) -> i32 {
    use crate::defaults::{on_enter, text_for, Default_, Host, Verb};
    let host = if paste.is_some() { Host::Agent } else { Host::Shell };
    crate::frecency::bump(&item.cmd);
    match on_enter(item, host) {
        Default_::Insert => {
            let cmd = if item.kind == Kind::Snippet {
                fill_placeholders(&item.cmd)
            } else {
                item.cmd.clone()
            };
            emit("INSERT", &cmd, paste);
        }
        Default_::InsertText(what) => emit("INSERT", &text_for(item, what), paste),
        Default_::Act(verb) => return act(item, verb, paste),
    }
    0
}

fn act(item: &Item, verb: crate::defaults::Verb, paste: &Option<String>) -> i32 {
    use crate::defaults::Verb::*;
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    match verb {
        // Opening a file, launching an app and following a link are all
        // harmless and reversible, unlike running a shell command — which is
        // what the insert-first rule actually exists to guard.
        OpenInEditor => {
            let p = if item.get("path").is_empty() { item.cmd.clone() } else { item.get("path").into() };
            emit("RUN", &format!("{editor} {}", shq(&p)), paste);
        }
        Launch | OpenUrl => emit("RUN", &item.cmd, paste),
        OpenConfig => {
            let p = first_of(item, &["config", "path", "file"]);
            emit("RUN", &format!("{editor} {}", shq(&p)), paste);
        }
        CopyResult => {
            copy(&item.cmd);
            eprintln!("copied: {}", item.cmd);
        }
        ResumeSession => emit("INSERT", &item.cmd, paste),
        RunHere => return crate::runhere::run_item(item),
        RunSkill => {
            // A skill name means nothing to a shell, so pick an agent that
            // actually has it and hand the invocation over.
            let agent = item.get("agent").split(',').next().unwrap_or("claude").trim().to_string();
            let agent = if agent == "shared" { "claude".to_string() } else { agent };
            emit("INSERT", &format!("{agent} {}", shq(&item.cmd)), paste);
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

/// The contract with the shell widget: one line, VERB<TAB>payload.
pub fn emit(verb: &str, cmd: &str, paste_target: &Option<String>) {
    if let Some(pane) = paste_target {
        paste_into_pane(pane, cmd, verb == "RUN");
        return;
    }
    println!("{verb}\t{cmd}");
}

/// Type text into another pane, exactly as if the user had typed it.
///
/// `send-keys -l` sends the string literally, so whatever is reading that
/// pane's input — a shell, an agent's prompt box, vim — receives it as
/// keystrokes. Nothing is submitted unless `run` is set: the
/// insert-don't-execute rule matters even more here, because in an agent
/// conversation Enter sends a message rather than running a command.
pub fn paste_into_pane(pane: &str, text: &str, run: bool) {
    let d = std::time::Duration::from_secs(5);
    crate::exec::run(&["tmux", "send-keys", "-t", pane, "-l", text], d);
    if run {
        crate::exec::run(&["tmux", "send-keys", "-t", pane, "Enter"], d);
    }
}

/// Work out which pane to type into.
///
/// `display-popup` does NOT expand #{...} formats in its shell-command
/// argument, so an explicitly-passed id can arrive as the literal string
/// "#{pane_id}". Happily a popup doesn't need to be told: tmux commands run
/// inside one still resolve to the underlying active pane, so we just ask.
pub fn resolve_pane(arg: Option<&str>) -> Option<String> {
    if let Some(a) = arg {
        if !a.starts_with("#{") && !a.is_empty() {
            return Some(a.to_string());
        }
    }
    let out = crate::exec::run(
        &["tmux", "display-message", "-p", "#{pane_id}"],
        std::time::Duration::from_secs(1),
    );
    let p = out.trim();
    if p.is_empty() { None } else { Some(p.to_string()) }
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
pub fn pick_raw(feed: &str, label: &str, prompt: &str, header: &str) -> Option<String> {
    let args = base_args(prompt, label, Some(&format!("{DIM}{header}{RESET}")));
    let mut modes: Vec<Vec<String>> = Vec::new();
    if env_flag("PRELUDE_IN_POPUP") {
        modes.push(vec![]);
    } else {
        if std::env::var_os("TMUX").is_some() && !env_flag("PRELUDE_NO_POPUP") {
            modes.push(vec!["--tmux".into(), "center,80%,80%".into()]);
        }
        modes.push(vec!["--height=60%".into()]);
    }
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
