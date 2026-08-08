# Working on Prelude

A Raycast-style launcher for the terminal. Rust, built on fzf, macOS only.
`README.md` describes what it does; this file is what a new session needs to
avoid repeating mistakes already made.

## Build and check

```sh
cargo build --release
cargo test                 # 50 tests
cargo clippy --release     # expected warning-free
./target/release/prelude bench     # gather must stay under 40ms
./target/release/prelude _dump       # empty-query agent home
./target/release/prelude _dump-root  # searchable root commands
./target/release/prelude _dump-all   # complete catalogue behind scopes
```

`_dump`, `_dump-root`, `_dump-all`, `_footer`, `_preview`, `_bind`, `_dynamic`,
`_copy`, `_runhere`, `_ask`, `_enter`, `_refresh`, `_copy-skill`, `_lend-skill`,
`_lend-mcp`, `_actions` are internal entry points. They
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
session. Be sceptical of new dependencies. `bench` must stay under 40ms; it
sits around 15 on a quiet machine, and every millisecond of that is a
subprocess. Per keystroke the binary costs about 2ms, of which 1.6 is the
kernel's fork and exec — there is nothing left to win there, so measure
`gather` and leave the helpers alone.

**A launch costs whatever its slowest subprocess costs.** The `FAST` sources
run on threads and are waited for together, so the floor is the slowest one
and the local work underneath it is free. That is the only shape worth
optimising for: shaving a source that is not the slowest changes nothing,
and the profile has to be re-read after every win because the floor moves.

**Two action keys: Enter is primary, Ctrl+K is the panel. Ctrl+P is a mode.**
Raycast has a third action — the secondary on its own key — and Prelude keeps
that action but not the key: where useful, it is the first selectable row
below Enter's non-selectable header. Neither action is a fixed verb; both are
per-item, and they are opposites — where one acts, the other hands you text.
A test asserts they never coincide. Ctrl+P is different: Quick Look replaces
the result area until Ctrl+P is pressed again, without selecting or acting on
anything. The preview is hidden by default and never owns a permanent column.

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
that owns them rather than to `$EDITOR`, folders go to Finder, and URLs go
to the default browser. These external objects are passed directly to macOS
Launch Services — never emitted as `open ...` shell commands, never written
to the prompt or history. `openwith.rs` remembers file overrides per
extension and `^K` is where they are made — that panel is the settings
surface, not a second list of shortcuts.

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
the same file or folder means "open it" at a shell and "here is the path" in
an agent's input box. URLs are deliberately consistent across hosts: Enter
opens the browser, while `^K` offers `Insert URL` for a conversation.

**Sources degrade to nothing.** A source that fails, or finds nothing,
returns an empty list. Never blocks, never panics, never prints. Anything
slower than ~25ms belongs behind the cache tier. And a source that can see
it will find nothing should not pay to find that out: `docker ps` costs the
same 14ms whether or not a daemon answers, because the cost is the CLI
starting, so `containers` resolves the socket the daemon would listen on and
does not ask when it is not there. Being *unsure* costs a launch 14ms —
anything that cannot be resolved falls through to the subprocess, because
the failure to avoid is a missing row, not a slow one.

**Never index or transmit credentials.** `secrets.rs` filters history and
text/file clipboard records. Route any new source that reads user data through
it. Clipboard image bytes stay as private 0600 files under Prelude's own data
directory; their pixels are not OCRed, indexed or transmitted.

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

**The home and root commands are not the catalogue.** An empty query renders
only Msg, Agent, Run, Skill and MCP. An ordinary query searches those plus
Search commands and fixed Quicklinks — never the thousands of files, history
entries, apps, clipboard rows and `$PATH` commands underneath. Exact `f`
shows one Search Files command; `f:` opens its results. `:` lists every scope,
and clearing restores the home. Keep `home.txt`, the root-command `list.txt`
and the complete scoped item cache on one gathered snapshot and one layout —
a scope must never gather a source on each keystroke. `a:` excludes Session
because `s:` owns the hundreds of old conversations.

**A provider is a command until it has an argument.** Exact `g` and `google`
show one Search Google row; Enter changes the query to `g ` without closing
fzf, and only `g term` becomes a Link. Scope commands use the same
`complete-query` mode (`f` → `f:`). Do not represent an incomplete provider
as Link: its footer, actions and Enter would all claim a URL exists when it
does not.

