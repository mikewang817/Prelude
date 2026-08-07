//! Shell and tmux integration, printed for the user to eval/source.

pub const ZSH: &str = r#"# Prelude — zsh integration
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
  esac
  zle reset-prompt
}
zle -N _prelude_widget

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
"#;
