# Raycast parity — interaction polish

This is a scorecard, not a wish list. Every row is decided by
`scripts/parity-check.sh`, which is what makes it usable as the stop condition
of a loop: the loop picks the topmost `todo`, implements it, and the check
tells it whether it is done. A row that cannot be decided by a machine does
not belong here — it belongs in an issue.

## What parity means here, and what it does not

Raycast is a native macOS application with a window, a React extension SDK and
a store. Prelude is a Rust program that renders a list of text through fzf.
Most of Raycast's catalogue is already matched or beaten — clipboard history
with typed records and image payloads, snippets, quicklinks with `{q}`
templates, a calculator, shell history, `$PATH`, listening ports, processes,
containers, and an Agent control plane Raycast has nothing corresponding to.

What Raycast is better at is **the feel of driving it**: naming things you use
often, getting to them without going through the front door, being told where
you are, and being told what just happened. That is this track. It is worth
doing precisely because it is not a feature list — none of these items would
show up in a comparison table, and all of them are felt on every use.

### Out of scope, with reasons

These are Raycast features that will not be built, so the loop does not
rediscover them every pass:

* **Window management, colour picker, floating notes.** Each needs a GUI
  canvas. Prelude's output is one column of text in somebody else's terminal.
  Building these means shipping a Swift helper, which is a different product.
* **An extension store.** The moat, and a separate track — it has to answer
  how an external process fits inside a 40 ms gather before any of it can be
  designed. Almost certainly the cache tier, and almost certainly not this
  loop.
* **Cloud sync.** Prelude keeps state in XDG paths and 0600 files and does not
  reach the network except for the update check, which the README documents as
  the one exception. Sync would make that paragraph false.

## Scorecard

| ID | Item | Status |
|---|---|---|
| P1a | Fallback row: is the query, survives, absent in a scope | **done** |
| P10 | Object chords appear on object rows and nowhere else | **done** |
| P1b | Fallbacks are an ordered, configurable list | todo |
| P2 | Alias any row, refused at the moment of naming | todo |
| P3 | The alias shows on the row it belongs to | todo |
| P4a | Launch with a query already typed | todo |
| P6 | The action panel names the row it is acting on | todo |
| P7 | Pin an app or a quicklink, inside its band | todo |
| P8 | A trashed object says where it went, and offers the way back | todo |
| P4b | A chord per command | spike |
| P5 | `prelude://` deeplinks | spike |

Suggested order is the table's: P6 and P7 are small and self-contained, P2/P3
are one feature in two halves, P1b and P4a each open a door the spikes need
later.

---

## P1a — the fallback row holds its three properties · done

Typing something the catalogue has nothing for must not draw an empty box.
`compute::web_search_row` already answers this, and the check pins the three
properties that make it safe to emit on every keystroke: its display text
**is** the query (titled anything else, the query that computed it would
filter it out), it is emitted **last** so `--tiebreak=index` leaves it under
anything that scored, and it is **absent inside a scope**, because a scope is
the person saying where to look.

This is `done` and stays checked because P1b edits exactly this code.

## P1b — fallbacks are an ordered, configurable list · todo

**Raycast**: a configurable, ordered set of fallbacks — search the web, ask
AI, open a specific extension — shown when nothing matches.

**Prelude now**: exactly one, following the `g` quicklink, with the built-in
template as the fallback for having deleted it.

**The shape of the answer**: an ordered list of quicklink keys. A `fallbacks`
scalar in `settings.toml` naming keys in order is the cheap version and the
one to build — it needs no new file, so `settings.rs` stays the single answer
to "what is this preference set to", and the entries are already validated
because they are quicklink keys.

Every entry must keep all three P1a properties. In particular each emitted row
still has to display the query, which means a fallback list is a list of
*providers*, not a list of arbitrary rows.

**Check**: `settings get fallbacks` parses, and the row count for an
unmatchable query equals the configured count.

## P2 — alias any row · todo

**Raycast**: any command can be given a short alias; typing it goes straight
there.

**Prelude now**: only a quicklink has a key. `is_special` resolves an exact
key ahead of the catalogue and `quicklink_with_neighbours` keeps the
catalogue underneath it. Nothing else — a scope, an app, a skill, a session —
can be named.

**The shape of the answer**: an `aliases.txt` of `alias → object key`, a
sibling of `favorites.txt` and built on the same stable object keys, which
already exist for Agent, Skill and MCP and are what P7 extends to the rest.
Resolution sits beside the quicklink exact-key path, not on top of it, and
must not calculate or shell out — it runs on every keystroke.

Refusal happens **at the moment of naming**, on the three grounds
`quicklink_conflict` already refuses: a scope prefix, a name another entry
carries, and a duplicate. That is not tidiness — an alias accepted and then
silently unreachable is the failure mode CLAUDE.md records for `[f]`. Keys
accept letters and digits of any script.

