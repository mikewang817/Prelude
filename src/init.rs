//! Shell and tmux integration, printed for the user to eval/source.

pub const ZSH: &str = r#"# Prelude — zsh integration
# Added by: prelude init zsh

_prelude_global_done() {
  [[ -n ${PRELUDE_GLOBAL_TOKEN:-} ]] || return 0
  local active="${XDG_CACHE_HOME:-$HOME/.cache}/prelude/global-active"
  local owner=''
  [[ -r "$active" ]] && IFS= read -r owner < "$active"
  if [[ "$owner" == "$PRELUDE_GLOBAL_TOKEN" ]]; then
    command rm -f -- "$active"
  fi
  unset PRELUDE_GLOBAL_TOKEN
}

_prelude_widget() {
  local out verb payload
  out="$(command prelude 2>/dev/null)" || { _prelude_global_done; zle reset-prompt; return 0; }
  [[ -z "$out" ]] && { _prelude_global_done; zle reset-prompt; return 0; }
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
  _prelude_global_done
  zle reset-prompt
}
zle -N _prelude_widget

# A terminal created by `prelude global` asks for exactly one invocation when
# its first prompt becomes editable. Use ZLE's lifecycle rather than sending a
# delayed Ctrl-R into a shell that may still be reading .zshrc. A widget cannot
# safely take over the terminal *inside* line-init, so the hook queues one
# private ZLE sequence and returns; ZLE then invokes the same _prelude_widget in
# its normal dispatch cycle. Both the hook and private binding remove themselves
# before opening Prelude, leaving a completely ordinary shell afterwards.
if [[ -n ${PRELUDE_AUTOSTART:-} ]]; then
  unset PRELUDE_AUTOSTART
  autoload -Uz add-zle-hook-widget
  _prelude_autostart_dispatch() {
    bindkey -r $'\e[27;99~'
    zle _prelude_widget
  }
  zle -N _prelude_autostart_dispatch
  bindkey $'\e[27;99~' _prelude_autostart_dispatch
  _prelude_autostart_once() {
    add-zle-hook-widget -d line-init _prelude_autostart_once
    zle -U $'\e[27;99~'
  }
  add-zle-hook-widget line-init _prelude_autostart_once
fi

# Ctrl-R by default: prelude is a superset of incremental history search, and
# that is already where your fingers go for "that command I ran before".
# Override by setting PRELUDE_KEY before the `eval` line, e.g. PRELUDE_KEY='^T'.
#
# Not Ctrl-Space: macOS binds it to "Select the previous input source",
# so the OS eats it before the terminal ever sees it.
: ${PRELUDE_KEY:='^R'}
bindkey "$PRELUDE_KEY" _prelude_widget

# Keep the old Ctrl-R behaviour on Ctrl-S. Ctrl-S is XOFF (freezes terminal
# output) under legacy flow control, so turn that off to free the key.
stty -ixon 2>/dev/null
bindkey '^S' history-incremental-search-backward
"#;

pub const TMUX: &str = r#"# Prelude — tmux integration
# Added by: prelude init tmux
#
# The zsh widget only works at a zsh prompt. This binding works *anywhere*,
# because tmux owns the terminal above whatever is running in the pane —
# an agent conversation, vim, a REPL, an ssh session.
#
# It opens the launcher in a floating popup, then types the chosen command
# into the pane underneath. It never presses Enter for you.

# prefix + r  (i.e. Ctrl-b then r)
bind r display-popup -E -w 92% -h 92% -d '#{pane_current_path}' \
  "PRELUDE_IN_POPUP=1 prelude paste"

# Optional: one-key access with no prefix. Uncomment if Alt-R is free for you.
# bind -n M-r display-popup -E -w 92% -h 92% -d '#{pane_current_path}' \
#   "PRELUDE_IN_POPUP=1 prelude paste"

# Optional: agents and their questions in the status bar. "2 waiting · 3 working" when there is
# a fleet, nothing at all when there is not. Costs no subprocesses per
# refresh beyond prelude itself — identities come from its cache.
# set -g status-interval 15
# set -g status-right '#(prelude fleet --status) · %H:%M '
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
    use super::ZSH;

    #[test]
    fn global_terminal_autostart_is_one_zle_invocation_not_a_timed_keypress() {
        assert!(ZSH.contains("${PRELUDE_AUTOSTART:-}"));
        assert!(ZSH.contains("add-zle-hook-widget line-init _prelude_autostart_once"));
        assert!(ZSH.contains("add-zle-hook-widget -d line-init _prelude_autostart_once"));
        assert!(ZSH.contains("zle -U $'\\e[27;99~'"));
        assert!(ZSH.contains("zle _prelude_widget"));
        assert!(ZSH.contains("bindkey -r $'\\e[27;99~'"));
        assert!(ZSH.contains("${XDG_CACHE_HOME:-$HOME/.cache}/prelude/global-active"));
        assert!(ZSH.contains("\"$owner\" == \"$PRELUDE_GLOBAL_TOKEN\""));
        assert!(ZSH.contains("_prelude_global_done; zle reset-prompt; return 0"));
        assert!(!ZSH.contains("sleep "));
        assert!(!ZSH.contains("osascript"));
        let remove_hook = ZSH.find("add-zle-hook-widget -d line-init").unwrap();
        let queue = ZSH.find("zle -U $'\\e[27;99~'").unwrap();
        let remove_binding = ZSH.find("bindkey -r $'\\e[27;99~'").unwrap();
        let invoke = ZSH.find("zle _prelude_widget").unwrap();
        assert!(remove_hook < queue, "the line hook must remove itself before dispatch");
        assert!(remove_binding < invoke, "the private binding must be gone before Prelude opens");
    }
}
