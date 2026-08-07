# Prelude

A Raycast-style launcher for your terminal — and the place your agents can
reach you from.

Press one key. A search box appears. Type a few letters. It searches
everything you might want to run — this project's scripts, your shell
history, listening ports, running processes, installed apps, your clipboard,
your snippets — and **types the command onto your prompt.**

You press Enter to actually run it.

```
╭─ Prelude ────────────────────────────────────────────────────────────╮
│ ⌕ dev                                                        4/2280  │
│ ▸ pnpm dev        · package.json · vite --host --port 3000    script │
│   npm run dev                                                history │
│   :3000 node      · node · pid 4821                             port │
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
│ ⌕                                                            1/2447  │
│ ▸ claude · api-gateway asks · asked 4m ago · The migration drops… asking you │
│   claude · docs        · waiting 12m · work:2.1 · fix the limiter    running │
│   pnpm dev             · package.json · vite --host --port 3000       script │
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

**A command line goes onto your prompt, never executed.** History, scripts,
`$PATH`, snippets, ports, processes — and agents, skills and sessions too,
because those are the ones you most often want to add something to:
`claude` wants `--resume`, or a model, or a question. One keystroke buys you
that, and costs nothing on the times you just press Enter.

That is also what makes it safe to bind this to a key you hit dozens of
times a day, and safe for the launcher to offer killing whatever holds port
3000 — but safety is the second reason, not the first. `claude` is harmless
and still gets handed over.

**An object just happens.** Files, apps, links, results: there is no command
here anyone would read, let alone edit. `open -a Zed foo.json` is not
something you want to proofread.

| | `Enter` |
|---|---|
| **a question an agent asked** | **answers it, and unblocks the agent** |
| an agent, a skill, a session | onto your prompt, ready to edit |
| a file or config | opens in an application |
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

### In a conversation, there is no prompt to paste onto

From the popup over an agent's input box, "onto your prompt" means something
else entirely: whatever you hand over lands in a *conversation*. So a file
gives its path and a command gives its text — both useful to say to an agent
— but an agent row cannot. Typing `codex` into Claude's box sends Claude the
word "codex".

The only reading of "start it" that means anything there is **a second agent
beside the first**, so that is what Enter does: it splits the pane you are
in, in the same directory, and starts the agent there. Both conversations
stay on screen — which is the whole reason to open one while talking to
another. Side by side when the window is wide enough for two, stacked below
about 170 columns, because an agent's TUI starts wrapping its own output
much under eighty.

`^K` always states the current default as its first entry, and
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

> Ctrl-R replaces zsh's incremental history search, which moves to Ctrl-S.
> Prelude is a superset of it. Override with `PRELUDE_KEY='^T'` set before the
> `eval` line. Ctrl-Space is deliberately not used: macOS binds it to
> "Select the previous input source" and the OS eats it first.

Two more, both optional and both one line:

```sh
prelude init tmux  >> ~/.tmux.conf     # the launcher over any pane, and a status segment
prelude init agent >> CLAUDE.md        # teach your agents they can reach you
```

The second is what makes the first half of this README happen at all — an
agent that has not been told it can ask you a question will not ask you a
question. See [teaching an agent](#teaching-an-agent-it-can-do-this).

## Running dozens of agents at once

`s:` searches conversations on disk. **`r:` searches what is alive right
now** — every agent process on the machine, wherever it happens to be
running: a tmux pane, a terminal tab, over ssh, or nothing at all:

```
▸ claude · api-gateway     · waiting 12m · work:2.1  · fix the rate limiter   running
  claude · TerminalRaycast · waiting 32m · pid 2557  · Create Raycast-like…   running
  codex  · docs            · working     · fleet:0.1 · rewrite the README     running
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

**None of that needs tmux.** The backbone is the process list — `ps` for what
is alive, one bulk `lsof` for where each one is — and the clock is the mtime
of the conversation file each agent is appending to. The detail pane reads
that file too, so you can see what an agent last said without going anywhere
near its terminal:

```
what it last said
⏺  Found it: one session's iCloud path is 127 columns wide…
›  use a percentile instead of the max
```

tmux is an enhancement, not a requirement. A run that has a pane also has an
address, and gains the two things a bare pid cannot offer:

