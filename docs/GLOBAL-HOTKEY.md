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
Because macOS still sees that hidden process as Ghostty, it can receive a later
ordinary Ghostty launch after its quick terminal was active. Every surface
therefore enters through a marker-aware gate: quick-terminal surfaces run
Prelude, while an ordinary surface is replaced by a new default-configured
Ghostty instance. Other applications retain normal Launch Services reuse.
Escape dismisses and resets. Any action that moves focus dismisses it too.

The launcher is not the destination, and it never creates one. A command goes
to the **clipboard** and the panel stands down. Objects — files, folders, URLs
and applications — go straight to Launch Services and cost no terminal at all.
No selected payload ever reaches a command line, an Apple Event, configuration
or a log.

## Architecture

```text
login -> LaunchAgent -> supervised hidden Ghostty instance (no window)
chord -> Ghostty's own global: keybind -> quick terminal panel
                                       -> prelude _surface [quick marker]
                                          -> prelude _panel -> prelude -> fzf
                                             -> INSERT|RUN -> clipboard, close
                                             -> object     -> Launch Services
ordinary Ghostty request routed here -> prelude _surface [no marker]
                                       -> open -n exact Ghostty.app, close
escape -> panel hidden, fzf aborted, loop starts a fresh launcher
```

`~/.config/prelude/quick-terminal.ghostty` is written by
`prelude global install` and validated with `ghostty +validate-config` before
it is installed. The LaunchAgent runs the dedicated Ghostty executable as its
actual `KeepAlive` job, so launchd restarts a panel that exits instead of merely
remembering that `/usr/bin/open` once succeeded. Prelude links no GUI framework
and does not ask macOS what is in front: it does not need to know.

Ghostty implements `global:` keybinds with a macOS Accessibility event tap.
That permission cannot be granted by an installer. Installation opens the
exact System Settings pane for the one required click, restarts Ghostty, and
reads Ghostty's unified-log registration result before claiming the shortcut
works. Process existence alone is never reported as hotkey readiness.

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
- [x] Cmd+Enter on a File/Find row opens its containing folder. The dedicated
      config translates it to Ctrl+G, leaving the person's normal Ghostty and
      the inline zsh widget untouched; no other row advertises the chord.
- [x] One instance or none, enforced; `RunAtLoad` is the start, not a race
      partner for an explicit launch.
- [x] The launcher is no longer the destination. Commands go to the tmux pane
      of the terminal that was in front — but only when exactly one tmux client
      is attached, which is the only unambiguous case — and otherwise to one
      window created on purpose. Objects go to Launch Services and cost no
      terminal. *(Superseded below: the panel copies.)*
- [x] The panel decides delivery itself rather than letting the child do it,
      so it can tell a delivery from a dismissal and stand down when nothing
      else took focus.
- [x] A handed-over command travels in a 0600 file, never on a command line.
      *(Superseded below: nothing is handed to a new shell.)*
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
- Reported from use: Enter on an agent row did nothing visible.
  `tmux display-message -p '#{pane_id}'`, asked from outside a client, had
  named `%39` in a `prelude-test` session left over from days earlier, and the
  command was typed there. Delivery now requires exactly one attached client.
  The panel also never stood down, because the child delivered and printed
  nothing, so the loop could not tell a delivery from a dismissal.
- The corrected path was driven on a pty, which is the same contract without
  the window server: Enter opened one window, the preload file was consumed by
  its shell, and the loop stayed warm.
- 139 Rust tests, release Clippy with `-D warnings`, `git diff --check`, and
  gather at a 17.9 ms median pass.

### Recorded limitation

The global launcher is Ghostty-only, because no other macOS terminal offers a
quick terminal.

## The panel copies, and Prelude no longer knows what tmux is `[x]`

The delivery decision above — pane when tmux could address one, a new window
otherwise — was two answers to a question a launcher cannot answer: *which
prompt did you mean?* Each was wrong in its own direction. The window opened in
the configured directory rather than the one being worked in, so a `cd` was the
first thing typed into it. The pane required exactly one attached client, which
is the narrow case, and the entry above records what the wide case cost.

A command now goes on the clipboard. Where it lands is asked of the only thing
that knows, at the moment it knows: the person, with `⌘V`, in the window they
were already in.

