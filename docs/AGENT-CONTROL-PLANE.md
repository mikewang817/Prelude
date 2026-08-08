# Agent Control Plane implementation plan

This is the execution record for Prelude's Agent Control Plane. It is not a
vision document: every checkbox is an implementation obligation, and every
status change must name its evidence.

The source of truth is this file, not a conversation summary. Before changing
Agent, Run, Session, Task, Skill, MCP, Config, Home, messaging or Agent doctor
behaviour, read this file.

**All nine milestones are complete.** There is no current milestone. What is
left is written down as recorded limitations and deliberate deferrals under the
milestone that owns each one — they are decisions with reasons, not a backlog,
and reopening one means arguing with the reason. New Control Plane work starts
by adding a milestone here, not by editing an existing one's criteria.

## How this document is used

Status markers:

- `[x]` implemented, tested and documented
- `[>]` implemented in part; unchecked acceptance criteria remain
- `[ ]` not started
- `[!]` blocked; the reason must be written next to it

Rules for every Control Plane change:

1. Pick work from the current milestone; do not silently jump ahead.
2. Change the milestone to `[>]` before or with the first implementation commit.
3. Keep authority boundaries and latency constraints below intact.
4. Add tests for pure decisions and destructive boundaries.
5. Run tests, release Clippy, `git diff --check`, and the gather benchmark.
6. Update this file in the same commit with checked criteria and measured evidence.
7. Mark a milestone `[x]` only when every acceptance criterion is checked.
8. Record deliberate deferrals under that milestone rather than calling it complete.

## Non-negotiable architecture

### Authority

- Agent CLI output, native Session files and live processes are authoritative.
- Prelude may add stable IDs, labels, relationships, Task state and archive state.
- Prelude must not rewrite native Session records to make its graph convenient.
- Prelude must not invent model, token, cost, capability or state information.
- Evidence priority is: structured Agent event, conversation evidence, process/file clocks.

### Identity

- Agent: native name such as `claude`, `codex`, `pi`.
- Run: `agent:pid:process-start-time`.
- Session: `agent:native-session-id`.
- Task: Prelude-generated immutable ID.
- Skill copy: name, canonical path and content fingerprint.
- MCP capability: normalized name plus redacted semantic fingerprint per owner.

### Privacy and safety

- Prompts and full process command lines are transient parsing input, never graph data.
- API keys, env/header values and credential-bearing arguments never enter Items,
  search rows, ordinary caches, previews, Control JSON or shell command lines.
- Explicit private staging files must be `0600` under Prelude's private cache.
- Destructive file actions move to the Trash, validate canonical ownership boundaries,
  re-check live state where relevant, and put Cancel first.
- Capability synchronization always shows a comparison before replacement and never
  silently overwrites.

### Latency

- Gather median and maximum must remain below the 40 ms product budget.
- No Agent CLI, tmux query, directory hash or relationship join runs per keystroke.
- Process liveness and mtimes may be updated through syscalls on the live path.
- Session inventory, Skill hashes, MCP health/tools and relationship snapshots use caches.
- Task and event persistence uses ordinary atomic JSON/JSONL files unless those files
  prove unable to preserve consistency.

## Entity model

```text
Agent
 ├─ Run
 │   ├─ Session
 │   ├─ Task
 │   ├─ explicit Capability load (Skill / MCP)
 │   └─ effective Config evidence
 ├─ Session
 ├─ Task
 ├─ Skill copies
 ├─ MCP variants
 └─ Config

Question / Message
 ├─ sender and recipient Agent
 ├─ Run / Session when known
 └─ Task when known
```

`src/control.rs` is the canonical serializable graph. Launcher `Item`s are
views over graph facts and source inventory; they are not a second authority.

## Milestones

### 1. Stable Agent / Run / Session graph — `[x]`

Implemented foundation:

- [x] Stable Agent, Run and Session IDs.
- [x] Explicit native resume-ID matching.
- [x] Conservative cwd-latest fallback only for one same-Agent Run in a project.
- [x] Ambiguity is represented instead of guessed.
- [x] Run → Session and Session → active Run edges.
- [x] Active Session avoids a competing resume.
- [x] `prelude control [--json]` exposes a versioned graph.
- [x] Process prompts/full commands are absent from graph serialization.
- [x] Relationship join is cached once for `s:` rather than repeated per key.

