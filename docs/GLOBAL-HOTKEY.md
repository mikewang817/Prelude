# The launcher panel

Implementation and acceptance record for Prelude's macOS-wide launcher entry
point. This is a production surface, not a prototype: installation,
configuration, lifecycle, diagnostics and removal are part of the feature.

## Product contract

The configured chord reveals a launcher that is already running, and dismissing
it hides that launcher rather than destroying it. Nothing is created on a press
and nothing is torn down afterwards.

The surface is a Ghostty quick terminal: a centred macOS panel belonging to a
dedicated Ghostty instance that is hidden from the Dock and the app switcher,
owns no window at rest, and never touches the Ghostty the person works in.
Escape dismisses and resets. Any action that moves focus dismisses it too.

The launcher is not the destination. A command goes to the terminal that was in
front when the chord was pressed, when tmux can address its pane, and otherwise
to one window created deliberately for it. Objects — files, folders, URLs and
applications — go straight to Launch Services and cost no terminal at all. No
selected payload ever reaches a command line, an Apple Event, configuration or
a log.

## Architecture

```text
login -> LaunchAgent -> hidden Ghostty instance (no window)
chord -> Ghostty's own global: keybind -> quick terminal panel
                                       -> prelude _panel, forever
                                          -> prelude -> fzf
                                          -> deliver: tmux pane
                                                    | one new window
                                                    | Launch Services
escape -> panel hidden, fzf aborted, loop starts a fresh launcher
```

`~/.config/prelude/quick-terminal.ghostty` is written by
`prelude global install` and validated with `ghostty +validate-config` before
it is installed. The LaunchAgent exists only to start the instance at login;
`open` exits as soon as Launch Services has the request, so the job is not the
thing that runs. Prelude links no GUI framework: the frontmost application is
read with `lsappinfo`.

## Superseded — production global launcher `[x]`

*Replaced by the launcher panel; kept for the traps it records.*

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

## Superseded — configurable, conflict-safe singleton `[x]`

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

## Superseded — a launcher that cannot strand itself `[x]`

The previous milestone was accepted on a manual validation run. Three faults
survived it, each of which stops the hotkey from working and none of which any
test could see.

- [x] A lease records the pid of the shell the launch created, and liveness is
      that pid, not a timeout. The zsh integration writes it while the terminal
      starts; a window that is quit, crashed or force-killed frees the next
      hotkey immediately.
- [x] An unclaimed lease survives only a short grace window, so a launch that
      produced no terminal cannot hold the chord. The thirty minute bound
      remains as a backstop against a recycled pid, not as the mechanism.
- [x] The Ghostty launch verifies it received a *new* application instance.
      Launch Services answering success while handing the request to the
      running instance — which discards the bootstrap arguments — is a launch
      failure, not a success.
- [x] A Ghostty launcher window declines saved state, so it neither restores
      the previous session's windows nor overwrites the saved state of the
      Ghostty being worked in, and its instance retires with its shell.
- [x] A busy chord no longer terminates the helper. Registration is retried
      until it succeeds, and the person is told once rather than every attempt.
- [x] The LaunchAgent restarts a helper that exits uncleanly.
- [x] `status` distinguishes ready, starting and held-by-a-live-shell instead
      of reporting any lease file as a healthy open launcher.

Validation added for this milestone:

- Measured on the machine that reported the fault. Before the change,
  `~/.cache/prelude` held a sixteen-minute-old lease with no launcher behind
  it and five orphaned `fzf` processes from two days earlier, and the hotkey
  answered `already-open` to every press.
- Ghostty 1.3.1 was re-checked against the recorded limitation: `+new-window`
  still reports "not supported on this platform" and the CLI still refuses to
  launch the emulator directly, so `open -na`/`NSWorkspace` remains the only
  supported path and a second instance is unavoidable.
- Three consecutive launches without `--window-save-state=never` left three
  Ghostty instances alive, each having restored two windows of its own — one
  press cost a duplicate of every window open at the time. With the flag, a
  launch produced exactly one instance with exactly one window, and the
  instance exited when its command did.
- The singleton was re-exercised: a second press produced `already-open` and
  no second instance. Cancelling Prelude released the lease with the shell
  still alive. `SIGTERM` and `SIGKILL` on the terminal both returned status to
  `ready` at once and left no orphaned `fzf`.
- `SIGKILL` on the helper was restarted by launchd (`runs = 2`) and the chord
  re-registered. `KeepAlive`'s `Crashed` key was tried first and rejected:
  launchd counts only the classic fault signals, so it did not restart.
- 136 Rust tests, release Clippy with `--all-targets -D warnings`,
  `git diff --check`, Swift warnings-as-errors and three gather benchmarks
  (medians 20.3–23.1 ms, maximum 32.1 ms against 40 ms) pass.

## Milestone — honest conflicts, and a window worth opening `[x]`

- [x] Conflict detection reads macOS's whole shortcut table rather than
      Spotlight's single entry, comparing key code and Cocoa modifier mask
      against every enabled record. An id absent from the table is at its
      default and still live, so the defaults for the four chords a launcher
      collides with are applied when nothing is recorded for them.
- [x] Known shortcuts are named; the rest are reported by id rather than
      guessed at. Status distinguishes "no known owner" from "no known owner,
      but some owner records could not be read".
- [x] Installer and helper apply the same table, so the helper never refuses a
      chord the CLI accepted.
