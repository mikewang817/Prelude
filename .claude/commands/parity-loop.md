---
description: Run one iteration of the Raycast-parity loop — pick the topmost unmet item, implement it, verify, review against the invariants, promote, commit.
argument-hint: "[item id, e.g. P6 — omit to take the topmost todo]"
---

Run **one** iteration of the parity loop. One item, start to finish. Do not
begin a second item in the same iteration; report and stop instead.

## 1. Pick

If `$ARGUMENTS` names an item, take that one. Otherwise run
`./scripts/parity-check.sh` and take the topmost `todo` in the order
`docs/PARITY.md` lists.

Two things end the loop rather than starting an iteration:

- **A `done` item FAILs.** That is a regression, and it is the whole
  iteration: fix it, verify, commit, stop. Promote nothing while one is red.
- **An item is READY** — passing while marked `todo`. Promote it in both
  `scripts/parity-check.sh` and `docs/PARITY.md`, commit that alone, stop.

Never touch a `spike`. Its note in `docs/PARITY.md` says what would unblock
it, and that answer is not yours to invent mid-loop.

## 2. Read before writing

Read the item's section in `docs/PARITY.md` in full. Each one names the shape
of the answer and, more importantly, the specific way it is expected to go
wrong — P7's warning about `by_rank` is not a style note, it is the bug that
will otherwise be reintroduced and will look like it works.

Then read the code the section names, and `CLAUDE.md` on whatever subsystem it
lands in.

## 3. Implement

Smallest change that makes the check pass honestly. Write code that reads like
the code around it.

**Do not weaken a check to make it pass.** If the check turns out to assert
the wrong thing, say so explicitly in your report, change the check *and* the
item's section in `docs/PARITY.md` together, and explain the reasoning — a
scorecard that gets edited to match the implementation is not a stop
condition.

Likewise do not adjust a test without naming, in the report, which invariant
you decided was wrong. In this repository a failing test is more often a
correct objection than a stale expectation.

## 4. Verify

Use the **prelude-verify** skill. All five steps, real outcomes reported —
a step that was skipped is reported as skipped.

## 5. Review

Use the **prelude-invariants** skill against `git diff`. Its job is to refute
the claim that the change is safe, so read its objections as objections: fix
what it finds and re-verify, or state plainly why an objection does not apply.
Do not proceed past an unanswered one.

If the diff is substantial, this is the point to get an independent pass on it
rather than reviewing your own work.

## 6. Promote and commit

In the same commit:

- flip the item to `done` in `scripts/parity-check.sh` and `docs/PARITY.md`
- update `docs/AGENT-CONTROL-PLANE.md` if Agent, Run, Session, Skill, MCP,
  Config, Home, messaging or doctor behaviour moved
- branch first if on `main`

## 7. Report

Say which item, what changed, the verify numbers (tests, clippy, `bench` p95),
what the invariants review objected to and how each was answered, and what the
next `todo` is.

If you could not finish — blocked, or the item turned out to need a decision
the scorecard does not contain — say so and stop. Do not skip ahead to an
easier item; the order is the point, and a loop that quietly reorders itself
is a loop nobody can read.
