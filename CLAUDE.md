# Working on Prelude

A general launcher in the terminal. Rust, built on fzf, macOS only.
`README.md` describes what it does; this file is what a new session needs to
avoid repeating mistakes already made.

## Build and check

```sh
cargo build --release
cargo test                 # 170 tests
cargo clippy --release     # expected warning-free
./target/release/prelude bench     # gather must stay under 40ms
./target/release/prelude _dump       # empty-query agent home
./target/release/prelude _dump-root  # searchable root commands
./target/release/prelude _dump-all   # complete catalogue behind scopes
```

`_surface`, `_panel`, `_dump`, `_dump-root`, `_dump-all`, `_footer`, `_focus`, `_preview`,
`_bind`, `_dynamic`, `_copy`, `_runhere`, `_ask`, `_enter`, `_refresh`, `_copy-skill`, `_lend-skill`,
`_lend-mcp`, `_actions` are internal entry points. They
exist so behaviour can be tested without standing up a terminal — use them
rather than trying to drive fzf.

`PRELUDE_TO_CLIPBOARD=1` in front of any of them renders the panel's surface
rather than the prompt's, which is how the two label sets and the two action
lists are compared without pressing the chord:

```sh
LINE=$(prelude _dump | head -1)
prelude _footer "$LINE"                        # Insert into prompt
PRELUDE_TO_CLIPBOARD=1 prelude _footer "$LINE" # Copy the command
PRELUDE_TO_CLIPBOARD=1 prelude _actions "$LINE"
```

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

## The launcher panel

`docs/GLOBAL-HOTKEY.md` is the acceptance record. The global launcher is a
**Ghostty quick terminal**: a hidden, dedicated Ghostty instance configured by
`~/.config/prelude/quick-terminal.ghostty`, hosting one long-lived
`prelude _panel` loop. Ghostty registers the chord itself with a `global:`
keybind, so nothing of Prelude's runs when the key is pressed. On macOS that
binding is an Accessibility event tap: installation opens the exact permission
pane and `global status` trusts Ghostty's registration log, never mere process
existence.

**A press reveals; it never creates.** The old design built a terminal on every
press — a new application instance, a window, a login shell — 373ms of
construction, torn down afterwards, including when the answer was a file that
never needed a terminal. Every bug in launch and teardown came from that. There
is now nothing to launch and nothing to strand: the loop outlives every press
and the panel is shown and hidden.

Four config lines are load-bearing and each was found the hard way.
`macos-hidden = always` keeps the launcher out of the Dock and the app switcher
— Ghostty documents it for exactly this. `initial-window = false` keeps the
instance at rest with no window and no shell. `window-save-state = never` stops
a second instance restoring the last session's windows, so one press does not
arrive with a crowd. And `unconsumed:escape=toggle_quick_terminal` is the
dismissal: it hides the panel *and* passes Escape to fzf, so the launcher
resets behind a hidden panel and the next press is a reveal, not a rebuild.

One instance or none. Two panels both claim the chord and the loser still
answers a toggle, so the panel appears to open every other press. The
LaunchAgent runs Ghostty itself as a `KeepAlive` job: this is both the one
start and the supervision. Never return it to an `/usr/bin/open` job — that
process exits immediately, so launchd cannot repair a panel that later dies.

**`cargo build` does not change the running panel.** The loop was started at
login and executes whatever the binary held then. Each press *does* spawn the
new binary as its child — so fzf, the rows and the footer all update, and it
looks like the build took — but the delivery decision is made by the parent,
which is still old. That failure mode reads as "the change did nothing", and
it lies in the most convincing possible way. Run `prelude global stop && prelude
global start` after any build you intend to press the key against. The Ghostty
process remains visible to launchd; `initial-window = false` means it owns no
surface and starts no `prelude _panel` child until the first press.

**The launcher is not the destination, and it never builds one.** A command
picked from the panel goes on the **clipboard**, and the panel stands down.
Objects — files, folders, URLs, applications — still go straight to Launch
Services and need no terminal at all.

