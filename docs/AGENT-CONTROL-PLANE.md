# Agent Control Plane

This is the current implementation reference for Prelude's Agent-facing
behavior. The authority is the code in `src/agent.rs`, `src/control.rs`,
`src/sources/{agents,running,sessions}.rs`, `src/bus.rs`, `src/lend.rs`, and
`src/doctor.rs`.

Before changing Agent, Run, Session, Skill, Config, Home, messaging, or
Agent Doctor behavior, update this document in the same change. Do not add a
second built-in-Agent registry outside `src/agent.rs`.

## Product boundary

Prelude is a launcher and local inventory. It does not schedule Agent work,
maintain a task graph, proxy conversations, or provide a replacement chat
protocol.

It does four things:

1. discovers installed Agent CLIs, native Sessions, and live processes
2. relates Agents, Runs, Sessions, Skills, and Config files
3. exposes contextual launcher actions using only syntax the owning CLI is
   known to support
4. provides a small file-backed message bus for questions, notices, and
   Agent-to-Agent inbox messages

MCP servers are not covered at all. Inventory, health, tool scanning, the
cross-agent definition matrix and one-run borrowing were all implemented and
then removed: a Skill reached by its absolute path needs none of them, and
nothing that remained justified the surface.

Native Agent CLIs, process state, and native Session files remain authoritative.
Prelude adds local names, pins, archive flags, Favorites, derived relationships,
and redacted capability comparisons.

## Built-in Agent registry

`src/agent.rs::SPECS` is the one registry for invocation syntax, conventional
settings paths, and capability flags.

`agent::SPECS` holds nineteen CLIs — every one installed on the machine this
was written on. Each entry carries only syntax read out of that CLI's own
`--help`, and `resume`, `ask` and `fork` are all `Option`, so "no known form"
is representable rather than invented. That matters more than breadth: a
command assembled from a guessed flag looks right on the clipboard and fails
after the launcher has closed.

| | resume by id | non-interactive ask | fork | Sessions found |
|---|---|---|---|---|
| claude | `--resume ID` | `-p` | yes | yes |
| codex | `resume ID` | `exec` | yes | yes |
| pi | `--session ID` | `--print` | yes | yes |
| omp | `--resume ID` | `-p` | — | yes |
| kimi | `--session ID` | `--prompt` | — | yes |
| cursor-agent | `--resume ID` | `-p` | — | yes |
| opencode | `--session ID` | `run` | — | no, SQLite |
| gemini | `-r ID` | `-p` | — | not yet used |
| qwen | `-r ID` | `-p` | — | not yet used |
| copilot | `--resume=ID` | `-p` | — | not yet used |
| qoder | `-r ID` | `-p` | `--fork-session` | not yet used |
| droid | `-r ID` | `exec` | — | not yet used |
| agy | `--conversation ID` | `-p` | — | not yet used |
| mastracode | `--thread ID` | `--prompt` | — | not yet used |
| amp | `threads continue ID` | `-x` | — | not yet used |
| grok | `-r ID` | — | — | not yet used |
| cline | `--id ID` | — | — | not yet used |
| kiro-cli | `chat --resume-id ID` | — | — | not yet used |
| kilo | — | — | — | not yet used |

"Sessions found" is about this machine, not the CLI: most of these keep no
local conversation store because they have not been used yet, and a scanner
cannot be written against a format nobody has produced. `kilo` offers only
`--continue`, so a named conversation has nothing honest to hand over and
`resume_cmd` falls back to the bare id rather than inventing a flag. The three
without an ask have output-format flags that shape a session they do not start.

Skills need none of this: a `SKILL.md` is read by name and path, so
`skill_dirs` covers forty-odd CLIs on paths alone.

Skills need none of this: a `SKILL.md` is read by name and path, so
`skill_dirs` covers forty-odd CLIs while this table stays at seven. An Agent
enters here only when Prelude can *drive* it — which for Sessions means a
resume command and a readable conversation store, both verified against the
CLI's own `--help` and the files on disk.

Concrete command forms:

| Operation | Claude Code | Codex | pi | OpenCode |
|---|---|---|---|---|
| start with prompt | `claude <prompt>` | `codex <prompt>` | `pi <prompt>` | `opencode run <prompt>` |
| one-off ask | `claude -p` | `codex exec --skip-git-repo-check` | `pi --print` | `opencode run` |
| resume | `claude --resume ID` | `codex resume ID` | `pi --session ID` | `opencode --session ID` |
| fork | `claude --resume ID --fork-session` | `codex fork ID` | `pi --fork ID` | unavailable |

