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
| P5 | `prelude://` deeplinks, which act rather than show | **done** |

**The decision the three spikes were waiting on has been made**: `prelude://`
exists, it acts rather than opening a launcher, and `global install` puts it
there. P4b folds into P5 — a chord bound to `prelude://run?alias=…` in whatever
hotkey tool the person already has costs this repository nothing — and P4a is
struck, because the only shape of it that survives that decision is not worth
building. Three items became one.

P2/P3 were one feature in two halves.

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

**P8 — "a trashed object says where it went, and offers the way back".** Both
halves already hold.

*Saying where it went*: all three destructive paths print the exact
destination, because `paths::trash` returns it and every caller uses it —
`moved to ~/.Trash/Chrome.app` for a file or application, the same for a
native Session, and `review deleted — now in ~/.Trash/review` for a Skill
copy. Each confirmation already puts Cancel first and already names the
destination and the consequence: *recoverable from Finder*. And the message
reaches a person on **both** entry points, which is the part that could have
been half-true: the panel prints it and holds for `CONFIRM`, and the zsh
widget uses `zle -M`, which sits below the prompt until the next keystroke.

*The way back*: `~/.Trash` is already a first-class Folder object. Typing the
path Prelude has just printed gives a row whose Enter opens it in Finder, with
Reveal, Copy path and Open terminal here beside it — and since P2 it can be
given a name, and since before that a Quicklink keyword.

The proposed `show-trash` action was refused rather than built, and that is
the part worth keeping. It would put a *global* destination-opener on every
deletable row, present before anything had been deleted and unrelated to the
row carrying it. Every other entry in `actions_for` is about the item it is
on. Raycast's toast-with-undo has a weaker guarantee than what is already
here: an undo that expires, against a Trash that does not.

The misreading is the same shape P6 was struck for — an item asserting
something is *missing*, written from a grep instead of from the call sites.
Both were authored in the same sitting, before either had been tested, so
this is one original error found twice rather than a lesson ignored. Of the
ten items on this scorecard, the two that were wrong were the two phrased as
absences; the six that survived were phrased as observed behaviour. That is
the rule to carry into any future scorecard.

**P4a — "launch with a query already typed".** Struck by the P5 decision
rather than by evidence, which makes it a different kind of removal from the
two above: the gap was real and stays real. The launcher has no way to start
with a query in the box.

Of its two shapes, one is impossible and one is refused. Seeding the running
panel cannot work because nothing outside Ghostty can reveal it, so the query
would wait in a hidden panel and ambush whoever pressed the chord next —
exactly the failure `refresh.rs` declines to cause on its own account. Opening
a new surface works and is the design `A press reveals; it never creates` was
written to have removed.

What survived was `prelude open QUERY` running the launcher inline in the
terminal that typed it. With `prelude://` deciding to act rather than to show,
nothing needs that as a payload any more, and on its own it saves a person at
a shell two keystrokes over `Ctrl+R` — which is not a CLI verb's worth of
surface. If a reason for it appears later, the mechanism is a `--query` on the
main list's fzf arguments and nothing else.

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

## P10 — the object chords hold · done

`ui::object_of` answers once — the path, and whether it is a directory — and
all three chords plus their footer labels read that one answer. Session and
skill rows carry them; an agent CLI has no object that is *theirs* and carries
none.

This is the regression guard under everything above it. P3 renders into the
detail column and P7 adds an action, and either is one careless change away
from the footer advertising a chord that does nothing. This sentence has now
been rewritten twice because it named items that were later struck; name the
work, not the item numbers, if it needs rewriting again.

---

## P5 — `prelude://` deeplinks · done

**Decided, then built.** The scheme exists, it **acts rather than showing a launcher**, and
`global install` puts it there. What follows is the record of that decision and
of the mechanism, which was proven end to end before being written down.

### It acts; it does not open a launcher

The panel cannot be revealed from outside — measured, and kept below — so a
URL that wanted to show a filtered launcher would have to *build* one, which is
the design `A press reveals; it never creates` exists to have removed. So a
deeplink does the thing instead: `prelude://run?alias=browser` opens Chrome,
with no interface in between.

