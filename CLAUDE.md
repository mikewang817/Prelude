# Working on Prelude

A Raycast-style launcher for the terminal. Rust, built on fzf, macOS only.
`README.md` describes what it does; this file is what a new session needs to
avoid repeating mistakes already made.

## Build and check

```sh
cargo build --release
cargo test                 # 23 tests
cargo clippy --release     # expected warning-free
./target/release/prelude bench     # gather must stay under 40ms
./target/release/prelude _dump     # non-interactive list, for diffing layout
```

`_dump`, `_footer`, `_preview`, `_bind`, `_dynamic`, `_copy`, `_runhere`,
`_ask`, `_enter`, `_refresh`, `_copy-skill`, `_lend-skill`, `_lend-mcp`,
`_actions` are internal entry points. They
exist so behaviour can be tested without standing up a terminal — use them
rather than trying to drive fzf.

The agent-facing verbs (`ask --no-wait`, `tell`, `say`, `inbox`, `answer`,
`answer-of`, `fleet --json`) are the same kind of door and are the fastest
way to exercise the bus end to end without a second agent:

```sh
ID=$(prelude ask --no-wait "proceed?")   # what an agent runs
prelude inbox --human                    # what the person sees
prelude answer "$ID" "go ahead"          # the return path
prelude answer-of "$ID"                  # what the agent collects
```

`--human` exists because `whoami()` reads the process tree, and a person
working inside an agent's terminal is correctly identified as that agent —
which would otherwise make their own inbox unreachable from the very window
they are sitting in.

## The rules that matter

**Latency is the product.** fzf re-invokes the binary on *every keystroke*
through a transform binding, so startup cost is paid hundreds of times per
session. Be sceptical of new dependencies. `bench` must stay under 40ms.

**Two keys: Enter is the primary action, Ctrl+K is the panel.** Raycast has
a third — the secondary action on its own key — and Prelude keeps the
*action* but not the key: it is the second entry of the panel, under Enter's.
Neither action is a fixed verb; both are per-item, and they are opposites —
where one acts, the other hands you text. A test asserts they never coincide.

Ctrl, because only Ctrl reliably arrives. Unless told otherwise macOS spends
Option on composing characters, so Option+K types `˚` into the search box and
Option+Enter comes through as a bare Enter — running the *primary* action,
silently. Cmd never reaches a terminal at all. A key that works here and
quietly does nothing on the next machine is worse than no key, which is why
`EXPECT` is down to `ctrl-x,ctrl-k` and there is no terminal configuration to
explain to anyone.

**The line is not danger, it is whether there is anything to edit.** A
command line goes onto the prompt — including agents, skills and sessions,
which are the ones most often the *start* of a command (`--resume`, a model,
a question). Safety is the second reason, not the first: `claude` is
harmless and is still handed over. An object just happens, because nobody
proofreads `open -a Zed foo.json`. Files therefore go to the application
that owns them rather than to `$EDITOR`, so a `.png` lands in Preview;
`openwith.rs` remembers overrides per extension and `^K` is where they are
made — that panel is the settings surface, not a second list of shortcuts.

In a conversation the rule cannot hold, because there is no prompt to paste
onto: whatever is handed over lands in a *conversation*. A path and a
command still read as things to say to an agent, but an agent row does not —
typing `codex` at claude sends claude the word "codex". There, Enter splits
the pane instead — same window, same directory, so both conversations stay
visible, which is the only reason to start a second one. `-h` above 170
columns and `-v` below, since two agent TUIs at sixty columns each wrap
their own output. tmux is guaranteed on that path, because the popup is how
`Host::Agent` is reached at all.

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

**Running is not the same as recorded, and tmux is not the requirement.**
`sessions.rs` reads conversation files; `running.rs` reads the machine. The
backbone is the process list — an agent in a terminal tab is no less running
than one in a pane, and a fleet view that sees half the fleet is worse than
none. tmux only adds an *address*, and with it the two things a bare pid
cannot do: jump there, and type into it.

The state signal is silence, and it has two clocks. A pane's
`#{window_activity}` moves when the TUI redraws; an agent's session file gets
appended to as it works and not at all while it waits. The second needs no
terminal, which is why it is the one that generalises — and it is why
`sessions.rs` records each session's `file`.

But silence alone lies, in the expensive direction: an agent three minutes
into a build is as quiet as one asking a question, and a badge that cries
wolf is worth less than none. So the clocks are the tiebreak and the
conversation is the evidence. `last_turn()` reads the tail of the session
file — an assistant turn ending in `tool_use` is *acting*, one ending in
prose has handed back — and in `classify` mid-turn beats any clock. Read only
the last 64KB: one tool result holding a large file can be tens of kilobytes
on its own, so a small window would miss the last complete line.

