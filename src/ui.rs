//! The interactive surface: fzf invocation, key handling, and what each key
//! does to the selected item.

use crate::ansi::*;
use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::render::{self, SEP};
use std::io::Write;
use std::process::{Command, Stdio};

/// Two keys. Enter does the obvious thing, ^K reaches everything else, esc
/// leaves. The old ^O/^X/^Y/^P still work for anyone who learned them, they
/// are simply no longer advertised — a launcher whose header is a row of
/// shortcuts has already failed to have an obvious default.
///
/// The footer says what the action is, then the key that runs it, along the
/// bottom of the panel. Keys are spelled out rather than drawn as glyphs — a
/// row of symbols is only legible to someone who already knows what they
/// mean. And since Enter does something different per item, the row has to
/// say which.
pub fn footer_for(primary: &str) -> String {
    let sep = format!("{DIM}   ·   {RESET}");
    [
        format!("{primary}{DIM}  Enter{RESET}"),
        format!("Actions{DIM}  Ctrl+K{RESET}"),
    ]
    .join(&sep)
}

/// Share of the width the list keeps when the detail pane is showing.
const PREVIEW_LIST_PCT: usize = 55;

/// The prefix language, stated once, where it cannot be missed.
///
/// Half of what this launcher can do is behind a prefix — `s:` for past
/// conversations, `r:` for the live fleet, `@` to put a question to an agent
/// — and none of it appeared anywhere in the interface. It was documented,
/// which is not the same as discoverable: a feature you have to read a README
/// to find is a feature most people never have.
///
/// It shows only on an empty query and disappears the moment you type, so it
/// costs a row exactly when there is nothing else to look at and never
/// competes with results.
pub const HINTS: &str = "s: conversations   r: running agents   f: files   @ ask an agent";

/// One key beyond Enter, and it is a Ctrl key because those are the only
/// ones a terminal reliably receives. macOS spends Option on composing
/// characters unless the terminal is told otherwise, and never delivers Cmd
/// at all; a key that works on one machine and silently does nothing on the
/// next is worse than no key.
///
/// The secondary action has no key of its own. It is not gone — it is the
/// second entry of the ^K panel, where it is also spelled out rather than
/// remembered.
const EXPECT: &str = "ctrl-x,ctrl-k";

pub struct FzfOut {
    pub key: String,
    pub item: Option<Item>,
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
        "--color=border:8,label:6,prompt:6,pointer:5,hl:2,hl+:2,info:8,header:8",
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
            modes.push(vec!["--tmux".into(), "center,92%,92%".into()]);
        }
        // Most of the terminal, not half of it: a launcher that shows twelve
        // rows out of two thousand is making you scroll for no reason.
        let h = std::env::var("PRELUDE_HEIGHT").unwrap_or_else(|_| "90%".into());
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
    // Before anything else, because it changes what Enter means on a skill.
    let host_agent = paste_target.as_deref().and_then(agent_in_pane);
    crate::defaults::set_host_agent(host_agent.map(str::to_string));
    crate::clipd::ensure_running();
    let items = crate::cache::gather();
    if items.is_empty() {
        eprintln!("prelude: nothing to search yet — run some commands first");
        return 2;
    }
    let term = crate::term_width();
    // Only on terminals wide enough that the list doesn't get cramped.
    let preview_min: usize = std::env::var("PRELUDE_PREVIEW_MIN").ok()
        .and_then(|v| v.parse().ok()).unwrap_or(150);
    let preview = term >= preview_min && !env_flag("PRELUDE_NO_PREVIEW");
    // Lay out against the *list* width, not the terminal's. With a detail
    // pane taking 45%, measuring the whole terminal overflows every row.
    let cols = if preview { term * PREVIEW_LIST_PCT / 100 } else { term };

    // Compute the layout once and hand it to the per-keystroke helper. Both
    // sides must agree; if each measured its own, computed rows would land
    // in a different column from the static ones.
    let tw = render::title_width(&items, cols);
    let widths = render::column_widths(&items, render::middle_budget(cols, tw));
    let feed = render::render_with(&items, cols, Some(&widths), Some(tw));

    // Park the static list on disk so the per-keystroke reload only has to
    // cat it, rather than re-gathering every source.
    let static_path = crate::paths::cache().join("list.txt");
    let _ = crate::cache::write_atomic(&static_path, feed.as_bytes());
    let me = std::env::current_exe().unwrap_or_default();
    let me = me.to_string_lossy().into_owned();

    let mut args = base_args("⌕ ", " Prelude ", Some(&footer_for("Select")));
    args.push(format!("--header={HINTS}"));
    args.push(format!("--expect={EXPECT}"));
    args.push("--bind".into());
    args.push(format!(
        "change:transform:{} _bind {{q}} {} {cols} {tw}",
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
        "focus:transform-footer:{} _footer {{2}}{}",
        shq(&me),
        match (&paste_target, host_agent) {
            // The helper is a separate process, so who we are typing into
            // travels on its argv rather than being asked for again on every
            // keystroke. The name is one of a fixed set, never user text.
            (Some(_), Some(a)) => format!(" --agent {a}"),
            (Some(_), None) => " --agent".to_string(),
            (None, _) => String::new(),
        }
    ));
    args.push("--bind".into());
    args.push(format!("ctrl-o:execute({} _runhere {{2}})", shq(&me)));
    args.push("--bind".into());
    args.push(format!("ctrl-y:execute-silent({} _copy {{2}})", shq(&me)));
    if preview {
        args.push("--preview".into());
        args.push(format!("{} _preview {{2}}", shq(&me)));
        args.push("--preview-window".into());
        args.push(format!("right,{}%,wrap,border-left", 100 - PREVIEW_LIST_PCT));
        args.push("--bind".into());
        args.push("ctrl-p:toggle-preview".into());
    }

    loop {
        let out = run_fzf(&feed, args.clone(), cols);
        if out.failed {
            let msg = out.stderr.trim().lines().last().unwrap_or("").to_string();
            eprintln!("prelude: fzf could not start{}", if msg.is_empty() { String::new() } else { format!(": {msg}") });
            return 2;
        }
        let Some(item) = out.item else { return 130 };

        match out.key.as_str() {
            "ctrl-k" => {
                let code = crate::actions::panel(&item, paste_target.clone());
                if code == crate::actions::PANEL_BACK {
                    // ^K is a modal over the list. Esc backs out one level
                    // instead of closing the launcher entirely.
                    continue;
                }
                return code;
            }
            "ctrl-x" => {
                crate::frecency::bump(&item.cmd);
                emit("RUN", &item.cmd, &paste_target);
                return 0;
            }
            _ => return apply_default(&item, &paste_target),
        }
    }
}

