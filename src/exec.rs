//! Running other programs, always with a deadline and always reaped.
//!
//! Three things a launcher cannot afford, each of which this file got wrong
//! once and is now structured to prevent.
//!
//! **The deadline has to be on the process, not on its output.** The old
//! version waited for stdout to reach EOF and called that the timeout. Those
//! are different events: a child that closes stdout and keeps working is past
//! the deadline while the reader thread sits happily blocked, and — worse —
//! `child.wait()` afterwards had no bound at all, so a source that "timed
//! out" could still hold a launch open indefinitely. The wait now happens on
//! its own thread and the deadline is applied to *that*.
//!
//! **Killing the child is not killing what the child started.** An agent CLI
//! is routinely a shell script, an MCP server is a `node` that spawns more
//! `node`, and every one of those grandchildren inherits the stdout pipe. Kill
//! the direct child alone and the pipe stays open, held by processes nobody is
//! tracking any more: the reader thread never returns and the fd never closes.
//! Each child therefore gets its **own process group** (`setpgid` in the
//! pre-exec hook) and the timeout kills the group, so the whole tree goes and
//! the pipes close with it.
//!
//! **Output has to be bounded.** `read_to_end` on a pipe is an unbounded
//! allocation controlled by another program. The caps here are generous —
//! nothing legitimate approaches them — but they exist, and `truncated` says
//! so rather than silently handing back a JSON document with its tail missing.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Enough for `ps -Ao` on a busy machine (~74KB) with two orders of magnitude
/// to spare, and small enough that a runaway producer cannot exhaust memory.
pub const MAX_OUTPUT: usize = 8 * 1024 * 1024;

/// What actually happened, rather than "some text, or none".
///
/// The old signature returned `String` and used empty to mean *failed to
/// spawn*, *timed out*, *exited non-zero* and *succeeded with no output*
/// alike. Every caller that wanted to tell those apart had to guess, and the
/// one that mattered most — a refresh deciding whether to overwrite a good
/// cache — guessed wrong: a `claude mcp list` that timed out looked exactly
/// like an agent with no servers, and the empty result was written over the
/// real one.
#[derive(Debug, Clone, Default)]
pub struct Output {
    /// The exit code, or `None` if the process was killed or never ran.
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// The deadline passed and the process group was killed.
    pub timed_out: bool,
    /// Output hit `MAX_OUTPUT` and the rest was discarded.
    pub truncated: bool,
    /// The program could not be started at all — missing, not executable.
    pub spawn_failed: bool,
}

