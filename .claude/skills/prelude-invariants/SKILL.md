---
name: prelude-invariants
description: Adversarially review a diff against Prelude's load-bearing invariants — the traps in CLAUDE.md that have each already caused a bug. Use before committing any change to src/, and as the reviewing half of the Raycast-parity loop. Default to objecting when unsure.
---

# Reviewing a Prelude diff

Your job is to **refute** the claim that this diff is safe. Every rule below
is here because breaking it already cost somebody a debugging session, and
most of them fail quietly — the launcher keeps working and is simply wrong.
That is why an automated loop needs this pass: the tests catch what is
testable, and these are largely not.

Read the diff, then work the checklist. For each objection give the file and
line and the concrete failure — inputs or state, then the wrong result. If you
are unsure whether a rule applies, object. A false objection costs one reply;
a missed one is a bug that nobody reports because nothing looks broken.

## 1. What does a keystroke now cost?

fzf re-invokes the binary on **every keystroke** through a transform binding.

- Does anything on the per-keystroke path (`is_special`, `_dynamic`,
  `_footer`, `_focus`, `_bind`, `_setting-key`) start a subprocess, touch the
  network, or read the full file index? All four are refusals.
- `is_special` recognises intent and must not calculate. The exact-quicklink
  lookup is the one admitted exception, and it is admitted because it was
  measured.
- Did a new dependency arrive in `Cargo.toml`? The rule is about what a
  keystroke pays. A library walk with no startup cost can be fine; anything
  linking work into process start is not.
- Did a source move into or out of `FAST`? FAST membership *is* the
  performance decision — the floor is the slowest FAST source. Anything over
  ~25 ms belongs behind the cache tier.
- Settings gather may use file checks and stats. Never `pgrep`, never another
  subprocess.

Ask for the `bench` and `bench --sources` numbers if the diff touches gather.

## 2. Did the layout gain a second opinion?

Widths and the title column are computed once and passed down, because the
per-keystroke helper runs in a **separate process**. If both sides measure,
they drift and computed rows land in a different column from static ones.

- A new field: does it go into the existing flexible column, or did it grow a
  new one? A column for a field few rows carry moves the dots for every row.
- Width taken with `width::dwidth`, never `len()` or `chars().count()`? CJK is
  two columns and East Asian *Ambiguous* characters are one or two.
- `f:`'s filename column stays width-derived, not result-derived.

## 3. Is the band still the band?

`cache::by_rank` compares **kind first, then frecency**. They were once added
into one number and the arithmetic could not hold: the agent cluster spans 25
points while the bonus reaches 60.

- Any new single total, or any tuning of the cap to make a case work? Both are
  the same bug returning.
- A source ranking its own kind must write `rank` into `data`, not only add to
  `score` — `read_cached` rebuilds the score, so a rank applied but not
  recorded vanishes on the next read.
- A Quicklink is banded by the person having named it (`Item::band`), not by
  what it points at.

## 4. Can this leak a credential?

- Any new source reading user data routed through `secrets.rs`?
- Does an `Item` or a cache now retain an MCP command line, env block, header,
  URL, process command line, or prompt? All of those can hold credentials —
  cache a redacted fingerprint, and ask the owner CLI again on an explicit
  action.
- A quicklink target or `{q}` template that looks credential-bearing must be
  refused, checked with the braces substituted.
- Clipboard image bytes stay 0600 under Prelude's own data directory, never
  OCRed, indexed or transmitted.

## 5. Does it write, and does it write correctly?

- `write_if_changed` for caches; the bytes are usually identical and writing
  measured slower than gathering. But staleness is then the newer of the cache
  mtime and `refresh/<name>.stamp` — a stable source must not look stale
  forever.
- `write_atomic` temp names carry pid, counter and clock, created with
  `create_new`. One name per destination means two Preludes splice two answers
  into one whole file.
- `write_state` (0600, flushed before the rename) for anything a person cannot
  rebuild: bus, favorites, frecency, the capability archive.
- Read-change-write holds `lock_for_write`, which gives up after 250 ms — a
  launcher may lose a use count but may never hang.