**fzf matches against displayed text.** A row computed *from* the query can
never fuzzy-match it, so `is_special()` recognizes intent and must not
calculate, shell out or use the network — it runs on every keystroke. Exact
Quicklink aliases are the one tiny config lookup, because aliases outrank
fuzzy matches. `{}` in a binding is the *transformed* text, so bindings use
`{2}` to reach the payload.

**Layout must be computed once and passed down.** The per-keystroke helper
runs in a separate process. If both sides measure their own widths they drift,
and computed rows land in a different column. Quick Look is hidden and
replaces the result area instead of splitting it, so rows are always measured
against the full terminal width.

**Clipboard is typeful and strictly chronological.** `pbpaste` sees only
text, so `clipd` keeps one sleeping JXA/AppKit process and watches
`NSPasteboard.changeCount`. It preserves `NSFilenamesPboardType` lists and
PNG/TIFF data rather than flattening them into strings. Clipboard timestamps
are source ranks at a scale wider than the whole frecency bonus: selecting an
old clipping must never move it above something copied later. Image payloads
are private, bounded, and removed when their history rows age out. Bump the
pidfile protocol version whenever an old daemon cannot produce the new record
format, or upgrades will leave obsolete watchers alive indefinitely.

**Kind decides the band; frecency only orders things inside it.** The two
questions — *what kind of thing is this* and *how much do you use this one* —
are separate, and `cache::by_rank` compares them in that order. They used to
be added into one number, and the arithmetic could not hold: the agent
cluster spans 25 points (Agent 1000 → Config 975) while the bonus reached 60,
so a skill used twice that morning sat above `claude` itself and a config
file sat above a skill. Do not reintroduce a single total and do not try to
fix it by tuning the cap — the cap can only move the threshold, and any later
edit to a priority silently re-opens the hole. A test walks every pair of
kinds with the lower band given an absurd score.

**A source ranks its own kind, through `Item::rank`.** The launcher's
frecency cannot know which skill you actually invoke or which session is the
newest; the source can, and had nowhere to say so. Skills were the visible
case: the row printed `used 8×` and sorted alphabetically, so a skill invoked
eight times sat below four never touched. Skills now rank by invocation count
(`PER_USE` 100, wide enough that one real use clears the 60-point frecency
cap, because clicking a skill row usually means reading its description);
sessions rank by recency (`RECENCY_WEIGHT` 200, because for a conversation
recency *is* the question). `rank` must be written into `data`, not only
added to `score` — `read_cached` rebuilds the score from kind plus rank, so a
rank that is applied but not recorded vanishes the next time the cache is
read, and sessions are always read back from it.

**Column widths are shared across all kinds and taken at a percentile.**
Per-kind widths only align within a kind, so the dots scatter. And one
outlier — a session in a deep iCloud path, 127 columns — will set the column
for two thousand rows if you take the maximum. The kind is the fifth column,
before the final free-text field; descriptions and run subjects own the
flexible sixth column at the right edge.

**Drain child output.** `exec::run` reads stdout on a helper thread because
waiting on a process while its pipe fills deadlocks past 64KB; `ps -Ao`
emits ~74KB. Keep stderr too: discarding it turned every agent failure into
a permanent, silent "asking…".

**Asking `ps` for `comm=` doubles the cost of the process list**, because the
kernel has to be asked for each process's argument block one at a time to
recover argv[0] — 21ms against 10ms for eight hundred and fifty processes,
and for years that was the single largest cost in a launch. It was being
paid for all of them to display two dozen: `procs` takes the fields that come
free out of the process table and reads argv[0] itself, through the same
sysctl, for the handful of rows that survive the filter.

Three things about that are load-bearing. It must be **argv[0], not
`proc_pidpath`** — an agent CLI is routinely a script, so the executable
path reports `pi` and `claude` as `node`, which are the two rows a launcher
for agents must not mislabel. The buffer must be **`KERN_ARGMAX`, not a
guess** — argc and the executable path are followed by *padding* before argv
begins, four kilobytes of it for a Chrome-style helper, and a buffer too
small to reach past it reads as a process with no arguments and silently
falls back. And `/bin/ps` is **setuid root** while we are not, so another
user's process answers EINVAL: those fall back to `proc_pidpath`, which is
readable for anything and is why the ordering cannot be swapped.

