# Prelude

**A launcher for people who work in a terminal.**

Press a key, type a few letters, get the thing you were after: a command, a
file, an app, a link, something you copied earlier, or the conversation you were
having with a coding agent.

Commands land on your clipboard, so you read them before you run them. Files,
folders and links just open.

![Prelude showing skills and recent agent sessions](docs/assets/prelude.png)

## Install

macOS, Apple Silicon or Intel:

```sh
curl -fsSL https://raw.githubusercontent.com/mikewang817/Prelude/main/install.sh | bash
```

Allow Ghostty in **System Settings → Privacy & Security → Accessibility** when
asked — that is what makes the shortcut work. Then press **`Cmd+Shift+Space`**,
from anywhere, in anything. `Ctrl+R` opens the same launcher at a zsh prompt.

The installer sets up everything it needs — [what it touches, and how to remove
it](#boundaries). Run it again to upgrade or repair; `prelude global status`
checks it and `prelude global uninstall` removes it.

## Use any skill, in the agent you already have open

You are in a terminal, talking to Claude Code. You remember a skill you want —
but you wrote it for Gemini, so it sits in a folder Claude has never read.
Normally that means copying the folder, or restarting with a flag, or giving up.

Instead, without leaving that conversation:

1. Press **`Cmd+Shift+Space`**. A panel drops in over whatever you are
   looking at — you do not leave the agent, and nothing new is launched.
2. Type **`/`** and a few letters of the skill's name.
3. Press **`Enter`**. The panel disappears and one line is on your clipboard.
4. Press **`Cmd+V`**, then `Enter`.

The line is an instruction, and it names the file:

```text
Use the skill "cnipa-ooa": read /Users/you/.gemini/config/skills/cnipa-ooa/SKILL.md and follow it.
```

That is the whole mechanism. A skill is a `SKILL.md` file, and any agent can
read a file — so nothing is installed, nothing is copied, no session is
restarted, and it works in the conversation you already had going. Prelude reads
the skill folder of every agent CLI it knows about, so every skill on the
machine is reachable from every agent on the machine, whoever it was written
for.

`Cmd+Shift+Space` is the one that matters here. `Ctrl+R` opens the same
launcher, but only from a shell prompt — while an agent is running, the shell
is not at a prompt and that key belongs to the agent. The chord works anywhere,
including from inside a conversation.

Type `skill:` to browse them all.

## Pick up a past conversation

The same shape, for sessions. Type `s:` to see conversations from Claude Code,
Codex, pi, omp and Kimi — newest first, with the project each one was in.

```text
s:                        every conversation
s:agent:claude since:24h  Claude Code, from today
s:project:Prelude         whatever you were doing in this project
```

`Enter` copies that agent's own resume command, so you paste and read it before
you run it:

```sh
claude --resume 3f9c1a2e
```

A resume command has to be the agent's own — `claude --resume` cannot open a
Codex conversation — so unlike skills, this works only for the agents Prelude
has verified syntax for. `Ctrl+K` on a session offers the rest: rename, pin,
fork, export, reveal it in Finder, or move it to the Trash.

## Search

An empty query is a compact Agent home. Type `:` to see every scope.

```text
a:waiting                 Runs or questions waiting for input
s:agent:claude since:24h  recent Claude Code Sessions
/cnipa-ooa                run an installed Skill and show its answer
@claude explain this      ask an installed Agent and show its answer
Prelude                   files and folders named Prelude
c:                        clipboard text, Finder objects and images
h:git rebase              recent, filtered shell history
app:zed                   installed applications
10kg to lb                unit conversion
```

Ordinary queries mix in the apps, files and folders whose own names match, and
end with a web search so nothing dead-ends. History, clipboard rows and `$PATH`
commands stay behind their scopes. Search folders start at `~/App`,
`~/Documents` and `~/Desktop`; `set:` changes that and everything else.

See [the search guide](docs/SEARCH.md) for the full grammar.

## Agents

Prelude manages local Agent facts rather than replacing the Agents. Nineteen
Agent CLIs are registered; Sessions are discovered for Claude Code, Codex, pi,
omp and Kimi. It lists installed Agents and live Runs, classifies each Run as
working or waiting, browses and resumes Sessions, and merges Skills by name
across Agent directories. An action is omitted when the owning CLI has no known
syntax for it, rather than built to look plausible.

The message bus is file-backed and needs no server:

```sh
prelude fleet                                       # what is running
prelude ask "This drops legacy_users. Proceed?"     # blocks until answered
prelude say api-gateway "rebase before editing"     # into another Run's inbox
prelude inbox --json
```

See [the control plane](docs/AGENT-CONTROL-PLANE.md) for the support matrix.

## Keys

| Key | Action |
|---|---|
| `Enter` | Perform the focused row's stated default |
| `Ctrl+K` | Open contextual actions |
| `Tab` | Complete the focused scope command or keyword |
| `Ctrl+R` | Move the typed text into `h:`; again to come back |
| `Ctrl+P` | Toggle Quick Look |
| `Ctrl+Enter` | Reveal in Finder |
| `Ctrl+Shift+Enter` | Open Ghostty in that folder |
| `Ctrl+Option+Enter` | Copy the absolute path and close |
| `←` `→` | Adjust the focused setting in `set:`; otherwise move a level |
| `Escape` | Go back one level; close at the outermost |

## Names

A **Quicklink** is a keyword you type to reach one thing; an **alias** is a name
for something Prelude already has; a **Favorite** is promotion without a name.
`Ctrl+K` creates all three, `ql:` and `set:` manage them.

```sh
prelude quicklink add notes ~/Documents/notes
prelude quicklink add jira 'https://jira.example.com/issues?jql={q}' Jira
prelude alias add browser "Google Chrome"
```

Prelude ships keywords for search (`g`, `gh`, `ddg`, `mdn`…), for coding (`so`,
`crates`, `pypi`, `caniuse`…) and for agents (`hf`, `arxiv`, `ccdocs`). A name is
refused the moment you type it if something already owns it, never later.

An alias is also a hotkey: bind `open 'prelude://run?alias=browser'` in any
hotkey tool. Such links may only name an alias you created and only act on
objects macOS opens — never the clipboard, an agent or a shell.

## Boundaries

- macOS only. Both shortcuts run through Ghostty, and `Ctrl+R` also needs zsh;
  the installer fetches `fzf` and Ghostty if you do not already have them.
- Everything Prelude keeps lives in its XDG directories. Outside them it writes
  three things during setup — managed blocks in `~/.zshrc` and Ghostty's
  configuration, the LaunchAgent, and the `prelude://` handler — and `prelude
  global uninstall` removes them.
- The network is reached only when asked. The one exception is the update check:
  at most twelve times an hour, unauthenticated, sending no identifier, and
  `prelude settings set update off` stops it.
- Secret-looking history, clipboard text, messages, tags and exported
  transcripts are filtered or redacted.
- Deletion goes through the Trash and says where it put things. Irreversible
  termination confirms first. A failed source degrades to an empty or cached
  result rather than blocking the launcher.

## Documentation

- [Search scopes and query grammar](docs/SEARCH.md)
- [Defaults, actions, and safety](docs/ACTIONS.md)
- [Global panel architecture and lifecycle](docs/GLOBAL-HOTKEY.md)
- [Agent control plane model and support matrix](docs/AGENT-CONTROL-PLANE.md)
- [Build from source and contribute](CONTRIBUTING.md)

Prelude is licensed under [Apache-2.0](LICENSE).