| | anywhere | in a tmux pane |
|---|---|---|
| listed, with project and age | ✅ | ✅ |
| working or waiting | ✅ | ✅ |
| read what it last said | ✅ | ✅ (its live screen) |
| end it | ✅ | ✅ |
| **Enter goes to it** | — | ✅ |
| **answer it without leaving** | — | ✅ |

So `^K` on a pane offers "Send it a line, without going there" — typed
straight into its pane — while on a stray it offers what a pid allows: its
directory, and ending it.

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

`fleet --status` is made for a tmux status bar — `prelude init tmux` prints
the two lines to uncomment — and never pays for subprocesses itself: cached
identities, live states, exactly the launcher's deal. An idle machine shows
an empty segment, not a permanent `0 waiting`.

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
            Go to it, zoomed full-screen
```

The tmux status bar leads with the count, because a question is your problem
in a way that a merely-quiet agent is not:

```
1 asking · 2 waiting · 3 working
```

### Agents talking to each other

`prelude fleet --json` gives an agent the same fleet the launcher shows —
project, state, working directory, pane address — so "check whether anyone
else is already on this" becomes a command rather than a guess.

`prelude say` delivers a line straight into another agent's conversation,
attributed so the receiver knows it came from a peer rather than from the
human:

```
[via prelude, from claude · api-gateway] I changed the auth schema — you will need to rebase
```

Address it by project, agent name, or pane. **If more than one thing matches
it refuses and lists them**, because a message delivered to the wrong
conversation reads as the human's own words and is worse than one not sent.
An agent with no pane cannot be typed into, so the line waits in its inbox
for `prelude inbox`.

### Teaching an agent it can do this

```sh
prelude init agent >> CLAUDE.md      # or AGENTS.md, or a skill
```

That prints the whole interface as instructions — the four verbs, what each
returns, and *when to reach for them*, which is the part that matters. A
capability an agent has to be reminded of every session is not a capability
it has.

## Using it inside an agent conversation

The Ctrl-R widget only exists at a shell prompt. Inside a coding agent's TUI
the agent owns the keyboard, so nothing bound in your shell can reach you.

tmux sits *above* whatever runs in the pane, so a tmux binding works
everywhere:

```sh
prelude init tmux >> ~/.tmux.conf
tmux source-file ~/.tmux.conf
```

**prefix + r** now opens the launcher in a floating popup over whatever you
are doing, and picking something **types it into the pane underneath** —
into the agent's input box, vim, a REPL, an ssh session — without submitting
it. `^O` instead runs the command inside the popup, so the agent never sees
it at all.

### Using another agent's skill mid-conversation

tmux says which agent a pane is running, so Prelude knows who you are talking
to, and a skill row hands over whichever of two things is useful:

| | `Enter` | secondary |
|---|---|---|
| the agent **has** the skill | `/skill-name` | `Read …/SKILL.md and follow it.` |
| it **does not** | `Read …/SKILL.md and follow it.` | `/skill-name` |

The second row is the point. `/skill-name` typed at an agent that does not
have the skill is a line of prose that means nothing, and it fails silently.
Pointing that agent at the file needs no restart, no flag, and no cooperation
from its CLI — a skill is a file of instructions, and every agent can read a
file. It is the only way in for codex and opencode, which cannot load a
borrowed skill at all.

If you want the skill loaded properly *as* a skill without losing the
conversation, resume it with the loan attached:

```sh
claude --resume <session-id> --plugin-dir ~/.cache/prelude/borrow/<skill>
```

Same conversation, borrowed skill available. `s:` finds the session id.

## Keys

Two, and neither needs anything configured:

| Key | Prelude |
|---|---|
| `Enter` | The obvious thing for what you selected |
| `Ctrl+K` | Everything else for this item |
| `Esc` | Close |

Ctrl, because Ctrl is what a terminal reliably receives. macOS spends Option
on composing characters unless the terminal is told otherwise — so `Option+K`
would type `˚` into the search box — and it never delivers Cmd to a terminal
at all. A key that works on one machine and silently does nothing on the next
is worse than no key.

There is still a **secondary action** — Enter's opposite, per item — but it
has no key of its own. It is the second entry of the `Ctrl+K` panel, right
under Enter's, where it is spelled out instead of remembered:

```
▸ Open in editor              Enter
  Insert the full path        the other one
  Copy absolute path
  cd to its folder
