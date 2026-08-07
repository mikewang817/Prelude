# The action panel

What `Ctrl+K` is for, what belongs in it, and in what order.

`Enter` answers one question — *the obvious thing for this row* — and
`defaults.rs` is where that is decided. This document is about everything
else. It exists because the panel was assembled one kind at a time, and a
list of twenty-five independent decisions is not a design: it produced
`Run in the shell below` on a calculator result, `Run here` on a full-screen
agent TUI, and a delete buried in the middle of a list.

## What the panel is

**Not a menu of everything possible.** A panel with nineteen entries has the
same problem as a launcher with no default: you have to read it. Every entry
has to earn its place by being something a person would actually reach for
*on this kind of row*.

**Not a second list of keyboard shortcuts.** Prelude has two keys on purpose
(see `CLAUDE.md`). The panel is where actions are *spelled out* instead of
remembered, which is why the secondary action lives here rather than on a
key of its own.

**It is the surface where you say "not that".** You pressed the hotkey,
looked at a row, and Enter was not quite it. The panel is the answer to that
sentence — including the version of it that means *not that, and not next
time either*.

## The five questions

Everything in the panel answers one of five questions. They are in the order
a person asks them, and that is the order the panel is in.

| | Group | The question | Examples |
|---|---|---|---|
| 0 | **Default** | What does Enter do here? | states it, always first |
| 1 | **Secondary** | And its opposite? | Enter's counterpart, always second |
| 2 | **Act** | Do something else to it | Kill it now · Follow its logs · Open with… · Lend it to codex · Answer "no" |
| 3 | **Take** | Give me something from it | Insert its name · Copy the pid · Show full description |
| 4 | **Go** | Take me where it lives | cd to its folder · Reveal in Finder · Go to it, zoomed |
| 5 | **Destroy** | Get rid of it | End it · Stop it · Delete claude's copy |

The ordering rule is **likelihood first, irreversibility last**. Nothing that
cannot be undone with another keystroke may sit near the top, where a
mis-aimed Enter lands.

### Why "change the default" is an Act

`Open with…` and `Always open .json files with…` sit in group 2, adjacent,
because they are one idea in two tenses: *not like that — like this*, and
*…and from now on*. Splitting them into "actions" and "settings" would put
the second one at the bottom, away from the moment you want it.

This is the one place Prelude has a settings surface at all, and it is
deliberately not a settings *screen*: you change the rule where you noticed
it was wrong, on the row that was wrong.

## The rules

**R1 — Enter's behaviour is always stated, and always first.** A launcher
whose default is a mystery has failed. The first entry is not an action you
would look for; it is the panel telling you what already happens.

**R2 — The secondary is always second, and is always Enter's opposite.**
Where Enter *does* something, the secondary hands you text; where Enter hands
you text, the secondary does the thing. A test asserts the two never
coincide, because two rows saying the same thing is worse than one.

**R3 — A generic verb is offered only where it means something.** `Run in the
shell` and `Run here` used to be appended to every kind that had not already
claimed them. That is not a fallback, it is a bug with a fallback's manners:

- `calc` — the row is `424.81`. There is nothing to run.
- `translate` — the row is the translation.
- `skill` — the row is `/cnipa-ooa`, a slash command. A shell will say
  "no such file".
- `session`, `agent` — the command starts a full-screen TUI. In the shell
  that is right and is offered explicitly; *inside the launcher's own pane*
  it is a TUI painting into a preview window.

So two predicates decide, on `Kind`:

| | means | gates |
|---|---|---|
| `is_command_line()` | `cmd` is something a shell could run | `Run in the shell` |
| `is_interactive()` | it takes over the terminal | suppresses `Run here` |

**R4 — Destructive actions come last, and say so.** Not "near the end" —
last, after the generic tail. `Delete claude's copy…` followed by
`Copy to clipboard` is an accident waiting for a fast scroll. Anything
irreversible also confirms, with Cancel as the default (`ui::confirm`).

**R5 — Nothing appears twice under different words.** A kind that defines its
own `run` (`Kill it now`, `Launch it now`, `Resume it now`) does not also get
`Run in the shell below`. Two rows that do the same thing is how a panel
stops being readable — and, twice here, how it stopped being safe:

- A port's and a process's command line *is* the kill. `Run here, inside this
  window` therefore ran it: a destructive action, with an innocuous label, in
  the third row — arriving there precisely because R4 had just moved the
  honest one (`Kill it now`) to the bottom. Both kinds now offer the kill
  once, named, last.
- On a file, `copy` and `copyabs` both copy the path. `copy` is suppressed.