Completed with the rest of the relationship model:

- [x] Task entity and Run/Session/Question → Task edges.
- [x] Run → explicitly loaded Skill/MCP edges.
- [x] Agent-level evidence for effective Config, recording per CLI what is
      actually exposed rather than inferring it from files.
- [x] Reverse indexes for all new Task and Capability edges: `SkillRecord.runs`,
      `McpRecord.runs`, `RunRecord.tasks`, `SessionRecord.tasks`.

**Recorded limitation — `Run → effective Config evidence` is not obtainable.**
A Run's effective configuration is its layered files *plus* the flags it was
started with *plus* its inherited environment, and the last two are precisely
what this plan forbids retaining. No installed Agent CLI reports the resolved
configuration of another process. What each one does expose was measured
rather than assumed: `codex doctor --json` reports genuinely resolved settings
(proved by `-c model=…` changing its answer and by `cwd` following the
directory); `opencode debug config` resolves per directory; `claude` has no
whole-configuration reporter and only `claude auto-mode config`, which is one
subsystem and is labelled as such; `pi config` is an interactive TUI with no
non-interactive form, so nothing is claimed for it. All of it is agent-level
and says so. Closing the run-level edge needs either a CLI that can be asked
about an existing pid, or — better, and already this plan's top evidence tier —
agents writing their resolved configuration into a structured start event.

Evidence: `1c18cb2`, `src/control.rs` (schema 3), `src/sources/agents.rs`
(`effective_config`, `read_effective`), relationship tests in `src/main.rs`,
`src/cache.rs::attach_runs`.

### 2. Session lifecycle — `[x]`

Implemented:

- [x] Pin / Unpin.
- [x] Local Rename and restore native title.
- [x] Archive / Unarchive as Prelude-only metadata.
- [x] Native Fork for Claude, Codex and pi; absent where syntax is unknown.
- [x] Active Run relationship shown in list and Quick Look.
- [x] Reveal authoritative native Session file.
- [x] Private raw JSONL export.
- [x] Safe Trash for inactive recognized native Session files.
- [x] Fresh fleet re-check before Trash.
- [x] `s:is:pinned`, `s:is:active`, `s:is:archived`, `s:is:all`.
- [x] Atomic `0600` metadata and private `0700` export directory.

Completed:

- [x] Resume with an explicitly selected Skill, built on `lend::skill_flags`
      and absent where the Agent has no one-run syntax.
- [x] Resume with an explicitly selected MCP where the Agent supports it,
      inheriting `lend`'s `--mcp-config=` and never-inline-an-env-block rules.
- [x] Portable Markdown transcript export distinct from raw native JSONL.
- [x] Detect duplicate Session IDs/files, in both directions.
- [x] Detect missing project directories, unreadable and malformed Session
      indexes, and unreadable native roots.
- [x] Group/filter by project, Agent, time and lifecycle state:
      `project:`, `agent:`, `since:` and `is:`, quote-aware, with an
      unrecognised filter collapsing the list rather than matching everything.
- [x] Archive gives way to a conversation somebody has resumed.

The archive rule, written down because it is a product decision rather than a
mechanism: an archived Session that acquires a live Run is visible again.
Archive states that a conversation is *finished*, and one somebody has just
resumed is not; hiding a live agent because of a label set weeks ago is the
launcher contradicting the machine. The flag is left set, so the row hides
again when the Run exits. `visible()` and the default `s:` view apply the same
rule — they disagreed once, which made a resumed conversation reappear on the
home while vanishing from the scope whose whole job is finding conversations.
`is:archived` still answers literally, because that is a question about
metadata.

Two things the transcript exporter learned the hard way. Claude Code delivers
shell output, slash-command metadata and injected file contents as ordinary
`user` turns, so an exporter that only skips `developer` and `toolResult`
roles copies all of it into a document whose header promises it does not.
And credential redaction cannot be line-by-line: a person writes the label on
one line and pastes the value on the next, so only the harmless line is ever
tested. A match now takes the whole run of non-blank lines it lands in.

Evidence: `fb10c99`, `src/sources/sessions.rs`, `src/secrets.rs`,
`docs/ACTIONS.md`, `prelude doctor sessions`.