Everything that used to sit between those two sentences is gone, and it is
worth knowing what, because each piece looked reasonable on its own.
`panel.rs` read the frontmost application through `lsappinfo` and either typed
the command into the tmux pane you were looking at or built a window to leave
it in; a window meant a login shell, which meant the command could not travel
on its argument list (a history entry can hold a token and `ps` is readable by
anything on the machine), which meant a 0600 preload file and a zle hook in
`prelude init zsh` to read it. Four mechanisms, all in service of a launcher
guessing which prompt you meant — and it guessed wrong in both directions: the
window arrived in a configured directory rather than the one you were working
in, and the pane was whichever one tmux considered current, which on a machine
with old sessions lying around is one nobody has looked at for days. A command
delivered out of sight is indistinguishable from a launcher that did nothing.

`^V` is not a worse answer than either. It is the only one that does not
require Prelude to know something it cannot know.

Two consequences to keep. The panel must **close** after copying — nothing
else took focus, so autohide has nothing to react to and the panel would sit
on top of whatever you meant to paste into; `run()` returns and Ghostty tears
down the surface, which the next press rebuilds. And `INSERT` and `RUN`
**collapse** here: the difference between them is whether a shell presses
Enter for you, and the clipboard cannot, so anything whose only alternative
was "and run it" must not offer that row.

This binds the global launcher to Ghostty; no other macOS terminal has a quick
terminal.

**Opening Ghostty must not open Prelude.** The hidden panel is a real Ghostty
process with Ghostty's bundle identity. After the quick terminal was active,
macOS can deliver a later ordinary Ghostty launch to that process. Its command
must therefore be `prelude _surface`, never `_panel` directly:
`GHOSTTY_QUICK_TERMINAL=1` enters the panel, while an unmarked surface opens a
new exact-path Ghostty instance and exits. Keep
`abnormal-command-exit-runtime = 0` so that intentional routing exit closes
without an error surface. Application rows also carry `open -na Ghostty` and
pass `-n` with the exact app path. Do not apply `-n` to every application —
ordinary apps should keep macOS's reuse semantics.

## Agent Control Plane work

`agent.rs` is the one registry for built-in Agent identity, invocation,
settings paths and support flags. Session, Run, Control, action and borrowing
code consumes it; do not add another supported-Agent list or advertise an
action by spelling an Agent name in a second module. CLI-specific parsers still
belong beside the output they parse.

Favorites are Prelude preferences for Agent, Skill and MCP object keys. They
never carry paths, commands or definitions, never write native Agent data, and
only promote inside the existing Kind band. Tests address a temporary
preference path and must not read the person's real `favorites.txt`.

Skill/MCP archive state reuses those stable object identities but lives as
atomic 0600 metadata at `$XDG_DATA_HOME/prelude/capabilities.json`. It is a
Prelude view overlay: never move a Skill, edit/disable an MCP definition, clear
a Favorite, or retain a path/command/definition in it. Archived capabilities
stay in the complete gathered snapshot so `skill:is:archived` and
`mcp:is:archived` can restore them, but leave Home, root search, `a:`, default
capability scopes, slash invocation and Session borrow pickers. Per-keystroke
rules read the decorated `archived` field and never the metadata file.

`docs/AGENT-CONTROL-PLANE.md` is the implementation source of truth. Read it
before changing Agent, Run, Session, Task, Skill, MCP, Config, Home, messaging
or Agent doctor behaviour. Work from its current milestone, update acceptance
criteria and progress evidence in the same commit, and never call a milestone
complete while it still has unchecked criteria. A conversation summary is not
a substitute for updating that file.

## Prelude's own settings

`settings.rs` is the one place that answers "what is this preference set to".
Every value shown in the launcher, printed by `prelude settings`, and obeyed at
runtime comes through it, so a row and the behaviour cannot disagree.

