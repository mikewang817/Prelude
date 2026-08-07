# The action panel, kind by kind

What `Ctrl+K` offers on each row, and why each entry is there — or was
removed. `Enter` is decided in `defaults.rs`; this is everything else.

> **Status: agreed and implemented.** Sections 1–3 record decisions taken
> after reading Raycast's Action Panel; the rest describes the code.

## The test every entry has to pass

> Would a person, having selected *this kind of row*, reach for this?
> **And is it something they cannot already do from another entry?**

A dozen kinds were relisting their own default or secondary under better
wording — `calc` had four rows and two actions; `app`'s "Launch it now" and
`link`'s "Open in browser" were Enter, the latter word for word. Checking
ids never catches that; the duplication is in the behaviour.

**A panel whose third row repeats its first is teaching you not to read it.**

## 0. How the contents of a panel are decided

This was the weakest part of the design for a long time, and it showed. The
method was *subtractive*: look at what is there, remove what repeats or makes
no sense. That cleans a panel but it can never tell you what is **missing** —
`Resume its most recent session` on an agent row was found by someone
pointing at a screenshot, not by the process.

So the method is a checklist, walked per kind. It is generative: it asks nine
questions about the *thing*, and each one either has an answer for this kind
or is refused **in writing**. The refusals matter as much as the entries —
they are what stops the next person re-deriving the same conclusion, and what
makes an omission visible as an omission rather than as an absence.

| | Slot | The question | e.g. |
|---|---|---|---|
| 1 | **Use** | do the thing it exists for | Open it · Run it with claude |
| 2 | **Use otherwise** | the same, but not like that | Open with… · Lend it for one run |
| 3 | **Understand** | what is it, what state is it in — without committing to anything | Show full description · Show what's using it |
| 4 | **Extract** | give me text about it | Copy the pid · Insert the path |
| 5 | **Locate** | take me where it lives | Reveal in Finder · cd to its folder |
| 6 | **Edit** | change the thing itself | Open in editor |
| 7 | **Propagate** | change who *else* has it | Copy it to codex |
| 8 | **Configure** | change what happens **next** time | Always open .json files with… |
| 9 | **Destroy** | end it | Delete a copy… · Kill it now |

Two notes on using it.

**Slot 1 is usually Enter and slot 2 is usually the secondary**, which is why
those two are the ones with keys: they are what the checklist produces first
for nearly every kind.

**A slot with no answer is a finding, not a blank.** Applied to the four kinds
this document was asked about, it immediately produced these, none of which
the subtractive pass had found:

| Kind | Slot | What is missing |
|---|---|---|
| **file** | 9 Destroy | **No way to move a file to the Trash.** A skill can be deleted; the file the launcher spends most of its rows on cannot. Raycast ships `Action.Trash` as a built-in for exactly this. |
| **app** | 5 Locate | **No Reveal in Finder**, though a file has one. The same slot, answered for one kind and not its neighbour. |
| **mcp** | 3 Understand | **No way to see the tools it exposes** — which is the entire reason an MCP server exists. The row says `✔ connected`; it cannot say *connected to what*. |
| **mcp** | 3 Understand | A row can say `⚠ not logged in` and offer **no way to log in**, and no way to see the error behind `✘ failed`. |
| **mcp** | 7 Propagate | **Lend, but never copy.** A skill can be installed into another agent for good; a server can only be borrowed for one run. |
| **mcp** | 8 Configure | **No enable / disable**, though the status column reports `⏸ disabled`. |
| **mcp** | 9 Destroy | **No remove.** Already recorded below as a gap; the checklist says which slot it is. |
| **skill** | 3 Understand | The row says `used 8× · 1d ago` and there is no way to see *which* conversations. Weak — recorded, not proposed. |

`mcp` is the striking one: five of the nine slots are empty, and it is the
only kind whose *primary* action is not "use it" — Enter opens its config,
because there is nothing else an MCP row can do. That is the finding the
subtractive method could never have produced, because nothing there was
wrong; there was simply almost nothing there.

## 1. Enter's action stays in the panel

It was removed for a turn — the list's footer already states it on every row
as you move, so the entry looked like a duplicate of the *key*. Reading
Raycast settled it the other way:

> *"The primary action appears both in the Action Panel and as the default
> triggered by ↵."* — and the panel opens with **"Search for actions…"**,
> whose fuzzy matching **collapses every section into one flat list**.