**A cache and the source that presents it must not both be put in the
list.** `finish` keeps the first of a duplicate pair, and the raw `fleet`
rows went in first — so every agent in the launcher showed a blank row while
the live state `running::live` had just computed was thrown away, and
`prelude fleet` was the only place the fleet worked. A cache with a presenter
in front of it belongs to the presenter.

**The gather deadline is measured from the start of a gather**, not from
where the local work happens to have finished. Measured from the end, the
real bound was that work *plus* the deadline — sixty milliseconds against a
budget of forty, and nothing said so.

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

A Run and a Session are related facts, not the same record. `control.rs`
builds the canonical Agent/Run/Session/Skill/MCP graph; launcher Items are
views over it. Run ids are `agent:pid:started`, Session ids are
`agent:native-id`, and both sides carry the edge. An explicit resume argument
is exact. Cwd-latest is allowed only when one run of that agent exists in the
directory; with two, mark it `ambiguous` rather than attaching both to the
same conversation. Never retain the full process command or prompt while
extracting the hint — either can hold credentials. An active Session jumps to
its pane, or hands over its project when no pane exists, instead of starting a
competing resume. `gather` writes the derived `sessions-linked` snapshot only
when its bytes change; `s:` filters that file. Never repeat the join or call
tmux in the per-keystroke helper.

**Session metadata is an overlay, never the conversation authority.** Local
names, pins and archive state live in the 0600 XDG data file
`sessions.json`; they decorate stable Session ids after native Claude, Codex
and pi files have been read. Archive hides a row and touches no Agent file.
Pinned rank is source rank and must be recorded in `data` just like recency.
Forking uses each native CLI (`claude --fork-session`, `codex fork`, `pi
--fork`) and is absent when no syntax is known; do not fake it with a fresh
Session. The explicit raw export stays under Prelude's 0700 data directory.

Trashing a Session follows a stricter boundary than ordinary file trash: it
is offered only while inactive, canonicalizes the path, requires a `.jsonl`
below one of the known native Session roots, confirms with Cancel first, and
uses `paths::trash`. Before moving it, re-find the fleet rather than trusting
the launcher snapshot; an exact edge or even an ambiguous same-Agent,
same-project Run refuses the move. Never broaden this to an arbitrary path
carried by a Session-shaped Item. Metadata is deliberately retained after trash so a file
restored from Finder recovers its name and pin.

Four traps, each already paid for. A pane reports the pid of its *root*
process, so an agent started by typing `claude` at that pane's shell is a
child; matching pids alone finds none of them and lists every one twice.
`finish` dedupes on `(kind, cmd)`, so a run's `cmd` must differ per run or
two agents in the same project collapse into one row — precisely the case
this source exists for. Batch runs (`claude -p`, `codex exec`) keep no
conversation file, so silence says nothing about them; they are never
reported as stuck. Finally, `etime` has day, hour and minute forms; parse all
of them before deriving a stable start time.

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

Four things are pinned by tests, each already a bug. `Kind::Msg` sits at 1010
because a question explicitly waiting on a person must lead the 1000-point
Agent band; `cache::by_rank` now compares kind before frecency, so use records
cannot lift an agent above it. `resolve` must not widen an exact project name
into a substring match, and `say`
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

Prelude writes its own config for explicit settings such as open-with rules
and Quicklinks (and versioned, one-time additions to their built-in search
providers). Quicklink writes are atomic and never overwrite a keyword;
generated blocks are marked so removing one preserves hand-written
`quicklinks.toml` comments and search templates byte-for-byte, and URLs that
look credential-bearing are refused rather than indexed. Outside Prelude's
config and caches, only explicit actions write user files: installing a skill
copy, exporting a raw Session, or moving a selected file, application, skill
copy or inactive native Session to the Trash. None is a default action.

Deleting a skill copy is built so that being wrong is survivable rather than
so that it cannot happen. It moves the directory to
`~/.Trash` — never `unlink`, never `remove_dir_all` — uniquifying the name
rather than overwriting what is already in there. `ui::confirm` puts Cancel
first so a stray Enter cancels. And `is_skill_dir` refuses anything that is
not a direct child of one of the five `skill_dirs`, compared after
`canonicalize` so `..` cannot dress a path up as a skill; a path that does
not resolve is refused rather than guessed at. A test walks the container
directory, `$HOME`, `/`, `/etc` and a traversal. The panel offers one entry
per agent that has a copy (`copies_of`), because a skill merged across four
agents is four directories behind one row — `dir` is only ever the first one
found, which is all borrowing needed and not enough to delete with.

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