Six settings own a file each and are written by the code that owns that file —
`roots.txt`, `global.toml`, `open.toml`, `snippets.toml`, `quicklinks.toml`,
`favorites.txt`. Do not write any of them from here; a chord goes through
`global::set_hotkey` so it gets the same validation, conflict check and panel
restart the CLI performs. The four that had only an environment variable get
`settings.toml`, and **the variable still wins** — a variable is a
per-invocation instruction and a file is a standing one, so the narrower has to
be able to override the broader. `toggle` says so when it is writing a file the
environment is already overriding, because a setting that visibly does nothing
is worse than one that refuses.

The effective value goes **on the row**, together with whether it is a default,
a saved value, or an environment override. A setting you cannot see the value
or source of is one you change by trial. Invalid file or environment values are
ignored at runtime, called out on the row and by `prelude settings check`, and
`set` validates before writing; resetting removes only the scalar override and
never a list file. `^K` holds the mutations and never repeats Enter — a test
walks every setting and asserts it, because each of these rows has an obvious
primary and listing it twice is the natural mistake. Keep the settings form in
its explicit rank order rather than letting frecency scatter related controls.

A pasted path is tried literally first and then unescaped, because a path with
a space in it reaches the clipboard already escaped — shell completion and
dragging a folder into a terminal both write `Mobile\ Documents`, and
`com~apple~CloudDocs` picks up backslashes too. Reading only the literal
answered `is not there` about a folder plainly on screen, which is the worst
available wording: it names the one thing that was not wrong. Keep the literal
reading first so a directory whose name really contains a backslash is not
taken away by the convenience. `settings::readings_of` is shared by settings
and computed local-path rows so the two surfaces cannot disagree. Path intent
is lexical in `is_special`; only `dynamic_rows_with` touches the filesystem.
An existing absolute, `~/`, `./`, `../`, `file:///` or slash-bearing relative
path becomes a File, Folder or Application object, with ordinary Quick Look and
Quicklink actions. Bare `/` remains the Skill browser.

`prelude settings add-root` and its neighbours exist so the guards can be
exercised without standing up fzf, which is the same reason `_dump` and the
agent verbs exist. **Adding a root goes through `paths::is_protected`.** That
is not tidiness: indexing `~` is seven levels of `fd` through `~/Library`,
which macOS protects as other applications' data, and the dialog that results
names the terminal rather than Prelude.

Search roots and the file index are separate rows: root edits are immediate,
while rebuilding is an explicit operation that may take a minute. Nothing here
may read the file index to draw a row. Its size is recorded beside it when it
is built; an index from an older build has no record, so the first reader counts
it once and writes the number down rather than re-reading a megabyte on every
gather. Finder tags are part of that explicit rebuild: one JXA process asks
Foundation for `NSURLTagNamesKey`, secret-looking or unbounded names are
rejected, and the bounded names are stored beside the path. `f:` may match them
normally or exclusively with `tag:`; it must never launch `mdfind`, `mdls` or
another metadata process on a keystroke. Settings gather may use file checks
and stats, never `pgrep` or another subprocess; live panel status belongs to
explicit `prelude global status`.

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

**One rule, two surfaces.** `defaults::Surface` is `Prompt` (the zsh widget)
or `Clipboard` (the panel). It changes no behaviour — Enter does the same
thing in both — only what may honestly be *said*: "Insert into prompt" is a
lie about a panel that copies, so the labels switch, and the rows whose only
content was "and submit it" disappear where nothing can submit. It is read
from `PRELUDE_TO_CLIPBOARD` at each entry point rather than threaded down from
one, because fzf's footer and preview helpers are separate processes and
inherit the environment for free — but every rule below `surface()` takes it
as a parameter, so the decisions stay testable without a process to read it
out of. Do not reintroduce an env read inside a rule; a test that has to set a
variable is a test that races every other test in the binary.