That is also what a per-command hotkey actually wants, and it composes with the
names that already exist. An alias is a stable name for an object; binding a
chord to `prelude://run?alias=…` in whatever hotkey tool the person already has
*is* P4b, at no cost to this repository, which is why P4b is folded in here
rather than kept as its own item.

The price, accepted: a **scope** is not an object, so `prelude://` cannot open
the clipboard history the way Raycast's hotkey does. Nothing that has no stable
name is reachable this way.

### A URL is untrusted input, and this is the part to get right

**Any web page can navigate to `prelude://…`.** That makes the verb table a
security boundary, not a convenience:

* Never accept a path, a command, a template or a target from the URL. The only
  thing a URL may name is something the person themselves created — an alias,
  which resolves through `aliases::target_of` to an object key and nothing
  else.
* An unknown verb, an unknown alias, or a malformed URL does nothing and says
  so where the person can see it. It must not fall through to a search.
* Decide explicitly whether a resolved object *acts* or is *handed over*. The
  launcher's own rule is that objects act and commands are copied; a Skill's
  Enter copies text, and a URL arriving from a web page should not be able to
  put text on the clipboard silently. Prefer the narrower answer.

### Mechanism, proven on this machine

`osacompile` an applet whose script is `on open location this_URL`, add
`CFBundleURLTypes` and `LSUIElement` to its `Info.plist` with `PlistBuddy`,
then `lsregister -f` it. `open "prelude://run?alias=browser"` reaches the
handler with the **whole URL, query string included**. All three tools ship
with macOS: no compiler, no Xcode, no new dependency.

**The trap, and it is a quiet one.** `CFBundleURLTypes` delivers the URL as an
Apple Event (`kAEGetURL`) to a running application — *not* as `argv`. A stub
`.app` whose `CFBundleExecutable` is a shell script therefore registers
correctly, claims the scheme in `lsregister -dump`, and then silently never
runs. That was this item's previous plan, written from documentation, and it
does not work.

**Location is load-bearing too.** The identical bundle under `/private/tmp`
registers its claim and still answers `kLSApplicationNotFoundErr`; from
`~/Applications` it works. Both failures look the same from `open`, so test the
real location.

### Installation

`global install` generates `~/Applications/Prelude Link.app` and registers it;
`global uninstall` removes it **and** `lsregister -u`s it, or the scheme stays
claimed by a bundle that is gone. Same lifecycle as the managed Ghostty block
and the LaunchAgent, so there is no second thing to remember.

This adds an entry to the closed list in `CLAUDE.md` of what Prelude writes
outside its own config and caches. That list exists to be short and explicit;
update it in the same commit.

### What the building turned up

**An AppleScript injection, introduced and then caught by the review.** The
first refusal path repeated the URL's verb back into the notification, and
`bus::post` builds an AppleScript literal whose `escape` covers quotes and
backslashes but **not newlines**. `prelude://x%0Adisplay%20dialog%20"…"` ended
the `display notification` statement and started another one — arbitrary
AppleScript, from a link any web page can navigate to. The verb is no longer
echoed at all, `width::flatten` guards every other sentence, and a test pins
both halves. The alias route was already safe by construction rather than by
filtering, because whatever arrives has to survive
`normalize_quicklink_key`.

`_open-url --dry-run` exists because a security boundary that can only be
tested by triggering it is one nobody tests. It walks every refusal and stops
one step short of Launch Services.

`link::install` failing does not fail `global install`. A panel that works
without a URL scheme is better than an install that refuses over one, so the
error becomes a line in the output.

**A known limit, not fixed**: a hostile page firing many `prelude://` URLs
raises one notification each. That is a nuisance rather than a hole — nothing
happens, and the notifications name Prelude, so the abuse announces itself —
but rate-limiting refusals is the obvious next thing if it ever matters.

**Check**: `_link-selftest` builds, registers and fires at a throwaway bundle
in `~/Applications` and removes it again — seconds, and not optional, because
asserting that the scheme is *claimed* passes in exactly the case the trap
describes. Then the verb table, in a throwaway `XDG_CONFIG_HOME`: a named
application is reachable, and a named Skill, an unknown alias, an unknown
verb, a path smuggled as an alias, a missing parameter and a bare
`prelude://` are each refused.