- [x] A launcher window dismissed without handing anything over closes itself.
      `INSERT`, `RUN`, `MSG` and a Prelude failure all leave something to read
      and keep the window. Ctrl+R is unchanged.
- [x] The self-close runs through the line editor rather than calling `exit`
      inside a widget, and does not reach the history file.
- [x] `prelude global directory [PATH|--default]` sets where a launcher window
      starts, default `$HOME`. It is validated once against a rule strict
      enough for both a process argument and an Apple Event, is left out of the
      config until it is asked for, and degrades to `$HOME` if the directory is
      removed later.

Validation added for this milestone:

- A pty exercise drives the generated integration through all five outcomes
  with a fixture `prelude`: dismissal (130) and a direct object action (0 with
  no output) exit the shell; `INSERT`, `MSG` and a failure keep it. Every case
  claims the lease with the shell's pid and releases it on the way out.
- That exercise caught `local status` failing the whole widget before Prelude
  ran — `status` is a read-only alias for `$?` in zsh, and no amount of reading
  the script would have shown it. It would have broken Ctrl+R as well.
- The chord `cmd+space` is reported as Spotlight's on this machine even though
  the plist records no entry 64, which is the ordinary state of an untouched
  Mac and the case the previous check answered "free" to.
- A launch with a configured directory opened there and closed itself when
  dismissed. The Terminal.app bootstrap was checked as shell syntax and put
  through a real Apple Event; the backend itself was not switched live because
  a launcher was in use at the time.
- 139 Rust tests, release Clippy, Swift warnings-as-errors, `git diff --check`
  and gather at a 19.0 ms median pass.

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
  1.3.1 build, and its CLI declines to launch the emulator at all on macOS. The
  helper therefore asks Launch Services for a new application instance, which
  gives the requested fresh window without Accessibility scripting. A launcher
  window is consequently a second Ghostty application rather than a window of
  the one already open, and it runs with saved state disabled — windows created
  inside it are not restored the next time that instance starts.
- A global window starts in `$HOME` unless `prelude global directory` says
  otherwise. There is no honest project directory to infer from an arbitrary
  foreground application, so Prelude asks rather than guesses.
- Naming the owner of a chord is best-effort even now. macOS's shortcut table
  and Raycast's preference are the two registries that can be read; an
  application that merely watches the key appears in neither, which is why the
  Carbon reservation remains the final generic check.


## Milestone — the launcher stops building terminals `[x]`

Everything above is the record of a design that created a terminal on every
press. It was made correct — verified launches, pid-checked leases, restart on
crash — and it was still the wrong shape. Measured on the machine that reported
it, press-to-usable was **373 ms, identical cold and warm**, because a new
macOS application instance was built every time. Prelude's own work, the 40 ms
gather budget this codebase is organised around, was 5% of that. The rest was
construction, and it was thrown away afterwards — including when the thing
chosen was a file, which never needed a terminal at all.

The replacement is a Ghostty quick terminal: a hidden instance hosting one
`prelude _panel` loop that outlives every press.

- [x] The chord is a `global:` Ghostty keybind. Nothing of Prelude's runs when
      the key is pressed, so there is no launch to fail.
- [x] The panel instance is hidden from the Dock and the app switcher, owns no
      window at rest, and declines saved state.
- [x] Escape hides the panel and reaches fzf, so the launcher resets behind a
      hidden panel and stays warm.
- [x] One instance or none, enforced; `RunAtLoad` is the start, not a race
      partner for an explicit launch.
- [x] The launcher is no longer the destination. Commands go to the tmux pane
      of the terminal that was in front, or to one window created on purpose.
      Objects go to Launch Services and cost no terminal.
- [x] A handed-over command travels in a 0600 file, never on a command line.
- [x] The Carbon helper, the lease, the grace window, the pid check, the
      autostart ZLE hook, the Launch Services verification, `global clear` and
      `macos/PreludeHotkey/main.swift` are deleted.

Validation, measured rather than assumed:

- Ghostty 1.3.1 was probed before anything was designed on top of it:
  `macos-hidden = always` puts the instance at `activationPolicy = accessory`;
  `initial-window = false` leaves it with no child process at all; `command` is
  honoured by the quick terminal surface; `global:` and `unconsumed:` keybind
  prefixes are supported on macOS.
- Hiding and showing the panel rebuilds nothing: surface and `fzf` pids are
  unchanged across toggles. Escape hides the panel *and* restarts the launcher
  (`fzf` 16029 → 16209), which is warmth and dismissal at once.
- Autohide fires on a genuine application switch, which is what dismisses the
  panel after any action that moves focus.
- `initial-window = false` was defeated on the first attempt by Ghostty
  restoring the previous session's windows — the same trap this document has
  now recorded three times.
- Two instances appeared on the first real install, from `RunAtLoad` racing an
  explicit start; the panel then opened on every other press.
- `pgrep -f --config-file=…` silently matched nothing: a pattern beginning with
  `--` is read as an option. The marker starts at the key.
- The preload handoff was exercised end to end: a fresh window read the 0600
  file and removed it.
- 139 Rust tests, release Clippy with `-D warnings`, `git diff --check`, and
  gather at a 17.9 ms median pass.

### Recorded limitation

The global launcher is now Ghostty-only, because no other macOS terminal offers
a quick terminal. `backend` chooses only where a handed-over command opens.
