//! `^O`: run the command inside the launcher window itself.
//!
//! The point of this is agent conversations. Enter types into whatever owns
//! the pane below — which, inside Claude Code, means your command lands in
//! the chat box as a message. Often you just want the thing to *run*. This
//! runs it here, shows the output, and returns you to the list without the
//! launcher ever closing or the agent seeing anything.

use crate::ansi::*;
use crate::item::{Item, Kind};

/// The rows Ctrl+O is allowed to run, and the same rule `actions_for` uses to
/// decide whether to offer `Run and show output`.
///
/// It has to be shared, because for a long time it was not. The key was bound
/// unconditionally and `run_item` ran whatever text the row carried straight
/// through `sh -c`, while the action panel two keystrokes away went to real
/// trouble over the same question: `useful_in_preview` names the six kinds
/// whose command is small and non-interactive, `generic_run_would_kill`
/// withholds the row from ports and processes, `is_destructive` reddens them
/// and `needs_confirming` puts Cancel first. Ctrl+O bypassed all four.
///
/// What that meant in practice: a Port row's command *is* `kill $(lsof -ti
/// tcp:3000)` and a Proc row's is `kill <pid>`, so one unadvertised keystroke
/// killed a process the panel would have asked about twice. On an `f:` row the
/// command is the path, so it tried to execute the file.
///
/// Everything else is inert, on Tab's precedent: a key that invents a
/// behaviour for rows that have none stops meaning one thing. The rows that
/// genuinely want output in the panel — an MCP server, a one-off agent
/// question, the update row — already run it on Enter.
pub fn can_run_here(it: &Item) -> bool {
    matches!(
        it.kind,
        Kind::History | Kind::Script | Kind::Path | Kind::Snippet | Kind::Sys | Kind::Git
    )
}

pub fn run_item(it: &Item) -> i32 {
    if !can_run_here(it) {
        return 0;
    }
    let cmd = if it.kind == Kind::Snippet {
        crate::ui::fill_placeholders(&it.cmd)
    } else {
        it.cmd.clone()
    };
    crate::frecency::bump(&cmd);
    run_in_window(&cmd, it.cwd.as_deref())
}

/// The same window, for a command the panel composed rather than one a row
/// carried — `claude mcp get <server>`, whose answer is the point and whose
/// output belongs here rather than on your prompt.
pub fn run_cmd(cmd: &str) -> i32 {
    run_in_window(cmd, None)
}

pub fn show_text(title: &str, lines: &[String]) -> i32 {
    use std::io::Write;
    let mut terminal = std::fs::OpenOptions::new()
        .read(true).write(true).open("/dev/tty").ok();
    if let Some(output) = terminal.as_mut() {
        let _ = writeln!(output, "{CYAN}{title}{RESET}\n");
        for line in lines { let _ = writeln!(output, "{line}"); }
        let _ = writeln!(output, "\n{DIM}press any key to go back{RESET}");
    } else {
        println!("{CYAN}{title}{RESET}\n");
        for line in lines { println!("{line}"); }
        println!("\n{DIM}press any key to go back{RESET}");
    }
    wait_key();
    0
}

fn run_in_window(cmd: &str, cwd: Option<&str>) -> i32 {
    use std::io::Write;
    use std::process::Stdio;

    let cmd = cmd.to_string();
    // The zsh widget invokes Prelude inside `$(...)`, so ordinary stdout is
    // captured as the INSERT/RUN protocol. An action-panel command that
    // printed an agent's answer there therefore appeared to return nothing.
    // Write the whole transient view to the controlling terminal instead.
    let mut terminal = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok();
    if let Some(t) = terminal.as_mut() {
        let _ = writeln!(t, "{DIM}$ {RESET}{cmd}\n");
    } else {
        println!("{DIM}$ {RESET}{cmd}\n");
    }

    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(&cmd).stdin(Stdio::null());
    if let Some(t) = terminal.as_ref() {
        if let (Ok(out), Ok(err)) = (t.try_clone(), t.try_clone()) {
            c.stdout(Stdio::from(out)).stderr(Stdio::from(err));
        }
    }
    if let Some(dir) = cwd {
        if std::path::Path::new(dir).is_dir() {
            c.current_dir(dir);
        }
    }
    let code = match c.status() {
        Ok(s) => s.code().unwrap_or(-1),
        Err(e) => {
            if let Some(t) = terminal.as_mut() {
                let _ = writeln!(t, "{YELLOW}failed: {e}{RESET}");
            } else {
                println!("{YELLOW}failed: {e}{RESET}");
            }
            1
        }
    };
    let mark = if code == 0 {
        format!("{GREEN}✓{RESET}")
    } else {
        format!("{RED}exit {code}{RESET}")
    };
    if let Some(t) = terminal.as_mut() {
        let _ = writeln!(t, "\n{mark}  {DIM}press any key to go back{RESET}");
    } else {
        println!("\n{mark}  {DIM}press any key to go back{RESET}");
    }
    wait_key();
    0
}

/// Single keypress, no Enter needed.
fn wait_key() {
    let saved = crate::tty::raw_mode();
    let mut buf = [0u8; 1];
    use std::io::Read;
    let _ = std::io::stdin().read(&mut buf);
    crate::tty::restore(saved);
}