### 3. Skill capability matrix — `[x]`

Implemented:

- [x] All owner paths retained per merged Skill.
- [x] Full effective-tree fingerprint including scripts, references and symlinks.
- [x] VCS/cache metadata excluded from identity and copying.
- [x] Credential-like paths/lines redacted before fingerprinting.
- [x] Background `skill-hashes` cache with recursive metadata stamps.
- [x] `single`, `identical`, `divergent`, `unknown`, `private-unknown` states.
- [x] Quick Look copy matrix with path, hash, file count and byte count.
- [x] Effective permanent-install and one-run-borrow targets.
- [x] Read-only recursive Diff between divergent copies.
- [x] Replacement only after Diff, fresh re-hash and confirmation.
- [x] Old target goes to Trash; half-copies do not remain installed.
- [x] Sensitive sources are not copied or lent.

Completed:

- [x] Source modification time per copy, taken during the walk that already
      visits every file, and shown in Quick Look and `doctor skills`.
- [x] Open all copies, from a complete deduplicated list rather than the first
      directory found per agent.
- [x] Validate `SKILL.md` frontmatter and required entry file, through the one
      parser the display path uses rather than a second with different rules.
- [x] Detect broken and escaping symlinks explicitly, without following them.
- [x] Detect same-name/case collisions within an agent root, and
      folder-name/frontmatter-name conflicts as a per-directory fault.
- [x] Targeted Skill diagnostics in `prelude doctor skills`.

Three notes worth keeping. The fingerprint is unchanged — `SKILL_POLICY` is
still `skill-tree-v4`, proved by hashing 21 real skills and 10 adversarial
fixtures before and after and diffing the results, including through a
symlinked `$HOME`. A case collision is only a collision *within one root*:
`~/.claude/skills/Deploy` and `~/.codex/skills/deploy` are different
directories in different parents and cannot collide on any filesystem, so
reporting them was a bogus finding with a test pinning it in place. And a
cache-reuse gate must never be keyed on a derived value: gating on
`modified > 0` re-hashed for ever any tree whose newest mtime was at or before
the epoch, which `touch -t 197001010000` produces in a westward timezone.
Records now carry an explicit version.

**Deliberate deferral.** `.env` at a skill *root* and `aws_secret_access_key`
are not redacted *by name*; they are caught by content when their bytes look
secret. Redacting them by name changes the hashed bytes of every tree
containing one, which is a `SKILL_POLICY` bump rather than a refactor. The gap
is asserted in a test and noted beside `sensitive_name` so the next bump can
take it.

Evidence: `02f3d65`, `src/capability.rs`, `src/sources/agents.rs`,
`prelude doctor skills`.

### 4. MCP capability matrix — `[x]`

Implemented foundation:

- [x] Owner and health variants grouped by normalized capability ID.
- [x] Redacted semantic/display fingerprints.
- [x] Complete definitions removed from Items, ordinary caches and Control JSON.
- [x] One-time scrub of legacy MCP/search caches.
- [x] Account-hosted definitions marked owner-only / non-portable.
- [x] Effective borrow/install targets account for portability and private fields.
- [x] Redacted comparison matrix in Quick Look and actions.
- [x] Replacement command is prepared only after comparison and inserted for review.
- [x] Existing Show tools route uses the owner CLI on an explicit action.
- [x] Login action appears for auth failures.

Completed acceptance criteria:

- [x] Record normalized transport: `stdio`, `http`, `sse`, hosted or unknown.
- [x] Record `health_checked_at` from the slow MCP snapshot.
- [x] Inventory actual stdio tool names/descriptions through MCP `tools/list` in the slow background cache.
- [x] Record `tools_checked_at` and distinguish unsupported, disabled and failed inventory.
- [x] Add `Test connection now` for Claude and an honest status refresh for Codex, never per key.
- [x] Verify Enable / Disable syntax before offering it. Current Claude and Codex MCP help exposes no server-level toggle, so no action is invented.
- [x] Produce a structural redacted definition Diff, not only summary plus hash.
- [x] Detect duplicate owner/name definitions in targeted diagnostics.
- [x] Add targeted MCP diagnostics through `prelude doctor mcp`.
- [x] Pin tests proving tool output and failures cannot carry secrets into caches.
- [x] Paginate `tools/list`, bound retained tools/descriptions, drain stderr without retaining it, and terminate the inventory server.
- [x] Represent HTTP/hosted tool inventory as unsupported when owner authentication is unavailable rather than reporting an empty successful list.

