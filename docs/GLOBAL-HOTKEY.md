# Global hotkey terminal

Implementation and acceptance record for Prelude's macOS-wide launcher entry
point. This is a production surface, not a prototype: installation,
configuration, lifecycle, diagnostics and removal are part of the feature.

## Product contract

`Cmd+Space` creates a **new terminal window** and invokes Prelude exactly as the
existing zsh `Ctrl+R` widget does. It never takes over, types into or guesses at
an existing terminal.

Backend selection is deterministic:

1. use Ghostty when Launch Services reports `com.mitchellh.ghostty` installed;
2. otherwise use Terminal.app;
3. allow an explicit `auto`, `ghostty` or `terminal` preference;
4. if an automatic Ghostty launch fails, report it and fall back to Terminal;
   an explicitly requested backend fails visibly rather than silently changing
   the user's choice.

Every press creates a fresh terminal process. Commands still land on a prompt
for review, objects still act directly, and no selected payload is ever put in
AppleScript, process arguments, preferences or helper logs.

## Architecture

The existing Rust binary and zsh widget remain authoritative for search and
result handling. A separate, dependency-free AppKit helper owns only the global
hotkey and terminal creation:

```text
Cmd+Space -> Prelude Hotkey.app -> Ghostty.app or Terminal.app
                                 -> PRELUDE_AUTOSTART=1 zsh -il
                                 -> one-shot _prelude_widget
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

## Milestone — production global launcher `[>]`

- [ ] `prelude init zsh` supports a one-shot `PRELUDE_AUTOSTART=1` ZLE hook and
      then restores an ordinary shell.
- [ ] The hook invokes the same `_prelude_widget`; it does not synthesize a
      delayed `Ctrl+R` or duplicate `INSERT`, `RUN` and `MSG` handling.
- [ ] A native helper registers `Cmd+Space`, ignores key repeat, and creates one
      fresh terminal per press.
- [ ] Ghostty is detected by bundle identifier and launched as a new app/window
      with a login zsh and `$HOME` working directory.
- [ ] Terminal.app is the fallback and receives only a fixed bootstrap command.
- [ ] `auto`, `ghostty` and `terminal` are supported, with validated atomic
      configuration under Prelude's XDG config directory.
- [ ] `prelude global install|uninstall|start|stop|status|open|backend` manages
      the app, LaunchAgent and preference without touching terminal or shell
      configuration.
- [ ] Installation is repeatable, upgrades atomically, uses no personal path,
      ad-hoc signs the app, and rolls back an incomplete replacement.
- [ ] Uninstallation stops the helper and removes only Prelude-owned global
      integration files; user configuration is retained unless explicitly
      reset.
- [ ] Status and Doctor report helper installation, process state, selected and
      detected backend, Ghostty availability, zsh integration and the Spotlight
      shortcut conflict where macOS exposes it.
- [ ] Launch failures are bounded, written without payloads to a private status
      file, and never leave the hotkey handler blocked.
- [ ] Documentation covers Spotlight reassignment, Automation permission for
      Terminal.app, backend selection, login behaviour and complete removal.
- [ ] Unit tests cover config parsing, generated app metadata, LaunchAgent
      safety, fixed launch arguments, autostart one-shot behaviour and path
      boundaries without launching GUI applications.
- [ ] Full tests, release Clippy, `git diff --check`, release build and repeated
      gather benchmarks pass; gather remains under 40ms.

## Recorded limitations

- macOS owns `Cmd+Space` for Spotlight by default. Prelude cannot and will not
  modify that system preference; installation tells the user exactly where to
  reassign it.
- Terminal.app asks for Automation consent the first time the helper controls
  it. Prelude cannot grant that permission itself.
- Ghostty's macOS `+new-window` action is unavailable in the currently tested
  1.3.1 build. The helper therefore asks Launch Services for a new application
  instance, which gives the requested fresh window without Accessibility
  scripting.
- A global window starts in `$HOME`; there is no honest project directory to
  infer from an arbitrary foreground application.