That is the argument. **A primary that is not in the list cannot be found by
typing its name**, and the panel stops being the complete inventory of what
is possible on a row. Our panel is an fzf and is searchable in exactly the
same way, so the same reasoning applies. It leads, and names its key.

The secondary follows it and names **no** key, because it has none — that is
the entire reason its row exists. It says what it does rather than
announcing itself as "the other one", which was an internal word for it
leaking onto the screen.

## 2. Destructive actions, Raycast's two measures

Raycast draws a **Danger zone** section and styles destructive actions red
(`Action.Style.Destructive`), and its rule for the stronger measure is
precise:

> *"Use the confirmation Alert if the action is doing something that user
> cannot revert."*

Both adopted, and the line between them matters:

| | red | confirms |
|---|---|---|
| Delete *claude*'s copy | ✅ | ✅ names the agent and the path |
| End it (a live agent) | ✅ | ✅ *the conversation in it is lost* |
| Kill it now (port / process) | ✅ | ✅ *the process does not come back* |
| Stop it (container) | ✅ | — `docker start` exists |
| Restart it | — | — |

fzf has no unselectable separator row, so a titled *Danger zone* is not
available — but the title's real job was never the word, it was that these
rows **look different from the ones above them**. Colour does that.

Confirmations put Cancel first, so a stray Enter cancels.

## 3. Submenus instead of enumeration