Definition-of-done evidence: `Ctrl+P`, Control schema 2 and `prelude doctor mcp`
now consume the same cached transport, health timestamp, tool timestamp,
portability and redacted public-definition records. On the validation machine,
`node_repl` reports three actual tools while hosted/HTTP definitions state why
tool inventory is unsupported.

### 5. Run effective context — `[x]`

Acceptance criteria:

- [x] Run Quick Look shows Agent, project, branch, Session, Task, state and start time.
- [x] Parse explicit one-run Skill/MCP flags without retaining the full command line.
- [x] Link only explicitly confirmed borrowed/loaded capabilities.
- [x] Distinguish installed inventory from capabilities confirmed for this Run.
- [x] Read Git branch without spawning Git on the gather or per-key path.
- [x] Show last verified structured event, then conversation evidence as fallback.
- [x] Show model only if a native structured source confirms it.
- [x] Never guess token usage or cost.
- [x] Add graph reverse edges from Capability/Task/Session to Run.

Branch comes from `.git/HEAD` read directly, walking ancestors and handling a
`gitdir:` file for worktrees and submodules, a detached HEAD (reported as a
short object id and labelled as detached, never as a branch name) and a cwd in
no repository at all. Measured at 20 µs inside a repository, memoised per cwd.

What each native format records about the model, established by reading real
files rather than assuming: claude puts `message.model` on every assistant
record and codex a `turn_context.payload.model` per turn, both in the tail; pi
writes `model_change` when a session opens, which is at the *head*, so pi and
only pi costs a second bounded read. `opencode` records nothing and is
therefore given no model line. Token usage is recorded by claude and codex and
is deliberately not read, summed or priced.

Two things this milestone got wrong first, both worth remembering. A
structured event is authoritative over the silence clock only while it is
*newer* than the clock — with no clock at all, an event of any age otherwise
pins the state for ever, and a batch run (which by design has no clock) was
reported as stuck by an event timestamped 1970, contrary to the rule in
CLAUDE.md. And `ps` joins argv with spaces, so an inline `--mcp-config={…}`
containing a space arrives as fragments: Prelude's *own* borrow command
recorded "an unnamed borrowed capability" until the parser learned to recover
server names from a bounded scan.

Evidence: `src/sources/running.rs`, `src/control.rs` (schema 3), `src/preview.rs`.

### 6. Task and structured events — `[x]`

Acceptance criteria:

- [x] Immutable Prelude Task ID and atomic Task metadata.
- [x] States: queued, working, waiting, done, failed, cancelled.
- [x] Project, title, prompt reference, assignment, dependencies and timestamps.
- [x] Result and related Question/Message/Run/Session edges.
- [x] CLI: `task start`, `progress`, `done`, `fail` with JSON output variants.
- [x] Assign, reassign, retry, continue elsewhere, send context, mark done, cancel.
- [x] Append-only local structured event JSONL.
- [x] Agent event is authoritative over inferred process state.
- [x] Orphan handling for a Task whose Run or Session disappeared.
- [x] Prompt/result content follows the credential filtering policy.

Ids are reserved with `O_EXCL`, so a check-then-write race cannot exist; 64
concurrent threads produce 64 distinct ids. `valid_id` is what stands between
`task show ../../../etc/passwd` and the filesystem, and it is applied to the
id *inside* a record as well as to the filename, because `save` would
otherwise trust a path the record supplied. "Continue elsewhere" and "send
context" are Milestone 8's handoff and `say`, reached from the Task panel.

The store is bounded on the read path, which is what lets a Task source sit on
gather at all: `home_tasks` reads an index of open work plus undismissed
results and **never** walks the directory, not even when the index is missing —
it hands that to a detached rebuild, because a foreground scan is 54 ms over
5,000 records and the whole launch budget is 40. `open_tasks` is the explicit
reader that may scan, and rebuilds the index it paid for. Both writers hold an
advisory lock, so a rebuild cannot lose a task opened beside it.