- [x] `INSERT` and `RUN` both copy. The difference between them is whether a
      shell presses Enter for you, and there is no shell in this surface; the
      distinction survives at the `Ctrl+R` widget, where it can mean something.
- [x] The panel prints `copied: …`, holds it for 1.2 s, and closes. Nothing
      else took focus, so autohide has nothing to react to and the panel would
      otherwise cover the paste target.
- [x] `destination`, `attached_pane`, `TERMINALS`, `stage_preload`,
      `open_working_window`, the `PRELUDE_PRELOAD` zle hook, `Backend` and
      `prelude global backend` are deleted. `backend` in an existing
      `global.toml` is ignored rather than rejected; the subcommand says why it
      is gone rather than silently doing nothing.
- [x] `defaults::Surface` keeps the labels honest: `Insert into prompt` at a
      prompt, `Copy the command` in the panel. It is read from
      `PRELUDE_TO_CLIPBOARD` at each entry point — fzf's footer and preview
      helpers are separate processes and inherit it — and passed as a parameter
      to every rule below that, so nothing races an env var in a test.
- [x] Rows whose only meaning was "and submit it for you" are suppressed when
      copying: the `Run it in the shell` secondary and the generic `Run now`
      tail. `Run and show output` stays, because it runs inside Prelude.

Prelude is now tmux-independent. The removals, and what each cost:

- [x] `prelude paste [pane]`, `prelude init tmux` and the `prefix + r` popup.
      With them goes `Host::Agent` — the entire second host, where Enter's
      answers inverted because the destination was a conversation rather than a
      prompt.
- [x] `Verb::JumpTo` and `Verb::SplitPane`. A running agent's Enter is now its
      directory; a Session a live run owns hands over that run's project.
- [x] `running.rs` no longer runs `tmux list-panes`. Run rows lost their pane
      and their `session:window.pane` address; `addr` is `pid N` and `cmd` is
      `kill <pid>`, which is what keeps `finish`'s `(kind, cmd)` dedupe from
      collapsing two agents in one project. State is decided by the session
      file's mtime alone — the pane's `#{window_activity}` was the second clock
      and only ever existed for some rows.
- [x] `bus.rs` no longer types into panes. `say` leaves every message in an
      inbox, through `bus::leave`, which the launcher's "Leave it a message…"
      also calls. `$TMUX_PANE` is gone from `whoami`, so `$PWD` is the whole of
      an inbox address — which makes `say`'s refusal of an ambiguous target
      load-bearing rather than merely careful. `delivered_line` went with the
      pane it was flattened for; the sender is named where the inbox is
      rendered, so the stored text keeps what was written.
- [x] A skill row carries both handover forms in `^K`, named. Prelude used to
      pick between them from `pane_current_command`; the guess is unavailable
      and the failure it avoided — `/name` at an agent that lacks the skill is
      silent prose — is not, so the person picks.
- [x] `preview.rs` no longer captures a pane. Quick Look on a run reads the
      conversation file, which every run has.
- [x] `doctor` drops the tmux check and the pane half of `undeliverable`.
      `PRELUDE_NO_POPUP` is gone; `PRELUDE_IN_POPUP` is now
      `PRELUDE_FULL_SURFACE`, which is what it always meant.

Validation:

- 141 Rust tests, release Clippy with `-D warnings`.
- `gather` fell from a 23.7 ms to a 20.4 ms median, and `doctor` reports 15 ms:
  `tmux list-panes` was a subprocess on every gather, paid whether or not tmux
  was running.
- `_footer` and `_actions` were compared with and without
  `PRELUDE_TO_CLIPBOARD` on a real agent row: the label changes, and the
  clipboard surface has no `secondary`/`Start now` row and no `run`/`Run now`
  row. The first attempt suppressed only the secondary, and the generic tail
  put `Run now` back — both paths are now pinned by one test.
- `grep -rn tmux src/` returns only prose that records why something was
  removed.

## Milestone — one command ends in a working shortcut `[x]`

The previous installer verified a config file and a process. Neither answered
the question the user asked: *does the shortcut work?* A stock Mac also owned
the default `Cmd+Space`, the LaunchAgent supervised a short-lived `open`
command rather than Ghostty, and Ghostty's undocumented-to-Prelude
Accessibility requirement was left for the user to discover after a successful
installation message.