Raycast uses a submenu when *"an action needs to select from a range of
options"*. Ours enumerated: a skill offered `Copy it to codex`, `Copy it to
pi`, `Copy it to opencode`, `Copy it to all missing agents`, `Use it in pi,
just this run`, and one `Delete …` per agent — **seven rows that are three
verbs and a choice of agent**.

Now the verb is the row and the agent is picked after:

```
Run it with…            2 agents
Use it in pi, just this run
Copy it to…             4 agents
Delete a copy…          to the Trash, after confirming
```

**A submenu over a single option is a keystroke that asks a question with one
answer**, so a verb with exactly one target is still a direct row — which is
why `Use it in pi` is spelled out above while the others are not.

## Order

Five groups, in the order the questions get asked. `actions.rs::group`
enforces it; the sort is stable, so each kind's own sequencing survives.

| | Group | Question |
|---|---|---|
| 0 | Default | What does Enter do here? |
| 1 | Secondary | And its opposite? |
| 2 | Act | Do something else to it |
| 3 | Take | Give me something from it |
| 4 | Go | Take me where it lives |
| 5 | Destroy | Get rid of it |

**Likelihood first, irreversibility last.** Nothing that cannot be undone
with another keystroke sits near the top, where a mis-aimed Enter lands.

---

## Every kind

### `msg` — a question an agent is blocked on

Enter answers it. Everything else is about *deciding faster*.

| | why |
|---|---|
| Answer "no" / "go ahead" | At bottom, everything an agent stops to ask is *may I*. At ten questions, being able to say yes or no without typing is the difference between clearing them and putting it off. |
| Copy the question | To paste into a conversation with someone else. |
| Go to it, zoomed | When the question needs more context than a line — the full screen it came from. |
| cd to its project | To look at the code before answering. |

Rejected: **Run in the shell** — the row is an English sentence. **Ask an
agent about this** — the row *is* an agent asking; the request is for *your*
answer.

### `agent` — an agent CLI you have installed

Enter puts its name on the prompt, because that is where you add `--resume`,
a model, or an opening question. So the panel is the things you would
otherwise have to look up.

| | why |
|---|---|
| Resume its most recent session | The commonest thing anyone does with an agent they have used before. Doing it by hand means `s:`, reading dates, and copying a uuid — and the row already says there are 121 sessions. |
| Ask *pi* something | A one-off question answered in the panel, without a full-screen TUI taking over the terminal. |
| Open its settings | Where you go when it is behaving oddly. `CLAUDE.md` is prose you wrote and is its own row; this is `settings.json`. |

Rejected: **Copy its name** — `pi` is two letters, typed faster than this
panel opens. **Ask an agent about this** — it would hand claude the word
"pi". **Run in the shell below** — the secondary already is exactly that.

Considered and deliberately left out: **Show its skills / its MCP servers**.
The row advertises the counts, but those are rows of their own in the main
list, one keystroke away through search. The launcher's own rule is that
everything else is reachable by searching; a panel that mirrors the list is
a second list.

### `run` — an agent alive right now

Enter goes to it. At eighty of them, the panel is the difference between a
fleet and a mess.

| | why |
|---|---|
| Send it a line, without going there | Answering a stuck agent is the single most common act at scale, and switching to it to type one line is most of the cost. |
| Go to it, zoomed full-screen | Enter goes there; this goes there and gives it the window. |
| cd to its project | Look at what it is doing to. |
| Copy its address | For a `tmux` command of your own. |
| **End it** | Destructive, last. Kills the pane rather than the pid when there is one — killing the process leaves a dead pane, which is the mess this exists to clear. |

### `session` — a past conversation

Enter puts the resume command on the prompt, so you can add a flag.

| | why |
|---|---|
| Resume it now | Skip the prompt when you have nothing to add. |
| Start a fresh session there | The other reason you looked it up: not that conversation, that *directory*. |
| Copy the session id | For `--resume` in a script or another tool. |
| cd to where it ran | Without starting anything. |

Rejected: **Resume this session** — it was there, with the same label as the
default and doing the same thing.

Gap: **sessions cannot be deleted.** There are 490 codex sessions on the
machine this was written on. The skill-deletion shape (recoverable,
confirmed, path-guarded) would apply.

### `skill` — a skill, merged across the agents that have it

| | why |
|---|---|
| Run it with *claude* | One per agent that has it. |
| Use it in *pi*, just this run | Borrowing: nothing installed, nothing left behind, ends with the process. Offered before copying because it is nearly always what was meant. |
| Copy it to *codex* | When you want it for good. One per agent that lacks it. |
| Show full description | Descriptions run to paragraphs; the row shows one line. |
| Open in editor · cd to its folder | It is a file you wrote. |
| **Delete *claude*'s copy…** | Destructive, last, one per copy — see below. |

Rejected: **Insert its name** — that is the secondary, two rows above.

A skill merged across four agents is four directories. `Delete it` would
mean something different depending on a number the row only hints at, so
there is one entry per agent. Deletion moves to `~/.Trash`, confirms with
Cancel as the default, and refuses any path that is not a direct child of a
known skills directory.

### `mcp` — an MCP server, with the status its agent reports

| | why |
|---|---|
| Lend it to *claude* for one run | The whole point of seeing every agent's servers in one list. |
| Insert the lookup command | `codex mcp get node_repl`, to inspect it yourself. |
| Run here | Same, with the answer in the panel. |
| Open in editor · cd to its folder | Its config. |

The label used to say **Insert its name** while inserting the whole lookup
command. Renamed rather than removed: the command is a third useful thing,
distinct from the name the secondary gives you.

Gap: **MCP servers cannot be removed or disabled.** That means editing the
*contents* of `~/.claude.json` or `~/.codex/config.toml`, not moving a
directory — a different risk surface, needing its own design.

### `file` / `find` / `config` — something on disk

Enter opens it in the application that owns it. This is the one place
Prelude has a settings surface, and it is deliberately not a settings
*screen*: you change the rule on the row where you noticed it was wrong.

| | why |
|---|---|
| Open with… | Not that application — this one, once. |
| Always open .json files with… | …and from now on. Adjacent, because they are one idea in two tenses. |
| Open in `$EDITOR` | The specific case of "not the owning app, my editor". |
| Copy absolute path | Distinct from the secondary, which *inserts* it. |
| Reveal in Finder · cd to its folder | The two meanings of "where is it". |

Rejected: **Insert the path** — the secondary, verbatim. **Copy to
clipboard** — for a file that copies the path, which `Copy absolute path`
already does.

### `port` / `proc` — something listening, or something heavy

Enter inserts the kill command so you can read it before running it.

| | why |
|---|---|
| Show what's using it | Before you kill the wrong thing. |
| Copy the pid | For a command of your own. |
| Ask an agent about this | "What is this process?" is a real question. |
| **Kill it now** | Destructive, last. |

Rejected: **Insert: kill whatever is on :3000** — that is Enter. **Run here,
inside this window** — and this is the important one. A port's command line
*is* its kill, so "Run here" ran it: a destructive action wearing an
innocuous label, in row three, arriving there precisely because the honest
one had just been moved to the bottom for safety.

**A safety rule that only moves the labelled danger leaves the unlabelled one
where it was.**

### `container` — a running container

| | why |
|---|---|
| Follow its logs · Restart it | The two things you do without stopping it. |
| Copy name | For a `docker` command of your own. |
| **Stop it** | Destructive, last. |

Enter already inserts `docker exec -it … sh`, so "Shell into it" was a second
name for it.

### `clip` — something you copied

Enter pastes it, the secondary puts it back on the clipboard. What is left is
the one thing you cannot get any other way:

| | why |
|---|---|
| Translate to English / Chinese | Offline, on-device. You copied something you cannot read. |

### `snippet`

| | why |
|---|---|
| Run here | Snippets are commands; seeing the output without leaving is the point. |
| Edit snippets file | Where you noticed it needed changing. |
| Copy raw | With `{{placeholders}}` intact. |

Enter already fills the blanks.

### `translate` / `calc` — a computed answer

Enter copies the result, the secondary inserts it. That is genuinely all
there is to do with a number.

`calc` therefore has a **two-line panel**, and that is the honest answer
rather than a failure. It used to have four lines and two actions: `Copy the
result` and `Insert the result` listed under the two rows that already were
both of those.

`translate` adds one: **Copy the original**, which nothing above provides.

### `ssh` / `app` / `link` / `dir` — objects with a command-shaped `cmd`

Their `cmd` reads like a command — `ssh host`, `open -a Zed`,
`open https://…`, `cd /tmp` — but the row denotes a host, an application, a
URL, a folder. Enter already launches the first three, so `Run in the shell
below` was the same action under a worse name; and `cd` in a subshell that
exits immediately does nothing at all. So `App`, `Link` and `Dir` are not
`is_command_line()`, and none of them gets a generic runner.