Three failures the event log had to survive before it was honest. Only
`detail` was length-bounded, so a 300 KB project name made the trim cut land
past the last newline and write back an empty file — 940 records to zero.
Every field is bounded now, and the trim drops oversized records wherever they
sit rather than cutting a prefix whose window one can occupy. The trim race
lost 3.5% of an 8-thread burst rather than the "one line" its comment claimed,
and now takes a `flock`. And the credential filter reached only two of nine
fields; the test that was supposed to prove otherwise exercised exactly those
two.

A path is not a sentence: filtering `cwd` and `project` with the prose rule
redacted a project directory legitimately named `token-service` or `secrets`,
so both use the path rule, which still refuses a path genuinely containing a
key.

Evidence: `src/task.rs`, `src/events.rs`, `src/cache.rs`, `prelude task --help`.

### 7. Agent Home and scoped control queries — `[x]`

Home ordering acceptance criteria:

1. [x] Explicit unanswered human Questions.
2. [x] Failed/abnormally exited Agent Tasks.
3. [x] Completed Tasks awaiting review.
4. [x] Waiting Agents.
5. [x] Working Agents.
6. [x] Queued Tasks.
7. [x] Agent launch entries.

Additional criteria:

- [x] Healthy inventory remains in explicit scopes.
- [x] Broken MCP and divergent/invalid Skill exceptions may enter Home.
- [x] `a:waiting`, `a:failed`, `a:claude Prelude`.
- [x] `a:using <capability>` and `a:without <capability>`.
- [x] Filters operate on one cached snapshot and start no Agent CLI.

The ordering interleaves two kinds — a failed Task above a waiting Run, a
queued Task below a working one — which `cache::by_rank` cannot express,
because it settles the kind band before it looks at anything else. Home is
therefore its own explicit ordering in `home_items`, and `by_rank` and every
`Kind::priority` are deliberately untouched. Search asks *what kind of thing is
this*; the home asks *what is outstanding*. They are different questions and
must not be made to agree. `home_rank` also gives the two states the plan does
not name — a waiting Task and a working Task — a slot beside their agent-state
counterparts.

"Awaiting review" is an explicit act, never inferred from a row having been
looked at: focus and Quick Look cross rows without any decision being made, and
a task is routinely opened while it is still running, so treating that as
acknowledgement would silently drop its later completion — failure in the
expensive direction. `prelude task ack` and a `Dismiss it` panel entry are the
act. A finished task holds a row for 24 hours, bounded to 8 per state, and the
bound is applied in the index writer rather than only in the answer, since a
bound applied after the read bounds nothing.

The home stopped being an inventory. A healthy Skill or MCP server is
reachable through `/name`, `a:`, `mcp:` and ordinary search but does not sit on
the empty query; a broken server or a Skill whose copies have drifted does,
because that is an exception somebody should be told about. A *deliberately
disabled* server is not a fault and does not — the home and `doctor mcp`
disagreeing about one server is worse than either being wrong alone.

The filters are pure, quote-aware, and read `run_skills` / `run_mcp` — the
capabilities a Run confirmed, never the installed inventory. An unrecognised
filter collapses the list rather than silently matching everything; the same
lie in the other direction is still a lie. Verified per keystroke by replacing
`PATH` with logging shims: every `a:` query spawns nothing.

Evidence: `src/compute.rs`, `src/cache.rs`, `src/task.rs`, `docs/SEARCH.md`.

### 8. Persistent messaging and task handoff — `[x]`

Acceptance criteria:

- [x] Thread ID and reply-to.
- [x] Delivered, Read and Answered states.
- [x] Optional Task, Run and Session edges.
- [x] Persistent Inbox delivery when no tmux address exists.
- [x] File-path attachments without copying file contents.
- [x] Timeout, cancellation and reassignment.
- [x] Agent-to-Agent Task handoff.
- [x] Completion result returns to the originating Inbox/thread.
- [x] Identity remains discovered, never manually declared.
- [x] Exact recipient resolution remains mandatory.

`answer: Some("")` was doing double duty as "delivered", which is why a notice
had to be described as born answered. States are explicit now, stored as a
word so an unknown one from a newer build degrades rather than making a
message unparseable, and every new field is `serde(default)` — an old message
file still parses, still reads as pending, and is still never swept.

