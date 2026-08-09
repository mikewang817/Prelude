# Prelude

A Raycast-style launcher for your terminal — and the place your agents can
reach you from.

Press one key. A search box appears with your agents, skills and MCP servers.
Type a name to search that root, or open an explicit scope for project scripts,
shell history, ports, processes, apps, clipboard and the other machine-wide
sources. Commands are **handed to you to read before they run** — onto your
prompt from the shell widget, onto the clipboard from the global panel; files,
folders, apps and links open directly in their owning macOS application.

Enter performs the default stated in the footer.

```
╭─ Prelude ────────────────────────────────────────────────────────────╮
│ ⌕ proj:dev                                                    1/5     │
│ ▸ pnpm dev        · package.json · vite --host --port 3000    script │
│ ──────────────────────────────────────────────────────────────────── │
│  Insert into prompt  Enter   ·   Actions  Ctrl+K                     │
╰──────────────────────────────────────────────────────────────────────╯
```

Search on top, results in the middle, and a footer that names the actions
and the keys that run them — the same three parts as Raycast. The footer
changes as you move, because every one of those keys does something
different depending on what is selected.

## The other half

An agent running in a terminal is a sealed box. It cannot see the other
agents on the machine, it cannot talk to them, and the only way it can reach
the person who started it is by printing into its own window — which that
person may not be looking at. So when it hits something it should not decide
alone, it stops, and **the stopping is invisible**. You come back forty
minutes later to an agent that has done nothing.

Prelude gives it a way to say so. One command, from the agent's own shell:

```sh
answer=$(prelude ask "The migration drops the legacy_users table. Proceed?")
```

You get a notification wherever you are. The question is the **top row** of
your launcher, above everything else on the machine. You answer, and the
agent — still blocked on that line — gets your answer on stdout and carries
on:

```
╭─ Prelude ────────────────────────────────────────────────────────────╮
│ ⌕                                                            1/25    │
│ ▸ claude · api-gateway asks · asked 4m ago · The migration drops… asking you │
│   claude · docs        · waiting 12m · work:2.1 · fix the limiter    running │
│   cnipa-ooa            · claude, shared · used 8× · 1d ago             skill │
│ ──────────────────────────────────────────────────────────────────── │
│  Answer it  Enter   ·   Actions  Ctrl+K                              │
╰──────────────────────────────────────────────────────────────────────╯
```

