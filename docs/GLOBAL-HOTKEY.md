# Global hotkey terminal

Implementation and acceptance record for Prelude's macOS-wide launcher entry
point. This is a production surface, not a prototype: installation,
configuration, lifecycle, diagnostics and removal are part of the feature.

## Product contract

The configured global hotkey (`Cmd+Space` by default) creates a **new terminal
window** and invokes Prelude exactly as the existing zsh `Ctrl+R` widget does.
It never takes over, types into or guesses at an existing terminal. While that
launcher is still open, another press focuses its terminal application instead
of creating a duplicate; once Prelude returns to the shell, the next press is a
fresh launcher.

Backend selection is deterministic:

1. use Ghostty when Launch Services reports `com.mitchellh.ghostty` installed;
2. otherwise use Terminal.app;
3. allow an explicit `auto`, `ghostty` or `terminal` preference;
4. if an automatic Ghostty launch fails, report it and fall back to Terminal;
   an explicitly requested backend fails visibly rather than silently changing
   the user's choice.

A launch creates a fresh terminal process. Commands still land on a prompt for
review, objects still act directly, and no selected payload is ever put in
AppleScript, process arguments, preferences or helper logs. Installation and
hotkey changes first reserve the requested chord temporarily; Spotlight,
Raycast or any other current owner must release it before Prelude changes the
running helper.

## Architecture

The existing Rust binary and zsh widget remain authoritative for search and
result handling. A separate, dependency-free AppKit helper owns only the global
hotkey and terminal creation:

```text
configured key -> Prelude Hotkey.app -> singleton lease
                                      -> Ghostty.app or Terminal.app
                                      -> PRELUDE_AUTOSTART=1 zsh -il
                                      -> one-shot _prelude_widget
                                      -> release lease when Prelude returns
```

The helper is an `LSUIElement` application with no Dock icon. It registers the
hotkey through Carbon `RegisterEventHotKey`, which does not require Accessibility
keyboard monitoring. Ghostty is discovered by bundle identifier through Launch
Services, never by a GUI process's incomplete `$PATH`. Terminal is controlled
with one fixed Apple Event command containing no user-selected data.

The helper source lives in `macos/PreludeHotkey`; `prelude global install`
builds and ad-hoc signs an app in `~/Applications`, writes a LaunchAgent, and
starts it. Installation is explicit and may invoke Xcode's installed Swift
compiler; no Swift or AppKit dependency enters the latency-sensitive CLI.

## Milestone — production global launcher `[x]`

- [x] `prelude init zsh` supports a one-shot `PRELUDE_AUTOSTART=1` ZLE hook and
      then restores an ordinary shell.
- [x] The hook invokes the same `_prelude_widget`; it does not synthesize a
      delayed `Ctrl+R` or duplicate `INSERT`, `RUN` and `MSG` handling.
- [x] A native helper registers the configured chord, ignores key repeat, and
      creates a fresh terminal when no launcher is already active.
- [x] Ghostty is detected by bundle identifier and launched as a new app/window
      with a login zsh and `$HOME` working directory.
- [x] Terminal.app is the fallback and receives only a fixed bootstrap command.
- [x] `auto`, `ghostty` and `terminal` are supported, with validated atomic
      configuration under Prelude's XDG config directory.
- [x] `prelude global install|uninstall|start|stop|status|open|hotkey|backend` manages
      the app, LaunchAgent and preference without touching terminal or shell
      configuration.
- [x] Installation is repeatable, upgrades atomically, uses no personal path,
      ad-hoc signs the app, and rolls back an incomplete replacement.
- [x] Uninstallation stops the helper and removes only Prelude-owned global
      integration files; user configuration is retained unless explicitly
      reset.
- [x] Status and Doctor report helper installation, process state, selected and
      detected backend, Ghostty availability, zsh integration and the Spotlight
      shortcut conflict where macOS exposes it.
- [x] Launch failures are bounded, written without payloads to a private status
      file, and never leave the hotkey handler blocked.
- [x] Documentation covers Spotlight reassignment, Automation permission for
      Terminal.app, backend selection, login behaviour and complete removal.
- [x] Unit tests cover config parsing, generated app metadata, LaunchAgent
      safety, fixed launch arguments, autostart one-shot behaviour and path
      boundaries without launching GUI applications.
- [x] Full tests, release Clippy, `git diff --check`, release build and repeated
      gather benchmarks pass; gather remains under 40ms.

## Validation record

- 131 Rust tests pass. A PTY exercise with a fixture `prelude` command confirmed
  that `PRELUDE_AUTOSTART` invokes the widget once, removes its hook, and leaves
  the next prompt ordinary.
- The Swift helper compiles optimized with warnings as errors and reports
  Ghostty through Launch Services on the validation machine.
