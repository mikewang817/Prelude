# Agent Control Plane implementation plan

This is the execution record for Prelude's Agent Control Plane. It is not a
vision document: every checkbox is an implementation obligation, and every
status change must name its evidence.

The source of truth is this file, not a conversation summary. Before changing
Agent, Run, Session, Task, Skill, MCP, Config, Home, messaging or Agent doctor
behaviour, read this file and work from **Current milestone** below.

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

### 1. Stable Agent / Run / Session graph — `[>]`

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

Still required before the full relationship model is complete:

- [ ] Task entity and Run/Session/Question → Task edges (Milestone 6).
- [ ] Run → explicitly loaded Skill/MCP edges (Milestone 5).
- [ ] Evidence for effective Config, where an Agent exposes it (Milestone 5).
- [ ] Reverse indexes for all new Task and Capability edges.

Evidence: `1c18cb2`, `src/control.rs`, relationship tests in `src/main.rs`.

### 2. Session lifecycle — `[>]`

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

Remaining:

- [ ] Resume with an explicitly selected Skill.
- [ ] Resume with an explicitly selected MCP where the Agent supports it.
- [ ] Portable transcript export distinct from raw native JSONL.
- [ ] Detect duplicate Session IDs/files.
- [ ] Detect missing project directories and malformed/unreadable Session indexes.
- [ ] Group/filter by project, Agent, time and lifecycle state.
- [ ] Define archive interaction for a Session that later becomes active.

Evidence: `fb10c99`, `src/sources/sessions.rs`, `docs/ACTIONS.md`.

### 3. Skill capability matrix — `[>]`

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

Remaining:

- [ ] Show source modification time per copy.
- [ ] Open all copies.
- [ ] Validate `SKILL.md` frontmatter and required entry file.
- [ ] Detect broken or escaping symlinks explicitly.
- [ ] Detect same-name/case collisions and folder-name/frontmatter-name conflicts.
- [ ] Add targeted Skill diagnostics to `prelude doctor skills`.

Evidence: `02f3d65`, `src/capability.rs`, `src/sources/agents.rs`.

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

### 5. Run effective context — `[ ]` **Current milestone**

Acceptance criteria:

- [ ] Run Quick Look shows Agent, project, branch, Session, Task, state and start time.
- [ ] Parse explicit one-run Skill/MCP flags without retaining the full command line.
- [ ] Link only explicitly confirmed borrowed/loaded capabilities.
- [ ] Distinguish installed inventory from capabilities confirmed for this Run.
- [ ] Read Git branch without spawning Git on the gather or per-key path.
- [ ] Show last verified structured event, then conversation evidence as fallback.
- [ ] Show model only if a native structured source confirms it.
- [ ] Never guess token usage or cost.
- [ ] Add graph reverse edges from Capability/Task/Session to Run.

### 6. Task and structured events — `[ ]`

Acceptance criteria:

- [ ] Immutable Prelude Task ID and atomic Task metadata.
- [ ] States: queued, working, waiting, done, failed, cancelled.
- [ ] Project, title, prompt reference, assignment, dependencies and timestamps.
- [ ] Result and related Question/Message/Run/Session edges.
- [ ] CLI: `task start`, `progress`, `done`, `fail` with JSON output variants.
- [ ] Assign, reassign, retry, continue elsewhere, send context, mark done, cancel.
- [ ] Append-only local structured event JSONL.
- [ ] Agent event is authoritative over inferred process state.
- [ ] Orphan handling for a Task whose Run or Session disappeared.
- [ ] Prompt/result content follows the credential filtering policy.

### 7. Agent Home and scoped control queries — `[ ]`

Home ordering acceptance criteria:

1. [ ] Explicit unanswered human Questions.
2. [ ] Failed/abnormally exited Agent Tasks.
3. [ ] Completed Tasks awaiting review.
4. [ ] Waiting Agents.
5. [ ] Working Agents.
6. [ ] Queued Tasks.
7. [ ] Agent launch entries.

Additional criteria:

- [ ] Healthy inventory remains in explicit scopes.
- [ ] Broken MCP and divergent/invalid Skill exceptions may enter Home.
- [ ] `a:waiting`, `a:failed`, `a:claude Prelude`.
- [ ] `a:using <capability>` and `a:without <capability>`.
- [ ] Filters operate on one cached snapshot and start no Agent CLI.

### 8. Persistent messaging and task handoff — `[ ]`

Acceptance criteria:

- [ ] Thread ID and reply-to.
- [ ] Delivered, Read and Answered states.
- [ ] Optional Task, Run and Session edges.
- [ ] Persistent Inbox delivery when no tmux address exists.
- [ ] File-path attachments without copying file contents.
- [ ] Timeout, cancellation and reassignment.
- [ ] Agent-to-Agent Task handoff.
- [ ] Completion result returns to the originating Inbox/thread.
- [ ] Identity remains discovered, never manually declared.
- [ ] Exact recipient resolution remains mandatory.

### 9. Maintenance and specialized Doctor — `[ ]`

Commands:

- [ ] `prelude doctor agents`
- [ ] `prelude doctor sessions`
- [ ] `prelude doctor skills`
- [ ] `prelude doctor mcp`

Checks:

- [ ] Agent executable/version/login/config readability.
- [ ] Broken Session index and missing project.
- [ ] Run without Session and ambiguous relationships.
- [ ] Skill collisions, divergence, invalid frontmatter and broken symlinks.
- [ ] MCP failure, duplicate, auth, stale health and tool inventory.
- [ ] Broken private borrow shims and stale staged files.
- [ ] Orphan Task, event, Inbox and relationship records.
- [ ] Doctor reports by default; every repair is a separate confirmed action.

## Current execution queue

Work must proceed in this order unless this document records a reason to change it:

1. Run explicit Capability extraction without retaining process arguments.
2. Git branch evidence without spawning Git on gather or per-key paths.
3. Effective-context Quick Look and graph reverse edges.
4. Task/event persistence and CLI.
5. Agent Home ordering/filtering on Task evidence.
6. Persistent threaded messaging and handoff.
7. Specialized Doctor and remaining Session/Skill maintenance gaps.

## Validation baseline

As of 2026-08-08:

- Branch: `feature/agent-control-plane`
- Tests: 55 passing
- Release Clippy: warning-free
- Gather benchmark: median 19.5 ms, max 23.1 ms, budget 40 ms
- Control schema: 2
- Working implementation commits:
  - `1c18cb2` — stable Agent/Run/Session graph
  - `fb10c99` — Session lifecycle metadata and actions
  - `02f3d65` — Skill/MCP integrity matrix foundation
  - `b5e4c39` — MCP transport, tools, refresh, public Diff and diagnostics

## Progress log

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
