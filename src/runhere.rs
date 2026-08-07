//! `^O`: run the command inside the launcher window itself.
//!
//! The point of this is agent conversations. Enter types into whatever owns
//! the pane below — which, inside Claude Code, means your command lands in
//! the chat box as a message. Often you just want the thing to *run*. This
//! runs it here, shows the output, and returns you to the list without the
//! launcher ever closing or the agent seeing anything.

use crate::ansi::*;
use crate::item::{Item, Kind};

pub fn run_item(it: &Item) -> i32 {
    let cmd = if it.kind == Kind::Snippet {
        crate::ui::fill_placeholders(&it.cmd)
    } else {
        it.cmd.clone()
    };
    crate::frecency::bump(&cmd);

    println!("{DIM}$ {RESET}{cmd}\n");
    let mut c = std::process::Command::new("sh");
    c.arg("-c").arg(&cmd);
    if let Some(dir) = &it.cwd {
        if std::path::Path::new(dir).is_dir() {
            c.current_dir(dir);
        }
    }
    let code = match c.status() {
        Ok(s) => s.code().unwrap_or(-1),
        Err(e) => {
            println!("{YELLOW}failed: {e}{RESET}");
            1
        }
    };
    let mark = if code == 0 {
        format!("{GREEN}✓{RESET}")
    } else {
        format!("{RED}exit {code}{RESET}")
    };
    println!("\n{mark}  {DIM}press any key to go back{RESET}");
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