Cost splits the work in two, and the split is the design. *Finding* the fleet
is ~95ms (`ps` with full command lines, one bulk `lsof` for directories,
tmux) and is cached. Deciding what each run is *doing* is a `stat` and a
`kill(pid, 0)` per row — syscalls, not subprocesses — so it runs live on
every gather. A cached state is a state that was true a minute ago, which for
this view is worse than none.

Three traps, each already paid for. A pane reports the pid of its *root*
process, so an agent started by typing `claude` at that pane's shell is a
child; matching pids alone finds none of them and lists every one twice.
`finish` dedupes on `(kind, cmd)`, so a run's `cmd` must differ per run or
two agents in the same project collapse into one row — precisely the case
this source exists for. And batch runs (`claude -p`, `codex exec`) keep no
conversation file, so silence says nothing about them; they are never
reported as stuck.

## The bus

`bus.rs` is the other half of the fleet view, and the reason this is not just
a launcher. `running.rs` detects that an agent has gone quiet *from outside*;
the bus lets it say so itself. Four verbs an agent runs from its own shell —
`ask` (blocks on stdout until a human answers), `tell`, `say` (to another
agent), `inbox` — plus `--json` on every listing so an agent reads fields
rather than a table.

**Identity is discovered, never declared.** `$TMUX_PANE` is a reply address
every pane exports for free, `$PWD` is the project, and the enclosing agent
comes from climbing the process tree — an agent's tool call runs `sh -c`, so
the agent is a grandparent, not a parent. That is why the interface is four
words with no configuration.

Four things are pinned by tests, each already a bug. `Kind::Msg` sits at 1100
because frecency adds up to 60 and a ten-point band was not a band at all —
a claude row picked daily floated above a question blocked on you. `resolve`
must not widen an exact project name into a substring match, and `say`
refuses on anything but exactly one hit: a message in the wrong conversation
reads as the human's own words. The flag split stops at the first non-flag
word, so a question containing `--no-verify` keeps it. And a question is
never `run` — it is an English sentence.

Delivery to a pane *does* press Enter, unlike everything else in this
codebase, because the sender is another agent and there is nobody to press it
for them. The line is attributed (`[via prelude, from …]`) so the receiver
does not read a peer's message as its owner's.

The fleet is also reachable without the launcher, through `fleet.rs`:
`prelude fleet` (the list as text, re-finding identities inline because an
explicit call wants the truth now), `prelude fleet --status` (one line for a
tmux status bar — cached identities only, it runs every few seconds), and
`prelude watch` (a daemon that posts a notification on the working→waiting
edge). The two decisions that matter — what the status line says, and when
a notification fires — are pure functions pinned by tests: the bar is empty
when there is nothing to say, and a run is announced once per stop,
including a run first seen already quiet.

MCP status is asked of each agent (`claude mcp list`, `codex mcp list
--json`), never read from config files — the config misses claude.ai-hosted
servers entirely and cannot report whether anything works.

Each agent has its own invocation syntax. `opencode` needs a subcommand
where the others take a prompt positionally; `codex exec` refuses to run
outside a git repository. See `AGENTS` in `sources/sessions.rs`.

Copying a skill between agents is the only place Prelude writes to a user's
files. It stays behind an explicit action, never a default, and never
overwrites.

**Borrowing is the lighter half of that, and the one to reach for first.**
Every agent has a flag for taking a capability it does not own, for one run
only — see the table in `lend.rs`. Three of the eight pairings have no such
flag, and those offer nothing rather than a command that fails after the
launcher has closed. Borrowing writes only inside Prelude's own cache: the
claude plugin shim symlinks the owner's skill rather than copying it, so
editing the original is enough.

Mid-conversation there is a third route that needs no flag at all: hand the
agent the skill's own file. `pane_current_command` says which agent a pane
runs (a Claude Code pane reports `claude`, not `node`), so a skill row gives
its owner `/name` and everyone else `Read <path> and follow it.` — the two
swap between Enter and the secondary. The host agent is ambient
(`defaults::host_agent`) and travels to the per-keystroke footer helper on
its argv, like the column widths; it is never asked for per keystroke.

Two traps are pinned by tests. `--mcp-config` is *variadic*, so written with
a space it swallows a prompt typed after it as another config file — always
the `=` form. And a server's env block routinely holds an API key, so it is
never inlined: claude gets a 0600 file in the cache, and codex, which has
only an inline form, is told no. That is `secrets.rs`'s rule applied to the
one path that hands user data to a command line.