Kimi, Cursor and omp resume with `kimi --session ID`,
`cursor-agent --resume ID` and `omp --resume ID`.
Kimi has no interactive-with-prompt form — its help lists `[command]` rather
than a positional prompt — so `kimi --prompt` is both its start and its ask,
the same arrangement `opencode run` already had.

Conventional settings paths are:

- Claude Code: `~/.claude/settings.json`
- Codex: `~/.codex/config.toml`
- pi: `~/.pi/agent/settings.json`
- OpenCode: `~/.config/opencode/opencode.jsonc`

The launcher home creates Agent rows only for installed executables. Control
JSON includes all four registry entries with an `installed` boolean.

## Canonical graph

`prelude control --json` serializes schema 4:

```text
Agent
 ├─ Run
 │   ├─ optional Session
 │   └─ explicitly named Skill
 ├─ Session
 ├─ Skill copies
 └─ Config paths
```

Stable identities:

- Agent: registry name, for example `claude`
- Run: `agent:pid:process-start-time`
- Session: `agent:native-session-id`
- Skill: `skill:<merged-name>`; copies retain Agent and canonical directory

The Snapshot contains Agents, Runs, Sessions and Skills. Run
records include PID, start time, live state, cwd/project, optional linked
Session, batch flag, branch, structured model evidence, and capability names
extracted from launch arguments. Session records include native/local title,
pin/archive state, cwd, native file, update time, and optional active Run. Skill
records include reverse Run edges.

Control JSON intentionally contains local relationship paths such as Run cwd,
native Session file, and Skill-copy directory. It is local machine data, not a
redacted export format. A native Session title may come from its opening user
text. The graph omits full process command lines, Run launch arguments, credential-bearing arguments.

## Agent inventory and Home

`cache::gather` creates one ranked catalogue. The empty-query Home filters it to:

1. questions waiting on a human
2. installed Agents
3. live Runs
4. visible Skills
5. the 15 newest visible Sessions

`a:` contains questions, Agents, Runs, visible Skills and Config rows. Sessions have their own `s:` scope. Of the rows in this scope,
Favorites can promote Agent and Skill ones, and only within their band.
Favorites are not an Agent feature and cover stable objects outside this
document too; `favorites.rs` is the whole list.

An Agent or Skill can also be given a name in `aliases.txt`, which
stores that same object key, resolves it against the catalogue as it is typed,
and shows the name on the row it belongs to. A built-in Agent's own name is
refused as an alias, because an installed Agent already leads it and the alias
would push that row down. Aliases write no Agent data and are not part of any
Agent's inventory; `aliases.rs` owns them and the Aliases settings row manages
them.

The named root commands—Agent Control Center, Running Agents, Past
Conversations, Skills, and Agent Config—make these views
reachable without knowing a prefix first.

## Runs and live state

### Discovery

The fleet source is process-based, not terminal-based. It finds matching
Claude Code, Codex, pi, and OpenCode processes wherever they run. Administrative
subcommands used for auth, config, and diagnostics are filtered out so
Prelude does not report its own probes as conversations.

Expensive identity discovery uses cached `ps` command lines plus one bulk `lsof`
for working directories. The complete command and prompt are discarded after
extracting bounded relationship hints and explicitly named capabilities.

On every gather, Prelude performs only live syscalls per cached row:

- `kill(pid, 0)` removes processes that no longer exist
- `stat` on a linked Session file determines silence
- direct `.git/HEAD` reads derive a branch/detached label per cwd

Exited processes are dropped from the live list rather than retained as dead
rows.

### Working versus waiting

A conversation-backed Run becomes a waiting candidate after its native Session
file has been silent for 30 seconds. Prelude then reads at most the last 64 KiB:

- a user/tool-result turn or assistant tool call means **working**
- an assistant prose turn followed by silence means **waiting**

A mid-tool turn beats the clock, so a long build is not labelled waiting merely
because the file is quiet. Batch forms (`claude -p`, `codex exec`, and other
recognized print/exec forms) and Runs with no Session clock remain working,
because silence cannot classify them.

This is evidence-based inference, not an Agent-reported status. An explicit
`prelude ask` question is stronger evidence and appears separately as a leading
Question row.

### Session relationship

Relationship order is conservative:

1. an explicit native resume id is exact
2. cwd-latest may link only when one Run of that Agent exists in the directory
3. multiple same-Agent Runs in one project remain ambiguous rather than being
   attached to the same Session

A Run uses a unique `kill <pid>` Item command so launcher deduplication does not
collapse two Agents in the same project.

### Effective context

Run Quick Look shows only available facts:

- Agent, project/cwd, state, start time, PID
- branch or detached commit read directly from Git metadata
- linked Session and match quality
- model only when a native structured field records it
- last conversation evidence from a bounded tail read

Claude Code records model in assistant messages, Codex in `turn_context`, and pi
in `model_change`. OpenCode currently contributes no structured model. Prelude
does not derive token use or cost.

No installed CLI can report the resolved configuration of an already-running
process. `doctor agents` may report current-directory Agent-level evidence:
Codex's redacted doctor config, OpenCode's resolved debug config, or Claude's
auto-mode subsystem. pi has no non-interactive resolved-config command.

## Sessions

### Native sources

Prelude discovers:

- Claude Code: `~/.claude/projects/**/*.jsonl`
- Codex: `~/.codex/sessions/**/*.jsonl` and
  `~/.codex/archived_sessions/**/*.jsonl`
- pi: `~/.pi/agent/sessions/**/*.jsonl`
- Kimi: `~/.kimi-code/sessions/<slug>/session_<uuid>/state.json`
- Cursor: `~/.cursor/chats/<workspace>/<chat>/meta.json` and
  `~/.cursor/acp-sessions/<id>/meta.json`
- omp: `~/.omp/agent/sessions/<encoded-cwd>/<timestamp>_<uuid>.jsonl`

The last two are not transcripts. Kimi and Cursor keep a small JSON sidecar
beside the conversation, so those scanners read one bounded document per
session and never open the transcript. Kimi's carries a title, a working
directory and both timestamps; Cursor's carries the directory and timestamps
only, because its transcript is a SQLite `store.db` and a database engine on
the launch path is not worth a display string — a Cursor row is its project and
its age. Sessions that were opened and never used are skipped: Cursor says so
itself with `hasConversation: false`, and Kimi's equivalent is a `New Session`
title whose created and updated stamps still agree.

The readability probe in `doctor sessions` therefore tries line-wise JSON first
and the whole document second. Line-wise alone reported every one of these
sidecars as malformed, since no single line of a pretty-printed object is valid
JSON on its own.

Only bounded heads are read for identity/title/cwd. The complete inventory is a
background cache; the launcher carries only 15 into the general catalogue, and
`s:` searches the linked cache with a limit of 80 rows.

OpenCode has resume syntax in the registry but no native Session scanner in the
current code.

### Prelude metadata

Local rename, pin, and archive state live in private atomic
`$XDG_DATA_HOME/prelude/sessions.json`. They never rewrite native JSONL.

- pin adds source rank above ordinary recency
- rename preserves the native title as a search term
- archive hides the Session by default
- an archived Session linked to a live Run is visible while active without
  clearing the archive flag

Supported filters are documented in [SEARCH.md](SEARCH.md): `is:pinned`,
`is:active`, `is:archived`, `is:all`, `agent:`, `project:`, and `since:`.

### Session actions and safety

Claude Code, Codex, and pi support native fork actions. A Session can resume
with a non-owned capability only where the target Agent's one-run flag exists.
Archived capabilities are excluded from those pickers.

Exports go under a private `0700` `$XDG_DATA_HOME/prelude/exports` directory:

- raw export copies the recognized native JSONL byte-for-byte to a `0600` file
- Markdown export extracts human/assistant prose, omits tool arguments and
  harness material, and redacts credential-shaped paragraphs

Trash is offered only for an inactive native `.jsonl` under a recognized
Session root. The action canonicalizes the path and refreshes the fleet before
moving it. Any exact or ambiguous same-Agent/same-project live relationship
refuses the move. Metadata remains so restoring the file from Trash restores
its local name/pin/archive state.

## Skills

### Discovery and identity

Skill roots are a table of filesystem conventions in
`sources/agents.rs::skill_dirs`, deliberately much longer than `agent::SPECS`.
The two answer different questions: `SPECS` is about invoking a CLI and a name
enters it only once Prelude can drive that CLI, while a Skill is a directory
with a `SKILL.md` in it whoever put it there. Since Enter became the portable
invocation, reading a root costs one `read_dir` and knowing how to launch the
owning Agent stopped being a precondition for using its Skills. Nothing in this
table confers a capability, so it is not a second Agent registry.