impl Output {
    /// Did this run do what it was asked, completely?
    ///
    /// Anything else means the caller learned nothing about the world, which
    /// is different from learning that the world is empty.
    pub fn ok(&self) -> bool {
        self.status == Some(0) && !self.timed_out && !self.truncated && !self.spawn_failed
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// One line naming what went wrong, for an explicit command that should
    /// say so. A source on the launch path stays silent and degrades instead.
    pub fn failure(&self) -> Option<String> {
        if self.ok() {
            return None;
        }
        if self.spawn_failed {
            return Some("could not be started".into());
        }
        if self.timed_out {
            return Some("timed out".into());
        }
        if self.truncated {
            return Some("produced more output than it is allowed".into());
        }
        Some(match self.status {
            Some(code) => format!("exited {code}"),
            None => "was killed".into(),
        })
    }
}

/// Run a command and return whatever it wrote to stdout.
///
/// The shape every source has always used. It is now a thin reading of
/// [`capture`], so the process-group kill, the real deadline and the output
/// cap apply to all thirty-odd call sites without any of them changing.
///
/// It deliberately does **not** filter on the exit status, because the old
/// one did not: several callers run a probe that reports through its output
/// and exits non-zero doing it — `codex login status`, a version flag on a
/// CLI that is not signed in — and swallowing stdout there would turn a
/// working diagnostic into a blank one. Callers that need to tell "it said
/// nothing" from "it failed" use [`capture`].
pub fn run(args: &[&str], timeout: Duration) -> String {
    capture(args, timeout).stdout_text()
}

/// How many commands this process started and learned nothing from.
///
/// A source cannot tell an empty world from a broken pipe, and its signature
/// (`fn() -> Vec<Item>`) has nowhere to say which it met. This counter is
/// where the difference survives: `capture` bumps it whenever a command could
/// not be started or had to be killed, and `cache::write_refreshed` reads it
/// to decide whether an empty result may overwrite a cache that is not empty.
/// It is a count rather than a flag so callers can take a *difference* across
/// one source's run instead of inspecting global state.
static LOST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn lost_commands() -> usize {
    LOST.load(std::sync::atomic::Ordering::Relaxed)
}

/// Run a command and report what happened, including how it failed.
pub fn capture(args: &[&str], timeout: Duration) -> Output {
    let Some((prog, rest)) = args.split_first() else {
        return lose(Output { spawn_failed: true, ..Output::default() });
    };
    let mut command = Command::new(prog);
    command
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Its own process group, so the deadline can reach everything it starts.
    // `setpgid(0, 0)` between fork and exec is async-signal-safe, which is the
    // whole of what a pre-exec hook is allowed to be.
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let Ok(mut child) = command.spawn() else {
        return lose(Output { spawn_failed: true, ..Output::default() });
    };
    let pid = child.id() as i32;

    // Both pipes are drained on their own threads. Waiting on the child while
    // a pipe fills deadlocks the moment output passes the 64KB buffer — `ps
    // -Ao` emits ~74KB here, and every process row silently vanished for as
    // long as that was the arrangement.
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);

    // The deadline belongs on the process, so the thing being waited on is
    // the process. Nothing here is polled.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let status = child.wait();
        let _ = tx.send(status.ok().and_then(|s| s.code()));
    });

    let (status, timed_out) = match rx.recv_timeout(timeout) {
        Ok(code) => (code, false),
        Err(_) => {
            // The group, not the pid: a shell wrapper's children hold the
            // pipes, and killing only the process we can see leaves them open.
            kill_group(pid);
            (rx.recv_timeout(REAP_GRACE).ok().flatten(), true)
        }
    };

    let (stdout, out_cut) = collect(out);
    let (stderr, err_cut) = collect(err);
    let out = Output {
        status,
        stdout,
        stderr,
        timed_out,
        truncated: out_cut || err_cut,
        spawn_failed: false,
    };
    if out.timed_out {
        return lose(out);
    }
    out
}

/// Record that this run taught us nothing about the world.
///
/// A non-zero exit is deliberately *not* counted: a CLI that answers "no
/// servers configured" with status 1 has told us something true, and holding
/// the previous cache on that basis would preserve rows the person has just
/// removed. Only a command that never ran, or had to be killed, leaves us
/// genuinely uninformed.
fn lose(out: Output) -> Output {
    LOST.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    out
}

/// How long to wait for a killed process to be reaped. SIGKILL is not
/// refusable, so this is the kernel's scheduling latency and nothing more; a
/// bound is here only so an unkillable state cannot hold the thread.
const REAP_GRACE: Duration = Duration::from_millis(500);

/// The same bound applied to reading a pipe after the process is gone. Once
/// the group is killed every writer is dead and EOF is immediate; a reader
/// still blocked past this held an fd we no longer have a claim on, and is
/// abandoned rather than waited for.
const READ_GRACE: Duration = Duration::from_millis(500);

type Drained = std::sync::mpsc::Receiver<(Vec<u8>, bool)>;

fn drain(mut pipe: impl Read + Send + 'static) -> Drained {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 16 * 1024];
        let mut truncated = false;
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if buf.len() + n > MAX_OUTPUT {
                        let room = MAX_OUTPUT.saturating_sub(buf.len());
                        buf.extend_from_slice(&chunk[..room]);
                        truncated = true;
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
        }
        let _ = tx.send((buf, truncated));
    });
    rx
}

