# Global launcher panel

This document describes the current implementation in `src/global.rs`,
`src/panel.rs`, and `install.sh`. The global launcher is macOS-only and
Ghostty-only.

## Product contract

`Cmd+Shift+Space` reveals a Ghostty Quick Terminal owned by one dedicated,
hidden Ghostty process. A keypress does not start Prelude, create an application
instance, or open a login shell. Ghostty owns the global keybind and toggles its
existing quick-terminal surface.

The panel is not a destination terminal:

- files, folders, applications, and URLs act directly through macOS Launch
  Services
- shell commands are copied to the system clipboard
- after copying, Prelude shows a short confirmation for 1.2 seconds and closes
  the surface so it does not cover the place where the command will be pasted
- `Escape` hides the panel and reaches fzf, so the search resets while hidden

Prelude never chooses a terminal, tab, pane, or shell on the user's behalf.
There is no terminal backend setting and no tmux integration.

## Process architecture

```text
login
  └─ launchd: app.prelude.hotkey
       └─ open -W -n -a Ghostty.app --args --config-file=…
            ├─ waits for the dedicated application instance
            └─ Launch Services starts hidden Ghostty
                 ├─ no window or shell while idle
                 └─ global chord toggles a Quick Terminal
                      └─ prelude _surface
                           ├─ quick-terminal marker == 1 → prelude _panel loop
                           │    └─ child prelude → fzf
                           │         └─ refresh thread → fzf --listen socket
                           └─ no marker → open -n exact Ghostty.app and exit
```

## Keeping the list current

The launcher starts before the reveal, not after it, so `gather` runs when the
previous interaction ended. Without anything further, the list on screen is a
snapshot from that moment — possibly hours old — and a clipping copied since
does not appear until the panel is dismissed and re-opened once.

A background thread in the launcher process re-gathers and hands the result to
fzf over a Unix-domain `--listen` socket at
`$XDG_DATA_HOME/prelude/panel.sock`.

- A tick every 3 seconds compares the modification times of the files behind
  the list; a gather runs only when one has moved, or every 30 seconds
  regardless, since a detached slow-source refresh lands without notice.
- The reload is delivered as a `transform`, so fzf evaluates it with the live
  query and cursor index. A panel with a typed query or a moved cursor is left
  untouched; only a panel in the state it was drawn in is redrawn.
- Column widths and the title column come from the initial layout, so computed
  rows and static rows stay in the same columns.
- The socket is absent while `Ctrl+K` runs, which skips a tick rather than
  ending the loop.

Only the global panel does this. The zsh widget's snapshot is never older than
the keypress that made it. Every failure path degrades to the previous
behavior: one snapshot per interaction.

The generated Ghostty configuration sets:

- `initial-window = false`
- `macos-hidden = always`
- `window-save-state = never`
- a centered `62% × 58%` Quick Terminal
- autohide and move-to-current-Space behavior
- `global:<chord>=toggle_quick_terminal`
- `unconsumed:escape=toggle_quick_terminal`
- `cmd+enter=text:\x07` for Prelude's private file-parent shortcut
- `command = <installed-prelude> _surface`
- `abnormal-command-exit-runtime = 0`

`macos-hidden` makes the dedicated instance a macOS UI element, keeping it out
of the Dock and app switcher. `initial-window = false` leaves it with no surface
at rest. Disabling saved state prevents the hidden instance from restoring
ordinary Ghostty windows.

One resident Ghostty application process is required: Ghostty owns the global
event tap and Quick Terminal. It should be invisible and idle, not represented
by a Dock tile. Quitting that dedicated instance causes it to return by design;
`KeepAlive` preserves the global shortcut. `prelude global stop` is the
intentional way to stop both supervision and the instance.

Ghostty requires macOS application launches to go through Launch Services. The
LaunchAgent therefore runs `/usr/bin/open -W -n -a Ghostty.app --args …` rather
than executing `Contents/MacOS/ghostty` directly. `-W` keeps the launcher alive
until the new application instance exits, giving launchd something real to
supervise without bypassing Ghostty's supported launch path. This is also what
allows `macos-hidden = always` to take effect.

## Why `_surface` exists

The hidden process still has Ghostty's ordinary bundle identity. After its
Quick Terminal was active, macOS may deliver a later `open Ghostty.app` request
to that hidden process. If the generated `command` directly entered Prelude,
the resulting ordinary window would also become Prelude.

Every generated surface therefore passes through `prelude _surface`:

- only the exact environment value `GHOSTTY_QUICK_TERMINAL=1` enters the panel
- an unmarked surface launches the exact installed Ghostty app with `open -n`
  and exits cleanly

This special instance behavior applies only to Ghostty. Other applications use
normal macOS instance reuse.

## Installation

The supported one-command path is:

```sh
curl -fsSL https://raw.githubusercontent.com/mikewang817/Prelude/main/install.sh | bash
```

The script:

1. requires macOS on `arm64` or `x86_64`
2. downloads the matching archive and `checksums.txt` from the latest GitHub
   release and verifies SHA-256
3. installs `prelude` in `${PRELUDE_INSTALL_DIR:-$HOME/.local/bin}`
4. reuses an `fzf` with footer support or installs the verified fzf 0.74.2
   binary beside Prelude
5. reuses Ghostty from `/Applications` or `~/Applications`, or verifies and
   installs the official Ghostty 1.3.1 app in `~/Applications`
6. appends one marked PATH/`prelude init zsh` block to `~/.zshrc` when absent
7. runs `prelude global install`