The vendor-neutral roots are listed first, because `dir`/`file` are the first
copy discovered and that is what Enter hands over — a Skill in several places
should be offered by the path that survives changing Agent.

- `~/.agents/skills`, `~/.config/agents/skills` (`shared`; locations, not Agents)
- `~/.claude/skills` (`claude`), `~/.codex/skills` (`codex`),
  `~/.pi/agent/skills` (`pi`), `~/.config/opencode/skills` (`opencode`)
- discovery-only roots for Kimi, ZCode, OpenClaw, Gemini CLI, Antigravity,
  Copilot, Cursor, Cline, Continue, Goose, Crush, Droid, Qwen, iFlow,
  OpenHands, Roo, Kilo, Kiro, Kode, Junie, Augment, CodeBuddy, Command Code,
  Cortex, Windsurf, Mistral Vibe, Mux, Qoder, Trae, Zencoder, Neovate, Pochi,
  MCPJam and AdaL

A root that does not exist is one failed `read_dir` and no rows; the whole table
measures at about 0.25 ms over the five it replaced. Being wrong is equally
cheap and therefore the real risk — an invented path never fails visibly, it
just never matches — so entries trace to vendor documentation, the cross-agent
survey tables, or a directory observed on a real machine. Where sources
disagree both are listed. Project-level roots (`.claude/skills` and friends
beside the working directory) are not discovered.

Prelude reads `SKILL.md` or `skill.md`, uses frontmatter name/description when
available, and merges rows by name while retaining every copy path. Usage rank
comes from native Sessions that invoked `/name`: count first, recency second.

### Integrity

The `skill-hashes` background source fingerprints the effective complete tree,
including scripts, references, and symlinks. It excludes VCS/cache metadata and
uses recursive metadata stamps to avoid rehashing unchanged trees.

A copy whose own root is a symlink is hashed as the tree it resolves to, not as
its link text, and records `linked` plus the canonical path it resolves to.
Prelude never creates such a link — it is how a person's own arrangement is read
correctly rather than reported as a divergent copy.
Symlinks *inside* a tree are unchanged: still hashed as link text, never
followed. The stamp used by the reuse gate resolves the root for the same
reason — a stamp taken at a link never moves when the tree behind it changes.

Credential-like paths and lines contribute a redaction marker rather than their
bytes. Integrity states are:

- `single`: one known copy
- `linked`: exactly one real tree, with every other copy a link resolving to it
- `identical`: multiple independent copies with one fingerprint
- `divergent`: multiple different public fingerprints
- `unknown`: a copy is unreadable or unhashed
- `private-unknown`: redacted private material prevents an equality claim

`linked` and `identical` are deliberately separate. `identical` describes
today — several independent trees that currently agree, one edit away from not
agreeing, with no rule saying which an Agent loads. `linked` describes what can
happen next, because there is one tree to edit. Prelude creates neither
arrangement; both are read off whatever the person has already built.

Diagnostics also validate frontmatter, required entry files, same-root case/name
collisions, broken symlinks, and symlinks that escape the Skill tree.

### Using, borrowing, replacing and removing

- one-run Skill borrowing supports Claude Code and pi
- Enter hands over the portable invocation — the Skill's name and the absolute
  path to its `SKILL.md` — which needs no install, works in an Agent that is
  already running, and is the only form all four Agents accept. Installing is
  now only for wanting `/name` in a particular Agent
- sensitive or incompletely read sources are not copied, lent, or synchronized
- replacement displays `diff -ru`, rehashes source/target, confirms, trashes the
  old target, and refuses a source that changed or contains private material
- deletion accepts only a canonical direct child of a known Skill root and
  moves one named copy to Trash

### Archive and Favorites

Favorites store only `skill<TAB>name` in
`$XDG_CONFIG_HOME/prelude/favorites.txt`. Archive stores only the stable Skill
key in private atomic `$XDG_DATA_HOME/prelude/capabilities.json`.

Archive does not move a copy. It removes the Skill from Home, root search, `a:`,
bare `skill:`, `/` browsing/invocation, and Session borrow pickers. Restore from
`skill:is:archived`; `skill:is:all` includes both states. Favorites survive.

## Message bus

The bus stores one atomic JSON file per message under
`$XDG_DATA_HOME/prelude/bus`. There is no bus daemon.