**Two general action keys: Enter is primary, Ctrl+K is the panel. Ctrl+P is a
mode.** Graphical launchers often put a secondary action on its own key;
Prelude keeps that action but not the key: where useful, it is the first selectable row
below Enter's non-selectable header. Neither action is a fixed verb; both are
per-item, and they are opposites — where one acts, the other hands you text. A
test asserts they never coincide. The global panel has one narrower object
shortcut, not a generic secondary: Cmd+Enter on File/Find opens the containing
directory. Ctrl+P is different: Quick Look replaces
the result area until Ctrl+P is pressed again, without selecting or acting on
anything. The preview is hidden by default and never owns a permanent column.
Clipboard rows are the deliberate contextual exception: while any real row is
focused inside `c:`, the otherwise-unused right side becomes a 55% preview —
pixels for images, full content and metadata for everything else — and it
disappears only when leaving the scope. The preview label is the state marker;
do not add a second focus helper or a subprocess to decide it. Hiding must put
`hidden` in `change-preview-window` itself — changing the window after
`hide-preview` shows it again, which is how text rows acquired a 99%-high
horizontal pane.

Ctrl, because only Ctrl reliably arrives. Unless told otherwise macOS spends
Option on composing characters, so Option+K types `˚` into the search box and
Option+Enter comes through as a bare Enter — running the *primary* action,
silently. Cmd never reaches a terminal application. The containing-directory
shortcut is honest only because Prelude owns the dedicated panel's Ghostty
config: `cmd+enter=text:\\x07` translates it to private Ctrl+G, `EXPECT`
includes `ctrl-g`, and the footer advertises Cmd+Enter only when
`PRELUDE_FULL_SURFACE` and File/Find are both true. Never claim it on the zsh
widget or alter the person's ordinary terminal config.

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

**Commands are handed over, objects act.** Inserting a file path when you
wanted to read the file is a step backwards; opening a file is harmless in a
way that `kill $(lsof -ti tcp:3000)` is not. This does not vary by surface —
a file opens from the panel exactly as it does from the prompt, because
Launch Services is the destination either way. `^K` still offers the text.

**A container is not a project, and `$HOME` is the one that bites.**
`project::root` walks up for a marker and, finding none, used to take the
current directory at its word. The global panel stands in `$HOME`, so "the
files in this project" became `fd --max-depth 6` over the entire home
directory on every open — six levels into `~/Library`, which macOS protects as
other applications' data. The symptom named neither Prelude nor the source: a
TCC panel saying *"Ghostty would like to access data from other apps"*, because
`fd` ran under the terminal and the terminal is who gets asked. The unmarked
fallback is deliberate and stays — a scratch folder of notes is its own
project — but it goes through `paths::is_protected`, which already draws this
line for the Trash and draws it in the same place. Anything that walks a
directory the person did not choose belongs behind that check.

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
the things this launcher manages — a question an agent is blocked on, the
Agents themselves, what they are running, their Skills, their MCP servers, and
the newest `sessions::IN_MAIN_LIST` conversations. Ordering is `by_rank` like
everywhere else, so the kind bands do the work and the home has no second
ordering rule of its own.

