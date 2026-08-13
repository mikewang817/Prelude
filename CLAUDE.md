# Working on Prelude

A general launcher in the terminal. Rust, built on fzf, macOS only.
`README.md` describes what it does; this file is what a new session needs to
avoid repeating mistakes already made.

## Build and check

```sh
cargo build --release
cargo test                 # 170 tests
cargo clippy --release     # expected warning-free
./target/release/prelude bench     # p95 gather must stay under 40ms; non-zero when it does not
./target/release/prelude bench --json   # the same distribution, for a gate to record
./target/release/prelude bench --process # a new process per sample: the launch a person meets
./target/release/prelude _dump       # empty-query agent home
./target/release/prelude _dump-root  # searchable root commands
./target/release/prelude _dump-all   # complete catalogue behind scopes
```

`_surface`, `_panel`, `_dump`, `_dump-root`, `_dump-all`, `_footer`, `_focus`, `_preview`,
`_bind`, `_dynamic`, `_copy`, `_runhere`, `_ask`, `_enter`, `_hist`, `_tab`, `_refresh`,
`_refresh-path`, `_refresh-panel-after-update`, `_restart-panel-after-update`, `_copy-skill`,
`_rm-skill`, `_lend-skill`, `_lend-mcp`, `_actions` are internal entry points. They
exist so behaviour can be tested without standing up a terminal — use them
rather than trying to drive fzf.

All launcher entry points use the clipboard surface. The internal doors render
the same labels and action lists without needing a surface environment switch:

```sh
LINE=$(prelude _dump | head -1)
prelude _footer "$LINE"
prelude _actions "$LINE"
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

`docs/GLOBAL-HOTKEY.md` is the current implementation record. The global
launcher is a **Ghostty quick terminal**: a hidden, dedicated Ghostty instance
configured by `~/.config/prelude/quick-terminal.ghostty`, hosting one long-lived
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

**The bill for that is that the list is older than the press, and `refresh.rs`
is how it is paid.** `once()` starts the launcher *before* the reveal, so
`gather` runs when the previous interaction ended: dismiss at nine, press at
two, and you are reading the morning's machine. The clipping you copied a
minute ago is not in it and will not be until you dismiss and re-open once.
People report this as "it takes a while to show up", which is the wrong shape
— nothing was slow, the snapshot predates what they came for.

A thread therefore re-gathers behind the hidden panel and hands the new list to
fzf through its own `--listen` Unix socket. Three things keep it from being
felt, and each is the part to preserve if this is ever rewritten:

* **Ask fzf before touching it.** The refresh is sent as a `transform`, which
  fzf evaluates with the live `{q}` and `{n}`, so `_tick` can answer with *no
  action* when a query is typed or the cursor has moved. Only fzf knows that;
  guessing would mean a background reload moving somebody's selection, which
  is worse than the staleness it fixes. `may_redraw` treats anything it does
  not recognise as "in use".
* **Do nothing when nothing happened.** A tick is a handful of `stat`s over
  the files behind the list. A gather runs only on a changed mtime, or every
  `FORCE_AFTER` regardless, because a detached slow-source refresh lands in
  its cache with nobody being told.
* **Reuse the layout.** Widths and the title column come from the caller.
  Recomputing them here would put the per-keystroke helper's rows in a
  different column from the static ones — the same trap as everywhere else.

`^K` is a modal: fzf exits, the action panel runs, the list is rebuilt after.
The socket is gone for that whole time, so its absence skips a tick and never
ends the loop. Ending there left every session that had opened `^K` once
silently un-refreshed for the rest of its life.

Everything degrades to nothing: no socket, a failed bind, a POST into the
void, and the panel behaves exactly as it did before — one snapshot per
interaction. Only the panel gets this; the zsh widget is opened, used and
closed in seconds, and its snapshot is never older than the press that made
it.

Four config lines are load-bearing and each was found the hard way.
`macos-hidden = always` keeps the launcher out of the Dock and the app switcher
— Ghostty documents it for exactly this. `initial-window = false` keeps the
instance at rest with no window and no shell. `window-save-state = never` stops
a second instance restoring the last session's windows, so one press does not
arrive with a crowd. And `unconsumed:escape=toggle_quick_terminal` is the
dismissal: it hides the panel *and* passes Escape to fzf, so the launcher
resets behind a hidden panel and the next press is a reveal, not a rebuild.

One instance or none. Two panels both claim the chord and the loser still
answers a toggle, so the panel appears to open every other press. Ghostty must
be launched through Launch Services on macOS: executing
`Contents/MacOS/ghostty` directly leaves it as a foreground application even
with `macos-hidden = always`, which puts the dedicated instance in the Dock.
The LaunchAgent therefore owns `open -W -n -a Ghostty.app --args …` as a
`KeepAlive` job. `-W` is load-bearing: ordinary `open` exits immediately and
leaves launchd unable to repair a panel that later dies; waiting ties the
supervisor's lifetime to the exact new Ghostty instance.

**`cargo build` does not change the running panel.** The loop was started at
login and executes whatever the binary held then. Each press *does* spawn the
new binary as its child — so fzf, the rows and the footer all update, and it
looks like the build took — but the delivery decision is made by the parent,
which is still old. That failure mode reads as "the change did nothing", and
it lies in the most convincing possible way. Run `prelude global stop &&
./target/release/prelude global start` after any build you intend to press the
key against. The Ghostty
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
before changing Agent, Run, Session, Skill, MCP, Config, Home, messaging or
Agent doctor behaviour, and update its current behavior, support matrix and
limitations in the same commit. A conversation summary is not a substitute for
updating that file.

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

The effective value goes **on the row**; storage source, defaults, environment
overrides and backing paths belong in Details and the JSON/CLI surface unless
an override is actively preventing a change from taking effect. The panel is a
four-column form—Category, Setting, Current, Effect—grouped as Search, Launcher,
Behavior and Library. A setting you cannot see the value of is one you change
by trial, while a row led by `saved` or `default` exposes implementation instead
of intent. Invalid file or environment values are ignored at runtime, called
out visibly and by `prelude settings check`, and `set` validates before writing;
resetting removes only the scalar override and never a list file. `^K` holds the
advanced alternatives and never repeats Enter or either arrow — tests walk
every setting and assert its two specific direction labels, because each row
has an obvious primary and listing it twice is the natural mistake. Direction
semantics are consistent by shape: boolean left/right is off/on, ordered enum
is previous/next without wrapping, scalar is reset/change, collection is
remove/add, and status is inspect/repair. Collection Enter opens a real manager;
opening its backing file is an advanced escape hatch, not management. Keep the
settings form in its explicit rank order rather than letting frecency scatter
related controls. Settings rows are controls and always act on Enter;
`classic_enter` applies to payloads, not the screen needed to turn that
preference back off.

A pasted path is tried literally first and then unescaped, because a path with
a space in it reaches the clipboard already escaped — shell completion and
dragging a folder into a terminal both write `Mobile\ Documents`, and
`com~apple~CloudDocs` picks up backslashes too. Reading only the literal
answered `is not there` about a folder plainly on screen, which is the worst
available wording: it names the one thing that was not wrong. Keep the literal
reading first so a directory whose name really contains a backslash is not
taken away by the convenience. `settings::readings_of` is shared by settings
and computed local-path rows so launcher entry points cannot disagree. Path intent
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

Search folders and the file/folder index are separate rows. On Search folders,
`→` uses the native macOS chooser, `←` selects one to remove, and Enter opens
one manager with the same add/remove paths plus per-folder Show in Finder. Root
edits are immediate and
schedule a detached rebuild. A missing roots file receives onboarding defaults;
an existing empty file is authoritative and must not silently restore them.
The previous generation remains searchable until the new one is complete and
atomically replaces it; a held kernel lock collapses simultaneous first-search/
root-edit requests into one builder. `prelude index` is the explicit repair
door, not a normal setup step.
Nothing in Settings may read the full index to draw a row. Versioned file and
folder counts are recorded beside it; an old file-only generation is readable
but never blessed as current, so the first search upgrades it in the
background. Finder tags are part of the rebuild: one JXA process asks
Foundation for `NSURLTagNamesKey`, secret-looking or unbounded names are
rejected, and the bounded names are stored beside the path. Ordinary search
matches the object's own name or tags and mixes in at most ten rows; parent
paths are display-only unless the query explicitly contains `/`. `f:` and
`dir:` may return up to one hundred. All three scan the complete index while
maintaining only a bounded Top-K heap; a broad term must not allocate/sort every
matching row. They must never launch `mdfind`, `mdls` or another metadata
process on a keystroke. Settings gather may use file checks and stats, never
`pgrep` or another subprocess; live panel status belongs to explicit
`prelude global status`.

## The rules that matter

**Latency is the product.** fzf re-invokes the binary on *every keystroke*
through a transform binding, so startup cost is paid hundreds of times per
session. Be sceptical of new dependencies — the rule is about what a
*keystroke* pays, which is why the `ignore` crate (a library walk inside
gather, zero startup cost) was admitted while a URL crate for `web_url` was
not. `bench` must stay under 40ms at p95; it sits around 4 on a quiet machine.
Per keystroke the binary costs about 2ms, of which 1.8 is the kernel's fork
and exec — there is nothing left to win there, so measure `gather` and leave
the helpers alone. `bench --sources` is the instrument: it times every phase
of one gather and prints them sorted, because the floor is the slowest FAST
source and the profile has to be re-read after every win.

**A launch costs whatever its slowest subprocess costs.** The `FAST` sources
run on threads and are waited for together, so the floor is the slowest one
and the local work underneath it is free. That is the only shape worth
optimising for: shaving a source that is not the slowest changes nothing,
and the profile has to be re-read after every win because the floor moves.
It has moved twice now, and both times the fix was the same: FAST membership
*is* the performance decision. `procs` was the floor at ~20ms — `ps` costs
12–14ms merely enumerating a thousand processes, whatever fields are asked
for — for rows that only appear inside `proc:`, filtered from the snapshot;
it went behind the cache on the reasoning that already put 65ms `lsof`
there. Then `files` was the floor at ~8ms, nearly all of it fd's fork, exec
and thread pool built per launch; the `ignore` crate is fd's engine as a
library and does the same walk in under 1ms. `files` stays FAST because a
file you just created must be in `f:` on the very next press — the failure
to avoid is a missing row.

**One rule, one surface, two ways in.** Ctrl+R in zsh and the global chord must
show the same full layout, catalogue, labels, action list and filesystem chords.
Both copy command text; neither edits or submits the shell line. Both stand in
`global::launch_directory`, because gathering project rows from the invoking
shell's cwd made the answer depend on which key opened Prelude.

`defaults::surface()` therefore always returns `Clipboard`. `Surface::Prompt`
remains only as an explicit input to pure historical-rule tests; no runtime
entry point selects it. Keep every rule below `surface()` parameterised so it
stays testable without process state, and do not reintroduce an environment
switch that can make the two entry points diverge.

**Escape means "back", one level at a time, and the arrows mean it only while
there is nothing to type through.** The launcher is a stack — list, action
panel, submenu, and a typed query is a level of its own — and one key had been
doing the job of all of them: Ghostty's `unconsumed:escape` hid the whole panel
at every level, so `PANEL_BACK` fired correctly and the person never saw it,
because the panel was already gone. That binding is now absent (see
`global::quick_config`) and Escape belongs to Prelude, which is the only party
that knows which level you are on.

`←`/`→` normally do the same walk, with one restriction: they are the query
line's cursor keys first. Settings are the deliberate exception because they
are a form rather than search results. Both arrows route through
`_setting-key`: outside `set:` it emits `backward-char`/`forward-char` (or opens
Actions from an empty home); inside `set:` the focused row emits its labelled
less/remove/reset or more/add/change operation and reloads the live rows in
place. Interactive chooser/prompt adjustments use visible `execute`; immediate
booleans/enums use `execute-silent`. Do not move these rules back into `_bind`
or scatter mutations through fzf strings—`settings::adjust` is the one semantic
dispatch. `^K` remains the unconditional low-frequency door. `→` cannot be an
`--expect` key because its meaning is contextual, so the empty-home Actions
case still `print`s `ui::OPEN_ACTIONS` and `run_fzf` scans for it.

**A key the launcher took over keeps meaning what the fingers think it means.**
Ctrl+R spent decades as incremental history search, and the widget sits on it;
the debt is paid *inside*: a second press moves the typed text into `h:`
(`compute::history_toggle`, via `_hist`), so Ctrl+R Ctrl+R at a shell is the
old search over the three thousand commands root search deliberately excludes,
and a third press carries the text back out. A query in another scope switches
rather than nests, because `h:f:serve` is a question with an empty answer. Tab
is the same rule for shell completion: it completes exactly the rows whose
Enter is already a completion — scope commands and providers, `tab_completion`
— and stays inert everywhere else, so the key never acquires a meaning that
varies by row. Both are bindings, not `--expect` keys, for the reason `→` is
not; both go through `fzf_action_arg` because a query can contain `)`.

**Two general action keys: Enter is primary, Ctrl+K is the panel. A row that
*is* a filesystem object adds three explicit Enter chords. Ctrl+P is a mode.**
Graphical launchers
often put a secondary action on its own key; Prelude keeps that action but not
the key: where useful, it is the first selectable row below Enter's
non-selectable header. Neither action is a fixed verb; both are per-item, and
they are opposites — where one acts, the other hands you text. A test asserts
they never coincide. Object rows add three narrower shortcuts, not
a generic secondary: Ctrl+Enter reveals the object in Finder,
Ctrl+Shift+Enter opens a Ghostty in its directory, and
Ctrl+Option+Enter copies the exact absolute path and closes the launcher. The
second is the one thing this launcher cannot hand over as text — a
`cd` is only useful in a shell you already have, and the point of the panel is
not having one.

**Which rows those are is one question, asked once, in `ui::object_of`.** The
three chords used to ask it separately and each answered `File | Find | Dir`,
which was narrower than the data by eight kinds. An application, a config, a
conversation, a live Run, a Skill, an MCP server and a clipped image all carry
a real path — and every one of them already offered *the same verbs by name* in
`^K`: `Reveal in Finder`, `Copy path`, `Open terminal in containing folder`. So
the keys were dead on exactly the rows whose own action panel proved the key
had something to do. `object_of` returns the path and whether it is a
directory, and all three chords plus their footer labels read that one answer.
Nothing in it guesses: a history entry, an agent CLI and a `$PATH` binary have
no object that is *theirs*, and a text clip has no payload on disk. A Setting
is the deliberate exclusion though it carries a path — its backing file is
storage rather than intent and belongs in Details, and `set:` is a form whose
two controls are `←` and `→`, which three more footer columns push off a narrow
window.

`terminal_directory` keeps a single meaning, now stated as *the directory this
row's work happens in*: a folder itself, a file's parent — and, for a row that
records its own `cwd`, that. A Run and a Session have both **said** where their
work is, which beats deriving a directory from where a file is stored: a
conversation's `.jsonl` lives in the agent's private storage, so the parent
rule would stand a terminal in `~/.claude/projects/…`, an answer to a question
nobody asked, about the one row where the right answer is written down. The
live Run is the case the chord's whole argument was written for and was the one
row it did not answer.

**A key that acts on every row needs the same per-kind rule the panel has.**
`ctrl-o` was bound unconditionally and ran the row's text straight through
`sh -c` — no kind check, no confirmation — while `actions_for` two keystrokes
away went to real trouble over the identical question: `useful_in_preview`
names the six kinds whose command is small and non-interactive,
`generic_run_would_kill` withholds the row from ports and processes,
`is_destructive` reddens them and `needs_confirming` puts Cancel first. Ctrl+O
bypassed all four, and a Port row's command *is* `kill $(lsof -ti tcp:3000)`.
On an `f:` row it tried to execute the file. `runhere::can_run_here` is now the
one predicate and both the key and the panel row read it; everything else is
inert, on Tab's precedent. The rows that genuinely want output in the panel —
an MCP server, a one-off agent question, the update row — already run it on
Enter. Ctrl+P is
different: Quick Look replaces
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

**`focus` is not enough on its own: bind `load` to the same helper.** fzf
raises `focus` on cursor movement and on a search-result update, and *not*
when `reload` replaces the list while the cursor stays where it already was —
on the first row. Every scope entry is exactly that reload, so the first row
of `c:`, `s:`, `f:`, `app:` and the rest arrived carrying the previous row's
state: the footer under a clipboard entry read "Open this search", which is
what `c:`'s own scope command says, and the contextual clipboard preview never
opened, because the same helper decides both. One arrow press put it right,
which is why it lasted — the state was only ever wrong before anybody had
touched anything. `load` fires when a list has finished arriving, including
after every reload, so both events run the same binding. It costs one extra
fork per list, and it also gives the very first row on startup a real footer
instead of the static one.

Ctrl, because only Ctrl reliably arrives. Unless told otherwise macOS spends
Option on composing characters, so Option+K types `˚` into the search box and
Option+Enter comes through as a bare Enter — running the *primary* action,
silently. Cmd never reaches a terminal application, and neither does the
modifier on a Return: fzf knows no `ctrl-enter`, because a bare Return carries
nothing a terminal application can read. So the filesystem shortcuts are Enter chords only in the fingers. Prelude owns
their translations in both the dedicated config and one marked
ordinary-Ghostty block: `ctrl+enter=text:\\x07`,
`ctrl+shift+enter=text:\\x1d`, and `ctrl+alt+enter=text:\\x19` become private
Ctrl+G, Ctrl+], and Ctrl+Y. `EXPECT` includes all three, and the footer advertises
them only when the focused row carries the corresponding path.

Ctrl+] is 0x1d and deliberately not 0x1f, which is `render::SEP` — the
delimiter every rendered row carries, and which fzf would read as a field
boundary rather than as a key. **Ctrl+[ can never be one of these**: it is
0x1b, the same byte as Escape, so no terminal can tell them apart and fzf
refuses the name outright — claiming it would take away the key that means
"back" at every level. A test pins that.

The chords are now part of both entry points. `global.rs` owns one marked,
idempotent block in Ghostty's ordinary config containing exactly the two text
translations, reloads it with `SIGUSR2`, and removes it on `global uninstall`;
everything outside the markers remains the person's. This is the deliberate
cost of making the terminal launcher and Quick Terminal literally one surface.
The new window is opened through Launch Services with `-n`, for the reasons
`global.rs` already documents: executing
the binary directly makes it a foreground application, and without a new
instance macOS can deliver the launch to the hidden panel, which shares
Ghostty's bundle identity.

**The line is not danger, it is whether there is anything to edit.** A
command line goes to the clipboard — including agents, skills and sessions,
which are often the *start* of a command (`--resume`, a model, a question).
Safety is the second reason, not the first: `claude` is harmless and is still
handed over for review. An object just happens, because nobody proofreads
`open -a Zed foo.json`. Files therefore go to the application that owns them
rather than to `$EDITOR`, folders go to Finder, and URLs go to the default
browser. These external objects are passed directly to macOS Launch Services —
never emitted as `open ...` shell commands, never written to a prompt or
history. `openwith.rs` remembers file overrides per extension. A file's `^K` can make
one in context; `set:` is the complete collection manager, with `←` remove,
`→` add, and Enter browse/manage. Neither surface edits a parsed rule by
inventing its own storage format.

**Commands are handed over, objects act.** Copying a file path when you wanted
to read the file is a step backwards; opening a file is harmless in a way that
`kill $(lsof -ti tcp:3000)` is not. A file opens identically from either entry
point because Launch Services is the destination. `^K` still offers the text.

**A container is not a project, and `$HOME` is the one that bites.**
`project::root` walks up for a marker and, finding none, used to take the
current directory at its word. The configured launcher directory defaults to
`$HOME`, so "the files in this project" became `fd --max-depth 6` over the entire home
directory on every open — six levels into `~/Library`, which macOS protects as
other applications' data. The symptom named neither Prelude nor the source: a
TCC panel saying *"Ghostty would like to access data from other apps"*, because
`fd` ran under the terminal and the terminal is who gets asked. The unmarked
fallback is deliberate and stays — a scratch folder of notes is its own
project — but it goes through `paths::is_protected`, which already draws this
line for the Trash and draws it in the same place. Anything that walks a
directory the person did not choose belongs behind that check.

**Finding nothing and learning nothing are different answers, and the type
cannot tell them apart.** A source returns `Vec<Item>`, so a `claude mcp list`
that timed out and an agent with no servers both arrive as an empty vector —
and both used to be written, which meant a refresh that failed *erased* the
inventory it was refreshing, then looked fresh enough that nothing retried
until the TTL expired. `exec::lost_commands` is where the missing bit lives:
`capture` counts every command that could not be started or had to be killed,
and `cache::write_refreshed` takes the difference across one source's run. An
empty result that coincides with a lost command keeps the last good answer; an
empty result on its own is written, because a launcher that goes on showing
containers which have stopped is the opposite failure. A non-zero exit is
deliberately *not* counted — a CLI answering "none configured" with status 1
has told us something true.

**An aggregated source fails in parts, and the guard above only sees the
whole.** The MCP inventory asks every agent and returns their rows together,
so a `claude` that timed out beside a `codex` that answered produces a result
that is *not empty* — and the empty-result rule never fires. The whole cache
was replaced and every claude row went with it: the launcher ran perfectly and
quietly held less data, which is the only failure shape here that nobody
reports. `exec::note_incomplete` names the partition that could not be asked and
`cache::carry_over` keeps the last good rows for those partitions, while every
other partition is replaced wholesale so an agent whose last server was
removed still loses the row.

`sources::agents::trusted` decides, and it decides after *parsing* and before
*acting*, because the case that matters cannot be seen before the first and
everything after the second is expensive. Three outcomes: exit 0 with no
records is an authoritative empty; a non-zero exit that still produced records
is an answer; a non-zero exit with none is a refusal. Ahead of all three sits
*unfinished* — never started, killed, killed by a signal, or truncated at the
output cap — because a fragment must not replace a complete answer. The last
two of those were the subtle ones: both make `ok()` false without being named,
so having parsed a single record was enough to slip past the refusal test. What makes the last one
subtle is that `Error: authentication required` splits on `": "` exactly as a
server line does — it parsed as a server *named* `Error`, so "records were
parsed" was true and a refusal replaced three real rows with one imaginary
one. The count that decides is therefore the number of lines carrying a health
status, which an error message cannot produce by accident; rows without one
are still displayed, so a format that stops printing statuses degrades to
showing rows rather than to erasing them. Output that will not parse at all is
never an answer, whatever the exit code — that is `codex mcp list --json`
returning something unreadable, which is not the same as no servers.

**One format, one parser.** `parse_claude_list` is shared by the inventory and
the tool scanner because two readings of one output is one too many: they
filtered differently, counted differently, and only one of them had learned
that an error message parses as a server. The other trusted it *and* ran
`claude mcp get Error` on the way — which is why the decision now happens
before the per-server work rather than after it. Everything downstream of that
parse costs a subprocess per entry and, in `mcp_tools`, a started MCP server;
none of it should be spent on an answer that is about to be discarded.

`exec::require` is `which` for the same reason. **PATH is not the same
everywhere Prelude runs**: the panel is a Ghostty started by launchd and does
not inherit a login shell's PATH, so `claude` can be missing there and present
in every terminal on the machine. Treated as an honest empty, that wiped the
person's MCP servers out of the panel and left nothing to say why.

**One refresh per source, and a rest after a failure — both held by the
kernel.** Every stale source used to spawn `_refresh` on every gather with
nothing coordinating them, so three shells meant three processes running the
same network health check to write the same answer. The claim is now a `flock`
held for the life of the refresh, and the concurrency limit is two more locks
rather than a count.

Both were tried the other way first and both were wrong for the same reason: a
file that *records* a claim needs somebody to decide when the claimant died. A
lease with a maximum age has to exceed the slowest legitimate refresh, and
`mcp-tools` has no such bound worth naming — fifteen seconds for `initialize`
plus twenty for each of ten `tools/list` pages is three and a half minutes for
*one* server — so any constant is either short enough to declare a working
refresh dead and start a second beside it, or long enough that a dead holder
blocks the source all afternoon. And a count taken before spawning is advice:
the counter exits, nothing holds anything, and every other entry point that
refreshes was outside the count entirely. A held lock has neither problem, and
the kernel releases it however the process ends.

A failure records a rest that is always at least twice that source's own TTL —
`backoff_delay` derives it per source, because a flat constant equal to
`mcp`'s sixty-second TTL was not a backoff at all.

**Do not write what has not changed — and then do not mistake that for
staleness.** Writing these caches measured slower than gathering them (3.7ms
and 2.7ms against a 0.9ms scan), and the bytes are usually identical, so
everything goes through `write_if_changed`. That has a trap attached, and it
was live for one commit: `stale()` read the cache file's mtime, so a source
whose output is *stable* would have looked stale forever and spawned a process
every launch — trading the 3ms write for something far more expensive. A
successful refresh therefore stamps `refresh/<name>.stamp`, and staleness is
the newer of the two. Unchanged bytes also stop moving the mtimes the panel's
refresh thread watches, which had been telling it every launch that everything
had changed.

**A temporary file belongs to one writer.** `write_atomic` staged through
`path.with_extension("tmp")` — one name per destination for the whole machine
— so two Preludes wrote into the same file and each renamed it. The rename is
atomic, so the result was never half a file; it was a whole file holding two
answers spliced together, which is worse for being harder to see. Names now
carry the pid, a counter and the clock, and are created with `create_new`.
`write_state` is the variant for what a person cannot rebuild — 0600, flushed
before the rename rather than after — and it owns the bus, favorites,
frecency and the capability archive. An atomic rename still does nothing for
read-change-write, where both writes land whole and one is simply gone, so
those cycles hold `lock_for_write`; it gives up after 250ms rather than
waiting, because a launcher may lose a use count but may never hang. The same
lock answers "is the clipboard watcher running" — `lock_is_held` cannot be
fooled by pid reuse the way a recorded pid can.

**Ask fzf before spending a gather on it, not after.** The panel's refresh
thread used to rebuild — a full gather, two renders and four hundred kilobytes
of catalogue — and only then ask `_tick` whether the panel could be touched.
While somebody was typing the answer was no, every three seconds, with all of
the work already done. fzf's listen socket answers `GET /` with its live query
and cursor, so the question is asked first for the price of one connection;
`_tick` still makes the final call, because the person can start typing during
the gather. A tick that finds the panel busy records nothing, so the change
stays due rather than being consumed by a refresh that never happened. The
interval is measured with `Instant`: `SystemTime::elapsed` is a wall clock, and
a laptop waking to an NTP correction postpones the forced gather by exactly
that correction.

**A cache with no eviction is a leak with a good excuse.** Every version ever
downloaded stayed in `update/` — five releases, twenty-one megabytes, nothing
that would ever remove them — and a hundred clipboard images at the twenty-five
megabytes each is individually allowed is 2.5GB of somebody's disk. Both are
bounded now, by total bytes rather than by count.

**Bound what another program's file size can make this program allocate.**
`paths::read_bounded` is for anything on the launch path that reads a file it
did not create: a generated `package.json`, a monorepo `Makefile`, a shell
history nobody has ever rotated. These formats are line-oriented and their
parsers are too, so a bounded read is a shorter list rather than a broken one.
For an append-only file the bound is taken from the *end* —
`read_tail_bounded` — because the prefix of a 40MB history is the commands
somebody ran years ago, and a list that is full of the wrong things is not a
failure anybody notices.

**Sources degrade to nothing.** A source that fails, or finds nothing,
returns an empty list. Never blocks, never panics, never prints. Anything
slower than ~25ms belongs behind the cache tier. And a source that can see
it will find nothing should not pay to find that out: `docker ps` costs the
same 14ms whether or not a daemon answers, because the cost is the CLI
starting, so `containers` resolves the socket the daemon would listen on and
does not ask when it is not there. Being *unsure* costs a launch 14ms —
anything that cannot be resolved falls through to the subprocess, because
the failure to avoid is a missing row, not a slow one.

**The update check is the one source that reaches the network without being
asked, and it says so out loud.** Every other network call in this program
answers something the person just typed. This one does not, so it is bounded
in every direction that can be bounded: it is a slow-tier source with a
six-hour TTL, refreshed detached, degrading to nothing; it asks GitHub's API
and falls back to the `releases/latest` redirect; it sends no identifier; and
`update = off` is a real off switch that returns before the request is
constructed. It governs the *automatic* check only: `prelude update` is a
request the person typed, and a setting that silently refused to answer a
typed command would be a different kind of dishonesty. The boundary paragraph
in the README states all of this rather than leaving it to be discovered in a
packet capture — a privacy claim that turns out to have an exception is worse
than one that never claimed it.

**The update row runs; it does not hand you the command.** It is a `Kind::Sys`
row, and `Sys` hands its command over — right for every other one and wrong for
this: `prelude update` takes no argument anybody would add, so the reason
commands are copied ("they are often the *start* of a command") does not apply,
and handing it over made the row a detour to itself. `on_enter` checks the
`update` field before the kind, the footer says `Update now`, and `^K` keeps
`Copy the command` because the opposite of acting is being handed the text.

**But it cannot restart the panel from inside the panel.** `stop_helper` pkills
the Ghostty instance this process descends from, so restarting mid-update kills
the process tree doing the updating — after the binary has already been
swapped, leaving a panel that is down rather than new. The newly installed
binary instead regenerates the managed Ghostty config and sends the dedicated
instance `SIGUSR2`, which reloads changed bindings without killing its child.
Then `apply_default` emits a `CLOSE` verb the panel loop turns into
`After::Close`; the next press runs the new `_surface`. The zsh widget has no
case for `CLOSE` and ignores it, which is correct there.

**The updater cannot describe the release that replaced it.** An atomic swap
changes the path on disk, not the process already executing `apply`, so calling
that process's `restart_panel` writes the old release's Ghostty config and can
put a retired binding back beside the new binary and footer. After a swap, the
old process therefore invokes an internal door on the newly installed binary:
outside the panel that binary regenerates the config and restarts the dedicated
instance; inside it takes the reload-in-place path above. `panel::run` repeats
the config comparison before fzf starts as the recovery path for upgrading
from an older release whose updater did not know this handoff.

**The redirect is eventually consistent, and the rate limit was never the
constraint.** `fetch_latest` used the `/releases/latest` redirect *instead of*
the API, reasoning that unauthenticated API calls are rate limited while the
redirect target already spells the answer out. Half of that was true and
irrelevant: sixty requests an hour against four a day is three orders of
magnitude of headroom, not a constraint. What it missed is that the redirect
is served from a cache — measured still answering `v0.8.1` five minutes after
`v0.8.2` was published, and catching up two minutes later. A check that can be
minutes wrong about the one fact it exists to report is worse than one that
spends a request nobody is counting. The API is primary now and the redirect
is the fallback; neither needs a parser, because `tag_name` is a substring.
The lesson is the shape of the error, not the endpoint: a cost that was easy
to name (rate limits) crowded out one that had to be measured (staleness).

**The row is a cache, so anything that changes the answer must clear it.**
`update.json` has a six-hour TTL. An update that does not remove it leaves the
panel advertising a version you have just installed, for the rest of the day,
on a row whose Enter would now download what you are already running.
`forget_cached_row` is called after a swap, after a rollback, and by an
explicit `--check` — you asked, so the panel should say what you were told.

**An update that does not restart the panel creates the exact state it was
meant to end.** `update.rs` exists as much for question 1 — *is the running
panel the binary on disk* — as for the release check, because that is the
failure people actually hit: the panel executes what the binary held at login,
each press spawns the new binary to draw the list, and so the rows update
while the delivery decision stays old. `panel.rs` records its own version and
pid at startup and `panel_is_stale` compares it; `prelude --version` and
`global status` say so. This is why `Mode::Apply` swaps only *as the panel
starts*: nothing is on screen, and the process about to serve keypresses is
the new binary. Applying at any other moment is how you manufacture the bug.

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
never the thousands of history entries, clipboard rows and
`$PATH` commands underneath. Ordinary queries now mix in at most ten local
file/folder name matches; exact `f` shows one Files & Folders command and `f:`
opens the longer result set. `:` lists every scope, and clearing restores the home.

**Applications are the exception that had to be made, and the shape of the
error is worth keeping.** Apps were in that excluded list beside history and
`$PATH`, and the exclusion was consistent: the catalogue is what this launcher
*manages*, and the machine's contents are behind scopes. But typing an
application's name is the commonest question anybody asks a launcher, and
`Chrome` answered it with eight `node_modules` icon files, a folder, and a
Google search — while `Google Chrome.app` sat unread in the very snapshot that
query had already parsed for its file rows. It was reachable only by first
knowing to type `app:`. That is not a discoverability gap; the person did not
fail to find a feature, they asked the launcher its most ordinary question and
were told about something else. Files had already been promoted into root
search and applications had not, which is backwards.

`root_application_rows` costs nothing — a filter over a `Vec` that ordinary
queries load anyway, measured at no change against an 18ms keystroke. Three
rules keep it honest, and each is the part to preserve. It is emitted **below
the catalogue**, so an installed Agent still leads its own name and a quicklink
keyword still resolves ahead of an application containing it. It is emitted
**above the files**, because between two objects of one name the application is
what a launcher is usually being asked for. And it takes **substring or
better** — `name_score`'s forgiving subsequence tier is refused here, because
`--tiebreak=index` puts this block above the file rows and a stretched app
match would outrank a file whose name the query actually spells. Keep


`home.txt`, the root-command `list.txt` and the complete scoped item cache on
one gathered snapshot and one layout — a scope must never gather a source on
each keystroke, and `a:waiting`, `a:failed`, `a:claude Prelude`, `a:using X`
and `a:without X` all filter that one snapshot with no Agent CLI behind them.
`a:`'s values are quote-aware, like `s:`'s: a Skill name is an identifier but
an MCP server's name is whatever its owner called it, and `claude.ai Google
Drive` split on whitespace becomes a different question with an empty answer.
`a:` excludes Session because `s:` owns the hundreds of old conversations.

**But a query that matches none of that must not answer with an empty box.**
That is what it did, and it was reported as "搜索用不了" — search does not
work — which is the right reading: a root list of fifty-seven rows has nothing
in it for `git commit`, so fzf drew `0/58` and a blank rectangle, and nothing
on screen said the machine had understood the question and declined to answer
it. The person most likely to hit this is the one who has just given Ctrl-R to
the launcher and typed the command they were looking for. So
`compute::web_search_row` computes one web-search row *from* the query and
`dynamic` prints it after everything else. It cannot fail to exist, which is
the whole point — every other row here has to be found.

Three things make it safe to have on every query, and each is the part to keep.
Its **display text is the query**, so fzf matches it by construction; titled
`Search Google for …` it would be filtered out by the very query that computed
it. It is emitted **last**, after the catalogue, so `--tiebreak=index` leaves
it under anything that scored the same and it leads only when it is the only
thing left. And it is **absent inside a scope** — `f:`, `c:`, `h:` and the rest
are a person saying where to look, and "or the web" is not an answer to that.
The provider follows the `g` quicklink rather than hard-coding Google, because
somebody who re-points that keyword has said where their web searches go; the
built-in template is the fallback for having deleted it, not the authority.

`/` has the two states a search provider has. An *incomplete* name browses —
`/cnipa-oo` lists the Skill rows it matches — and a complete one is an
invocation, so `/cnipa-ooa` is the single row that runs it and the Skill row
is gone. Nothing here shows two rows for one intent, and MCP servers are not
on `/` at all: they are not invoked by name, and `mcp:` is their scope.

**A Quicklink is banded by the person having named it, not by what it points
at.** `Item::band` is `Kind::priority` for everything else and
`Kind::QUICKLINK` for a saved quicklink, and `by_rank` reads that instead of
the kind. It was the kind's, which read as principled and was hopeless in
practice: `File` is 60 and `Link` 147, so a keyword somebody invented sat
below every scope command in `root_items`, and the only thing that ever
rescued it was `is_special` short-circuiting on an *exactly complete* key.
Three letters into a six-letter keyword it was at the bottom of the list. The
score is reset rather than added to, so quicklinks start level with each other
and frecency alone orders them — banded by kind they sorted Link, App, Dir,
File, an order nobody chose and no row explained. The target's `Kind` stays on
the item, because Enter and `^K` must still act on what it points at; only the
band and the label come from its being a quicklink. `Item::style` is why the
label is not `kind.style()` at any call site.

**The kind column names what a row *is*, not what Enter does to it.** Look at
the other twenty-six labels — `agent`, `session`, `skill`, `mcp`, `clipboard`,
`history`, `app`, `folder`, `branch`, `script`, `port`, `process` — they are
nouns naming a source. What Enter does is already stated twice, in the footer
and at the top of the `^K` panel, and a third statement in a five-column row
is not worth the column.

So **both** shapes of a Quicklink read `quicklink`, and `search` is left
meaning the one thing it names: a scope command into Prelude's own index.
`Kind::Search` carries both populations and they are not the same thing — `f:`
is built in and goes to the file index, `gh <query>` is a line in the person's
`quicklinks.toml` and goes to the web. That the two behave alike on Enter is a
coincidence of both needing an argument.

This was briefly the other way round, and the correction is worth keeping
because the reasoning that produced it was the giveaway: "the kind column
answers what Enter will do" is a rule invented to justify one row, and it is
contradicted by twenty-five of the twenty-seven labels. A principle that holds
for the case in front of you and nothing else is not the principle.

The *result* of a template — the Link row `g rust async` produces — carries
its provider's key but is not a saved quicklink (`ql=result`). It keeps
saying `open`, and it keeps `Create Quicklink…`, which it did not: the row
suppressed creation because it carried a key, so the one URL a person most
wants to keep was the one URL they could not.

**An exact alias leads the list; it does not clear the room.** `is_special` is
true for an exact key, and `dynamic_rows_with` used to answer it with
`vec![item]` — so the catalogue underneath was suppressed and one keystroke
turned two sensible candidates into one. At `githu` both a `github` quicklink
and `Search GitHub` were on screen; typing the last character deleted one of
them, and nothing said which or why. `quicklink_with_neighbours` keeps the
exact row first and appends every root item that still matches, deduped on the
key *and* on `(kind, cmd)` — a catalogue cached by an older build carries
neither the key nor anything else new, and a duplicated row is precisely what
an upgrade would otherwise show.

Do not try to solve this by dropping the exact match and letting fzf rank it.
That was measured against the real root list with `--tiebreak=index`: `github`
does put the quicklink first, but `google` loses to an MCP server and `g` and
`b` lose to skills. fzf scores its own way and the band only breaks its ties;
an alias that wins only when its name is long and unusual is not an alias.
This is why an exact key is the one thing `is_special` still resolves from
config, and why `needs_static_items` now says yes to it — one 400KB parse on
the handful of keystrokes that complete a keyword, measured at 0.4ms against a
2ms keystroke.

**Every lookup folds case, and every refusal happens at the moment of naming.**
`minitoml` keeps a section name as written and every lookup lowercases the
query, so a hand-written `[Design]` matched nothing, produced no row and
reported no error — `quicklink_entry` is now the only way in. The same
applies to the three ways a key could be accepted and then never work: a scope
prefix (`dynamic_rows_with` settles `f:` before any quicklink, so `[f]` was
written to the file and unreachable forever), a name another entry already
carries (`[g]` is called "Google", and it swallowed a fixed `[google]` until
exact keys were tried first), and a duplicate. All three are refused by
`quicklink_conflict` and `append_quicklink` when the person types the keyword,
because that is the only moment a reason can be given. Keys accept letters and
digits of *any* script: ASCII-only meant a Chinese user could not name a
quicklink in the language the thing being named was written in, and
`quicklink_suggestion` handed them an empty prompt on top of it.

**Reading quicklinks must not write them.** `quicklinks_text` was
create-the-file-and-maybe-migrate-it, and `is_special` plus `dynamic_rows_with`
called it up to four times per keystroke. It is now a memoised pure read that
falls back to `QUICKLINKS_DEFAULT` in memory when the file is absent;
`ensure_quicklinks_file` owns the creation and the versioned migration and is
called from `quicklink_items` (once per gather), the settings editor, the CLI
and `doctor`. Mutations read uncached and invalidate.

**`ql:` shows the entries that do not work.** The ordinary catalogue is right
to drop an entry it cannot render — a broken row is not a search result — but
that left the one screen whose job is repair unable to show the thing needing
repair, and for a long time there was no such screen at all: the only way to
answer "what quicklinks do I have" was to open the TOML. `quicklink_scope_rows`
adds a stating row for each entry `quicklink_items_from` refused, carrying the
same rename, re-point and remove actions. `prelude quicklink` is the same set
of verbs without fzf, for the reason `settings add-root` exists.

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
removes superseded payloads. Never hash those images on gather.

**AppKit has no pasteboard notification, so the poll interval is the whole of
the latency, and it follows the activity.** A flat one-second poll measured a
654ms median — exactly a uniform draw from a one-second window — which made
the launcher wrong about the thing it is most often opened for: copy, press
the chord, and half the time the clipping is not there. v4 polls `IDLE` (0.5s)
between bursts and `ALERT` (0.08s) for fifteen seconds after any change,
because copying comes in runs; measured 83–86ms in a burst and ≤524ms cold.
**The version was bumped for a change the record format did not need**, and
that is the precedent: a watcher that is merely half a second late is not
wrong, so nothing would ever have replaced it, and it would have gone on being
late for as long as the machine stayed up. A version that only advances on
format changes strands the one kind of fix a person can actually feel. **A path transfer is cheap for Prelude and not for the terminal, so scale
before handing it over.** The native transfer writes a path and returns in
under 2ms, which is why this looks free; the decode is on the other side of it,
and a 3.7MB Retina screenshot measured 86ms. That is paid on every *focus
change* — every arrow press through `c:` — so `scaled_for_preview` puts a
pane-sized copy in the cache tier, keyed by path and mtime: decode 86ms down to
22ms, and 30ms per image down to 2ms once warm. The pane is sixty columns;
sending thirty times the pixels it can show is work nobody sees. The directory
is pruned, because clipboard images age out of their own history and their
thumbnails would not.

It is *not* paid per redraw, and the first version of this note said it was.
fzf runs a preview command on focus change and on nothing else — measured: a
resize does not re-run it, a redraw does not re-run it. So revealing the panel
does not re-decode anything, and an explanation that sounded mechanical ("we
delete and re-transmit every render, therefore every redraw decodes") was an
inference from the code's shape that one experiment falsified. Where the
remaining reveal-time cost lives is the terminal's own compositing of a
placement it already holds — which is Ghostty's side of the line, and smaller
now only because the picture is. A Ghostty or
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

**The deadline goes on the process, and the kill goes to the group.** Draining
stdout on a thread solved the deadlock and quietly replaced the timeout with
the wrong question: waiting for stdout to reach EOF is not waiting for a
process to exit, so a child that closed its output and kept working sailed
past its deadline, and the `child.wait()` afterwards had no bound at all. The
wait now happens on its own thread and the deadline applies to that. And
killing the child is not killing what the child started — an agent CLI is
routinely a script and an MCP server is `npx` starting `node`, each
grandchild holding the same pipe — so every child gets its own process group
(`own_process_group`) and the timeout kills the group. `mcp_tools` spawns
servers itself and needs the identical treatment; without it a reader thread
joins on a pipe that will never close and the refresh process hangs forever.
Output is capped at `MAX_OUTPUT`: `read_to_end` on a pipe is an unbounded
allocation controlled by another program.

**`bench` measures the gather; `bench --process` measures the launch.** Forty
calls to `gather` inside one process share a warm allocator, loaded code pages
and every `OnceLock` the first call filled, which is the right instrument for
"did a source get slower" and the wrong one for "what does pressing the key
cost". `--process` spawns the real binary per sample and reports the launch
beside the per-keystroke helpers, because both are paid where a person can
feel them.

**`bench` gates on p95, and exits non-zero when it fails.** The median was the
gate, and a median cannot see what the budget exists to prevent: a launch that
is usually 6ms and occasionally 59ms is felt as a launcher that hitches, and it
passed — the middle of five samples said `OK` while a run three times over
budget sat in the same printed set. Five samples cannot have a 95th percentile
either, so it takes forty after a warm-up, and `--json` exists for anything
that wants to record the distribution rather than read it.

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
offers one closed repair class: moving a Prelude-owned staging entry to the
Trash. Each finding is confirmed separately with Cancel first. A report is
printed, read, and then answered one question at a time, so minutes pass
between seeing something and acting on it
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
and Quicklinks (and versioned, one-time additions to their built-in
keywords — `DEFAULT_BLOCKS`, one entry per block, appended to a file that has
not seen that block's marker; an existing keyword is skipped rather than
overwritten and a deleted default is not restored, because the marker records
that a block was *offered*, not that its contents survived. A built-in keyword
may not be a scope prefix or a built-in Agent's name: an exact keyword
resolves ahead of the catalogue, so shipping `claude` would push the Agent row
it names down a line for everyone, by default, and a test walks the table for
both). Quicklink writes are atomic and never overwrite a keyword;
generated blocks are marked so removing one preserves hand-written
`quicklinks.toml` comments and search templates byte-for-byte, and URLs that
look credential-bearing are refused rather than indexed — in a fixed target
and in a `{q}` template, which is checked with the braces substituted so a
credential cannot ride along in every search. Removing, renaming and
re-pointing reach hand-written entries too, bounded by the section header
rather than the markers; refusing them was the older behaviour and it said so
with "that quicklink is managed in the config file", a sentence whose plain
reading is the opposite of what it meant. Outside Prelude's config and caches,
only explicit setup or actions write user files: `global install/start/update`
own the marked three-key block in ordinary Ghostty config; capability install,
raw Session export, and moving a selected file, application, skill copy or
inactive native Session to the Trash are the other cases. None is a default
launcher action.

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
`Copy the slash command` (`/name`) and `Point an agent at its file`
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