```

The two are opposites: where Enter *does* something, the other one hands you
the text; where Enter hands you text, the other one does the thing.

| | `Enter` | the other one |
|---|---|---|
| a command | insert it | run it |
| an agent | insert it | start it |
| a file | open it | insert its path |
| an app | launch it | insert its name |
| a port | insert the kill | show what is using it |
| a result | copy it | insert it |
| a session | resume it | cd to where it ran |

Keys are spelled out rather than drawn as glyphs. A row of symbols is only
legible to someone who already knows what they mean.

```
⏎ open in editor        ^K actions   esc close     ← on a file
⏎ run it with an agent  ^K actions   esc close     ← on a skill
```

A launcher whose header is a row of shortcuts has already failed to have an
obvious default. `^O` (run here), `^X` (run in the shell), `^Y` (copy) and
`^P` (detail pane) still work if you learned them; they are simply no longer
advertised, and all of them are in `^K`.

Open the launcher itself with `^R`.

## What it searches

| Source | Where it comes from |
|---|---|
| **Project scripts** | `package.json`, `Makefile`, `justfile`, `Cargo.toml`, `pyproject.toml`, `docker-compose.yml` — found by walking up from `$PWD`, with the runner picked from your lockfile |
| **Ports** | Listening TCP ports — "what's on :3000, kill it" |
| **Processes** | Heaviest by CPU and memory |
| **Containers** | Running Docker containers |
| **Clipboard** | The last things you copied |
| **Snippets** | `snippets.toml`, with `{{placeholder}}` blanks |
| **Agent skills** | Merged across Claude Code, Codex, pi, opencode |
| **MCP servers** | Merged across every agent that has any |
| **Apps** | Every installed `.app` |
| **SSH hosts** | `~/.ssh/config` |
| **Files** | The current project, plus an indexed set of roots via `f:name` |
| **History** | Deduped, newest first |
| **Folders** | zoxide's database, or `cd` targets mined from history |
| **Git** | Branches read straight off `.git/refs` |
| **`$PATH`** | Every executable, ranked lowest |

Plus rows computed from what you type: arithmetic, unit and currency
conversion, date arithmetic, on-device translation, and quicklinks. Once a
query declares an intent this way, the rest of the list disappears — you
asked for one thing, so you see one thing.

```
10kg to lb        →  22.046226 lb        1847*0.23     →  424.81
1gb to mb         →  1,024 mb            now + 3 days  →  2026-08-10 …
100 usd to cny    →  676.04 CNY          1699999999    →  2023-11-15 …
en:你好            →  Hello               g rust async  →  opens Google
```

## The action panel

`^K` gives a result verbs appropriate to *what it is*, not just "insert":

| Kind | Actions |
|---|---|
| **question** | Answer "go ahead" · Answer "no" · Go to it, zoomed · cd to its project |
| **port** | Insert kill · Kill now · Show what's using it · Copy pid |
| **process** | Insert kill · Kill now · Show full command · Copy pid |
| **container** | Shell into · Follow logs · Stop · Restart |
| **skill / mcp** | Lend it to another agent for one run · Copy it there for good · Insert name · Show description · Open in editor |
| **file** | Insert path · Open in `$EDITOR` · Copy absolute path |
| **snippet** | Insert and fill blanks · Edit snippets file |

That is the difference between a command picker and a launcher: a port is not
text to insert, it is something you kill or inspect.

## One surface for every agent

Open the launcher and this is what you see first, before typing anything:

```
▸ claude    agent · 9 skills · 3 mcp ·  28 sessions
  codex     agent · 2 skills · 3 mcp · 490 sessions
  pi        agent · 2 skills · 0 mcp · 120 sessions
  cnipa-ooa skill · claude, shared · used 8× · 19h ago
  Gmail     mcp   · claude · ✔ connected
```

`a:` shows exactly the same thing, filtered — the two share one code path so
they cannot drift apart. Everything else is one keystroke away through
search.

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

The most recent 15 appear in the main list; `s:` searches all of them.

**Lend a skill or an MCP server to an agent that lacks it.** Prelude knows
which agents have a skill, so it knows which do not — and every agent turns
out to have a flag for taking one it does not own, for a single run:

```
▸ my-skill    skill · claude, shared · missing: codex, pi
     ^K  →  Use it in pi, just this run       ← nothing installed
            Copy it to codex / Copy it to pi  ← keep it for good

