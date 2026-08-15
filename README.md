# Prelude

**A launcher for people who work in a terminal.** Press a key, type a few
letters, get the thing you were after — a command, a file, an app, something you
copied earlier, or the conversation you were having with a coding agent.
Commands land on your clipboard so you read them before you run them; files,
folders and links just open.

## Use any skill, in the agent you already have open

You are talking to Claude Code. The skill you want was written for Gemini, so it
sits in a folder Claude has never read. Without leaving that conversation:

1. **`Cmd+Shift+Space`** — a panel drops in over whatever you are looking at,
   with your skills already on it. Nothing to type.
2. Move to the one you want.
3. **`Enter`** — the panel goes, and one line is on your clipboard.
4. **`Cmd+V`**, then `Enter`.

```text
Use the skill "example-skill": read /Users/you/.gemini/config/skills/example-skill/SKILL.md and follow it.
```

A skill is a `SKILL.md` file, and any agent can read a file — so nothing is
installed, nothing is copied, nothing restarts, and it works in the conversation
you already had going. Prelude reads the skill folder of every agent CLI it
knows about, so every skill on the machine is reachable from every agent on the
machine, whoever it was written for.

![Prelude showing skills and recent agent sessions](docs/assets/prelude.png)

## Install

macOS, Apple Silicon or Intel:

```sh
curl -fsSL https://raw.githubusercontent.com/mikewang817/Prelude/main/install.sh | bash
```

Allow Ghostty in **System Settings → Privacy & Security → Accessibility** when
asked — that is what makes the shortcut work. `Cmd+Shift+Space` then works
anywhere, including inside a running agent; `Ctrl+R` opens the same launcher at
a zsh prompt.

The installer sets up everything it needs — [what it touches, and how to remove
it](#boundaries). Run it again to upgrade or repair; `prelude global status`
checks it and `prelude global uninstall` removes it.

## Resume a conversation

The same shape, for sessions. `s:` lists conversations from Claude Code, Codex,
pi, omp and Kimi, newest first, with the project each was in. `Enter` copies
that agent's own resume command — `claude --resume 3f9c1a2e` — so you read it
before running it, and `Ctrl+K` offers rename, pin, fork, export, reveal and
Trash.

```text
s:agent:claude since:24h  Claude Code, from today
s:project:Prelude         whatever you were doing in this project
a:waiting                 Runs or questions waiting for input
```

Unlike skills, this needs the agent's own syntax — `claude --resume` cannot open
a Codex conversation — so it covers the agents Prelude has verified. Nineteen
Agent CLIs are registered in all, and an action is omitted when the owning CLI
has no known syntax for it, rather than built to look plausible.

Agents can also talk to you, and to each other, over a file-backed bus that
needs no server:

```sh
prelude fleet                                       # what is running
prelude ask "This drops legacy_users. Proceed?"     # blocks until answered
prelude say api-gateway "rebase before editing"     # into another Run's inbox
```

## Search

An empty query is a compact home. Type `:` to see every scope.

```text
@claude explain this      ask an installed Agent and show its answer
Prelude                   files and folders named Prelude
c:                        clipboard text, Finder objects and images
h:git rebase              recent, filtered shell history
app:zed                   installed applications
10kg to lb                unit conversion
```

Ordinary queries mix in the apps, files and folders whose own names match, and
end with a web search so nothing dead-ends. History, clipboard rows and `$PATH`
commands stay behind their scopes. Search starts at `~/App`, `~/Documents` and
`~/Desktop`; `set:` changes that and everything else. Full grammar in [the
search guide](docs/SEARCH.md).

## Keys

| Key | Action |
|---|---|
| `Enter` | Perform the focused row's stated default |
| `Ctrl+K` | Open contextual actions |
| `Tab` | Complete the focused scope command or keyword |
| `Ctrl+P` | Toggle Quick Look |
| `Ctrl+Enter` | Reveal in Finder |
| `Ctrl+Shift+Enter` | Open Ghostty in that folder |
| `Ctrl+Option+Enter` | Copy the absolute path and close |
| `←` `→` | Adjust the focused setting in `set:`; otherwise move a level |
| `Escape` | Go back one level; close at the outermost |

## Names

A **Quicklink** is a keyword you type to reach one thing; an **alias** is a name
for something Prelude already has; a **Favorite** is promotion without a name.
`Ctrl+K` creates all three, `ql:` and `set:` manage them. A name is refused the
moment you type it if something already owns it, never later.

```sh
prelude quicklink add jira 'https://jira.example.com/issues?jql={q}' Jira
prelude alias add browser "Google Chrome"
```

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