**Check**: `prelude alias list` works, and `alias add f:` and `alias add
claude` are both refused.

## P3 — the alias shows on the row · todo

**Raycast**: the alias renders on its row, so you learn it by seeing it.

**Prelude now**: nothing renders. You would have to remember what you named.

**The shape of the answer**: into `fields`, which the flexible column already
carries — not a sixth column. Column widths are shared across all kinds and
taken at a percentile; a new column for a field almost no row has would move
the dots for two thousand rows.

**Check**: at least one root row's payload carries `alias`.

## P4a — launch with a query already typed · todo

**Raycast**: a command can be opened directly, from a hotkey or a deeplink.

**Prelude now**: both entry points open on the home. Getting to `c:` is
always: press the chord, type `c:`.

**The shape of the answer**: `prelude open [--dry-run] QUERY` seeds the panel
or a new surface with a query. It is the half of P4b that this repository
owns, and P5's payload once something can deliver a URL.

Both entry points must keep standing in `global::launch_directory`. A query
seeded from outside is not a reason for the answer to depend on who seeded it.

**Check**: `prelude open --dry-run 'c:'` reports the query it would seed.

## P6 — the action panel names the row · todo

**Raycast**: the panel is titled with what you are acting on.

**Prelude now**: `actions.rs:936` builds the header as `Default: <verb> ·
Enter`. It states the verb and not the noun. You reach `^K` after typing a
query and arrowing through results, and the panel that opens tells you what
Enter does to a row it declines to name.

**The shape of the answer**: the focused row's title in the header beside the
default verb. The header is a line that cannot be selected, so this costs no
row and no column. Mind the width: CLAUDE.md already records that a header
long enough stops reading as a hint, and the title needs `width::dwidth`, not
`len()`.

This also needs an `_actions --header` door so the header can be read without
standing up fzf — the reason every other internal door exists.

**Check**: `_actions --header <session row>` contains that session's title.

## P7 — pin an app or a quicklink · todo

**Raycast**: pin any result to the top.

**Prelude now**: `favorite` appears on skill, mcp and agent rows and nowhere
else. An app and a saved quicklink both have stable identities and neither can
be promoted.

**The shape of the answer**: widen the object keys Favorites accepts. Nothing
about the mechanism changes — favorites stay Prelude preferences over object
keys, never carrying paths, commands or definitions.

**Promotion stays inside the Kind band.** Do not reach for `by_rank`. Kind
decides the band and frecency only orders inside it; the two were once added
into one number and the arithmetic could not hold, and a test now walks every
pair of kinds with the lower band given an absurd score. A cross-band pin
reopens exactly that hole, and it will look like it works right up until a
pinned config file sits above `claude`.

**Check**: app and link rows offer `favorite`.

## P8 — say where it went · todo

**Raycast**: a toast after a destructive action, with undo.

**Prelude now**: `ui::confirm` puts Cancel first, and `paths::trash` moves to
`~/.Trash` rather than unlinking, uniquifying rather than overwriting. So the
undo already exists — macOS owns it. What is missing is being told, and a way
back to it.

**The shape of the answer**: the confirmation names the destination, and the
row's `^K` grows a `show-trash`. Not a new undo mechanism: building one would
mean Prelude retaining what it moved, which is a worse answer than the one the
system already gives.

**Check**: an app row offers `show-trash`.

## P10 — the object chords hold · done

`ui::object_of` answers once — the path, and whether it is a directory — and
all three chords plus their footer labels read that one answer. Session and
skill rows carry them; an agent CLI has no object that is *theirs* and carries
none.

This is the regression guard under everything above it. P3 renders into a
column, P6 edits the panel, P7 adds an action: all three are one careless
change away from the footer advertising a chord that does nothing.

---

## Spikes

### P4b — a chord per command

Ghostty's `global:` keybinds run **Ghostty actions** — `toggle_quick_terminal`
and the rest — not arbitrary commands, and they take no argument. There is no
second action to bind a second chord to, and no way to say "reveal, with `c:`
typed".

So this cannot be built the way P4a can. The routes are: a second hotkey
daemon as a dependency, which is a large answer to a small question and one
CLAUDE.md's stance on dependencies argues against; or P5's URL scheme plus
whatever the person already uses to bind keys, which costs Prelude nothing and
is probably the right shape. Resolve P5 first.

### P5 — `prelude://` deeplinks

A URL scheme is registered by `CFBundleURLTypes` in an application bundle.
Prelude is a bare binary and has no bundle. The installer already places
Ghostty in `~/Applications`, so a minimal stub `.app` that forwards to
`prelude open` is not without precedent — but it is a new category of file
written outside Prelude's own config and caches, and CLAUDE.md keeps an
explicit list of those. Adding to that list is a decision, not an
implementation detail.

Answer that first, then P4a is already the payload and P4b follows for free.
