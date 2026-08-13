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
| P2 | Alias a stable object, refused at the moment of naming | **done** |
| P1b | Fallbacks are an ordered, configurable list | **done** |
| P3 | The alias shows on the row, and has a manager | **done** |
| P7 | Pin an app or a quicklink, inside its band | **done** |
| P8 | A trashed object says where it went, and offers the way back | todo |
| P4a | Launch with a query already typed | spike |
| P4b | A chord per command | spike |
| P5 | `prelude://` deeplinks | spike |

Suggested order is the table's. P2/P3 are one feature in two halves. The three
spikes are one question wearing three hats and are answered together or not at
all; see below.

### Struck

An item removed because it turned out not to be a gap. Recorded rather than
deleted, for the same reason the out-of-scope list is: so the next pass does
not rediscover it.

**P6 — "the action panel names the row it is acting on".** It does already.
`panel()` builds the header as `Default: <verb> · Enter` and, two lines below,
passes ` <title> · <kind> ` as the fzf border label, which `base_args` renders
because `--border=rounded` and `--border-label-pos=3` are on every panel. The
submenu reuses that same `title` local, and `apply` names the object again in
any confirmation it raises. So the noun is stated at all three levels, already
through `width::dtrunc` and `width::flatten`.

The item was written after reading only the `header` variable and not the
`label` argument under it. The lesson is not about this panel: a scorecard
item asserting something is *missing* is the one shape that a quick grep can
confirm and cannot refute, and it needs the call site read before it earns a
row. The two `done` items were both written from an observed behaviour
instead, which is why neither was wrong.

Nothing was implemented for it, and no test was added. The invariant it was
groping at — every level names the row — is held structurally by the levels
sharing one `title` local, and a test over that would be a test of `format!`.

---

## P1a — the fallback row holds its three properties · done

Typing something the catalogue has nothing for must not draw an empty box.
`compute::fallback_rows` answers this, and the check pins the three
properties that make it safe to emit on every keystroke: its display text
**is** the query (titled anything else, the query that computed it would
filter it out), it is emitted **last** so `--tiebreak=index` leaves it under
anything that scored, and it is **absent inside a scope**, because a scope is
the person saying where to look.

This is `done` and stays checked because P1b edits exactly this code.

## P1b — fallbacks are an ordered, configurable list · done

**Raycast**: a configurable, ordered set of fallbacks — search the web, ask
AI, open a specific extension — shown when nothing matches.

**What was built**: a `fallbacks` key in `settings.toml`, an ordered list of
quicklink keywords defaulting to the one `g`, with a "When nothing matches" row
in the Search group. `compute::fallback_rows_from` is the pure function;
`web_search_row_from` is gone, because a name promising one row while returning
the first of several is worse than no name.

All three P1a properties hold **per row**, which is the part that could have
gone quietly wrong: a second provider titled anything but the query is
filtered out by the query that computed it, so it would be missing exactly when
it was wanted, and the check now asserts it for every row rather than the
first.

A keyword must name a quicklink containing `{q}` — a fixed target has nowhere
to put the query, which is what makes this a list of *providers* rather than of
arbitrary rows. Unusable keywords are skipped when used and reported by
`settings check`, never refused at write time: a quicklink deleted later would
otherwise make a saved setting retroactively unwritable.

**If nothing resolves, the built-in provider is emitted anyway.** An empty or
broken list must not be able to make a query dead-end — that is the one failure
this row exists to prevent.

The row's value column is the list *as stored*, not the resolved provider
names, because `get` prints that column and `set` has to accept what `get`
printed. The first attempt showed `Google` and could not be round-tripped. The
resolved names are in Details and anything unusable is called out in the Effect
column, the way an environment override already is.

**Cost**: `_dynamic` now initialises the memoised `prefs()` where it did not
before — one small file read. Interleaved A/B: Δp50 +0.19, +0.31, +0.12 ms on
~18 ms keystrokes.

**Check**: rewritten with its own fixture in a throwaway `XDG_CONFIG_HOME`, for
the reason P3's has one. It configures two providers and asserts two rows in
that order, each displaying the query; then configures a keyword that names
nothing and asserts one built-in row and a `settings check` warning; then that
a scope still gets none. Counting the person's configured entries would have
been wrong in a way that mattered: an entry naming a quicklink they deleted is
*correctly* skipped, so the count and the config legitimately disagree.

## P2 — alias a stable object · done

**Raycast**: any command can be given a short alias; typing it goes straight
there.

**What was built**: `aliases.rs`, an `aliases.txt` of `alias<TAB>object key`
where the key is exactly what `favorites::key` writes, so the two files speak
one vocabulary and neither knows the other's format. `aliases::target_of` sits
beside the quicklink exact-key lookup in `is_special` and `needs_static_items`,
and `compute::alias_rows` resolves the key against the catalogue.

