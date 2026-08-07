# Working on Prelude

A Raycast-style launcher for the terminal. Rust, built on fzf, macOS only.
`README.md` describes what it does; this file is what a new session needs to
avoid repeating mistakes already made.

## Build and check

```sh
cargo build --release
cargo test                 # 7 tests
cargo clippy --release     # expected warning-free
./target/release/prelude bench     # gather must stay under 40ms
./target/release/prelude _dump     # non-interactive list, for diffing layout
```

`_dump`, `_footer`, `_preview`, `_bind`, `_dynamic`, `_copy`, `_runhere`,
`_ask`, `_enter`, `_refresh`, `_copy-skill` are internal entry points. They
exist so behaviour can be tested without standing up a terminal — use them
rather than trying to drive fzf.

## The rules that matter

**Latency is the product.** fzf re-invokes the binary on *every keystroke*
through a transform binding, so startup cost is paid hundreds of times per
session. Be sceptical of new dependencies. `bench` must stay under 40ms.

**Enter is the primary action, Option+Enter the secondary, Option+K the
panel.** Matching Raycast, whose manual defines exactly these three. Neither
action is a fixed verb; both are per-item, and they are opposites — where one
acts, the other hands you text. A test asserts they never coincide.

**Commands insert, objects act.** Inserting a file path when you wanted to
read the file is a step backwards; opening a file is harmless in a way that
`kill $(lsof -ti tcp:3000)` is not. The default also depends on the host:
the same file means "open it" at a shell and "here is the path" in an agent's
input box.

**Sources degrade to nothing.** A source that fails, or finds nothing,
returns an empty list. Never blocks, never panics, never prints. Anything
slower than ~25ms belongs behind the cache tier.

**Never index or transmit credentials.** `secrets.rs` filters history and
clipboard. Route any new source that reads user data through it.

**No hard-coded personal paths.** Everything goes through `paths.rs` and the
XDG variables. The repository is meant to be publishable.

## Traps that have each already caused a bug

**zsh metafies its history file.** Bytes in `0x80-0x9f` are stored as `0x83`
followed by `byte ^ 32`. Decode it as plain UTF-8 and every multi-byte
character is mangled — and the replacement characters have the wrong display
width, so column alignment breaks silently. `unmetafy()` undoes it. A test
pins `e5 83 bf ba` decoding as 基.

**Display width is not character count.** CJK is two columns; East Asian
*Ambiguous* characters (`·` `—` `“”` `→`) are one or two depending on the
terminal, and `·` is the separator on every row. `doctor` measures it with a
cursor-position report and caches the answer. Always use `width::dwidth`.

**fzf matches against displayed text.** A row computed *from* the query can
never fuzzy-match it, so `is_special()` exists and must stay pure
pattern-matching — it runs on every keystroke and must not evaluate anything.
`{}` in a binding is the *transformed* text, so bindings use `{2}` to reach
the payload.

**Layout must be computed once and passed down.** The per-keystroke helper
runs in a separate process. If both sides measure their own widths they
drift, and computed rows land in a different column. With the detail pane
showing, measure the *list* width, not the terminal's.

**Column widths are shared across all kinds and taken at a percentile.**
Per-kind widths only align within a kind, so the dots scatter. And one
outlier — a session in a deep iCloud path, 127 columns — will set the column
for two thousand rows if you take the maximum.

**Drain child output.** `exec::run` reads stdout on a helper thread because
waiting on a process while its pipe fills deadlocks past 64KB; `ps -Ao`
emits ~74KB. Keep stderr too: discarding it turned every agent failure into
a permanent, silent "asking…".

## Agents

Agent rows sort above everything else, in their own priority band, so that
learned ranking cannot lift another kind past them. This is deliberate:
agents are what the launcher is for.

MCP status is asked of each agent (`claude mcp list`, `codex mcp list
--json`), never read from config files — the config misses claude.ai-hosted
servers entirely and cannot report whether anything works.

Each agent has its own invocation syntax. `opencode` needs a subcommand
where the others take a prompt positionally; `codex exec` refuses to run
outside a git repository. See `AGENTS` in `sources/sessions.rs`.

Copying a skill between agents is the only place Prelude writes to a user's
files. It stays behind an explicit action, never a default, and never
overwrites.