- **Outside Prelude's own config and caches, writing a user file is a closed
  list**: `global install/start/update`'s marked Ghostty block, capability
  install, raw session export, and moving something to the Trash. Does the
  diff add to that list? That is a decision, not an implementation detail.

## 6. Empty, or failed?

A source returns `Vec<Item>`, so "no servers" and "the CLI timed out" arrive
identically — and writing the second erases the inventory it was refreshing.

- New source that can fail: does an empty result coinciding with a lost
  command keep the last good answer (`exec::lost_commands`)?
- Aggregated source: does a partition that could not be asked call
  `exec::note_incomplete` so `cache::carry_over` keeps its rows? The
  whole-result guard cannot see a partial failure.
- A non-zero exit that still produced records is an answer. Output that will
  not parse is never an answer, whatever the exit code.

## 7. Sources degrade to nothing

Never blocks, never panics, never prints. A deadline goes on the process and
the kill goes to the **process group** — an agent CLI is routinely a script
and an MCP server is `npx` starting `node`. Child output is drained on a
helper thread and capped at `MAX_OUTPUT`; keep stderr.

## 8. Do the two entry points still agree?

Ctrl+R in zsh and the global chord are one surface. `defaults::surface()`
always returns `Clipboard`; both stand in `global::launch_directory`.

- Any new environment switch that can make them diverge? Refuse it.
- A new query source (a seeded query, a deeplink) must not change which
  directory the launcher stands in.

## 9. Keys and levels

- Escape means back, one level at a time, and belongs to Prelude. Do not
  reintroduce Ghostty's `unconsumed:escape`.
- Arrows are the query line's cursor keys first. Settings are the exception
  and route through `settings::adjust` — one semantic dispatch, not mutations
  scattered through fzf strings or `_bind`.
- A key that acts on every row needs the same per-kind rule the panel has.
  `runhere::can_run_here` is the one predicate; anything else is inert.
- `Ctrl+[` can never be a chord: it is 0x1b, the same byte as Escape.
  `Ctrl+]` is 0x1d and deliberately not 0x1f, which is `render::SEP`.
- `focus` alone is not enough — bind `load` to the same helper, or the first
  row after every reload carries the previous row's state.
- Does the footer now advertise a chord the row has no object for?
  `ui::object_of` is asked once and the chords and their labels read that one
  answer.

## 10. Refusal happens at the moment of naming

A key that is accepted and then silently unreachable is the worst outcome.
Anything that names something — an alias, a quicklink, a root — refuses a
scope prefix, a name another entry carries, and a duplicate, **when the person
types it**, with a reason. Lookups fold case. Names accept letters and digits
of any script.

## 11. Rows, identity, and the things that dedupe

- `finish` keeps the first of a duplicate `(kind, cmd)` pair. A new row source
  must make `cmd` unique per row, or two of them collapse into one.
- A cache and the presenter in front of it must not both be put in the list.
- Favorites and archive state carry object keys only — never paths, commands
  or definitions.

## 12. Adding a path, walking a directory

Anything that walks a directory the person did not choose goes through
`paths::is_protected`. Indexing `~` is seven levels of walk through
`~/Library`, and the TCC dialog that results names the terminal, not Prelude.

A pasted path is tried literally first, then unescaped. Path intent is lexical
in `is_special`; only `dynamic_rows_with` touches the filesystem.

## 13. Destructive actions

`ui::confirm` puts Cancel first. `paths::trash` moves and uniquifies — never
`unlink`, never `remove_dir_all`. A repair re-verifies its evidence before
acting, because minutes pass between a report being read and answered.

## 14. Did the record get updated?

- Agent, Run, Session, Skill, MCP, Config, Home, messaging or Agent doctor
  behaviour changed → `docs/AGENT-CONTROL-PLANE.md` in the **same commit**. A
  conversation summary is not a substitute.
- A parity item now passing → promoted to `done` in both
  `scripts/parity-check.sh` and `docs/PARITY.md`, same commit.
- A test that touches preferences addresses a temporary preference path and
  must not read the person's real `favorites.txt`.
- Is there a second supported-Agent list anywhere outside `agent.rs`, or an
  action advertised by spelling an Agent's name in another module?