The round trip is the feature, and three things nearly lost it. `sweep`
deleted the message a live Task points at, because a handoff becomes
*delivered* the moment it lands in a pane — so any task outliving a day lost
its result entirely; the sweep now protects the message edge of every open
Task. A second handoff left the first recipient holding a live instruction,
two agents on one job, so it now withdraws its predecessor. And a result
addressed only by pane and cwd was unreachable for an originator that had
since `cd`'d, so a message carries the originating agent as a last address.

Delivery flattens the text before it reaches tmux. A multi-line message
arrived as several *separately submitted* inputs, only the first carrying the
`[via prelude, from …]` attribution — which is exactly what that attribution
exists to prevent.

`bus::items()` runs on every gather and was the one unbounded reader: 3,000
messages put gather at 37.9 ms and 5,000 at 60.5 ms, and undelivered mail is
never swept, so that ceiling was permanent. It reads a pending-question index
now, on the same terms as the Task store — never scanning, handing a missing
index to a detached rebuild. 5,000 messages went from a 68.8 ms median to
15.0 ms.

Thread order is the reply chain, walked depth-first from the opener. Sorting
on whole seconds gave three different orderings across six identical runs, and
`thread --json` is the form an agent reads to pick up handed-over work.

Evidence: `src/bus.rs`, `src/fleet.rs`, `src/task.rs`, `prelude thread`.

### 9. Maintenance and specialized Doctor — `[x]`

Commands:

- [x] `prelude doctor agents`
- [x] `prelude doctor sessions`
- [x] `prelude doctor skills`
- [x] `prelude doctor mcp`

Checks:

- [x] Agent executable/version/login/config readability.
- [x] Broken Session index and missing project.
- [x] Run without Session and ambiguous relationships.
- [x] Skill collisions, divergence, invalid frontmatter and broken symlinks.
- [x] MCP failure, duplicate, auth, stale health and tool inventory.
- [x] Broken private borrow shims and stale staged files.
- [x] Orphan Task, event, Inbox and relationship records.
- [x] Doctor reports by default; every repair is a separate confirmed action.

Each check returns data and is rendered twice, so `--json` duplicates no logic
and cannot drift from the table. Login state is reported as each CLI actually
answers it and no further: claude's `auth status --json` also returns email,
org id, org name and subscription, of which two fields are read and a test
asserts no account identity reaches the report; pi authenticates per provider
and so is reported *unknown* with the reason, rather than picking one provider
and passing its answer off as pi's.

Only two repairs exist, both on files Prelude wrote itself, and both re-verify
the finding at apply time rather than only the boundary — staging names are
deterministic, so a borrow staged while the confirmation is on screen would
otherwise be trashed by an answer about the previous file. `--repair` without a
tty declines instead of hanging, which it did for two minutes before it was
caught, and `--json --repair` together are refused rather than ordered.

The trap this milestone had to be told about: `cache::finish` dedupes on
`(kind, cmd)`, and two files sharing one Session id produce an identical `cmd`
— so `duplicate_sessions` handed the *finished* launcher list finds nothing by
construction, which is the exact case it exists for. It is fed `sessions::all()`,
and a test proves the distinction by running both.

Evidence: `src/doctor.rs`, `src/sources/agents.rs`, `src/capability.rs`,
`src/sources/sessions.rs`, `src/task.rs`.

## Recorded limitations

Each is a decision with a reason, written down so it is argued with rather than
rediscovered. None is a pending task.

1. **`Run → effective Config evidence` is not obtainable** with today's Agent
   CLIs. Milestone 1 records what each one exposes and what would close it.
2. **`.env` at a Skill root is redacted by content, not by name.** Redacting it
   by name changes hashed bytes and needs a `SKILL_POLICY` bump (Milestone 3).
3. **No server-level MCP Enable/Disable action**, because current Claude and
   Codex help exposes no such command (Milestone 4).
4. **Tool inventory for HTTP and hosted MCP servers is `unsupported`**, which is
   deliberately not the same as a successful empty list (Milestone 4).
5. **Skill borrowing is absent for three of the eight Agent/capability
   pairings**, because those CLIs have no one-run flag. Offering nothing beats
   a command that fails after the launcher has closed.
