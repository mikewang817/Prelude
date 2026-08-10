//! The interactive surface: fzf invocation, key handling, and what each key
//! does to the selected item.

use crate::ansi::*;
use crate::exec::shq;
use crate::item::{Item, Kind};
use crate::render::{self, SEP};
use std::io::Write;
use std::process::{Command, Stdio};

/// Enter does the obvious thing, ^K reaches every alternative, ^P briefly
/// replaces the list with Quick Look, and esc leaves. Preview earns a place
/// in the footer because it is a mode rather than an item action: it does not
/// belong in ^K, and a hidden one-key view is not discoverable.
///
/// Keys are spelled out rather than drawn as glyphs — a row of symbols is
/// only legible to someone who already knows what they mean. And since Enter
/// does something different per item, the row has to say which.
pub fn footer_for(primary: &str) -> String {
    let sep = format!("{DIM}   ·   {RESET}");
    let mut parts = vec![
        format!("{primary}{DIM}  Enter{RESET}"),
        format!("Actions{DIM}  Ctrl+K{RESET}"),
    ];
    if crate::settings::preview_enabled() {
        parts.push(format!("Preview{DIM}  Ctrl+P{RESET}"));
    }
    parts.join(&sep)
}

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
pub const HINTS: &str = "@ ask agent   / skill   s: sessions   f: files   c: clipboard   : scopes";

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

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

pub fn run_fzf(feed: &str, args: Vec<String>, cols: usize) -> FzfOut {
    // Two surfaces:
    //   PRELUDE_FULL_SURFACE  the window is already ours — the global panel,
    //                         which exists to hold this and nothing else — so
    //                         fill it. `--height` would use a fraction of an
    //                         already-small window and waste the rest.
    //   inline                N lines under the prompt, fzf-style.
    let mut modes: Vec<Vec<String>> = Vec::new();
    if env_flag("PRELUDE_FULL_SURFACE") {
        modes.push(vec![]);
    } else {
        // Most of the terminal, not half of it: a launcher that shows twelve
        // rows out of two thousand is making you scroll for no reason.
        modes.push(vec![format!("--height={}", crate::settings::height())]);
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
        // result: a refusal to start and an honest no-match both exit 1.
        // stderr is what separates them — anything written there means fzf
        // never ran.
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

pub fn search() -> i32 {
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
    let root_feed = render::render_with(&root, cols, Some(&widths), Some(tw));
    let home = crate::compute::home_items(&items);
    let home_feed = render::render_with(&home, cols, Some(&widths), Some(tw));

    // Root Search has two deliberately small surfaces: the agent home before
    // typing, and agent/Quicklink/search commands afterwards. The full
    // gathered catalogue is data for explicit scopes, not an eager fuzzy
    // list. Keep it as JSON so a scope never runs a source on a keystroke.
    let static_path = crate::paths::cache().join("list.txt");
    let home_path = crate::paths::cache().join("home.txt");
    let items_path = crate::paths::cache().join("search-items.json");
    let _ = crate::cache::write_atomic(&static_path, root_feed.as_bytes());
    let _ = crate::cache::write_atomic(&home_path, home_feed.as_bytes());
    if let Ok(json) = serde_json::to_vec(&items) {
        let _ = crate::cache::write_atomic(&items_path, &json);
    }
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
    // kind of thing it is. Where the answer *lands* — a prompt or the
    // clipboard — the helper inherits from our environment, so nothing about
    // the surface has to travel on its argv.
    args.push("--bind".into());
    args.push(if preview {
        // One helper updates both the item-specific footer and the contextual
        // c: image preview, so moving the cursor does not pay for two forks.
        format!("focus:transform:{} _focus {{q}} {{2}}", shq(&me))
    } else {
        format!("focus:transform-footer:{} _footer {{2}}", shq(&me))
    });
    args.push("--bind".into());
    args.push(format!("ctrl-o:execute({} _runhere {{2}})", shq(&me)));
    args.push("--bind".into());
    args.push(format!("ctrl-y:execute-silent({} _copy {{2}})", shq(&me)));
    if preview {
        args.push("--preview".into());
        args.push(format!("{} _preview {{2}}", shq(&me)));
        args.push("--preview-window".into());
        args.push("down,99%,wrap,border-top,hidden".into());
        args.push("--bind".into());
        args.push("ctrl-p:toggle-preview".into());
    }

    loop {
        let out = run_fzf(&home_feed, args.clone(), cols);
        if out.failed {
            let msg = out.stderr.trim().lines().last().unwrap_or("").to_string();
            eprintln!("prelude: fzf could not start{}", if msg.is_empty() { String::new() } else { format!(": {msg}") });
            return 2;
        }
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
            _ => return apply_default(&item),
        }
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
            let result = if item.kind == Kind::Dir {
                crate::openwith::open_now(&p, None)
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
            if let Err(e) = crate::openwith::open_now(item.get("path"), None) {
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
        RunHere => return crate::runhere::run_item(item),
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
        RunSkill => {
            // A skill name means nothing to a shell, so pick an agent that
            // actually has it and hand the invocation over.
            let agent = item.get("agent").split(',').next().unwrap_or("claude").trim().to_string();
            let agent = if agent == "shared" { "claude".to_string() } else { agent };
            emit("INSERT", &format!("{agent} {}", shq(&item.cmd)));
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

/// The contract with whatever started us: one line, VERB<TAB>payload.
///
/// Deliberately the same line for both callers. The zsh widget puts an INSERT
/// on the prompt and submits a RUN; the panel has no prompt and copies either.
/// Deciding that here would mean this process needing to know which surface it
/// is, and it does not: it reports, and the caller delivers.
pub fn emit(verb: &str, cmd: &str) {
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
    if env_flag("PRELUDE_FULL_SURFACE") {
        modes.push(vec![]);
    } else {
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
