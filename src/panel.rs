//! The launcher surface, and where its answers go.
//!
//! The global hotkey used to build a terminal: a new application instance, a
//! window, a login shell, then Prelude. Three hundred and seventy milliseconds
//! of construction before the launcher existed, paid again on every press, and
//! torn down afterwards — including when the thing chosen was a file, which
//! never needed a terminal at all.
//!
//! Here the surface is a Ghostty quick terminal: a panel that is revealed
//! rather than created. A press costs the panel animation.
//!
//! **The panel never opens a terminal, and never types into one.** It used to
//! do both — reading the frontmost application to decide whether to deliver
//! into the tmux pane you were looking at or to build a window for the
//! occasion. Both were the same mistake in two costumes: a launcher deciding,
//! from the outside, which prompt you meant. The window it built was the wrong
//! directory as often as not, and the pane it typed into was whichever one
//! tmux happened to consider current.
//!
//! So a command goes on the clipboard, and the panel stands down. Where it
//! lands is then the one question a launcher has no business answering, asked
//! of the only thing that knows: you, with ⌘V, in the window you were already
//! in. Objects never came this way at all — a file, a folder, a URL or an
//! application goes straight to Launch Services and needs no prompt anywhere.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A launcher run that ended without producing anything: fzf's own dismissal
/// code, which Prelude passes through.
const DISMISSED: i32 = 130;

/// How long a confirmation stays on the panel before it closes.
///
/// Long enough to read four words, short enough that it is not a step in the
/// way of the paste. The panel has to close either way — nothing else took
/// focus, so it is still covering whatever you wanted to paste into.
const CONFIRM: Duration = Duration::from_millis(1200);

/// What the loop should do after one run of the launcher.
#[derive(Debug, PartialEq, Eq)]
enum After {
    /// Stay warm and show a fresh launcher. The panel is already gone, or the
    /// person dismissed it and expects it gone.
    Continue,
    /// Leave, which closes the surface and with it the panel. Used when the
    /// answer is on the clipboard: nothing else took focus, so autohide has
    /// nothing to react to and the panel would sit on top of the destination.
    Close,
}

/// One run of the launcher, as a child process.
///
/// A child rather than a call, because the verb contract on stdout is what the
/// zsh widget has always consumed and is worth having exactly one of. fzf
/// draws on `/dev/tty`, so capturing stdout takes nothing away from it.
///
/// The child is told two things about the surface it is drawing into, both by
/// environment so that fzf's per-keystroke helpers inherit them for free:
/// the window is entirely ours, and text it hands over will be copied rather
/// than put on a prompt. The second is what keeps every label in the footer
/// and the action panel honest.
fn once() -> (i32, After) {
    let Ok(exe) = std::env::current_exe() else {
        return (2, After::Continue);
    };
    let Ok(out) = Command::new(exe)
        .env("PRELUDE_FULL_SURFACE", "1")
        .env("PRELUDE_TO_CLIPBOARD", "1")
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
        //
        // A dismissal is now Escape at the outermost level and nothing else:
        // Ghostty stopped binding the key, and every inner level backs out to
        // the one above rather than exiting. So it means what it says, and the
        // panel closes. It used to be indistinguishable from Escape anywhere
        // else, which is why it could not.
        if code == DISMISSED {
            return (code, After::Close);
        }
        return (code, After::Continue);
    };
    match verb {
        // INSERT and RUN are one thing here. The difference between them is
        // whether a shell should press Enter for you, and there is no shell in
        // this surface to press it in — the distinction survives at the zsh
        // widget, which is the only place it can mean anything.
        "INSERT" | "RUN" => {
            crate::ui::copy(payload);
            println!("copied: {}", crate::width::flatten(payload));
            std::thread::sleep(CONFIRM);
            (code, After::Close)
        }
        "MSG" => {
            println!("prelude: {payload}");
            std::thread::sleep(CONFIRM);
            (code, After::Continue)
        }
        _ => (code, After::Continue),
    }
}

/// `prelude _panel` — the process the quick terminal runs.
pub fn run() -> i32 {
    // Nothing is on screen yet and the process about to serve keypresses is
    // this one, so this is the only moment an update can be applied without
    // creating the exact state it exists to prevent: a new binary and a panel
    // still running the old one. Nothing happens here unless the `update`
    // setting says `apply` and a verified archive is already staged.
    if let Some(version) = crate::update::apply_staged_if_any() {
        eprintln!("prelude: installed {version}; it takes effect on the next panel start");
    }
    // What is *actually* serving keypresses, recorded rather than inferred.
    crate::update::record_panel(std::process::id());
    let mut consecutive_faults = 0;
    loop {
        let started = Instant::now();
        let (code, after) = once();
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

    /// The whole of the delivery decision, which is that there isn't one.
    ///
    /// Both verbs copy. A test rather than a comment because the two used to
    /// take different roads out of here — one typed into a tmux pane, one
    /// opened a window — and each road had its own way of putting a command
    /// somewhere nobody was looking.
    #[test]
    fn both_handover_verbs_copy_and_stand_the_panel_down() {
        for verb in ["INSERT", "RUN"] {
            assert_eq!(after_for(verb), After::Close, "{verb}");
        }
        // A refusal has nothing to paste, so the panel stays and shows it.
        assert_eq!(after_for("MSG"), After::Continue);
        // Anything else means the launcher acted on an object itself; the
        // application it opened took focus and autohide has already run.
        assert_eq!(after_for(""), After::Continue);
    }

    /// The branch of `once` that decides, without running a launcher.
    fn after_for(verb: &str) -> After {
        match verb {
            "INSERT" | "RUN" => After::Close,
            _ => After::Continue,
        }
    }

    /// The child is told what surface it is drawing into, and it is told by
    /// environment rather than argv so that fzf's per-keystroke footer and
    /// preview helpers — grandchildren of this process — inherit it without
    /// anything being threaded through them.
    #[test]
    fn the_surface_is_declared_by_inheritance() {
        let source = include_str!("panel.rs");
        let body = source.split("fn once()").nth(1).expect("the launcher call");
        let body = &body[..body.find("\n/// `prelude _panel`").unwrap_or(body.len())];
        assert!(body.contains("PRELUDE_FULL_SURFACE"));
        assert!(body.contains("PRELUDE_TO_CLIPBOARD"));
    }

    /// Nothing in this module may reach for a terminal or a pane. Both used to
    /// live here, and both were a launcher guessing which prompt you meant.
    #[test]
    fn the_panel_owns_no_terminal() {
        let source = include_str!("panel.rs");
        let code = source.split("#[cfg(test)]").next().unwrap_or(source);
        for forbidden in ["tmux", "lsappinfo", "open_working_window", "-e", "osascript"] {
            assert!(
                !code.contains(&format!("\"{forbidden}\"")),
                "the panel must not invoke {forbidden}"
            );
        }
    }
}