That was briefly not so, and the correction is worth keeping. The home was made
an *attention list*: healthy Skills and servers were pushed into `/name` and
`mcp:` so only exceptions — a server that had stopped answering, a Skill whose
copies had drifted — reached the empty query. It reads well as a principle and
was wrong in front of a person. A launcher you open to see what you have is not
improved by hiding what you have, and the panel went quiet exactly when nothing
was broken, which is most of the time. It came from an acceptance criterion in
`docs/AGENT-CONTROL-PLANE.md`, implemented faithfully and only then looked at.
Sessions are on the home now, which they never were before. An ordinary query
searches all of that plus Search commands and fixed Quicklinks —
never the thousands of files, history entries, apps, clipboard rows and
`$PATH` commands underneath. Exact `f` shows one Search Files command; `f:`
opens its results. `:` lists every scope, and clearing restores the home. Keep
`home.txt`, the root-command `list.txt` and the complete scoped item cache on
one gathered snapshot and one layout — a scope must never gather a source on
each keystroke, and `a:waiting`, `a:failed`, `a:claude Prelude`, `a:using X`
and `a:without X` all filter that one snapshot with no Agent CLI behind them.
`a:`'s values are quote-aware, like `s:`'s: a Skill name is an identifier but
an MCP server's name is whatever its owner called it, and `claude.ai Google
Drive` split on whitespace becomes a different question with an empty answer.
`a:` excludes Session because `s:` owns the hundreds of old conversations.

`/` has the two states a search provider has. An *incomplete* name browses —
`/cnipa-oo` lists the Skill rows it matches — and a complete one is an
invocation, so `/cnipa-ooa` is the single row that runs it and the Skill row
is gone. Nothing here shows two rows for one intent, and MCP servers are not
on `/` at all: they are not invoked by name, and `mcp:` is their scope.

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
and computed rows land in a different column. Ordinary Quick Look is hidden
and replaces the result area, so rows are always measured against the full
terminal width. The contextual `c:` image pane still uses those full-width
rows and lets fzf clip the left list; do not introduce a second layout for that
transient pane. `f:` is the explicit stable exception to the catalogue table:
it is always filename, kind, parent path, with a width-derived (not
result-derived) filename column so filtering cannot make it jump. The parent
never repeats the filename and loses its middle as `...` before either useful
end is discarded.

**Clipboard is typeful and strictly chronological.** `pbpaste` sees only
text, so `clipd` keeps one sleeping JXA/AppKit process and watches
`NSPasteboard.changeCount`. It preserves `NSFilenamesPboardType` lists and
PNG/TIFF data rather than flattening them into strings. Clipboard timestamps
are source ranks at a scale wider than the whole frecency bonus: selecting an
old clipping must never move it above something copied later. Image payloads
are private, bounded, and removed when their history rows age out. Some
screenshot tools bump `NSPasteboard.changeCount` several times while publishing
one image; clipd v3 records a private content fingerprint, migrates old rows
behind the daemon boundary, keeps only the newest byte-identical occurrence and
removes superseded payloads. Never hash those images on gather. A Ghostty or
Kitty preview uses the native `t=f` path transfer before Chafa, with one fixed
Prelude image id deleted before every render; this keeps an arrow press to a
path-sized terminal message and stops graphics placements overlapping after
fzf clears its text cells. A placement supplies only its limiting `c` or `r`,
never both — Kitty stretches into a box when both are given, while one lets the
terminal derive the other dimension with the original aspect ratio. Chafa is
the fallback and replaces the preview helper with `exec`, so cancellation kills
the renderer rather than orphaning it.
Bump the pidfile protocol version whenever an old daemon cannot produce the new
record format, or upgrades will leave obsolete watchers alive indefinitely.

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

**Running is not the same as recorded, and no terminal is the requirement.**
`sessions.rs` reads conversation files; `running.rs` reads the machine. The
backbone is the process list — an agent in a terminal tab is no less running
than one anywhere else, and a fleet view that sees half the fleet is worse
than none.

It once asked tmux as well, which bought an *address* for the subset of runs
that had a pane, and with it the two things a bare pid cannot do: jump there,
and type into it. Both are gone. Every run now answers the same questions, and
a view that is sharper about some of its rows than others is harder to read
than one that treats them alike.

The state signal is silence, and there is one clock: an agent's session file
gets appended to as it works and not at all while it waits. It needs no
terminal, which is why it is the one that survived — and it is why
`sessions.rs` records each session's `file`. (A pane's `#{window_activity}`
was the second, consulted alongside it where there was one.)

But silence alone lies, in the expensive direction: an agent three minutes
into a build is as quiet as one asking a question, and a badge that cries
wolf is worth less than none. So the clocks are the tiebreak and the
conversation is the evidence. `last_turn()` reads the tail of the session
file — an assistant turn ending in `tool_use` is *acting*, one ending in
prose has handed back — and in `classify` mid-turn beats any clock. Read only
the last 64KB: one tool result holding a large file can be tens of kilobytes
on its own, so a small window would miss the last complete line.

