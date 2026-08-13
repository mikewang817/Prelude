---
name: prelude-verify
description: Verify a change to Prelude end to end — build, tests, clippy, the 40ms p95 gather budget, and the Raycast-parity scorecard. Run this before every commit and at the end of every loop iteration. Use whenever a change to src/ is claimed to be finished, or when asked whether the tree is green.
---

# Verifying a change to Prelude

This is the loop's stop condition made executable. Run all five. Report each
one's real outcome — a step that was skipped is reported as skipped, not as
passing.

```sh
cargo build --release
cargo test                                  # every test, not a filtered subset
cargo clippy --release                      # expected warning-free
./target/release/prelude bench              # p95 gather; non-zero when over 40ms
./scripts/parity-check.sh                   # the scorecard; non-zero on a regression
```

## What each one is actually protecting

**`cargo test`** — many of these tests exist because the thing they pin has
already been broken once. A failing test in this repository is far more often
a correct objection than a stale expectation. Do not adjust a test to match
new behaviour without saying, in the report, which invariant you decided was
wrong and why.

**`bench`** — fzf re-invokes the binary on every keystroke, so startup cost is
paid hundreds of times a session. The gate is p95 over forty samples after a
warm-up, not the median: a launch that is usually 6 ms and occasionally 59 ms
is felt as a launcher that hitches, and a median cannot see it.

If it fails, `bench --sources` is the instrument — it times every phase of one
gather and prints them sorted. The floor is the slowest FAST source, so
shaving anything else changes nothing, and the profile has to be re-read after
every win because the floor moves.

`bench --process` is the different question: a new process per sample, which
is the launch a person actually meets. Use it when the change touched startup
rather than a source.

**`parity-check.sh`** — see `docs/PARITY.md`. Two outcomes need action beyond
"it passed":

* A **FAIL** on a `done` item is a regression. Fix it in this iteration; do
  not promote anything else while one is red.
* A **READY** means an item marked `todo` now passes. Promote it to `done` in
  `scripts/parity-check.sh` and `docs/PARITY.md` in the same commit that made
  it pass. An item left at `todo` while passing protects nothing.

## Behaviour, without standing up a terminal

Do not try to drive fzf to check something. The internal doors exist for this:

```sh
LINE=$(./target/release/prelude _dump | head -1)
./target/release/prelude _footer  "$LINE"      # what the keys say for this row
./target/release/prelude _actions "$LINE"      # what ^K offers it
./target/release/prelude _dynamic "some query" # per-keystroke rows
./target/release/prelude _dump-root            # searchable root commands
./target/release/prelude _dump-all             # the complete catalogue
```

A rendered row is `<display>\037<json>`; take the second field and pipe it to
`jq`. `scripts/parity-check.sh` has the helpers.

The agent verbs are the same kind of door for the bus:

```sh
ID=$(./target/release/prelude ask --no-wait "proceed?")
./target/release/prelude inbox --human
./target/release/prelude answer "$ID" "go ahead"
```

## Before reporting done

`cargo build` does not change the running panel. The loop was started at login
and executes whatever the binary held then; each press spawns the new binary
to draw the list, so the rows update while the delivery decision stays old.
That failure reads as "the change did nothing" and lies convincingly.

If the change is one a person will press the key against:

```sh
./target/release/prelude global stop && ./target/release/prelude global start
```