That is the whole idea: an agent that can be written **as though a person
were sitting next to it**. No polling, no "I'll assume yes", no waking up to
a wrong guess made at 2am. [Jump to the details](#agents-can-use-it-too).

## What Enter does

The line is not danger. It is whether there is anything you might want to
change before it happens.

**A command line is handed to you, never executed.** History, scripts,
`$PATH`, snippets, ports, processes — and agents, skills and sessions too,
because those are the ones you most often want to add something to:
`claude` wants `--resume`, or a model, or a question. One keystroke buys you
that, and costs nothing on the times you just press Enter.

Where it is handed *to* depends on which surface you opened. `Ctrl+R` puts it
on the prompt you are standing at. The global panel has no prompt — it is not
the destination — so it copies, and gets out of the way. The footer says
which.

That is also what makes it safe to bind this to a key you hit dozens of
times a day, and safe for the launcher to offer killing whatever holds port
3000 — but safety is the second reason, not the first. `claude` is harmless
and still gets handed over.

**An object just happens.** Files, folders, apps, links and results are not
command lines anyone wants to proofread. Prelude hands them directly to macOS
Launch Services — no `open ...` command touches the prompt or shell history.

| | `Enter` |
|---|---|
| **a question an agent asked** | **answers it, and unblocks the agent** |
| a running agent | hands you `cd` to what it is working on |
| an agent, a skill, a session | handed over, ready to edit |
| a file or config | opens in its default application |
| a folder | opens in Finder |
| an app | launches it |
| a link | opens in the browser |
| a result | copies it |

Nobody selects `.claude.json` hoping for `vi`. Files go to the application
that owns them — the same one the Finder would use — and `^K` is where you
override that, for once or for good:

```
▸ Open it                             Enter
  Insert the full path
  Open with…                          currently Zed
  Always open .json files with…       makes it stick
  Reveal in Finder
  Open in $EDITOR
```

Choices are remembered per extension in `~/.config/prelude/open.toml`, so a
blanket `"*"` still leaves `.png` going to Preview.

`^K` states the current Enter default in a non-selectable header; the rows
under it are alternatives rather than a repetition of Enter.
`PRELUDE_CLASSIC_ENTER=1` restores insert-everything.

## Install

Requires [fzf](https://github.com/junegunn/fzf) and a Rust toolchain.

```sh
git clone https://github.com/YOUR_NAME/prelude
cd prelude
cargo build --release
ln -s "$PWD/target/release/prelude" /usr/local/bin/prelude   # or anywhere on $PATH

echo 'eval "$(prelude init zsh)"' >> ~/.zshrc
exec zsh
```

Press **Ctrl-R**. Run `prelude doctor` to check the setup.

> Ctrl-R replaces zsh's incremental history search, which moves to Ctrl-S;
> Prelude's full history is under `h:`. Override with `PRELUDE_KEY='^T'` set
> before the `eval` line. Ctrl-Space is deliberately not used: macOS binds it
> to "Select the previous input source" and the OS eats it first.

One more, optional and one line:

```sh
prelude init agent >> CLAUDE.md        # teach your agents they can reach you
```

It is what makes the first half of this README happen at all — an agent that
has not been told it can ask you a question will not ask you a question. See
[teaching an agent](#teaching-an-agent-it-can-do-this).

### Open Prelude globally with Cmd+Space

The `Ctrl+R` binding is local to a shell prompt. To reach the launcher from
Finder, a browser or any other application, install the global panel:

```sh
prelude global install
prelude global status
```

The panel is a **Ghostty quick terminal**: a hidden, dedicated Ghostty
instance, kept out of the Dock and the app switcher, hosting one long-lived
`prelude _panel` loop. Ghostty registers the chord itself, so nothing of
Prelude's runs when you press the key.

**A press reveals; it never creates.** An earlier design built a terminal on
every press — a new application instance, a window, a login shell — 373ms of
construction, torn down afterwards, including when the answer turned out to be
a file that never needed a terminal. Every bug in launch and teardown came from
that. There is nothing to launch now and nothing to strand.

The default is `Cmd+Space`. Before installation or a key change Prelude checks
Spotlight, Raycast's configured global key, and a native Carbon reservation. A
conflict stops installation and leaves the previous working configuration
alone; Prelude never rewrites another application's shortcut. Disable **Show
Spotlight search** under **System Settings → Keyboard → Keyboard Shortcuts →
Spotlight**, change Raycast's Hotkey setting, or choose another chord:

```sh
prelude global hotkey cmd+shift+space
prelude global directory ~/src        # where the panel stands; default $HOME
prelude global open                   # reveal it without the key
```

A chord is one or more of `cmd`, `option`, `ctrl`, `shift` plus `space`, a
letter or a digit. Modifiers are normalized, malformed or modifier-free keys
are refused, and `prelude global status` names known conflicts.

#### The panel copies; it does not open a terminal

**A command you pick from the panel goes on the clipboard**, and the panel
stands down. Objects are unaffected — a file, a folder, a URL or an
application goes straight to Launch Services, which is the whole point of it
not being a command.

This is the one place Prelude used to guess. It read the frontmost application
and either typed the command into the tmux pane you were looking at, or built
a window to leave it in. Both were the same mistake wearing different clothes:
a launcher deciding, from the outside, which prompt you meant. The window
arrived in the wrong directory as often as not, and the pane was whichever one
tmux happened to consider current — a command delivered out of sight is
indistinguishable from a launcher that did nothing.

Where it lands is the one question a launcher has no business answering, so it
is asked of the only thing that knows: you, with ⌘V, in the window you were
already in.

The footer says which surface you are in, because "hands you text" is two
different sentences:

```text
Ctrl+R at a prompt     Insert into prompt  Enter   ·   Actions  Ctrl+K
the global panel       Copy the command    Enter   ·   Actions  Ctrl+K
```

`^K` follows the same rule. At a prompt a command's alternative is **Run it in
the shell**; from the panel that row is gone, because a submitted command and
an unsubmitted one are the same bytes on a clipboard and two rows saying the
same thing is worse than one. **Run and show output** stays in both — it runs
inside Prelude and shows you the result, which needs no shell on the other end.

`prelude global uninstall` removes the panel configuration and the LaunchAgent;
add `--reset` to remove Prelude's global preference and status files too. See
[the implementation and acceptance record](docs/GLOBAL-HOTKEY.md).

## Running dozens of agents at once

`s:` searches conversations on disk. **`r:` searches what is alive right
now** — every agent process on the machine, wherever it happens to be
running: a terminal tab, a split, over ssh, or nothing at all:

```
▸ claude · api-gateway     · waiting 12m · pid 2119 · fix the rate limiter   running
  claude · TerminalRaycast · waiting 32m · pid 2557 · Create Raycast-like…   running
  codex  · docs            · working     · pid 3081 · rewrite the README     running
```

The middle column is the one that matters. An agent that is working *prints*
— tokens, tool output, a spinner — and appends to its conversation file as it
goes. It writes nothing at all while waiting for you. So silence is what a
question looks like from outside the process, and "waiting 12m" means *that
one asked you something twelve minutes ago and has been sitting there since*.
Those sort to the top; `r:waiting` shows only them.

Silence alone would lie, though, and in the expensive direction: an agent
three minutes into a build is also silent, and a badge that cries wolf is
worth less than no badge. So the clock is only the tiebreak — the
conversation file says which kind of quiet it is. **A turn that ends in a
tool call is still going; a turn that ends in prose has handed back to you.**
A ten-minute test run never reports as stuck.

**None of this needs a particular terminal.** The backbone is the process list
— `ps` for what is alive, one bulk `lsof` for where each one is — and the
clock is the mtime of the conversation file each agent is appending to. Quick
Look reads that file too, so you can see what an agent last said without going
anywhere near its window:

```
what it last said
⏺  Found it: one session's iCloud path is 127 columns wide…
›  use a percentile instead of the max
```

Every run therefore answers the same questions, whatever started it: what
project, how long quiet, working or waiting, what it last said, its directory,
and ending it. `^K` also leaves it a line in its inbox, which it collects with
`prelude inbox`.

Two things used to be offered for the subset of runs that had a tmux pane —
Enter putting your cursor in it, and a message typed straight into it — and
both are gone. Prelude does not reach into a terminal it does not own, and a
fleet view that is sharper about some of its rows than others is harder to read
than one that treats them alike.

Batch runs (`claude -p`, `codex exec`) are marked rather than mixed in. They
keep no conversation file and have no terminal, so silence means nothing
about them and they are never reported as stuck.

### Without opening the launcher

The waiting signal is only worth having if you do not need to *remember to
look*. The same view is three plain commands, on the same code path as `r:`
so they cannot disagree with it:

```sh
prelude fleet              # the list as text, waiting first
prelude fleet --status     # "2 waiting · 3 working" — or nothing at all
prelude watch              # notify the moment an agent stops and waits
```

`fleet --status` is made for a status bar or a prompt, and never pays for
subprocesses itself: cached identities, live states, exactly the launcher's
deal. An idle machine prints nothing at all rather than a permanent
`0 waiting`.

`prelude watch` closes the loop. Silence is what a question looks like from
outside the process, and the watcher makes it audible: the moment a run goes
quiet you get a macOS notification naming the agent, the project, and what
you were talking about. It fires once per stop — an agent that stays stuck
is not announced again until it has worked in between. Start it with
`prelude watch &`, or from a LaunchAgent if you want it always on.

## Agents can use it too

Everything above assumes a person at the keyboard. `r:` detects an agent's
silence *from outside*; these four verbs let the agent say so itself. Each is
a plain shell command it runs from its own conversation:

| | |
|---|---|
| `prelude ask TEXT` | ask the human, **block**, answer on stdout |
| `prelude tell TEXT` | tell them something, do not wait |
| `prelude say WHO TEXT` | send a line to another running agent |
| `prelude inbox [--json]` | what other agents left for you |
| `prelude fleet --json` | who else is running, and which are stuck |
| `prelude control --json` | Agent, Run, Session, Skill and MCP relationships |

### `prelude ask` — the one that changes what an agent can be

```sh
answer=$(prelude ask "The migration drops the legacy_users table. Proceed?")
```

The question goes where the launcher can see it, the human is notified
**wherever they are**, and the command blocks until an answer comes back.
stdout carries the answer and nothing else, so that one line is the whole
integration.

| exit | meaning |
|---|---|
| `0` | they answered — the answer is on stdout |
| `3` | nobody answered within the timeout; the question stays in the inbox |

Those being distinct is the point: a script can tell *"they said no"* from
*"nobody was there"*, and do the conservative thing only in the second case.
`--timeout=N` (default 600s) fits it inside your own tool deadline;
`--no-wait` returns an id to collect later with `prelude answer-of <id>`.

The person answers from wherever *they* are — the launcher's top row with
`Enter`, one keystroke in `^K`, or `prelude reply` from any terminal, which
takes the oldest one waiting:

```
▸ claude · api-gateway asks  · asked 4m ago  · the migration drops legacy_…  asking you
     ^K  →  Answer "no"          ← one keystroke, unblocks it
            Answer "go ahead"
            Show conversation context
```

The status line leads with the count, because a question is your problem in a
way that a merely-quiet agent is not:

```
1 asking · 2 waiting · 3 working
```

### Agents talking to each other

`prelude fleet --json` gives an agent the same fleet the launcher shows —
project, state, working directory, pid — so "check whether anyone else is
already on this" becomes a command rather than a guess.

`prelude say` leaves a line in another agent's inbox, attributed so the
receiver knows it came from a peer rather than from the human:

```
$ prelude inbox
· claude · api-gateway  I changed the auth schema — you will need to rebase  [17…-4]
```

Address it by project, agent name, or pid. **If more than one thing matches it
refuses and lists them**, because a message delivered to the wrong conversation
reads as the human's own words and is worse than one not sent — and a working
directory is the whole of an inbox address.

Delivery is always to the inbox. There was a faster path once, for a target
that had a tmux pane: the line was typed into it and submitted, putting it in
front of the agent immediately. It was also the only delivery that worked for
part of the fleet and not the rest, and a sending agent could not tell which
kind of peer it was addressing. One route for everyone is worth more than a
fast route for some.

### Teaching an agent it can do this

```sh
prelude init agent >> CLAUDE.md      # or AGENTS.md, or a skill
```

That prints the whole interface as instructions — the four verbs, what each
returns, and *when to reach for them*, which is the part that matters. A
capability an agent has to be reminded of every session is not a capability
it has.

## Using another agent's skill mid-conversation

A skill row hands over either of two things, both named in `^K`:

| | |
|---|---|
| `Insert the slash command` | `/skill-name` |
| `Point an agent at its file` | `Read …/SKILL.md and follow it.` |

The second is the one worth knowing about. `/skill-name` typed at an agent that
does not have the skill is a line of prose that means nothing, and it fails
silently. Pointing that agent at the file needs no restart, no flag, and no
cooperation from its CLI — a skill is a file of instructions, and every agent
can read a file. It is the only way in for codex and opencode, which cannot
load a borrowed skill at all.

Prelude used to pick between them for you: `prefix + r` opened the launcher in
a tmux popup over the pane you were in, tmux said which agent that pane was
running, and the row handed over whichever form suited it. Nothing can answer
that question now, and the failure it was avoiding is still real — so both rows
are present and say which is which, and you paste the one you want.

If you want the skill loaded properly *as* a skill without losing the
conversation, resume it with the loan attached:

```sh
claude --resume <session-id> --plugin-dir ~/.cache/prelude/borrow/<skill>
```

Same conversation, borrowed skill available. `s:` finds the session id.

## Keys

Two action keys, plus one Quick Look mode; none needs terminal configuration:

| Key | Prelude |
|---|---|
| `Enter` | The obvious thing for what you selected |
| `Ctrl+K` | Everything else for this item |
| `Ctrl+P` | Show or hide Quick Look in the result area |
| `Esc` | Close |

Ctrl, because Ctrl is what a terminal reliably receives. macOS spends Option
on composing characters unless the terminal is told otherwise — so `Option+K`
would type `˚` into the search box — and it never delivers Cmd to a terminal
at all. A key that works on one machine and silently does nothing on the next
is worse than no key.

`Ctrl+K` opens the alternatives to Enter — and only the alternatives.
**Enter's own action is the header, not a row**, because opening the panel is
already the statement that you wanted something else:

```
 README.md · file
 Default: Open it · Enter

 Action › Insert the full path
          Open with…
          Open in editor
          Reveal in Finder
          Copy path
          Create Quicklink…
          Change default app for .md files…
          Move it to the Trash…
```

**Every label says whether it acts or hands you text.** `Open in editor`
opens it; `Insert cd command` puts a command on your prompt; `Insert login
command` does not pretend you are logged in. That distinction is the whole
of the launcher's safety model, so it belongs in the words rather than in
something you have to remember.

There is still a **secondary action** — Enter's opposite, per item — where it
adds a real choice, since it has no portable key of its own. Richer kinds
name their alternatives specifically instead:

| | `Enter` | its opposite |
|---|---|---|
| a command | insert it | run it |
| an agent | insert it | start it |
| a file | open it | insert its path |
| a folder | open it in Finder | insert its path |
| a URL | open it in the browser | insert the URL |
| an app | launch it | insert its name |
| a port | insert the kill | show what is using it |
| a result | copy it | insert it |

**The panel stays open when it makes sense to.** Copying and reading details
return you to the list; opening, inserting and navigating finish the job.
`Esc` steps back one level — out of a submenu to the actions, out of the
actions to your search.

Keys are spelled out rather than drawn as glyphs. A row of symbols is only
legible to someone who already knows what they mean.

```
Open it  Enter   ·   Actions  Ctrl+K   ·   Preview  Ctrl+P
```

Quick Look is not a permanent second column. The list uses the full window;
`Ctrl+P` replaces only the result area with the selected item's detail, and
pressing it again returns to the list. This keeps long rows searchable and
gives images enough room without sacrificing half the launcher when no
preview is needed.

`^O` (run here), `^X` (run in the shell) and `^Y` (copy) remain compatibility
shortcuts for anyone who learned them. Their useful item-specific equivalents
are named in `^K`.

Open the launcher itself with `^R`.

## Search starts as an agent home

With an empty query Prelude shows only the things that belong to its main
job: questions waiting for you, running agents, installed agents, skills and
MCP servers. On the current test machine that is 25 useful rows instead of a
2,400-row fzf catalogue dominated by `$PATH`.

Start typing and Prelude searches the root commands — agents, skills, MCP,
Quicklinks, web providers and the commands that open each larger source. It
does not throw the 2,400 underlying rows back onto the screen. For example,
`f` produces `Search Files · f:`; Enter completes the prefix, and only `f:`
shows files. The same rule applies to the other sources:

```text
a: agents      r: running      s: sessions      f: files
c: clipboard   h: history      app: apps         cmd: commands
set: settings
```

Applications, history, files, clipboard entries and `$PATH` executables
therefore require their explicit scope. This trades a little syntax for a
stable, agent-first root instead of making every single letter match hundreds
of unrelated machine objects.

Type `:` to see every scope, including Skills, project items, folders, SSH
hosts, snippets, ports, processes, containers, MCP and config. An exact prefix works
without a term: `c:` opens clipboard history and `f:` opens file search.
Clearing the query returns to the agent home.

See [the search model](docs/SEARCH.md) for the complete scope table and the
rules that keep computed rows from being filtered out by fzf.

## What it searches

| Source | Open it | Where it comes from |
|---|---|---|
| **Agent control centre** | `a:` | Agents, running agents, skills, MCP and config |
| **Sessions** | `s:` | Conversations from Claude Code, Codex and pi |
| **Skills** | `skill:` | Installed capabilities across every supported Agent; `/` invokes them |
| **Project** | `proj:` | Scripts from `package.json`, Makefile, justfile, Cargo, Python and Compose; current files and Git |
| **Files** | `f:` | The current project, plus the roots built by `prelude index` |
| **Clipboard** | `c:` | Text, Finder files and images, strictly newest first |
| **History** | `h:` | Deduped, newest first |
| **Apps** | `app:` | Every installed `.app` |
| **Commands** | `cmd:` | `$PATH` executables and system commands |
| **Folders** | `dir:` | zoxide, or `cd` targets mined from history |
| **SSH hosts** | `ssh:` | `~/.ssh/config` |
| **Snippets** | `snip:` | `snippets.toml`, with `{{placeholder}}` blanks |
| **Ports / processes** | `port:` / `proc:` | Listening TCP ports and the heaviest processes |
| **Containers** | `docker:` | Running Docker containers |
| **MCP / config** | `mcp:` / `cfg:` | Agent integrations and settings |
| **Prelude settings** | `set:` | Search roots, hotkey, keys and rules |

Clipboard history is chronological rather than frecent: the thing copied most
recently is always first. Text is retained as text; one or several files remain
Finder file objects; screenshots and copied bitmap data remain images. Enter
inserts text or shell-quoted paths, while `Ctrl+K` can restore the original
file/image object to the system clipboard. `Ctrl+P` renders images in place.

Plus rows computed from what you type: web addresses, arithmetic, unit and
currency conversion, date arithmetic, on-device translation, and quicklinks.
A URL appears at the top and Enter hands it directly to macOS's default
browser — no command is pasted, no shell runs, and nothing enters history.
Explicit calculator, translation and prefix queries hide unrelated rows.

```
10kg to lb        →  22.046226 lb        1847*0.23     →  424.81
1gb to mb         →  1,024 mb            now + 3 days  →  2026-08-10 …
100 usd to cny    →  676.04 CNY          1699999999    →  2023-11-15 …
en:你好            →  Hello               g rust async  →  opens Google
localhost:3000     →  opens the browser    github.com    →  opens the browser
```

### Quicklinks

A quicklink gives a stable object a short keyword without turning it into a
shell command. Select a file, folder, URL, config or application, then choose
`Ctrl+K → Create Quicklink…` and edit the suggested keyword:

```text
README.md → preadme
```

From then on, typing `preadme` resolves back to a file row. Enter uses the
same default as the original — its owning application for a file, Finder for
a folder, the default browser for a URL.

Created definitions are appended to `quicklinks.toml` without rewriting its
comments or hand-written search templates. A generated definition can be
edited or removed from its own action panel; removing it never touches the
target. Keywords are unique and are never silently overwritten. URLs that
look like they carry credentials are refused rather than indexed.

Search templates are commands before they have an argument. Typing `g`
shows `Search Google · g <query>`; Enter keeps Prelude open and prepares `g `
for the search term. Providers are also searchable by name, so `google` and
`baidu` find them without knowing their aliases. Once a term exists, the
single result is the URL that Enter opens directly:

```text
g                    → Search Google · g <query>
g rust async         → Google
gs elliptic curve   → Google Scholar
b Rust 异步         → Baidu
bing rust async     → Bing
ddg rust async      → DuckDuckGo
gh prelude          → GitHub search
```

## The action panel

`^K` gives each result verbs appropriate to *what it is*, not just "insert":

| Kind | Actions |
|---|---|
| **question** | Answer “go ahead” · Answer “no” · Show conversation context · Go to agent full-screen |
| **agent** | Resume latest session · Ask it a one-off question · Open settings |
| **running agent** | Send a message… · Show last response · Go to pane full-screen · **End agent…** |
| **skill** | Run with claude · Prepare one-off run with pi · Install in pi · Read instructions · Open SKILL.md in editor · **Delete a copy…** |
| **mcp** | Prepare one-off use with codex · Insert install command · Insert login command · **Insert remove command…** |
| **file** | Open with… · Open in editor · Reveal in Finder · Copy file · Copy path · Create Quicklink… · Change default app… · **Move it to the Trash…** |
| **clipboard file / image** | Restore the original clipboard object · Open · Reveal in Finder · Copy path |
| **port / process** | Copy PID · **Kill process…** |
| **container** | Insert follow-logs command · Insert restart command · **Insert stop command** |

That is the difference between a command picker and a launcher: a port is not
text to insert, it is something you kill or inspect.

**There is no generic tail.** Nothing is appended to a kind merely because it
could be constructed — a calculator result had `Run in the shell below` on it,
which offered to execute `424.81`, and an agent row had `Ask an agent about
this`, which handed claude the word "pi". Each kind opts into the actions that
answer a real question about it, and a two-line panel is the honest answer for
a number rather than a failure.

**Destructive actions are red, last, and confirm when they cannot be undone.**
Ending a live agent loses the conversation in it; killing a process is not
reversible; deleting is. So the first two ask, with Cancel selected by
default, while stopping a container — which `docker start` undoes — is merely
red. Nothing that bites sits near the top, where a mis-aimed Enter lands.

**Deleting always means the Trash, never `unlink`.** A skill is often the only
copy and often not in git, so it is moved rather than removed, a name already
there is never overwritten, and `$HOME`, the root and the system directories
are refused however the row got here.

**Where an agent has its own CLI, Prelude uses it.** `Insert login command`,
`Insert install command` and `Insert remove command` hand you
`claude mcp …` or `codex mcp …` rather than editing `~/.claude.json` — the
agent knows its own format, that file holds far more than MCP servers, and a
command on your prompt is this launcher's own form of confirmation. Installing
a server whose environment holds an API key is refused for both, because
`mcp add` takes the definition inline and there is no version of that which
keeps the key out of your shell history.

**The panel knows which surface you opened it from.** From the global panel it
drops the rows whose only meaning was "and submit it for you", because a
clipboard cannot submit anything. Everything that acts — opening settings, an
editor, Finder, running something inside Prelude and showing the output — is
identical in both.

The reasoning for every entry, and for what was deliberately left out, is in
[docs/ACTIONS.md](docs/ACTIONS.md).

## One surface for every agent

Open the launcher and this is what you see first, before typing anything:

```
▸ claude    agent · 9 skills · 3 mcp ·  28 sessions
  codex     agent · 2 skills · 3 mcp · 490 sessions
  pi        agent · 2 skills · 0 mcp · 120 sessions
  cnipa-ooa skill · claude, shared · used 8× · 19h ago
  Gmail     mcp   · claude · ✔ connected
```

Ordinary search also exposes visible management commands — **Agent Control
Center**, **Running Agents**, **Past Conversations**, **Skills**, **MCP
Servers** and **Agent Config** — so the prefixes are accelerators rather than
knowledge required before the feature can be found. `a:` opens the complete
agent control centre; `s:` owns the hundreds of older conversations while the
newest few remain on Home. Other machine sources open through the same kind of
scope command rather than joining ordinary root search.

Under those rows Prelude now keeps one relationship graph rather than five
unrelated lists. Runs have stable ids, Sessions have agent-qualified ids, and
each side names the other when the evidence is sound. An explicit native
resume id is exact; the newest Session in a directory is inferred only when
there is exactly one run of that Agent there. Two Claude processes in the same
project remain visibly ambiguous instead of both claiming the same Session.
`Ctrl+P` shows how a match was made. Selecting a Session a live run already
owns hands you that run's project rather than starting a competing resume of
the same conversation.

The same graph is scriptable without launcher rows:

```sh
prelude control
prelude control --json
```

Its versioned `schema: 3` snapshot contains installed Agent executables and
supported operations, Run ids and state, active Session edges, Skill
fingerprints and copy state, MCP owner/health/definition variants, effective
borrow/install targets and config paths. Process prompts, full command lines and complete MCP definitions are
deliberately absent because they may contain credentials.

The Agent experience is tracked as checked acceptance criteria in
[docs/AGENT-CONTROL-PLANE.md](docs/AGENT-CONTROL-PLANE.md). That file records
the current milestone, deliberate gaps, validation baseline and implementation
commits; it is the execution source of truth rather than a roadmap kept in a
conversation.

**MCP servers report real status**, asked of each agent rather than read out
of config files: `✔ connected`, `⏸ disabled`, `⚠ not logged in`. Reading the
config missed every claude.ai-hosted server and could not tell you whether
anything actually worked.

**Skills say whether you have ever used them**, mined from past sessions —
`used 8× · 19h ago`, or `never used`. Nothing else can answer this, because
nothing else sees both your skills and your conversations.

**`/skill-name args`** invokes a skill and answers in the panel, picking an
agent that has it.


**Resume a past conversation** without hunting for a uuid. Sessions from
Claude Code, Codex and pi are merged, newest first, with Claude's
AI-generated titles where available:

```
⌕ s:inkquest
▸ Develop InkQuest math problem app     session · claude · ~/App/InkQuest · 2h ago
  InkQuest                              session · pi     · ~/App/InkQuest · 1d ago
```

Only the newest conversations reach the empty-query home. The visible **Past
Conversations** command opens `s:`, which searches all of them. `Ctrl+K` turns
that browser into a lifecycle surface:
fork through the Agent's own CLI, pin important conversations above recency,
give them local names, archive them, export the untouched JSONL, reveal the
native file, or move an inactive one to the Trash. A local name never rewrites the Agent's record, and its native title
remains searchable.

```text
s:is:pinned       pinned conversations
s:is:active       Sessions owned by a live Run
s:is:archived     archived conversations
s:is:all oauth    include archived Sessions while searching for oauth
```

Archive is only Prelude metadata. Native Claude, Codex and pi files remain the
authority. Trash is deliberately narrower: it is unavailable while the Session
is active, re-finds the fleet before acting, accepts only canonical JSONL files
inside those Agents' known Session roots, asks first, and moves rather than
unlinks.

**Lend a skill or an MCP server to an agent that lacks it.** Prelude knows
which agents have a capability and which do not. Where the receiving CLI has
a one-run flag, Prelude offers the lighter option before permanent install:

```
▸ my-skill    skill · claude, shared · missing: codex, pi
     ^K  →  Prepare one-off run with pi       ← nothing installed
            Install in codex / Install in pi  ← keep it for good

▸ node_repl   mcp · codex · ✔ connected
     ^K  →  Lend it to claude for one run
```

Borrowing installs nothing and leaves nothing behind. You get the command,
you press Enter, and the loan ends when that process does. `Install in pi`
is the other half, for when the loan should have been a move — and for MCP
servers it hands you that agent's own `mcp add`, rather than editing its
config file on its behalf.

**Delete one you are done with.** A skill merged across four agents is four
directories behind one row, so `^K` offers them one at a time — "delete it"
should not quietly mean something different depending on how many agents
happen to have it:

```
▸ …
  Delete claude's copy…            to the Trash, after confirming
  Delete codex's copy…             to the Trash, after confirming
```

Files and applications can also be moved to the Trash from their own panels.
Skill deletion needs a stricter guard because one merged row can represent
several directories, so it is built to make a wrong choice survivable:

- **It goes to the Trash, never to `unlink`.** A skill is somebody's work,
  often the only copy and often not in git. Drag it back out if you were
  wrong. Nothing here is worth a permanent delete, and a name already in the
  Trash is never overwritten — you get `my-skill 2`.
- **The confirmation's default is Cancel**, and it names the agent and the
  full path. A confirmation whose default is "yes" only adds a keystroke to
  the accident it was meant to prevent.
- **It refuses any path that is not a skill directory** — a direct child of
  one of the five skill directories, compared after resolving symlinks and
  `..`. The path arrives off a row that has been through JSON and a shell,
  and a launcher that removes whatever it is handed is one malformed field
  away from removing something else.

| | MCP | skill |
|---|---|---|
| claude | `--mcp-config` | `--plugin-dir` |
| codex | `-c mcp_servers.…` | — |
| pi | — | `--skill` |
| opencode | — | — |

A dash means no such flag exists, so no one-run action is offered; permanent
`Install in …` may still be available. Two cases refuse on purpose: a
**claude.ai-hosted** server has no definition to lend, because its credentials
live with the Claude
account rather than on this machine; and a server whose env holds an API key
is never handed to codex, whose only form is inline, where the key would end
up in your shell history. claude gets a `0600` file in Prelude's cache
instead.

**Ask an agent without leaving the launcher.** `@claude what does EADDRINUSE
mean` — Enter renders the answer in the panel, streaming as it arrives, and
the launcher stays open so you can ask the next thing:

```
⌕ @claude 1+1                     1/1  │ asking claude…
▸ claude: 1+1     session · ⏎ answers  │ 2
```

It uses each agent's non-interactive mode (`claude -p`, `codex exec`,
`pi --print`, `opencode run`) so the reply is plain text rather than a
full-screen TUI taking over the terminal. Resuming an existing session is
still handed over rather than run here, because a long-lived session belongs
in a real terminal rather than a launcher that closes.

Each agent is invoked with its own syntax (`opencode` needs `opencode run`,
the others take the prompt positionally), and `@` completes only against
agents actually installed. One typed Agent registry owns those commands,
settings paths and support flags, so Quick Look, Control JSON and the action
panel cannot each advertise a different Agent.

An Agent's `Ctrl+P` view shows its executable, settings path, active projects,
most recent conversation and supported operations without running the Agent
CLI. `Ctrl+K` reaches current instances, the latest or any recent conversation,
a one-off Ask, settings and diagnostics.

Agents, Skills and MCP servers can be added to **Favorites** from `Ctrl+K`.
The preference is stored in Prelude's config directory, promotes the object
only inside its existing kind, and never writes to an Agent's native files.

**Jump to any agent config** — `CLAUDE.md`, `AGENTS.md`, `settings.json`,
`config.toml`, `.mcp.json` — as a first-class row.

Prelude inventories skills and MCP servers across every installed agent and
merges them into one list. A skill present in several agents is shown once,
labelled with which agents have it, rather than duplicated or silently
deduped.

Skills come from `~/.claude/skills`, `~/.agents/skills`, `~/.codex/skills`,
`~/.pi/agent/skills` and `~/.config/opencode/skills`. Each installed copy gets
a full-tree content fingerprint covering instructions, scripts, references and
symlinks while ignoring VCS metadata. The work runs behind `skill-hashes.json`;
unchanged trees need only a metadata stamp and never enter the launch path.
`Ctrl+P` shows the copy matrix and marks copies as single, identical, divergent,
unknown, or private-unknown when redaction prevents a truthful equality claim.

A divergent Skill offers a recursive read-only Diff first. Replacement is an
explicit second action: Prelude hashes both copies around the comparison,
refuses if either changed, moves the target to the Trash, then copies. It also
refuses to copy a source containing credential-like files or lines.

MCP inventory and health come from each agent's own CLI (`claude mcp list`,
`codex mcp list --json`), which includes hosted servers and real connection
state. Servers with the same name form one capability matrix across owners,
health, normalized transport, timestamps and redacted definition fingerprints.
A five-minute background snapshot starts enabled stdio servers long enough to
perform the MCP initialize and paginated `tools/list` handshake. Quick Look
then shows the actual bounded tool names and descriptions. HTTP and hosted
servers say that tool inventory is unsupported when Prelude has no owner
authentication; they are not presented as successful empty lists.

Complete definitions—arguments, env and headers included—are never retained in
launcher Items or caches; an explicit lend/install action asks the owner CLI
again. Account-hosted servers with no transferable local definition show as
owner-only and have no fake borrow or install targets. `Ctrl+K` can refresh
health, refresh a stdio tool inventory, and show a structural redacted
Definition Diff. A one-time migration removes definitions and derived search
rows written by older builds. `prelude doctor mcp` reports transport, stale
health/tools, duplicate owner/name records, auth failures and privacy state.

## Settings

Prelude has no settings window. It has settings *rows*, in their own scope,
each carrying its current value:

```
⌕ set:
▸ Search roots            · 3 folders · 53,409 files · indexed 2d ago  setting
  Global hotkey           · cmd+shift+space                            setting
  Panel directory         · ~                                          setting
  Launcher key at a shell · ^R                                         setting
  Inline height           · 90%                                        setting
  Quick Look              · on                                         setting
  What Enter does         · per kind                                   setting
  Open-with rules         · 4 extensions                               setting
  Snippets                · 5 saved                                    setting
  Quicklinks              · 11 keywords                                setting
  Favorites               · 3 promoted                                 setting
```

The value on the row is the point. A setting you cannot see the value of is
one you change by trial, and most of these were previously invisible: four
existed only as environment variables that had to be exported before the
`eval` line in `.zshrc`, and `roots.txt` — which decides what `f:` can find at
all — was documented in a README, defaulted from a hard-coded list, and had to
be created by hand.

Enter changes the setting; `^K` holds everything else, and never repeats
Enter. **Search roots** is the one worth knowing:

```
 Search roots · setting
 Default: Add a folder… · Enter

 Action › Remove a folder…            leaves the folder alone
          Rebuild the index now       runs prelude index here
          Show every root
          Open the file               ~/.config/prelude/roots.txt
```

The row says `indexed 2d ago`, and says **roots changed — run Rebuild** when
you have added a folder the index does not know about yet. That silence was
the actual bug: you add a folder, `f:` keeps answering from the old set, and
nothing anywhere tells you the index is why.

Adding a root goes through the same guard the Trash uses, so `~`, `/`,
`/Users` and `/Applications` are refused. Indexing a home directory means
seven levels of `fd` through `~/Library`, which macOS protects as other
applications' data — and the only symptom is a system dialog naming your
*terminal*, with nothing in it that says Prelude.

Everything is also a plain command, on the same code path:

```sh
prelude settings                      # the table above
prelude settings --json               # …as fields
prelude settings roots                # each root, and whether it still exists
prelude settings add-root ~/work
prelude settings remove-root ~/work
prelude settings set key '^T'         # also height, preview, classic_enter
```

Six settings own a file each and are edited by the code that owns it, so a
chord typed here goes through the same validation and the same panel restart
as `prelude global hotkey`. The four that had only an environment variable get
`settings.toml` — and the variable still wins wherever it is exported, because
a variable is a per-invocation instruction and a file is a standing one.

## It learns

Every selection is recorded in `frecency.tsv` — a plain text file you can read
and edit. What you actually pick floats up, with recent use weighted far above
old use.

**Within its kind, and never across.** Sorting asks two questions in order:
*what kind of thing is this*, then *how much do you use this one*. So a skill
you invoke often leads the other skills and still sits below the agents; a
question an agent is blocked on leads everything, whatever you have been
picking all week.

**Skills are ordered by the number the row already shows you.** `used 8× ·
1d ago` is mined from your past conversations, and it decides — not clicks in
the launcher, which usually mean reading a description or lending the skill
somewhere. Recency separates skills you reach for equally often but can never
lift one over a skill you have used more: eight invocations across a month is
*yours*, and should not fall behind one from yesterday. Sessions are the
mirror image, since for a conversation recency **is** the question — you
resume what you were just doing.

That order used to be one number — kind priority plus a bonus — and the
arithmetic could not hold it. The agent cluster spans 25 points while the
bonus reached 60, so the bands stopped being bands: a much-used skill
outranked `claude` itself, and a config file outranked a skill. Comparing the
band first makes it structural rather than a matter of two constants staying
out of each other's way.

## Latency

Latency is the whole product. A launcher that takes 250ms to appear feels
broken.

```
$ prelude bench
gather: 2459 items  min 24.6ms  median 25.0ms  max 25.2ms
budget: 40ms  ->  OK
```

The per-keystroke helpers take roughly 2–3ms on the same machine. That matters
because fzf re-invokes the binary whenever the query changes.

Sources are tiered by cost:

- **Local** (history, scripts, skills, snippets, git, apps) — file reads only.
- **Fast external** (zoxide, docker, `fd`, `ps`) — run concurrently against a
  50ms deadline; a straggler falls back to its cached result and finishes
  writing a fresh one while you read the list.
- **Slow external** (`lsof` for ports, ~65ms and not improvable — restricting
  it to one user is *worse*) — always served from cache, refreshed detached.
  Safe to be stale because the generated command is `kill $(lsof -ti tcp:3000)`,
  which re-resolves the pid at run time rather than trusting the cached one.
- **`$PATH`** (~250ms to scan) — cached, refreshed detached. Only the very
  first run pays full price.

## Every command

`prelude --help` prints this. It is split by who runs it, because that is the
real division: the top half you press a key for, the bottom half your agents
run for themselves.

**You**

| | |
|---|---|
| `prelude` | search — what the Ctrl-R widget calls |
| `prelude reply` | answer the oldest question an agent is blocked on |
| `prelude fleet` | every agent running, and its state |
| `prelude fleet --status` | one line for a status bar |
| `prelude watch` | notify the moment an agent stops and waits |
| `prelude global install` | install and start the Cmd+Space launcher panel |
| `prelude global status [--json]` | panel, chord, zsh and Spotlight diagnostics |
| `prelude global open` | reveal the panel without pressing the hotkey |
| `prelude global hotkey CHORD` | inspect or change the validated global chord |
| `prelude global directory [PATH]` | where the panel stands · `--default` for `$HOME` |
| `prelude global start\|stop\|uninstall` | manage or remove the panel instance |
| `prelude settings [--json]` | every preference and its current value |
| `prelude settings add-root PATH` | what `f:` indexes · `remove-root` · `roots` |
| `prelude settings set KEY VALUE` | `key`, `height`, `preview`, `classic_enter` |

**Your agents**

| | |
|---|---|
| `prelude ask TEXT` | ask you and wait · `--timeout=N` · `--no-wait` |
| `prelude tell TEXT` | tell you, without waiting |
| `prelude say WHO TEXT` | send a line to another running agent |
| `prelude inbox [--json]` | what was left for you · `--all` · `--human` |
| `prelude drain` | mark your inbox collected |
| `prelude answer ID TEXT` | the return path |
| `prelude answer-of ID` | collect a `--no-wait` answer |
| `prelude fleet --json` | who else is running |
| `prelude control [--json]` | Agent/Run/Session/Skill/MCP relationship graph |
| `prelude agents [--json]` | the agent overview, as data |
| `prelude sessions [--json]` | every past conversation, as data |
| `prelude skills [--json]` | every skill and who has it, as data |

**Setup**

| | |
|---|---|
| `prelude init zsh` · `agent` | shell integration, and the block for CLAUDE.md |
| `prelude index` | build the file index for `f:name` |
| `prelude doctor` | diagnose the setup |
| `prelude doctor mcp` | MCP transport, health, tool inventory and privacy diagnostics |
| `prelude bench` | measure candidate-gathering |
| `prelude build-translate` | compile the Apple translation helper |

## Configuration

Everything below has a row in `set:` and a `prelude settings` command. The
variables remain, and still win where they are exported.

| Variable | Setting | Effect |
|---|---|---|
| `PRELUDE_KEY='^T'` | `key` | Change the shell hotkey (default `^R`) |
| `PRELUDE_HEIGHT=80%` | `height` | How much of the terminal the inline view uses |
| `PRELUDE_NO_PREVIEW=1` | `preview` | Disable `Ctrl+P` Quick Look |
| `PRELUDE_CLASSIC_ENTER=1` | `classic_enter` | Enter inserts everything, whatever kind |
| `PRELUDE_DEBUG=1` | — | Report source failures and fzf fallbacks on stderr |

Files, all under the usual XDG locations:

| Path | What |
|---|---|
| `$XDG_CONFIG_HOME/prelude/settings.toml` | Key, height, Quick Look and Enter behaviour |
| `$XDG_CONFIG_HOME/prelude/snippets.toml` | Your snippets |
| `$XDG_CONFIG_HOME/prelude/quicklinks.toml` | Your quicklinks |
| `$XDG_CONFIG_HOME/prelude/global.toml` | Global hotkey and the panel's starting directory |
| `$XDG_CONFIG_HOME/prelude/roots.txt` | Which folders `f:` indexes |
| `$XDG_DATA_HOME/prelude/frecency.tsv` | What you pick, so it learns |
| `$XDG_DATA_HOME/prelude/sessions.json` | Local Session names, pins and archive state |
| `$XDG_DATA_HOME/prelude/exports/` | Private raw Session exports |
| `$XDG_DATA_HOME/prelude/clipboard.jsonl` | Clipboard history metadata |
| `$XDG_DATA_HOME/prelude/clipboard/` | Private image payloads retained by clipboard history |
| `$XDG_DATA_HOME/prelude/bus/` | Questions agents are waiting on — data, not cache, because an unanswered question must survive a cleared cache |
| `$XDG_CACHE_HOME/prelude/skill-hashes.json` | Background Skill-tree fingerprints and metadata stamps |
| `$XDG_CACHE_HOME/prelude/` | Other source caches |

## Translation

`en:文字` or `zh:text` translates using Apple's on-device models — offline,
nothing leaves your machine. Build the helper once:

```sh
prelude build-translate     # needs Xcode's swiftc
```

Apple's `Translation` framework is only vended through a SwiftUI view modifier
and refuses to serve a bare CLI binary — the XPC call to `translationd` hangs
forever with no error. It works as soon as the caller has real app-bundle
identity, so this compiles a tiny Swift helper and wraps it in an
ad-hoc-signed `.app`. No developer account needed.

Two quirks, both handled: auto-detection hangs indefinitely on very short
input, so the source language is pinned by script instead; and the model is
fine for casual text but unreliable for legal or technical register.

## Platform

**macOS only, for now.** The core is portable, but ports, processes, apps,
clipboard, the system commands and translation all use macOS-specific
interfaces. Clipboard history reads `NSPasteboard`, so copied Finder objects
remain real files and images rather than being flattened into path text.
Images use Chafa when available, then Ghostty/Kitty or iTerm's native inline
protocol. Linux support is welcome but not present.

## Notes on the awkward parts

**zsh metafies its history file.** Any byte in `0x80-0x9f` is stored as `0x83`
followed by `byte ^ 32`. Read it as plain UTF-8 and every multi-byte character
comes back mangled, and the replacement characters have the wrong display
width — which silently breaks column alignment. Prelude undoes it first.

**East Asian *Ambiguous* width is a real fork.** `·` `—` `“”` `→` are one
column in most Western terminals and two in CJK-configured ones, and `·` is
the separator used on every row. Rather than infer it from `$LANG`,
`prelude doctor` prints one, asks the terminal where the cursor ended up
(`ESC[6n`), and caches the answer.

**fzf matches against displayed text.** A computed row can therefore never
fuzzy-match the query that produced it — you would type `en:…` and watch your
own answer get filtered out. Prelude uses a `change:transform` binding to
disable fzf's filtering for computed queries and re-enable it otherwise.

**Anything that reports the fleet must not be part of it.** Every agent
binary is also its own admin CLI, and Prelude asks each one for its MCP
status on every refresh — so `r:` listed *its own probes* as a dozen phantom
agents in whatever project you launched from. A subcommand like `mcp` or
`doctor` is tooling, not a conversation, and is filtered before anything else
happens. (`claude -p` and `codex exec` are real runs, and are marked rather
than dropped.)

**Silence lies, in the expensive direction.** An agent three minutes into a
build is exactly as quiet as one that asked you a question, so a clock alone
reports it as stuck — and a badge that cries wolf twice is worth less than no
badge at all. The conversation file settles it: an assistant turn ending in a
tool call is *acting*, one ending in prose has handed back to you. Mid-turn
beats any clock, and the file is only read when the clock was about to say
"waiting", so eighty busy agents cost nothing.

**A silent refusal is worse than a wrong one.** The zsh widget captures our
stdout inside `$(...)` with stderr discarded, so for a long time every
explanation — *that agent has no way to borrow a skill*, *this server's env
holds an API key* — arrived as nothing at all: the window shut, the prompt
was unchanged, and all the care taken over when to say no was invisible.
Refusals now travel the same road as results, as a third verb the widget
knows and renders with `zle -M`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports with a screenshot of
the list rendering wrong are the most useful thing you can send.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