| | why |
|---|---|
| ssh: Edit `~/.ssh/config` | Where you noticed the host was wrong. |
| app / link: Insert the open command | Distinct from the secondary's bare name or URL. |
| app: cd to its folder | Into the `.app` bundle. |
| dir: Copy to clipboard | The path. |

`dir` has a three-line panel. Enter inserts `cd …`, the secondary inserts the
bare path — there is nothing else a folder is.

### `history` / `script` / `path` / `git` — a command line

Enter inserts it, the secondary runs it. Both stated at the top, so the arm
itself adds nothing:

| | why |
|---|---|
| Run here, inside this window | Run it *and see the output* without leaving. The one thing the two keys do not cover. |
| Ask an agent about this | "What does this command do?" — the reason this entry exists at all. |
| Copy to clipboard | |

### `sys` — a system command Prelude knows

Same as above. `Insert the command` and `Run it now` were the default and the
secondary, relisted.

---

## The rules, extracted

**R1** Enter's behaviour is always stated — by the footer, as you move, and
(proposed) by the panel's header, not by a row. *A panel entry may not
duplicate a key any more than it may duplicate another entry.*

**R2** The secondary is always first among the actions, and is always
Enter's opposite. It has no key, so this row is the whole of it.

**R3** A generic verb is offered only where it means something.
`is_command_line()` gates `Run in the shell`; `is_interactive()` suppresses
`Run here` for anything that paints a full screen; `worth_asking_about()`
gates `Ask an agent about this`, because `about this: pi` is not a question.

**R4** Destructive entries come last — after the generic tail, not merely
near the end — and confirm if irreversible, with Cancel as the default.

**R5** Nothing appears twice, *including under a different label*. If the
primary or secondary already runs the command, launches it, opens the URL or
copies the result, the generic version is suppressed; and no kind may relist
its own default.

**R6** One row standing for several things enumerates them — per agent for
lending, copying and deleting.

## Known gaps

- Sessions and MCP servers cannot be deleted (above).
- Nothing can be renamed. Probably correct: renaming a skill breaks every
  `/name` in every past conversation.
- Group boundaries are enforced but not drawn, except that the destructive
  group is red. fzf has no non-selectable separator row, and a fake one that
  could be selected is worse than none.
- Submenus are a second fzf rather than a nested list, so Esc from one
  returns to the shell rather than to the panel. Raycast's returns to the
  panel; ours would need the panel to become a loop.

## Where this lives

| | |
|---|---|
| `defaults.rs` | groups 0 and 1 |
| `actions.rs::actions_for_host` | the per-kind lists |
| `actions.rs::group` | which question an entry answers |
| `item.rs` | `is_command_line` · `is_interactive` · `worth_asking_about` |
| `ui.rs::confirm` | R4's confirmation, Cancel first |

`prelude _actions '<row json>'` prints a panel without a terminal. Every
table above was generated from it, and four invariants are tested across
every kind: destructive entries form a suffix, the two defaults lead,
running is offered only where there is something to run, and no id or action
appears twice.
