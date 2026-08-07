# The action panel, kind by kind

What `Ctrl+K` offers on each row, and why each entry is there — or was
removed. `Enter` is decided in `defaults.rs`; this is everything else.

> **Status: sections 1–2 are proposals awaiting agreement. Not implemented.**
> Everything from "Every kind" onwards describes the code as it stands.

## The test every entry has to pass

> Would a person, having selected *this kind of row*, reach for this?
> **And is it something they cannot already do without opening this panel?**

The second half is the whole of it, and it has two halves of its own. The
easy one is *no entry may repeat another entry* — a dozen kinds were
relisting their own default under better wording, and that is fixed.

The hard one is **no entry may repeat a key**. That one is still broken.

## 1. The `Enter` row should go

On an agent row the panel opens with:

```
Insert into prompt              · Enter
```

But the main list's footer, while you were standing on that row, already
said:

```
Insert into prompt  Enter   ·   Actions  Ctrl+K
```

So the first row of the panel tells you what the screen you just left was
already telling you, and offers to do what the key you did not press would
have done. It is not redundant with another row — it is redundant with
`Enter` itself. The rule that put it there (*"Enter's behaviour is always
stated, and always first"*) was written before the footer existed, and the
footer does that job better: it states it **without being asked**, on every
row, as you move.

**Proposed:** drop the row. Keep the information by putting it in the
panel's own header, which is currently empty and unselectable — so it costs
no entry and cannot be chosen by accident:

```
╭─ pi ──────────────────────────────────────────────╮
│ ⌘                                            4/4  │
│ Enter, back in the list, inserts it into your prompt
│ ▸ Run it in the shell                             │
│   Resume its most recent session   Prelude · 8h ago│
│   Ask pi something                                │
│   Open its settings                ~/.pi/agent/…  │
╰───────────────────────────────────────────────────╯
```

Four rows, four things you cannot do any other way.

## 2. "the other one" should go

The second row is the *secondary action*, which is real and has no key of
its own — so unlike the Enter row it genuinely belongs here. But it is
labelled:

```
Run it in the shell             · the other one
```

"the other one" is an internal concept leaking out. To the person reading
it, this is simply the most likely alternative, and it should read as an
action like every other row.

**Proposed:** keep the row, drop the sub-label. `on_secondary` stays as the
mechanism by which a kind names its most likely alternative; it just stops
announcing itself.

## 2a. What Raycast actually does

Prelude takes its shape from Raycast, so its Action Panel is worth reading
rather than remembering. Four things it does, and what each means here.

**It keeps the primary action inside the panel.** *"The primary action
appears both in the Action Panel and as the default triggered by ↵."* The
bottom action bar names it as well — so Raycast has exactly the duplication
section 1 proposes to remove, and keeps it on purpose. The reason is
searchability: the panel opens with *"Search for actions…"* and fuzzy
matching **collapses every section into one flat list**. A primary that is
not in the list cannot be found by typing its name, and the panel stops
being the complete inventory of what is possible.

Our panel is an fzf, so it is searchable in exactly the same way. This is a
real argument against section 1, and it is Raycast's, not mine.

**It draws named sections.** Not just an order — visible groups with titles:
*Favorites*, *Configure*, *Deeplink*, *Manage*, and a **Danger zone** for
destructive actions. Our five groups are enforced and invisible. fzf has no
non-selectable row, so titled sections are not directly available — but the
thing the *Danger zone* title is really doing is available another way.

**It styles destructive actions red**, via `Action.Style.Destructive`, and
its rule for the stronger measure is precise: *"Use the confirmation Alert if
the action is doing something that user cannot revert."* By that rule Prelude
is inconsistent — deleting a skill confirms, but **End it** on a live agent
and **Kill it now** on a process do not, and neither can be undone.

**It uses submenus when an action needs to pick from a range.** Ours
enumerate instead: a skill offers `Copy it to codex`, `Copy it to pi`,
`Copy it to opencode`, `Copy it to all missing agents`, `Use it in pi, just
this run`, `Delete claude's copy…`, `Delete codex's copy…` — seven rows that
are three verbs and a choice of agent.

## 3. Open questions for you

- **Should the panel be able to run the default at all?** Dropping the row
  means the answer is no — you press Esc and Enter. That is one extra
  keystroke in the case where you opened the panel and changed your mind.
- **Is the header the right home for it**, or should the border label carry
  it (`─ pi · Enter inserts it ─`)? The header is a full line and can hold a
  sentence; the border label is tighter but competes with the row's name.
- **Do the counts stay?** With both rows gone, `calc` has a **one-line**
  panel (`Copy the original` is `translate` only) — arguably it should not
  open a panel at all, and `^K` on a calc row could say so rather than show
  a list of one.

---

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
- On a port or a process the secondary and `Show what's using it` are the
  same action. R5 says one should go, but the secondary row is a *statement*
  of what Enter's counterpart does, and removing the kind's own entry would
  make the panel's contents depend on the secondary in a way nothing else
  does. Left deliberately; noted so it is not rediscovered as a bug.
- The secondary's sub-label is "the other one". Honest — it has no key — but
  it reads as a placeholder.
- Group boundaries are enforced but not drawn. fzf has no non-selectable
  separator row, and a fake one that could be selected is worse than none.

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