- A hermetic HOME/XDG installation built, signed, loaded, reported, stopped and
  uninstalled without touching the person's integration. A repeated real
  installation preserved `auto`, replaced the app, restarted its LaunchAgent
  and left no installer backup.
- `prelude global open` created both a Ghostty window in `auto` mode and a
  Terminal.app window when explicitly selected; each attempt produced only a
  bounded, payload-free status event. The final installed state reports app,
  LaunchAgent, process, hotkey and zsh widget healthy, Ghostty effective, and
  Spotlight released.
- Config, LaunchAgent, helper logs and status are mode 0600. App and LaunchAgent
  removal were exercised in an isolated HOME; `--reset` removed the Prelude
  preference and status while ordinary uninstall retained configuration.
- Release Clippy passed with `--all-targets -D warnings`; `git diff --check` and
  the release build passed. Five final settled gather runs had medians
  15.7–21.7 ms and an observed maximum 25.3 ms against the 40 ms budget.

## Milestone — configurable, conflict-safe singleton `[x]`

- [x] The global chord is configurable with `prelude global hotkey`, stored
      atomically beside the backend and parsed identically by installer and
      helper.
- [x] Supported chords have one or more of `cmd`, `option`, `ctrl`, `shift` and
      one `space`, letter or digit key; invalid or modifier-free values are
      refused before configuration changes.
- [x] Installation and hotkey changes perform a Carbon reservation check before
      replacing or restarting the helper. A conflict leaves the previous app,
      config and registration running and names Spotlight/Raycast as likely
      owners without pretending macOS reveals every app that observes a chord.
- [x] Spotlight is checked directly when the requested chord is `Cmd+Space`;
      Prelude never changes the system preference.
- [x] The helper maintains one private, token-checked launcher lease. Repeated
      hotkeys while Prelude is open focus the effective terminal application
      and create no second terminal.
- [x] The existing zsh widget releases that lease on cancel, direct object
      action, `INSERT`, `RUN`, `MSG` or failure, without retaining the selected
      payload or changing normal `Ctrl+R` behaviour.
- [x] A bounded stale lease cannot lock the launcher out permanently, and
      status/Doctor expose whether a launcher is active.
- [x] Tests, both real terminal backends, repeated installation, release Clippy,
      Swift warnings-as-errors and gather benchmark pass under the existing
      budget.

Validation added for this milestone:

- Fixture Spotlight and Raycast plists both stopped installation before an app
  was built. On the validation machine Raycast owns `cmd+space`; an attempted
  change was refused while the working `cmd+shift+space` helper remained
  running and registered.
- Live changes between two free chords restarted the helper cleanly. Invalid,
  duplicate and modifier-free chords left configuration unchanged.
- Ghostty is launched with its macOS-supported `-e` argv path. An earlier
  `--initial-command` form returned Launch Services success while opening no
  Prelude in the single-instance app; validation now requires both one new
  Ghostty process/window and one new fzf child. Two immediate `global open`
  calls created those once, not twice; the second event was `already-open` and
  focused the effective backend. The same lease behaviour passed with
  Terminal.app.
- A PTY fixture confirmed the one-shot widget removes only its matching token
  and leaves the next prompt ordinary. The lease is private, carries only a
  random UUID and backend, expires after 30 minutes, and has an explicit
  `prelude global clear` recovery command.

## Implementation commits

- `ddcba60` — production contract, architecture and acceptance plan.
- `75c635c` — AppKit helper, terminal backends, installer, lifecycle,
  diagnostics, one-shot zsh integration, tests and documentation.
- `62d52d3` — configurable-key and singleton refinement plan.
- `add0b23` — Spotlight/Raycast preflight, live hotkey changes, native
  defense-in-depth checks and token-checked singleton lifecycle.
- `54ea942` — Ghostty's supported `-e` launch path and exact active-application
  focusing; replaces a Launch Services success that opened no Prelude.

## Recorded limitations

- macOS owns `Cmd+Space` for Spotlight by default, and Raycast commonly uses the
  same chord. Prelude cannot and will not modify either preference; installation
  names a known owner and asks the user to reassign it or choose another chord.
  macOS exposes no complete public registry naming every event-tap owner, so a
  Carbon reservation is the final generic check rather than a claim to identify
  every possible application.
- Terminal.app asks for Automation consent the first time the helper controls
  it. Prelude cannot grant that permission itself.
- Ghostty's macOS `+new-window` action is unavailable in the currently tested
  1.3.1 build. The helper therefore asks Launch Services for a new application
  instance, which gives the requested fresh window without Accessibility
  scripting.
- A global window starts in `$HOME`; there is no honest project directory to
  infer from an arbitrary foreground application.