6. **`fleet::watch` still reads the whole bus** on its multi-second loop. It is
   a daemon, not the gather path, and is the last unbounded bus reader.

## Validation baseline

As of 2026-08-09:

- Branch: `feature/agent-control-plane`
- Tests: 180 passing, and hermetic — no test reads or writes the user's real
  data directory, and none mutates the environment of a running process
- Release Clippy: warning-free, including `--all-targets`
- Gather benchmark: median 17.9–24.0 ms, budget 40 ms
- Gather under load: 5,000 Tasks, 5,000 bus messages and a 512 KB event log
  together measured 15.0 ms, against 68.8 ms before the read paths were bounded
- Control schema: 3
- `#[allow(dead_code)]` in `src/`: none
- Working implementation commits:
  - `1c18cb2` — stable Agent/Run/Session graph
  - `fb10c99` — Session lifecycle metadata and actions
  - `02f3d65` — Skill/MCP integrity matrix foundation
  - `b5e4c39` — MCP transport, tools, refresh, public Diff and diagnostics

## Progress log

### 2026-08-09 — Milestones 1, 2, 3, 5, 6, 7, 8 and 9 completed

- Closed the last Session lifecycle gaps: resume with a borrowed Skill or MCP
  server, portable Markdown transcript export, duplicate and broken-index
  detection, project/agent/time/state filters, and the archive-gives-way rule.
- Closed the Skill matrix gaps: per-copy modification time, open all copies,
  frontmatter validation, explicit symlink faults, collision detection and
  `doctor skills`. Fingerprints proved byte-identical across 21 real Skills and
  10 adversarial fixtures; `SKILL_POLICY` unchanged.
- Added Run effective context: branch from `.git/HEAD` without spawning Git,
  one-run capability extraction that retains a name and nothing else, model
  only where a native structured field records it, structured events ahead of
  the silence clock, and reverse edges throughout. Control schema 2 → 3.
- Added the Task entity, the append-only event log, `prelude task`, and a
  read path bounded well enough to sit on gather.
- Made the Agent Home an attention list rather than an inventory, ordered by
  what is outstanding, with `a:` filters that run on one snapshot and start no
  Agent CLI.
- Made messaging persistent and threaded, with delivered/read/answered states,
  attachments that are paths rather than copies, expiry, cancellation,
  reassignment, Task handoff and results returning to the originating thread.
- Added `doctor agents|sessions|skills|mcp`, each with `--json` and a
  `--repair` that confirms one finding at a time and re-verifies it on apply.
- Recorded what each Agent CLI genuinely exposes about effective configuration,
  and why the per-Run edge is not obtainable, rather than inventing one.

Every milestone was reviewed adversarially before being checked off, and the
reviews are the reason several of these entries exist. Defects found and fixed
included: a transcript exporter that copied harness-injected file contents into
a document promising it did not; a credential filter that caught roughly one
shape in eight, and a test that certified the one it already caught; a cache
gate keyed on a derived mtime that would have re-hashed every Skill tree for
ever; a single oversized event that emptied the whole event log; seven Task
fields that carried credentials to disk; a day-old handoff losing its result
because the sweep deleted the edge it travelled along; multi-line messages
arriving at another agent as several unattributed inputs; batch runs reported
as stuck by an event timestamped 1970; and Prelude's own borrow command
recording "an unnamed borrowed capability".

### 2026-08-08 — MCP inventory and diagnostics completed

- Added normalized transport and authoritative health timestamps.
- Added a five-minute background MCP stdio handshake with paginated `tools/list`.
- Retained only bounded, credential-filtered tool names/descriptions and generic errors.
- Added health/tool refresh actions, structural public-definition Diff and `doctor mcp`.
- Verified current Claude/Codex help has no server-level Enable/Disable command;
  Prelude therefore offers none.
- Advanced the current milestone to Run effective context.

### 2026-08-08 — plan made explicit

- Converted the original ten-part proposal into tracked acceptance criteria.
- Corrected milestone status: Session, Skill and MCP work is substantial but not complete.
- Set MCP completion, not Run Context, as the current work item.
- Added the rule that no future Control Plane milestone may be called complete while
  unchecked criteria remain in this file.
