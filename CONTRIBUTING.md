# Contributing

Bug reports—especially a screenshot plus the query and terminal width—are very
useful. Prelude is a latency-sensitive launcher, so behavioral changes should
include both tests and a gather measurement.

## Developer Certificate of Origin

This project uses the [Developer Certificate of Origin](https://developercertificate.org/)
rather than a Contributor License Agreement. Sign off commits:

```sh
git commit -s -m "your message"
```

The `Signed-off-by:` line certifies that you wrote the patch or otherwise have
the right to submit it. Contributions remain under Apache-2.0; contributors
keep their copyright.

## Build and check

```sh
cargo build --release --locked
cargo test --locked
cargo clippy --release --all-targets --locked -- -D warnings
git diff --check
./target/release/prelude bench
```

`bench` must remain below the 40 ms gather budget. Internal commands such as
`_dump`, `_dump-root`, `_dump-all`, `_dynamic`, `_footer`, `_preview`, and
`_actions` make launcher behavior testable without driving an fzf terminal.

For global-panel changes, a release build is not enough: the long-lived panel
parent keeps running the binary it started with. Restart it before manual use:

```sh
prelude global stop
./target/release/prelude global start
```

## Product rules

### Latency is the product

The initial gather runs once when Prelude opens, and fzf starts Prelude helper
processes on every query change. Startup cost and per-keystroke helper cost both
matter.

- Be skeptical of new dependencies.
- Start subprocess-backed fast sources together and optimize the current slowest
  one rather than adding serial work.
- Anything too slow or network-dependent belongs behind a cache or an explicit
  action.
- A source that fails degrades to an empty or previous cached result; it must not
  panic or print into the launcher.
- `is_special` recognizes query intent only. Evaluation belongs in the dynamic
  helper, not in the recognition path.

### Commands are handed over; objects act

The default behavior is per Kind:

- shell commands are inserted into the zsh prompt, or copied from the global
  panel
- files, folders, applications, and URLs act directly through macOS Launch
  Services
- explicit `Run now` can submit a command only from the zsh surface
- explicit `Run and show output` runs inside Prelude

Do not reduce this to “Enter always inserts” or “Enter always runs.” Add or
change defaults in `defaults.rs`, contextual alternatives in `actions.rs`, and
keep [docs/ACTIONS.md](docs/ACTIONS.md) synchronized.

### Never guess Agent facts

`agent.rs` is the only built-in Agent registry. Native Session files, Agent CLI
output, and live processes are authoritative. Prelude may derive stable ids,
relationships, redacted fingerprints, and local archive/Favorite overlays; it
must not invent a model, capability, state, token count, cost, or supported CLI
verb.

Read [docs/AGENT-CONTROL-PLANE.md](docs/AGENT-CONTROL-PLANE.md) before changing
Agent, Run, Session, Skill, MCP, Config, Home, bus, or Agent Doctor behavior.
Update that file in the same commit.

### Never retain credentials

`secrets.rs` filters history, clipboard text, messages, tags, transcripts, Skill
material, and configuration evidence. Complete MCP definitions may be resolved
for an explicit action but must not survive in an Item, ordinary cache, preview,
or Control JSON.

Private staging belongs under Prelude's XDG directories with restrictive
permissions. Destructive native-file operations canonicalize their ownership
boundary, confirm with Cancel first, and move to Trash.

## Rendering

Three invariants are easy to break:

- **Compute layout once and pass it down.** The initial launcher and
  per-keystroke helper are separate processes. Shared title/column widths must
  be computed from the gathered catalogue and passed to rendering; do not let
  each process derive a different layout. `f:` is the deliberate width-derived
  filename/parent exception.
- **Display width is not character count.** CJK is two columns, and East Asian
  Ambiguous characters depend on the terminal. Use `width::dwidth` and the
  width helpers rather than `.len()` or `.chars().count()`.
- **fzf matches displayed text.** Computed and scoped rows cannot fuzzy-match
  the syntax that produced them. Their intent must be recognized and fzf search
  disabled while Prelude supplies the rows.

## Documentation

The user-facing reference is intentionally code-shaped:

- `README.md`: product boundary and installation
- `docs/SEARCH.md`: `compute.rs`, scope/source behavior, and cache limits
- `docs/ACTIONS.md`: `defaults.rs`, `actions.rs`, and surface handoff
- `docs/GLOBAL-HOTKEY.md`: `global.rs`, `panel.rs`, and `install.sh`
- `docs/AGENT-CONTROL-PLANE.md`: Agent registry, graph, sources, bus, and Doctor

When behavior changes, update the corresponding document and generated text
such as `prelude --help` or `prelude init agent`. Historical implementation
plans do not substitute for describing the current code.

## Platform

Prelude currently supports macOS only. Applications, clipboard, Finder tags,
processes, ports, Launch Services, the Ghostty panel, and translation use
macOS-specific interfaces. The inline shell integration is zsh-specific.