- [x] Fresh installs use `Cmd+Shift+Space`; an old saved `Cmd+Space` default is
      migrated when Spotlight still owns it, while an intentional free
      `Cmd+Space` remains untouched.
- [x] `install.sh` downloads verified native Prelude and fzf binaries, installs
      the official Ghostty app into `~/Applications` when absent, adds the zsh
      integration, and installs the global panel. Rust, Homebrew, cloning and
      hand-editing dotfiles are not prerequisites.
- [x] Tagged releases build ad-hoc-signed arm64 and x86_64 binaries and publish
      them with SHA-256 checksums for the one-line installer.
- [x] Ghostty's actual Accessibility event-tap result is the hotkey authority.
      Installation opens the correct System Settings pane for the unavoidable
      click, waits, restarts, and refuses to print success until Ghostty reports
      `global event tap enabled for global keybinds`.
- [x] The LaunchAgent runs Ghostty itself with `KeepAlive`; killing the panel is
      repaired by launchd without a command the user has to know.
- [x] The generated Ghostty config carries a useful PATH, including the
      installer's private binary directory, so a login-launched panel can find
      fzf without relying on `.zshrc`.
- [x] `global status` separately reports configuration, launchd supervision,
      process state, Accessibility and verified chord readiness. Ctrl+R's zsh
      widget is accurately optional for the clipboard-based global panel.

Validation:

- 164 tests pass and release Clippy is warning-free. The v0.6.0 release
  workflow built and published ad-hoc-signed arm64 and x86_64 archives with
  checksums.
- The public README command was then run through its actual curl pipe. It
  downloaded and verified the arm64 release, reused current fzf/Ghostty,
  installed `~/.local/bin/prelude`, added the managed zsh block and ended with
  every schema-6 status field healthy. The resulting quick terminal ran that
  installed binary, not the checkout.
- A real installation reported schema 6 with supervision, Accessibility and
  chord readiness all true. `SIGKILL` changed Ghostty pid 85687 to 86566 under
  launchd, after which status remained healthy.
- A generated Cmd+Shift+Space event opened the replacement process's quick
  terminal and produced one `prelude _panel` and one fzf child. Escape reset it.
- Three release gathers remained below the 40 ms median budget (27.0–35.8 ms).

## Milestone — opening Ghostty does not open Prelude `[x]`

The hidden panel has Ghostty's bundle identity even though it is absent from the
Dock and app switcher. After the quick terminal becomes the most recently active
Ghostty instance, a plain application launch can be delivered to that process.
The panel configuration used to apply `command = prelude _panel` to every
surface, so each ordinary window created there became another Prelude launcher.

- [x] The generated command is `prelude _surface`, not `_panel`. Ghostty's
      `GHOSTTY_QUICK_TERMINAL=1` marker is the authority that enters the panel;
      missing, false-looking or invented values never do.
- [x] An unmarked surface launches the exact discovered `Ghostty.app` path with
      `open -n` and exits, replacing the misrouted surface with an ordinary,
      default-configured Ghostty instance.
- [x] The routing surface sets `abnormal-command-exit-runtime = 0`, so its
      intentional immediate exit closes rather than leaving an error terminal.
- [x] The Ghostty Application row also carries `open -na Ghostty`, so copying or
      inserting its command preserves the independent-instance requirement.
- [x] No other application receives `-n`; ordinary macOS application reuse is
      unchanged, and launch arguments go directly to `/usr/bin/open` without
      shell interpolation.

Validation:

- Unit coverage pins the exact quick-terminal marker, generated surface gate,
  Ghostty's displayed command and direct launch arguments. Visual Studio Code
  retains the ordinary `open -a` command and path-only Launch Services arguments.
- With only the supervised hidden instance running, a real plain `open
  /Applications/Ghostty.app` first reached that instance, passed through the
  unmarked gate, and left no child there. It produced a second Ghostty process
  with an ordinary login-zsh child and no Prelude child — the exact misrouting
  path that previously reproduced the fault.
- 165 tests, release Clippy, `git diff --check` and a release build pass. A
  settled gather remains within budget at a 24.3 ms median and 33.4 ms maximum.