fn collect(drained: Option<Drained>) -> (Vec<u8>, bool) {
    drained
        .and_then(|rx| rx.recv_timeout(READ_GRACE).ok())
        .unwrap_or_default()
}

fn kill_group(pid: i32) {
    if pid <= 0 {
        return;
    }
    // Negative pid means "the group", which is the point. The direct pid is
    // signalled too, in case the pre-exec hook never ran and the child is
    // still in ours — where killing the group would mean killing ourselves.
    unsafe {
        kill(-pid, SIGKILL);
        kill(pid, SIGKILL);
    }
}

const SIGKILL: i32 = 9;

unsafe extern "C" {
    unsafe fn setpgid(pid: i32, pgid: i32) -> i32;
    unsafe fn kill(pid: i32, sig: i32) -> i32;
}

/// Whether a process still exists. Signal 0 tests existence without changing
/// the process, and is the only thing a recorded pid can be checked against.
pub fn alive(pid: i32) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The deadline is on the process, not on its stdout. A child that closes
    /// its output and keeps running used to sail past it — the reader thread
    /// saw EOF, reported success, and `child.wait()` then blocked with no
    /// bound at all.
    #[test]
    fn a_process_that_closes_stdout_and_keeps_running_still_times_out() {
        let t = std::time::Instant::now();
        let out = capture(
            &["sh", "-c", "echo hello; exec 1>&-; sleep 30"],
            Duration::from_millis(300),
        );
        assert!(out.timed_out, "the deadline must apply to the process");
        assert!(t.elapsed() < Duration::from_secs(5), "it must not wait for the child");
        assert!(!out.ok());
    }

    /// A shell wrapper's children inherit the pipe. Killing only the process
    /// we can see leaves them holding it open.
    #[test]
    fn the_whole_process_group_goes_when_the_deadline_passes() {
        let t = std::time::Instant::now();
        let out = capture(
            &["sh", "-c", "sleep 30 & sleep 30"],
            Duration::from_millis(300),
        );
        assert!(out.timed_out);
        // The reader threads only finish when every writer is gone, so
        // returning promptly *is* the assertion that the group was killed.
        assert!(t.elapsed() < Duration::from_secs(5), "a grandchild held the pipe open");
    }

    #[test]
    fn output_is_reported_with_its_status_rather_than_flattened_to_emptiness() {
        let ok = capture(&["sh", "-c", "printf out; printf err >&2; exit 0"], Duration::from_secs(5));
        assert!(ok.ok());
        assert_eq!(ok.stdout_text(), "out");
        assert_eq!(ok.stderr_text(), "err");
        assert_eq!(ok.failure(), None);

        let bad = capture(&["sh", "-c", "exit 3"], Duration::from_secs(5));
        assert_eq!(bad.status, Some(3));
        assert!(!bad.ok());
        assert_eq!(bad.failure().as_deref(), Some("exited 3"));

        // A program that is not there is not a program that answered nothing.
        let missing = capture(&["prelude-does-not-exist-anywhere"], Duration::from_secs(5));
        assert!(missing.spawn_failed);
        assert!(!missing.ok());
        // …and `run` still degrades to the empty string every source expects.
        assert_eq!(run(&["prelude-does-not-exist-anywhere"], Duration::from_secs(5)), "");
    }

    /// `run` reports what a program printed, not whether it approved of
    /// itself. Several probes say what they have to say and exit non-zero
    /// doing it; filtering on the status blanked their diagnostics.
    #[test]
    fn run_keeps_the_output_of_a_command_that_exits_non_zero() {
        assert_eq!(
            run(&["sh", "-c", "printf 'not logged in'; exit 1"], Duration::from_secs(5)),
            "not logged in"
        );
    }

    #[test]
    fn a_successful_command_with_no_output_is_not_a_failure() {
        let out = capture(&["true"], Duration::from_secs(5));
        assert!(out.ok(), "empty output is a result, not an error");
        assert!(out.stdout.is_empty());
    }
}
