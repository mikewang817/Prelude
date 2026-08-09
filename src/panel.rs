//! The launcher surface, and where its answers go.
//!
//! The global hotkey used to build a terminal: a new application instance, a
//! window, a login shell, then Prelude. Three hundred and seventy milliseconds
//! of construction before the launcher existed, paid again on every press, and
//! torn down afterwards — including when the thing chosen was a file, which
//! never needed a terminal at all.
//!
//! Here the surface is a Ghostty quick terminal: a panel that is revealed
//! rather than created, hosting this loop, which outlives every press. A press
//! costs the panel animation. Nothing is constructed and nothing is destroyed,
//! so there is no launch to fail and no teardown to strand.
//!
//! That only works because the launcher stops being the destination. A panel
//! is no place to leave a command sitting on a prompt, so an answer is
//! delivered: to the terminal you were already in when you pressed the key, or
//! to one window created on purpose for it. Which of the two is decided by the
//! application that was frontmost — the panel never takes that status, so the
//! question is still answerable at the moment it is asked.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Terminals whose window can be typed into, given tmux. The list is only ever
/// used to decide *whether the person was looking at a terminal*; delivery
/// itself goes through tmux, which is the only way to reach a pane without
/// synthesizing keystrokes into somebody else's window.
const TERMINALS: &[&str] = &[
    "com.mitchellh.ghostty",
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "io.alacritty",
    "dev.warp.Warp-Stable",
    "co.zeit.hyper",
    "com.github.wez.wezterm",
];

/// A launcher run that ended without producing anything: fzf's own dismissal
/// code, which Prelude passes through.
const DISMISSED: i32 = 130;

#[derive(Debug, PartialEq, Eq)]
enum Destination {
    /// The person pressed the hotkey while looking at a terminal, and tmux can
    /// address the pane they were in.
    Pane(String),
    /// Everywhere else. A command gets one window, created deliberately.
    NewWindow,
}

/// The frontmost application's bundle identifier.
///
/// `NSWorkspace` would answer this too, but `lsappinfo` answers it without
/// linking AppKit into the launcher — and this is asked once per action, not
/// per keystroke, so two short subprocesses are affordable where they would
/// not be anywhere else in this codebase.
fn frontmost_bundle() -> Option<String> {
    let asn = crate::exec::run(&["/usr/bin/lsappinfo", "front"], Duration::from_secs(2));
    let asn = asn.trim();
    if asn.is_empty() {
        return None;
    }
    let info = crate::exec::run(
        &["/usr/bin/lsappinfo", "info", "-only", "bundleID", asn],
        Duration::from_secs(2),
    );
    // "CFBundleIdentifier"="com.apple.finder"
    let id = info.split('=').nth(1)?.trim().trim_matches('"').to_string();
    if id.is_empty() { None } else { Some(id) }
}

fn destination() -> Destination {
    let front = frontmost_bundle();
    let in_terminal = front
        .as_deref()
        .is_some_and(|id| TERMINALS.iter().any(|t| t.eq_ignore_ascii_case(id)));
    if !in_terminal {
        return Destination::NewWindow;
    }
    // A terminal was in front, but only tmux can say which pane and only tmux
    // can type into it. Without it there is nothing to address, and guessing
    // would put a command in a window nobody was looking at.
    match crate::ui::resolve_pane(None) {
        Some(pane) => Destination::Pane(pane),
        None => Destination::NewWindow,
    }
}

fn preload_path() -> std::path::PathBuf {
    crate::paths::cache().join("preload")
}

/// Hand a command to a window that does not exist yet.
///
/// The command never goes on a command line. A history entry can hold a
/// token and a `ps` listing is readable by every process on the machine, so
/// the payload goes into a 0600 file the new shell reads and removes, and the
/// argument list carries only the fact that there is one.
fn stage_preload(path: &std::path::Path, verb: &str, cmd: &str) -> bool {
    if crate::cache::write_atomic(path, format!("{verb}\n{cmd}").as_bytes()).is_err() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).is_err() {
            let _ = std::fs::remove_file(path);
            return false;
        }
    }
    true
}

fn open_window_with(verb: &str, cmd: &str) -> bool {
    let path = preload_path();
    if !stage_preload(&path, verb, cmd) {
        return false;
    }
    let ok = crate::global::open_working_window();
    if !ok {
        let _ = std::fs::remove_file(&path);
    }
    ok
}

/// What the loop should do after one run of the launcher.
#[derive(Debug, PartialEq, Eq)]
enum After {
    /// Stay warm and show a fresh launcher. The panel is already gone, or the
    /// person dismissed it and expects it gone.
    Continue,
    /// Leave, which closes the surface and with it the panel. Used when
    /// something was delivered into the window directly behind the panel:
    /// nothing else took focus, so autohide has nothing to react to.
    Close,
}