The shell block affects zsh processes started after installation; the installer
does not mutate the already-running shell. The global panel is started during
installation and does not depend on `.zshrc`.

The script is idempotent. Running it again replaces the Prelude binary and
reinstalls/restarts the managed panel while preserving settings.

### Accessibility

Ghostty implements global keybinds with a macOS Accessibility event tap. An
installer cannot grant that permission.

`prelude global install` starts the panel and inspects Ghostty's unified log for
its actual registration result. If permission is missing and stdin is a TTY,
it opens **System Settings → Privacy & Security → Accessibility**, asks the
user to enable Ghostty, restarts the process, and verifies the event tap again.
A running process by itself is not considered a working shortcut.

For a non-interactive install, missing permission is reported as an error with
the command to retry after enabling Ghostty.

## Managed files

With default XDG paths:

| Path | Purpose |
|---|---|
| `~/.config/prelude/global.toml` | selected chord and optional panel directory |
| `~/.config/prelude/quick-terminal.ghostty` | generated dedicated Ghostty configuration |
| `~/Library/LaunchAgents/app.prelude.hotkey.plist` | launchd supervision |
| `~/.cache/prelude/global-hotkey.log` | Ghostty stdout |
| `~/.cache/prelude/global-hotkey-error.log` | Ghostty stderr |
| `~/.cache/prelude/global-event-tap` | registration result tied to the current PID |

`global.toml`, the LaunchAgent, logs, and event-tap record are made private
(`0600`). The generated Ghostty config is not a secret-bearing file: it
contains the Prelude executable path, panel working directory, and a useful
PATH, but never a selected launcher payload.

`global.toml` defaults to:

```toml
hotkey = "cmd+shift+space"
```

The panel directory is omitted until explicitly set, which means `$HOME`.

## Commands

```sh
prelude global install
prelude global uninstall
prelude global uninstall --reset
prelude global start
prelude global stop
prelude global status
prelude global status --json
prelude global open
prelude global hotkey
prelude global hotkey cmd+shift+k
prelude global directory
prelude global directory /absolute/path
prelude global directory --default
```

Behavior:

- `install` validates the generated Ghostty config, writes the LaunchAgent,
  enforces exactly one instance, starts it, and verifies Accessibility.
- `start` refreshes the generated config and LaunchAgent from the current
  executable, starts supervision, and verifies the event tap.
- `stop` boots out launchd supervision and kills any dedicated instance.
- `open` starts/verifies a missing panel instance. It cannot reveal an already
  running panel; only Ghostty's configured chord can do that.
- `hotkey` reads or changes the chord. A successful installed-panel change
  restarts Ghostty; failure restores the previous config.
- `directory` reads or changes where the panel process stands. The value must
  be an existing absolute directory when set. `--default` returns to `$HOME`.
- `uninstall` removes the generated Ghostty config and LaunchAgent but keeps
  preferences/logs. `--reset` also removes `global.toml`, both logs, and the
  event-tap record.

`prelude global backend` is intentionally rejected: the global panel copies
commands and therefore has no destination terminal to choose.

## Chord validation and conflicts

A chord needs at least one of `cmd`, `option`, `ctrl`, or `shift`, followed by
one supported letter, digit, or `space`. Names are canonicalized, for example
`Shift+Command+K` becomes `cmd+shift+k`.

Before install or change, Prelude checks:

- enabled macOS symbolic hotkeys, including defaults that macOS omits from its
  preference file
- Raycast's configured global hotkey when its preference exists

Known system conflicts are named, including Spotlight and input-source
shortcuts. Prelude does not modify those settings. macOS has no public registry
of every application that watches a key, so “no known owner” is not a claim
that no third-party event tap exists.

Fresh installs use `Cmd+Shift+Space`, not stock macOS's Spotlight-owned
`Cmd+Space`. An old saved `Cmd+Space` default may be migrated during install
when the new default is free; an intentional non-default choice is never
silently replaced.

## Status

`prelude global status --json` emits schema 6 with independent fields for:

- panel configuration file
- LaunchAgent file
- launchd supervision
- dedicated process state
- Ghostty Accessibility result
- verified hotkey registration
- configured chord and known owner
- completeness of owner checks
- configured panel directory and its existence
- Ghostty availability
- optional zsh-widget availability

Text status returns success only when configuration, supervision, a running
panel process, Accessibility, registration, Ghostty, and conflict checks are
healthy. Install/start enforce the single-instance invariant; schema 6 does not
expose an instance count. The zsh widget is displayed but is not required for
the global panel.

## Build and upgrade note

`cargo build --release` does not replace an installed binary and does not
restart the long-lived `_panel` parent. The child launcher may appear updated
while the parent still applies old delivery behavior.

After building a checkout you intend to use as the installed panel, reinstall
that binary or run:

```sh
prelude global stop
./target/release/prelude global start
```

`start` rewrites managed paths from the executable that invoked it.

## Current limitations

- The global surface requires Ghostty because no other supported macOS terminal
  exposes this Quick Terminal/global-keybind combination.
- `global open` starts the hidden instance but cannot programmatically reveal
  the Quick Terminal.
- Accessibility must be granted by the user.
- Conflict-owner discovery is best effort for third-party event-tap apps.
- The configured panel directory is global. Prelude does not infer a working
  directory from the foreground application.
- Live refresh cannot redraw a panel that is being used, because a reload
  resets the cursor. A panel left mid-query keeps the list it had until it is
  dismissed.
- Ghostty reports no reveal event, so freshness is bounded by the tick
  interval rather than triggered by the press.