Identity is discovered rather than passed as flags:

- the enclosing Agent comes from climbing the process tree
- cwd is the project/inbox address
- a bare shell is identified as `shell`

Commands:

```sh
prelude ask [--timeout=N] [--no-wait] TEXT
prelude tell TEXT
prelude say WHO TEXT
prelude inbox [--json] [--all] [--human]
prelude drain
prelude answer ID TEXT
prelude answer-of ID
prelude reply
```

Behavior:

- `ask` writes a human-directed question, posts a macOS notification, and waits
  up to 600 seconds by default; stdout contains only the answer, exit 3 means
  no answer arrived
- `--no-wait` prints an id for later `answer-of`
- `tell` writes an already-handled human notice and posts a notification
- `say` resolves a live Run by exact project/PID/address first, then Agent or
  project/cwd text; anything other than one match is refused
- every `say` waits in the target cwd's inbox; no terminal is focused or typed
  into
- from an Agent, `inbox` returns uncollected `say` records for its cwd; from a
  bare shell it returns pending human questions
- `--human` forces the human view when a person is working inside an Agent's
  process tree
- `drain` marks matching Agent inbox messages collected
- answered records become eligible for cleanup after 24 hours when a later
  `ask` runs; unanswered questions are never swept automatically

Message text is limited to 4,000 display columns and secret-looking lines are
replaced with `[redacted]`. Message ids accept only bounded alphanumeric/dash
names before becoming file paths.

`prelude fleet --json` is the companion discovery surface. `prelude watch` is a
foreground daemon that sends a notification on the working-to-waiting edge and
when a newly observed Run is already waiting.

## Doctor

There are five explicit reports:

```sh
prelude doctor
prelude doctor agents [--json|--repair]
prelude doctor sessions [--json|--repair]
prelude doctor skills [--json|--repair]
```

The general report checks launcher/runtime dependencies. Specialized reports
cover executable/version/login/config evidence, Session indexes and projects,
Run relationships, Skill integrity, bus orphans,
and Prelude's private borrow staging.

Text and JSON are rendered from the same `Report` data. `--json` and `--repair`
cannot be combined. Repair requires an interactive TTY, asks separately with
Cancel first, and can only move Prelude-owned staging entries to Trash. It does
not repair Agent configs, native Sessions, or Skills. Each
staging repair rechecks path ownership, mtime, and mode immediately before
moving it.

## Cache and latency boundaries

| Source | Current strategy |
|---|---|
| fleet identity (`ps` + bulk `lsof`) | slow background cache |
| Run liveness/state | live syscalls on gather |
| native Sessions | slow background cache |
| Skill hashes | 30-second background cache with metadata reuse |
| relationship join | once per gather into `sessions-linked` |
| `a:` / `s:` filtering | cached snapshot, no Agent CLI per key |

Gather has a 40 ms deadline. A late fast source falls back to its previous cache
and updates that cache in the background. Explicit commands such as Doctor,
Control, Diff, and export may do slower work because the user asked
for it directly.

## Privacy and authority rules

- Native Session files, Agent CLI output, and live process state are evidence;
  Prelude metadata is an overlay.
- Full process command lines and Run launch arguments are transient parsing
  input and are not retained; native Session titles remain native conversation
  metadata and may reflect opening user text.
- Skill archive/Favorite files contain stable keys only.
- Destructive native-file operations canonicalize ownership boundaries, confirm
  with Cancel first, move to Trash, and recheck live state where relevant.
- Capability replacement requires a visible comparison and fresh evidence.

## Known limitations

- OpenCode Sessions are not discovered, although a known resume command exists:
  they live in a SQLite `~/.local/share/opencode/opencode.db`, and a database
  engine on the launch path is a dependency this does not spend.
- Copilot, Qwen, Amp, Qoder, Kilo, Mastra, Grok, Cline, Droid, Antigravity and
  Kiro are installed on the machine this was written on and keep no local
  conversation store that could be found, so there is nothing to discover
  rather than something unimplemented.
- Cursor Sessions carry no title: the transcript is SQLite and is not read.
- Kimi and Cursor have no known fork syntax, so no fork action is offered.
- Working/waiting is inferred from process liveness, Session-file silence, and
  the last structured turn; batch/no-clock Runs cannot be classified as waiting.
- No installed Agent CLI reports the resolved config of another live process.
- Two Agents in one cwd share one message inbox address, so `say` refuses the
  ambiguous live target rather than guessing.