Cost splits the work in two, and the split is the design. *Finding* the fleet
(`ps` with full command lines, one bulk `lsof` for directories) is cached. Deciding what each run is *doing* is a `stat` and a
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
extracting the hint — either can hold credentials. An active Session hands over
its project instead of starting a competing resume. `gather` writes the
derived `sessions-linked` snapshot only when its bytes change; `s:` filters
that file. Never repeat the join in the per-keystroke helper.

**Session metadata is an overlay, never the conversation authority.** Local
names, pins and archive state live in the 0600 XDG data file
`sessions.json`; they decorate stable Session ids after native Claude, Codex
and pi files have been read. Archive hides a row and touches no Agent file.
Pinned rank is source rank and must be recorded in `data` just like recency.
Forking uses each native CLI (`claude --fork-session`, `codex fork`, `pi
--fork`) and is absent when no syntax is known; do not fake it with a fresh
Session. The explicit raw export stays under Prelude's 0700 data directory.

**A doctor reports; `--repair` re-verifies before it acts.** `doctor.rs`
offers exactly two repairs, both on Prelude's own records, each confirmed
separately with Cancel first. A report is printed, read, and then answered one
question at a time, so minutes pass between seeing something and acting on it
— and staging names are deterministic (`borrow/<server>.json`,
`borrow/<skill>/`), so a borrow staged *while the confirmation is on screen*
wears the name the question is about. Every `Repair` therefore carries the
evidence its finding was made on — mtime and mode for a Trash — and declines
when that no longer matches, the same rule Session trash follows when it
re-finds the fleet rather than trusting the launcher's snapshot. A
broken symlink under `borrow/` is reported without a repair: `paths::trash`
gates on `exists()`, which follows the link, so the offer could only fail at
the moment somebody said yes to it. And a staging root that will not open is
not a staging root that is absent — reporting the second as the first is the
diagnostic calling a place it could not look into empty.

Trashing a Session follows a stricter boundary than ordinary file trash: it
is offered only while inactive, canonicalizes the path, requires a `.jsonl`
below one of the known native Session roots, confirms with Cancel first, and
uses `paths::trash`. Before moving it, re-find the fleet rather than trusting
the launcher snapshot; an exact edge or even an ambiguous same-Agent,
same-project Run refuses the move. Never broaden this to an arbitrary path
carried by a Session-shaped Item. Metadata is deliberately retained after trash so a file
restored from Finder recovers its name and pin.

Three traps, each already paid for. `finish` dedupes on `(kind, cmd)`, so a
run's `cmd` must differ per run or two agents in the same project collapse
into one row — precisely the case this source exists for; `kill <pid>` is
what makes it unique now that no address does. Batch runs (`claude -p`, `codex exec`) keep no
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

**Identity is discovered, never declared.** `$PWD` is the project, and the
enclosing agent comes from climbing the process tree — an agent's tool call
runs `sh -c`, so the agent is a grandparent, not a parent. That is why the
interface is four words with no configuration.

There used to be a third signal, `$TMUX_PANE`, and it was the only one that
could carry a message *to* an agent rather than merely label one it came from,
because a pane can be typed into. So `$PWD` is now the whole of an inbox
address, and two agents in one directory share one — which is exactly why
`say` refusing an ambiguous target matters more than it used to.

Four things are pinned by tests, each already a bug. `Kind::Msg` sits at 1010
because a question explicitly waiting on a person must lead the 1000-point
Agent band; `cache::by_rank` now compares kind before frecency, so use records
cannot lift an agent above it. `resolve` must not widen an exact project name
into a substring match, and `say`
refuses on anything but exactly one hit: a message in the wrong conversation
reads as the human's own words. The flag split stops at the first non-flag
word, so a question containing `--no-verify` keeps it. And a question is
never `run` — it is an English sentence.

Delivery is always to the inbox, and `bus::leave` is the one door — `prelude
say` and the launcher's "Leave it a message…" both go through it, so the two
cannot disagree about what a message is. The sender is named where the inbox
is rendered rather than baked into the stored text, so the record keeps what
was written and the receiver still cannot read a peer's message as its
owner's.