The title changed from "any row". `favorites::key` answers for the objects
with an identity that outlives a gather — an Agent, a Skill, an MCP server, an
application, a saved Quicklink. A session, a file and a history entry have no
such identity, and naming one would mean storing a path in this file. Raycast
can alias anything because everything it lists is a command it defined;
Prelude lists the machine, and most of the machine has no stable name.

**Refusal happens at the moment of naming**, through `aliases::vet`, and every
caller goes through it *before* looking at the target. Four grounds: the key
itself (`normalize_quicklink_key` excludes `:` `/` `@` `.` and accepts letters
and digits of any script), a scope command, a built-in Agent's own name, a
keyword a Quicklink already carries, and a duplicate.

An alias whose object is absent from this gather resolves to nothing and falls
through to ordinary search, rather than standing in for an application that
has been uninstalled.

**Cost**: one memoised open and parse of a small file per process, which is
per keystroke. Measured by interleaving both binaries in one run rather than
in blocks — measured in blocks the machine's own drift was several times the
effect. Δp50 −0.02ms, +0.14ms, +0.04ms over three queries.

**Check**: `prelude alias list` works, and `alias add f:` and `alias add
claude` are both refused *by name*.

The check was tightened while this item was being built, and the reason is
worth keeping. It asserted only a non-zero exit, and the first implementation
satisfied it while resolving the target first and never examining the key —
the refusal said "nothing here is called *whatever*", which is true, useless,
and about the wrong half. Right answer, wrong question, and a check reading
exit status alone could not tell. It now requires the refusal to name the key,
which is the behaviour the item is actually about.

## P3 — the alias shows on the row, and has a manager · done

**Raycast**: the alias renders on its row, so you learn it by seeing it.

**What was built**: `aliases::decorate`, beside `favorites::decorate` in
`gather` and deliberately not in `cache::finish` — file scope calls that helper
per keystroke and must not read a preference file on every letter. It returns
immediately when there are no aliases, which is the common case, so the two and
a half thousand calls to `favorites::key` are never made; the phase does not
reach `bench --sources`' cutoff.

The name goes **first** in `fields`, where a Quicklink already puts the keyword
the person chose. No new column: `render_general` derives its widths from the
terminal, and `fields` are joined into the free-text detail it already draws.

**The trap this item nearly shipped**: `render_general` shows `fields` *or* the
subtitle, never both. Inserting a name into an empty `fields` therefore deletes
the only detail an application row has — `/Applications` vanished the first
time. The subtitle is carried into `fields` ahead of the insert, and both a
Rust test and the check assert it.

Resolution is live and the label is not. `is_special` reads `aliases.txt` each
keystroke, but the per-keystroke helper reads the cached catalogue snapshot, so
a new name works at once and appears on its row at the next gather — the cadence
a new Favorite already follows. `alias_rows` dedupes on the name as well as on
`(kind, cmd)`, guarded by the name being present at all, because a snapshot
written by an older build carries no marker to dedupe on.

**The second half P2 left here** is the Aliases settings row: the Favorites
shape, `←` remove, `→` add, Enter opening the generic collection manager, and
`prelude settings path aliases`. Adding an alias picks the object first and
asks for the name second — the object is the part a person can recognise, the
name is the only part that can be refused, and the refusal comes from
`aliases::vet`, the same door `prelude alias` uses, so the two surfaces cannot
disagree about what a name may be.

**A defect this item shipped, found while building P1b and fixed there.**
`defaults::name` holds a per-setting table of what Enter does, and the Aliases
row was not in it, so it fell through to `"Open the file"` while Enter actually
opened `manage_collection`. The footer described an action the key did not
perform. The panel test did not catch it — it asserts only that `^K` never
repeats Enter's label, and those two happened not to collide. Adding a
collection row means adding an arm to that table; the fallthrough now says so.

**Check**: the check was rewritten and now brings its own fixture in a
throwaway `XDG_CONFIG_HOME` — it names a real application, asserts the row
carries the name first in `fields` and did not lose its subtitle, removes it,
and then asserts the settings collection row exists. It previously grepped
`_dump-root` for any row carrying an alias, which asked whether the person
happened to have named something that happens to be a root command: a question
about their config rather than about the code, and one that failed on this
machine for exactly that reason while the behaviour was correct.

## P4a — launch with a query already typed · spike

Written as the half of P4b this repository owns, on the assumption that
seeding a query was Prelude's side of the problem and only the chord belonged
to Ghostty. That was wrong, and the measurement is in the spikes section
below: **nothing outside Ghostty can reveal the quick terminal**, so there is
no moment at which a seeded query would be seen.

The item is reclassified rather than struck. Its check is still the right
check and stays in the script, unrun, for whenever the answer to P5 arrives.
Nothing was implemented — see the spike note for what remains possible and
what it costs.

**Check** (not run while this is a spike): `prelude open --dry-run 'c:'`
reports the query it would seed.