/// Carry out whatever Enter means for this item in this host.
pub fn apply_default(item: &Item, paste: &Option<String>) -> i32 {
    use crate::defaults::{on_enter, Host};
    let host = if paste.is_some() { Host::Agent } else { Host::Shell };
    crate::frecency::bump(&item.cmd);
    perform(item, on_enter(item, host), paste)
}

pub fn perform(item: &Item, what: crate::defaults::Default_, paste: &Option<String>) -> i32 {
    use crate::defaults::{text_for, Default_};
    match what {
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
    match verb {
        // Opening a file, launching an app and following a link are all
        // harmless and reversible, unlike running a shell command — which is
        // what the insert-first rule actually exists to guard.
        // `open` hands the file to the application that owns it, or to the
        // one the user picked in ^K. It goes onto the prompt like any other
        // command rather than being spawned from here, so the shell owns the
        // process and you can see what ran.
        Open => {
            let p = first_of(item, &["path", "file", "config"]);
            let p = if p.is_empty() { item.cmd.clone() } else { p };
            emit("RUN", &crate::openwith::open_default(&p), paste);
        }
        // Beside the conversation, not on top of it and not off in another
        // window: the reason to start a second agent while talking to the
        // first is to watch both. It opens in the directory the pane
        // underneath is in, since that is the project you were discussing.
        //
        // tmux is guaranteed here — `Host::Agent` is only ever reached
        // through the popup, which is a tmux binding — so the fallback is
        // for the impossible case rather than a supported one.
        // Move the cursor to a pane somewhere in the tmux server. Inside
        // tmux the client is simply pointed at it; from a terminal with no
        // tmux client of its own, the only way to "go there" is to attach,
        // which takes over this terminal — so that one goes on the prompt to
        // be agreed to rather than happening under you.
        JumpTo => return act_jump(item, paste, false),
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
        SplitPane => {
            let Some(pane) = paste else {
                emit("INSERT", &item.cmd, paste);
                return 0;
            };
            let d = std::time::Duration::from_secs(2);
            let info = crate::exec::run(
                &["tmux", "display", "-p", "-t", pane, "#{pane_current_path}\t#{window_width}"],
                d,
            );
            let (cwd, width) = info.trim().split_once('\t').unwrap_or((info.trim(), ""));
            let (cwd, width) = (cwd.to_string(), width.parse::<usize>().unwrap_or(0));
            // Side by side while there is room for two — an agent TUI below
            // about eighty columns starts wrapping its own output, so two of
            // them in a narrow window is worse than one above the other.
            let how = if width >= 170 { "-h" } else { "-v" };
            let mut argv: Vec<&str> = vec!["tmux", "split-window", how, "-t", pane];
            if !cwd.is_empty() {
                argv.push("-c");
                argv.push(&cwd);
            }
            argv.push(&item.cmd);
            crate::exec::run(&argv, d);
        }
        Launch | OpenUrl => emit("RUN", &item.cmd, paste),
        CopyResult => {
            copy(&item.cmd);
            eprintln!("copied: {}", item.cmd);
        }
        ResumeSession => emit("INSERT", &item.cmd, paste),
        RunHere => return crate::runhere::run_item(item),
        RunInShell => {
            let cmd = if item.kind == Kind::Snippet {
                fill_placeholders(&item.cmd)
            } else {
                item.cmd.clone()
            };
            emit("RUN", &cmd, paste);
        }
        Inspect => {
            let c = if item.kind == Kind::Proc {
                format!("ps -p {} -o command=", item.get("pid"))
            } else {
                format!("lsof -nP -iTCP:{} -sTCP:LISTEN", item.get("port"))
            };
            emit("INSERT", &c, paste);
        }
        CdThere => {
            let d = if item.get("cwd").is_empty() { item.get("path") } else { item.get("cwd") };
            emit("INSERT", &format!("cd {}", shq(d)), paste);
        }
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
/// touching what is on it and clears itself at the next keystroke. In a pane
/// there is no widget, so tmux's own message line does the same job.
///
/// `MSG` is terminal — an action either did something or explains why it did
/// not, never both — which keeps the widget's one-line contract intact.
pub fn note(text: &str, paste_target: &Option<String>) {
    if paste_target.is_some() {
        crate::exec::run(
            &["tmux", "display-message", &format!("prelude: {text}")],
            std::time::Duration::from_secs(1),
        );
        return;
    }
    // Newlines would break the widget's one-line contract.
    println!("MSG\t{}", crate::width::flatten(text));
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
/// Which agent is running in the pane we are about to type into.
///
/// tmux knows, and says so plainly — a pane running Claude Code reports
/// `claude`, not `node`. Worth asking, because it decides whether a skill
/// row hands over `/name` or the path to the skill's own file: the slash
/// command means nothing to an agent that does not have it, and means
/// nothing *silently*.
///
/// Asked once, in `search`, never per keystroke.
pub fn agent_in_pane(pane: &str) -> Option<&'static str> {
    let out = crate::exec::run(
        &["tmux", "display", "-p", "-t", pane, "#{pane_current_command}"],
        std::time::Duration::from_secs(1),
    );
    let cmd = out.trim().to_string();
    crate::sources::sessions::AGENTS
        .iter()
        .map(|a| a.name)
        .find(|n| *n == cmd)
}

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
/// Ask for a line of free text, using the surface already in front of you.
///
/// fzf with `--print-query` over an empty list: whatever you type is the
/// answer. A second binary for a one-line prompt would be a dependency, and
/// this way the input box looks exactly like the one you just came from.
pub fn prompt_line(label: &str) -> Option<String> {
    let mut args = base_args("› ", label, Some(&format!("{DIM}Send  Enter   ·   Cancel  Esc{RESET}")));
    args.push("--print-query".into());
    args.push("--no-info".into());
    let out = run_fzf("", args, 0);
    if out.failed {
        return None;
    }
    let line = out.key.trim().to_string();
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
    pick_raw(&feed, &format!(" {question} "), "? ", "Choose  Enter   ·   Cancel  Esc", "")
        .as_deref()
        == Some("yes")
}

/// Put the cursor in a running agent's pane, optionally zoomed.
///
/// Inside tmux the client is simply pointed at it. From a terminal with no
/// tmux client of its own, the only way to "go there" is to attach, which
/// takes over this terminal — so that goes onto the prompt to be agreed to
/// rather than happening under you.
pub fn act_jump(item: &Item, paste: &Option<String>, zoom: bool) -> i32 {
    let pane = item.get("pane");
    if pane.is_empty() {
        emit("INSERT", &item.cmd, paste);
        return 0;
    }
    let d = std::time::Duration::from_secs(2);
    if std::env::var_os("TMUX").is_some() {
        crate::exec::run(&["tmux", "select-window", "-t", pane], d);
        crate::exec::run(&["tmux", "select-pane", "-t", pane], d);
        if zoom {
            crate::exec::run(&["tmux", "resize-pane", "-Z", "-t", pane], d);
        }
        crate::exec::run(&["tmux", "switch-client", "-t", pane], d);
        0
    } else {
        let z = if zoom { format!(" \\; resize-pane -Z -t {pane}") } else { String::new() };
        emit(
            "RUN",
            &format!("tmux select-window -t {pane} \\; select-pane -t {pane}{z} \\; attach -t {pane}"),
            paste,
        );
        0
    }
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
    let mut modes: Vec<Vec<String>> = Vec::new();
    if env_flag("PRELUDE_IN_POPUP") {
        modes.push(vec![]);
    } else {
        if std::env::var_os("TMUX").is_some() && !env_flag("PRELUDE_NO_POPUP") {
            modes.push(vec!["--tmux".into(), "center,92%,92%".into()]);
        }
        modes.push(vec!["--height=90%".into()]);
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