The first is the general lesson: a safety rule that only moves the *labelled*
danger leaves the unlabelled one where it was.

**R6 — One row can stand for several things, and then the panel enumerates
them.** A skill merged across four agents is four directories. `Delete it`
would mean something different depending on a number the row only hints at,
so there is one entry per agent. The same applies to lending and copying.

## What each kind offers

Groups are marked `A`ct `T`ake `G`o `D`estroy. Every kind also has the
default and secondary above these, and `Ask an agent about this` at the end
of the generic tail.

| Kind | A | T | G | D |
|---|---|---|---|---|
| **msg** | Answer "go ahead" · Answer "no" | Copy the question | Go to it, zoomed | — |
| **agent** | Ask it something · Run in the shell | Copy its name | — | — |
| **run** | Send it a line · Run here | Copy its address | Go to it, zoomed · cd to its project | End it |
| **session** | Resume it now · Start a fresh session there | Insert · Copy the session id | cd to where it ran | — |
| **skill** | Run it with claude · Lend it to pi · Copy it to codex | Insert its name · Show full description | Open in editor · cd to its folder | Delete each agent's copy |
| **mcp** | Lend it to claude · Run in the shell | Insert its name | Open in editor · cd to its folder | — |
| **file / find / config** | Open with… · Always open .json with… · Open in $EDITOR | Copy absolute path · Insert the path | Reveal in Finder · cd to its folder | — |
| **port** | — | Insert the kill · Show what's using it · Copy the pid | — | Kill it now |
| **proc** | — | Insert the kill · Show its full command · Copy the pid | — | Kill it now |
| | *no `Run here`: their command line is the kill — see R5* | | | |
| **container** | Follow its logs · Restart it · Shell into it | Copy name | — | Stop it |
| **clip** | Translate to English · Translate to Chinese · Run here · Run in the shell | Paste it · Copy it again | — | — |
| **snippet** | Run here · Run in the shell | Insert and fill blanks · Copy raw | Edit snippets file | — |
| **translate** | — | Insert the translation · Copy it · Copy the original | — | — |
| **calc** | — | Copy the result · Insert the result | — | — |
| **ssh** | Connect | Copy host | Edit ~/.ssh/config | — |
| **app** | Launch it now · Run here | Insert the open command · Copy its path | cd to its folder | — |
| **link** | Open in browser · Run here | Insert the open command · Copy the URL | — | — |
| **sys / script / history / path / git / dir** | Run here · Run in the shell | Insert · Copy | cd (dir: insert path without cd) | — |

`app` and `link` keep `Run here` even though their own launch verb is already
above it, because the alternative is an exception list. `is_command_line()
&& !is_interactive()` is a rule two people can apply the same way a year from
now; "these fourteen kinds, except the two where the launch is explicit" is
not. One redundant entry near the bottom of a panel is a smaller cost than a
rule nobody can restate.

## Known gaps

Recorded rather than quietly left out.

- **MCP servers cannot be removed or disabled.** Deleting one means editing
  the *contents* of `~/.claude.json` or `~/.codex/config.toml`, not moving a
  directory — a different risk surface from deleting a skill, and it needs
  its own design. Today the panel can lend and inspect only.
- **Sessions cannot be deleted.** There are 490 codex sessions on the machine
  this was written on. The same "recoverable, confirmed, path-guarded" shape
  as skill deletion would apply.
- **Nothing can be renamed.** Probably correct: renaming a skill breaks every
  `/name` in every past conversation.
- **The secondary's sub-label is "the other one".** Honest — it has no key —
  but it reads as a placeholder.
- **On a port or a process, the secondary and `inspect` are the same
  action.** The secondary is `Show what is using it`; the kind also lists
  `Show what's using it`. R5 says one of them should go, but the secondary
  row is a *statement of what Enter's counterpart does* and removing the
  kind's own entry would make the panel's contents depend on the secondary
  in a way nothing else does. Left as is, deliberately, and noted here so it
  is not rediscovered as a bug.
- **No grouping is drawn.** The order is enforced but invisible; fzf has no
  non-selectable separator rows, and a fake one that could be selected is
  worse than none.

## Where this lives in the code

| | |
|---|---|
| `defaults.rs` | what Enter and the secondary do — groups 0 and 1 |
| `actions.rs::actions_for_host` | the per-kind lists |
| `actions.rs::group` | which of the five questions an entry answers |
| `actions.rs::apply` | carrying one out |
| `item.rs::is_command_line` / `is_interactive` | R3 |
| `ui.rs::confirm` | R4's confirmation, Cancel first |

`prelude _actions '<row json>'` prints a panel without a terminal; that is
how the tables above were checked and how the invariants are tested.