## P7 — pin an app or a quicklink · done

**Raycast**: pin any result to the top.

**What was built**: `favorites::key` widened from Agent/Skill/MCP to include
an application, keyed by its name, and a saved Quicklink, keyed by the keyword
the person gave it. `parse` accepts the two new prefixes. Nothing about the
mechanism changed — favorites stay preferences over object keys and still
carry no path, command or definition.

`by_rank` was not touched, which was the whole risk. It compares `band()` and
then `score`, so the +1000 bonus can only move a row among its own kind; a
test now walks a favourited application and quicklink against an unfavourited
agent and asserts the agent still leads.

Three decisions worth keeping, each of which could have gone the quiet way:

* **What named it wins over what it points at.** A Quicklink aimed at an
  application is keyed as that quicklink, the same order `Item::band` uses.
  Keyed the other way it would stop being the thing that was named and would
  collide with pinning the application directly.
* **A template result has no key.** It carries its provider's keyword so the
  provider can be edited, but pinning one search would silently pin `g`.
  `Item::is_quicklink` already drew that line.
* **An application is keyed by name.** The bundle path is the only other thing
  on the row and is the one thing that may not be stored; a bundle identifier
  would mean an `Info.plist` read per application per gather.

The Settings collection manager moved from `gather_agents` to the full
catalogue in the same change. Without it the manager could remove an
application favourite it had no way to add, which is half a collection.

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
column, P7 adds an action, P8 adds another: each is one careless change away
from the footer advertising a chord that does nothing.

---

## Spikes

All three are the same question: **the panel can only be revealed by a key
Ghostty itself handles.** Measured, on Ghostty 1.2 (`+list-actions`,
`+help`, `+show-config --default`, and the binary's own strings):

* `toggle_quick_terminal` exists only as a **keybind** action. Keybind actions
  cannot be invoked from outside a running instance.
* The `+action` CLI surface is a closed list of fifteen — version, help, the
  five `list-*`, ssh-cache, edit-config, show-config, validate-config,
  show-face, crash-report, boo, new-window — and none of them reaches a
  running instance. `+new-window` answers *"not supported on this platform"*
  on macOS.
* There is no IPC, socket or listen configuration key.
* There is no `Quick Terminal` menu item, so there is no System Events route
  either — which follows from `macos-hidden = always`, the setting that keeps
  the panel out of the Dock and the switcher in the first place.

Until that changes, everything below is blocked on one product decision, and
it is P5's.

### P4a — launch with a query already typed

Two shapes were available and both are refused for reasons already settled
elsewhere:

**Seed the running panel.** The machinery exists — `refresh.rs` already holds
fzf's `--listen` socket and could POST a `change-query`. But Prelude cannot
then reveal it, so the query would sit in a hidden panel until the person
happened to press the chord, and they would arrive at a launcher already
filtered by something they typed in another context minutes earlier. That is
the exact failure `refresh.rs` refuses to cause on its own account: a
background change moving somebody's state is worse than the staleness it
fixes.

**Open a new surface with the query.** This works, and it is the design that
was removed. *A press reveals; it never creates* — the old launcher built an
application instance, a window and a login shell per invocation, 373 ms of
construction and teardown, and CLAUDE.md records that every launch-and-teardown
bug came from it. Reintroducing it for a seeded query buys a feature with the
defect the current architecture exists to have eliminated.

What remains possible and is **not** blocked: `prelude open QUERY` running the
launcher *inline, in the terminal that typed it*, with the query already in
the box. It contradicts nothing and it is what a `prelude://` handler would
call. But on its own it is a scripting door nobody asked for, and whether it
should exist before P5 is answered is the decision — not an implementation
detail, and not one to make while the loop is holding the pen.

### P4b — a chord per command

Ghostty's `global:` keybinds run Ghostty actions and take no argument, so
there is no second action to bind a second chord to and no way to say "reveal,
with `c:` typed".

The routes are a second hotkey daemon as a dependency — a large answer to a
small question, and one CLAUDE.md's stance on dependencies argues against — or
P5's URL scheme plus whatever the person already uses to bind keys, which
costs Prelude nothing and is probably the right shape.

### P5 — `prelude://` deeplinks

A URL scheme is registered by `CFBundleURLTypes` in an application bundle.
Prelude is a bare binary and has no bundle. The installer already places
Ghostty in `~/Applications`, so a minimal stub `.app` that forwards to
`prelude open` is not without precedent — but it is a new category of file
written outside Prelude's own config and caches, and CLAUDE.md keeps an
explicit list of those. Adding to that list is a decision, not an
implementation detail.

Answer that first. P4a becomes the payload and P4b follows for free — but the
order is fixed, because both of them are shapes of *how the answer is
delivered* and P5 is the question of whether Prelude gets to be a thing macOS
can deliver to at all.
