//! Shell integration, printed for the user to eval.

/// The zsh block, with the configured key substituted in.
///
/// The key has to be baked in rather than read at shell start: this text is
/// what `eval` sees, and the shell has no way to ask Prelude anything before
/// running it. Which is also why changing the key reaches the *next* shell —
/// said on the row that changes it, so nobody waits for a binding that was
/// never going to appear.
pub fn zsh() -> String {
    ZSH.replace("@KEY@", &crate::settings::launcher_key())
}

const ZSH: &str = r#"# Prelude — zsh integration
# Added by: prelude init zsh

_prelude_widget() {
  local out verb payload
  out="$(command prelude 2>/dev/null)" || { zle reset-prompt; return 0; }
  [[ -z "$out" ]] && { zle reset-prompt; return 0; }
  verb="${out%%$'\t'*}"
  payload="${out#*$'\t'}"
  case "$verb" in
    INSERT)
      # The whole point: put it on the prompt, let the human press Enter.
      LBUFFER="$payload"
      RBUFFER=""
      ;;
    RUN)
      BUFFER="$payload"
      zle accept-line
      ;;
    MSG)
      # Something was refused, or failed, and said why. `zle -M` prints
      # below the prompt without disturbing what you have typed, and clears
      # itself on the next keystroke — so an explanation costs nothing and
      # a silent no-op never happens.
      zle -M "prelude: $payload"
      ;;
  esac
  zle reset-prompt
}
zle -N _prelude_widget

# Ctrl-R by default: prelude is a superset of incremental history search, and
# that is already where your fingers go for "that command I ran before".
# Change it with `prelude settings set key '^T'`, or by exporting PRELUDE_KEY
# before the `eval` line — the variable still wins. Either way this line was
# written when the shell started, so a change reaches the next shell.
#
# Not Ctrl-Space: macOS binds it to "Select the previous input source",
# so the OS eats it before the terminal ever sees it.
: ${PRELUDE_KEY:='@KEY@'}
bindkey "$PRELUDE_KEY" _prelude_widget

# Keep the old Ctrl-R behaviour on Ctrl-S. Ctrl-S is XOFF (freezes terminal
# output) under legacy flow control, so turn that off to free the key.
stty -ixon 2>/dev/null
bindkey '^S' history-incremental-search-backward
"#;

/// What an agent needs to know, in the form an agent actually reads.
///
/// Append it to `CLAUDE.md` or `AGENTS.md` and the agent has the whole
/// interface: four verbs, what each returns, and — the part that matters
/// most — *when to reach for them*. A capability an agent has to be told
/// about every time is not a capability it has.
///
/// The wording is deliberate on two points. It says `prelude ask` blocks and
/// returns the answer on stdout, because an agent that does not know it will
/// get a reply will not wait for one. And it says to ask instead of guessing,
/// because the failure this exists to prevent is the confident wrong choice
/// made at 2am rather than the question left unasked.
pub const AGENT: &str = r#"## Reaching the human, and the other agents

`prelude` is on this machine. It is how you talk to the person who started
you, and to the other agents running alongside you. Everything below is a
plain shell command.

**Ask the human a question and wait for the answer.** They are notified
wherever they are — they do not have to be looking at your window — and the
answer arrives on stdout. Use this whenever you would otherwise guess at
something you cannot undo:

```sh
answer=$(prelude ask "The migration drops the legacy_users table. Proceed?")
```

Exit code 0 means they answered, 3 means nobody did within the timeout
(default 600s; pass `--timeout=120` to fit inside your own tool deadline).
On 3, do the conservative thing and say that you did.

**Tell them something, without waiting.** For the end of long work, or
anything they would want to know but need not act on:

```sh
prelude tell "migration finished — 41k rows, no errors"
```

**See the other agents on this machine, and talk to them.** `--json` gives
you their project, state, working directory and pane address:

```sh
prelude fleet --json
prelude say api-gateway "I changed the auth schema — you will need to rebase"
```

`say` types the line straight into that agent's conversation, attributed so
it knows the message came from a peer rather than from the human. Address it
by project, by agent name, or by pane. If more than one thing matches it will
refuse and list them rather than deliver to the wrong one.

**Collect what was left for you**, when an agent could not be typed into
directly:

```sh
prelude inbox --json && prelude drain
```

### When to use which

- About to do something irreversible, ambiguous, or outside what you were
  asked → `prelude ask`. Waiting for a person is cheaper than being wrong.
- Finished something long, or hit a problem they should know about but that
  does not block you → `prelude tell`.
- Your change breaks an assumption another agent is working under →
  `prelude say <their project>`.
- Wondering whether someone else is already on it → `prelude fleet --json`.

Do not use these for ordinary progress chatter. A notification that arrives
for nothing teaches the person to ignore the next one.
"#;

#[cfg(test)]
mod tests {
    use super::{zsh, ZSH};

    /// The key is baked in, because `eval` is all the shell sees and it has no
    /// way to ask Prelude anything first. The placeholder must therefore never
    /// survive into what a shell runs.
    #[test]
    fn the_configured_key_reaches_the_shell_block() {
        let out = zsh();
        assert!(!out.contains("@KEY@"), "the placeholder was not substituted");
        assert!(out.contains(&format!(": ${{PRELUDE_KEY:='{}'}}", crate::settings::launcher_key())));
        // …and the variable still overrides it, for a one-shell change.
        assert!(out.contains("bindkey \"$PRELUDE_KEY\" _prelude_widget"));
    }

    #[test]
    fn the_widget_is_the_only_thing_this_shell_learns() {
        // The three verbs, and nothing else. There used to be a second block
        // here that bootstrapped a shell the panel had opened for a command,
        // reading the command out of a private file so it would not show up in
        // `ps`. The panel copies now: no window is opened, so no shell needs
        // teaching how to receive one.
        assert!(!ZSH.contains("PRELUDE_PRELOAD"));
        assert!(!ZSH.contains("prelude/preload"));
        assert!(!ZSH.contains("add-zle-hook-widget"));
        // INSERT waits for a human to press Enter; RUN was agreed to in the
        // launcher; MSG explains a refusal without touching the line.
        assert!(ZSH.contains("LBUFFER=\"$payload\""));
        assert!(ZSH.contains("zle accept-line"));
        assert!(ZSH.contains("zle -M \"prelude: $payload\""));
        assert!(!ZSH.contains("sleep "));
        assert!(!ZSH.contains("osascript"));
    }

    #[test]
    fn the_lease_and_the_autostart_hook_are_gone_with_the_terminal_they_managed() {
        // Nothing is created on a press any more, so there is nothing to hold
        // a lease on and no shell to bootstrap into a launcher.
        assert!(!ZSH.contains("PRELUDE_AUTOSTART"));
        assert!(!ZSH.contains("PRELUDE_GLOBAL_TOKEN"));
        assert!(!ZSH.contains("global-active"));
        // Ctrl+R is untouched, and never closes a terminal.
        assert!(ZSH.contains("bindkey \"$PRELUDE_KEY\" _prelude_widget"));
        assert!(!ZSH.contains("BUFFER=' exit'"));
    }
}