fn deliver(verb: &str, cmd: &str, to: &Destination) -> After {
    match to {
        Destination::Pane(pane) => {
            crate::ui::paste_into_pane(pane, cmd, verb == "RUN");
            // The terminal was already frontmost, so nothing changed focus and
            // the panel is still covering the answer. Standing down is the
            // only way to get out of the way.
            After::Close
        }
        Destination::NewWindow => {
            if open_window_with(verb, cmd) {
                // The new window takes focus, the panel autohides, and this
                // loop stays warm for the next press.
                After::Continue
            } else {
                println!("prelude: could not open a window for that command");
                After::Continue
            }
        }
    }
}

/// One run of the launcher, as a child process.
///
/// A child rather than a call, because the verb contract on stdout is what the
/// zsh widget has always consumed and is worth having exactly one of. fzf
/// draws on `/dev/tty`, so capturing stdout takes nothing away from it.
fn once(to: &Destination) -> (i32, After) {
    let Ok(exe) = std::env::current_exe() else {
        return (2, After::Continue);
    };
    let mut command = Command::new(exe);
    // In a pane the child delivers the answer itself, through the same road
    // the tmux popup uses. Everywhere else it reports back and this loop
    // decides.
    if let Destination::Pane(pane) = to {
        command.arg("paste").arg(pane);
    }
    let Ok(out) = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return (2, After::Continue);
    };
    let code = out.status.code().unwrap_or(2);
    let text = String::from_utf8_lossy(&out.stdout);
    let Some((verb, payload)) = text.trim_end_matches('\n').split_once('\t') else {
        // Nothing to hand over. Either the launcher was dismissed, or it acted
        // on an object directly and the result is in another application —
        // which took focus, so the panel is already gone.
        return (code, After::Continue);
    };
    match verb {
        "INSERT" | "RUN" => (code, deliver(verb, payload, to)),
        "MSG" => {
            println!("prelude: {payload}");
            std::thread::sleep(Duration::from_millis(1200));
            (code, After::Continue)
        }
        _ => (code, After::Continue),
    }
}

/// `prelude _panel` — the process the quick terminal runs.
pub fn run() -> i32 {
    let mut consecutive_faults = 0;
    loop {
        let started = Instant::now();
        let to = destination();
        let (code, after) = once(&to);
        if after == After::Close {
            return 0;
        }
        // A launcher that cannot start would otherwise spin invisibly behind a
        // hidden panel, forever. Faults are counted, not tolerated.
        let faulted = code != 0 && code != DISMISSED;
        let instant = started.elapsed() < Duration::from_millis(150);
        if faulted && instant {
            consecutive_faults += 1;
        } else {
            consecutive_faults = 0;
        }
        if consecutive_faults >= 3 {
            eprintln!("prelude: the launcher failed three times in a row (exit {code}).");
            eprintln!("prelude: run `prelude doctor`. This panel will stay put.");
            return code;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_terminal_in_front_can_be_typed_into() {
        // The list decides one thing: whether the person was looking at a
        // terminal. Anything else has to get its own window, because typing a
        // command into a pane nobody is looking at is worse than opening one.
        assert!(TERMINALS.contains(&"com.mitchellh.ghostty"));
        assert!(TERMINALS.contains(&"com.apple.Terminal"));
        assert!(!TERMINALS.contains(&"com.google.Chrome"));
        assert!(!TERMINALS.contains(&"com.apple.finder"));
    }

    #[test]
    fn a_delivered_answer_stands_down_and_a_new_window_does_not() {
        // Nothing else took focus in the pane case, so the panel is still on
        // top of the answer and has to leave. A new window takes focus itself,
        // which is what dismisses the panel, so the loop stays warm.
        assert_eq!(
            deliver_decision(&Destination::Pane("%1".into())),
            After::Close
        );
        assert_eq!(deliver_decision(&Destination::NewWindow), After::Continue);
    }

    /// The branch of `deliver` that decides, without performing the delivery.
    fn deliver_decision(to: &Destination) -> After {
        match to {
            Destination::Pane(_) => After::Close,
            Destination::NewWindow => After::Continue,
        }
    }

    #[test]
    fn a_handed_over_command_waits_in_a_private_file_not_on_a_command_line() {
        // A history entry can hold a token and `ps` is readable by anything on
        // the machine, so the payload travels by file and the argument list
        // carries only the fact that there is one.
        let path = std::env::temp_dir().join(format!("prelude-preload-{}", std::process::id()));
        assert!(stage_preload(&path, "INSERT", "deploy --token=hunter2"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "INSERT\ndeploy --token=hunter2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_window_a_command_gets_is_told_only_that_one_is_waiting() {
        let source = include_str!("global.rs");
        let launch = source
            .split("pub fn open_working_window")
            .nth(1)
            .expect("the working-window launch");
        let launch = &launch[..launch.find("\nfn ").unwrap_or(launch.len())];
        assert!(launch.contains("PRELUDE_PRELOAD=1"));
        // Saved state would bring the whole of the last session along with the
        // one window that was asked for.
        assert!(launch.contains("--window-save-state=never"));
    }
}