▸ node_repl   mcp · codex · ✔ connected
     ^K  →  Lend it to claude for one run
```

Borrowing installs nothing and leaves nothing behind. You get the command,
you press Enter, and the loan ends when that process does.

| | MCP | skill |
|---|---|---|
| claude | `--mcp-config` | `--plugin-dir` |
| codex | `-c mcp_servers.…` | — |
| pi | — | `--skill` |
| opencode | — | — |

A dash means no such flag exists, so nothing is offered — `Copy it to …` is
the answer there. Two cases refuse on purpose: a **claude.ai-hosted** server
has no definition to lend, because its credentials live with the Claude
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
full-screen TUI taking over the terminal. Resuming an existing session still
goes onto your prompt, because a long-lived session belongs in a real
terminal rather than a popup that closes.

Each agent is invoked with its own syntax (`opencode` needs `opencode run`,
the others take the prompt positionally), and `@` completes only against
agents actually installed.

**Jump to any agent config** — `CLAUDE.md`, `AGENTS.md`, `settings.json`,
`config.toml`, `.mcp.json` — as a first-class row.



Prelude reads skills and MCP servers from every agent CLI you have installed
and merges them into one list. A skill present in several agents is shown
once, labelled with which agents have it, rather than duplicated or silently
deduped.

Skills come from `~/.claude/skills`, `~/.agents/skills`, `~/.codex/skills`,
`~/.pi/agent/skills` and `~/.config/opencode/skills`; MCP servers from
`~/.codex/config.toml`, `~/.claude.json` and any project-local `.mcp.json`.
`prelude doctor` prints the inventory.

## It learns

Every selection is recorded in `frecency.tsv` — a plain text file you can read
and edit. What you actually pick floats up, with recent use weighted far above
old use.

## Latency

Latency is the whole product. A launcher that takes 250ms to appear feels
broken.

```
$ prelude bench
gather: 2364 items  min 20.4ms  median 22.1ms  max 28.6ms
budget: 40ms  ->  OK
```

Startup is ~1.7ms, which matters because fzf re-invokes the binary on every
keystroke to decide what to show.

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
| `prelude paste [pane]` | type the result into a tmux pane instead |
| `prelude reply` | answer the oldest question an agent is blocked on |
| `prelude fleet` | every agent running, and its state |
| `prelude fleet --status` | one line for a tmux status bar |
| `prelude watch` | notify the moment an agent stops and waits |

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
| `prelude agents [--json]` | the agent overview, as data |
| `prelude sessions [--json]` | every past conversation, as data |
| `prelude skills [--json]` | every skill and who has it, as data |

**Setup**

| | |
|---|---|
| `prelude init zsh` · `tmux` · `agent` | shell, tmux, and the block for CLAUDE.md |
| `prelude index` | build the file index for `f:name` |
| `prelude doctor` | diagnose the setup |
| `prelude bench` | measure candidate-gathering |
| `prelude build-translate` | compile the Apple translation helper |

## Configuration

| Variable | Effect |
|---|---|
| `PRELUDE_KEY='^T'` | Change the hotkey (default `^R`) |
| `PRELUDE_HEIGHT=80%` | How much of the terminal the inline view uses |
| `PRELUDE_NO_POPUP=1` | Never open a tmux popup; render inline |
| `PRELUDE_PREVIEW_MIN=150` | Terminal width at which the detail pane appears |
| `PRELUDE_NO_PREVIEW=1` | Never show the detail pane |
| `PRELUDE_DEBUG=1` | Report source failures and fzf fallbacks on stderr |

Files, all under the usual XDG locations:

| Path | What |
|---|---|
| `$XDG_CONFIG_HOME/prelude/snippets.toml` | Your snippets |
| `$XDG_CONFIG_HOME/prelude/quicklinks.toml` | Your quicklinks |
| `$XDG_CONFIG_HOME/prelude/roots.txt` | Which folders `f:` indexes |
| `$XDG_DATA_HOME/prelude/frecency.tsv` | What you pick, so it learns |
| `$XDG_DATA_HOME/prelude/clipboard.jsonl` | Clipboard history |
| `$XDG_DATA_HOME/prelude/bus/` | Questions agents are waiting on — data, not cache, because an unanswered question must survive a cleared cache |
| `$XDG_CACHE_HOME/prelude/` | Source caches |

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
interfaces. Linux support is welcome but not present.

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
