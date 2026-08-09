//! Running other programs, always with a deadline.

use std::process::{Command, Stdio};
use std::time::Duration;

/// Run a command, returning stdout on success and an empty string otherwise.
/// A source that shells out must degrade to nothing, never hang the launcher.
/// Run a command, returning stdout on success and an empty string otherwise.
/// A source that shells out must degrade to nothing, never hang the launcher.
///
/// The output is drained on a helper thread rather than polled for exit.
/// Waiting on the child while its stdout pipe fills deadlocks the moment the
/// output exceeds the 64KB pipe buffer — `ps -Ao` emits ~74KB here, so the
/// process blocked writing while we blocked waiting, and every process row
/// silently vanished.
pub fn run(args: &[&str], timeout: Duration) -> String {
    let Some((prog, rest)) = args.split_first() else {
        return String::new();
    };
    let Ok(mut child) = Command::new(prog)
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return String::new();
    };
    let Some(mut out) = child.stdout.take() else {
        return String::new();
    };

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    match rx.recv_timeout(timeout) {
        Ok(buf) => {
            let _ = child.wait();
            String::from_utf8_lossy(&buf).into_owned()
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            String::new()
        }
    }
}

/// Whether a process still exists. Signal 0 tests existence without changing
/// the process, and is the only thing a recorded pid can be checked against.
pub fn alive(pid: i32) -> bool {
    unsafe extern "C" {
        unsafe fn kill(pid: i32, sig: i32) -> i32;
    }
    pid > 0 && unsafe { kill(pid, 0) == 0 }
}

pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Quote for the shell only when it actually needs it.
pub fn shq(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./~-".contains(c));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}