The fleet is also reachable without the launcher, through `fleet.rs`:
`prelude fleet` (the list as text, re-finding identities inline because an
explicit call wants the truth now), `prelude fleet --status` (one line for a
status bar — cached identities only, it runs every few seconds), and
`prelude watch` (a daemon that posts a notification on the working→waiting
edge). The two decisions that matter — what the status line says, and when
a notification fires — are pure functions pinned by tests: the bar is empty
when there is nothing to say, and a run is announced once per stop,
including a run first seen already quiet.

MCP status is asked of each agent (`claude mcp list`, `codex mcp list
--json`), never read from config files — the config misses claude.ai-hosted
servers entirely and cannot report whether anything works. Complete MCP
definitions must never survive in an Item or cache: command arguments, env,
headers and even URLs can hold credentials. Cache only a redacted semantic
fingerprint and display summary; `lend::resolve` asks the owner CLI again on
an explicit action. Account-hosted servers with no local definition are
`portable=false` and must expose no borrow/install target. `privacy_migrations`
scrubs old MCP and derived search caches once.

Actual MCP tools come from `mcp_tools.rs`, not from pretending `mcp get` is a
tool list. The slow source starts enabled stdio servers, performs initialize
plus paginated `tools/list`, keeps only bounded credential-filtered names and
descriptions, drains but never retains stderr, and kills the child. HTTP and
hosted servers are explicitly `unsupported` when Prelude has no owner auth;
that is different from a successful empty list. Tool inventory has a five
minute TTL and never runs per key. Current Claude/Codex help has no
server-level Enable/Disable verb, so no such action is offered. Prelude's 0600 `borrow/` staging files are the deliberate
exception and are never search input.

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

**Capability identity belongs behind the cache tier.** `capability.rs`
fingerprints every effective Skill file except VCS metadata, treating
symlinks as links rather than following them. Credential-like paths and lines
contribute only a redaction marker. Full-tree hashing runs in the
`skill-hashes` background source; unchanged trees use a recursive metadata
stamp. Never move this work into `skills_with` or a per-keystroke helper.

Copies with one known public fingerprint are identical; more than one are
divergent; a missing or unreadable hash is unknown. If redacted private lines
exist across copies, equality is `private-unknown`, never identical. Replacement must show `diff -ru`,
re-hash both copies around that comparison, reject changed or sensitive
sources, move the target to Trash, and only then copy. `copy_tree` excludes
VCS/cache metadata but not runtime dependencies. MCP matrices use the same
principle with redacted public fingerprints; incomparable source formats say
so instead of claiming equality.

**Borrowing is the lighter half of that, and the one to reach for first.**
Every agent has a flag for taking a capability it does not own, for one run
only — see the table in `lend.rs`. Three of the eight pairings have no such
flag, and those offer nothing rather than a command that fails after the
launcher has closed. Borrowing writes only inside Prelude's own cache: the
claude plugin shim symlinks the owner's skill rather than copying it, so
editing the original is enough.

There is a third route that needs no flag at all: hand the agent the skill's
own file. A skill row therefore carries both bare forms in `^K` —
`Insert the slash command` (`/name`) and `Point an agent at its file`
(`Read <path> and follow it.`) — named, rather than one of them chosen for
you. Prelude used to choose, by asking `pane_current_command` which agent the
pane under the popup was running and handing its owner `/name` and everyone
else the file. Nothing can answer that now; the failure it avoided is still
real, because `/name` at an agent that lacks the skill is prose that does
nothing, and does nothing *silently*.

Two traps are pinned by tests. `--mcp-config` is *variadic*, so written with
a space it swallows a prompt typed after it as another config file — always
the `=` form. And a server's env block routinely holds an API key, so it is
never inlined: claude gets a 0600 file in the cache, and codex, which has
only an inline form, is told no. That is `secrets.rs`'s rule applied to the
one path that hands user data to a command line.
