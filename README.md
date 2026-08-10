# Prelude

<p align="center">
  <strong>A fast launcher and control plane for terminal-first work on macOS.</strong><br>
  Search the Mac, manage coding agents, and keep commands reviewable.
</p>

<p align="center">
  <a href="#install"><strong>Install</strong></a> ·
  <a href="#how-prelude-is-different"><strong>Why Prelude</strong></a> ·
  <a href="#agent-control-plane"><strong>Agents</strong></a> ·
  <a href="docs/SEARCH.md"><strong>Search guide</strong></a>
</p>

![Prelude showing agents, active runs, skills, MCP servers, and recent sessions](docs/assets/prelude.png)

Prelude puts commands, files, apps, clipboard history, projects, coding agents,
sessions, skills, and MCP servers behind one shortcut. It is built in Rust on
[fzf](https://github.com/junegunn/fzf), stays local, and targets a gather time
below 40 ms.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/mikewang817/Prelude/main/install.sh | bash
```

That one command downloads the native binary for your Mac, provides a compatible
`fzf`, installs the official signed Ghostty app when needed, adds the zsh widget,
and starts the login-persistent global panel. No Rust toolchain, repository
clone, Homebrew setup, or manual shell editing is required.

On first install, macOS asks for Ghostty's Accessibility permission so it can
register the global shortcut. Then:

- **`Cmd+Shift+Space`** opens Prelude from anywhere.
- **`Ctrl+R`** opens Prelude at the current zsh prompt.

Run the same command again to upgrade or repair the installation. The installer
is idempotent.

## How Prelude is different

Most desktop launchers are excellent at opening apps, files, and web searches.
Prelude keeps those basics, but is designed around work that begins in a
terminal and increasingly spans several coding agents.

| A conventional launcher | Prelude |
|---|---|
| Opens apps, files, and links | Also searches commands, shell history, clipboard objects, projects, sessions, skills, MCP servers, and live agent runs |
| Runs a selected command immediately | Inserts it into the prompt, or copies it from the global panel, so it remains visible and editable |
| Treats every result as text | Knows whether a result is a command or an object; objects open directly through macOS, while commands are handed back to you |
| Adds AI as a chat box | Shows the operational state around local agents: runs, waiting questions, conversations, capabilities, ownership, and health |
| Integrates with one assistant at a time | Normalizes Claude Code, Codex, pi, and OpenCode in one local inventory |
| Offers fixed secondary actions | Builds a contextual `Ctrl+K` panel for the focused item and states exactly what `Enter` will do |

Prelude is not a terminal dashboard and does not create a terminal for every
action. The global panel is a hidden Ghostty Quick Terminal: objects open where
they belong, while commands go to the clipboard for deliberate handoff.

## Agent control plane

Prelude treats agents as managed local objects rather than prompt providers.
From one surface you can:

- see which agents are **working, waiting, or no longer running**
- answer a waiting question without finding the originating terminal
- resume native Claude Code, Codex, and pi conversations
- inspect Skill ownership and integrity across supported agents
- archive, restore, copy, compare, or lend Skills for one run
- compare MCP ownership, health, tools, and redacted definitions
- archive and restore MCP servers in Prelude without disabling native configs
- send local agent-to-human and agent-to-agent messages

Archiving is a reversible Prelude preference: it never removes a Skill or
changes an MCP definition. Its metadata contains only stable object keys—never
paths, commands, definitions, or credentials. Shared run records likewise omit
full prompts and private MCP definitions.

The same state is available from the shell:

```sh
prelude fleet
prelude watch
prelude ask "The migration drops legacy_users. Proceed?"
prelude say api-gateway "I changed the auth schema; rebase before editing"
```

No separate server is required. Identity comes from the process tree and the
current project; messages stay on the local machine.

## Search without the noise

Large collections live behind explicit scopes, so an empty query remains a
useful agent home instead of a wall of files and history.

```text
a:waiting           agents waiting for input
s:is:pinned         pinned Claude, Codex, and pi conversations
skill:is:archived   archived Skills, ready to restore
mcp:is:archived     archived MCP servers, ready to restore
/cnipa-ooa          invoke an installed Skill
@claude explain …   ask an installed agent
f:tag:work          files carrying the Finder tag “work”
c:                  clipboard text, images, and Finder objects
h:git rebase        complete shell history
app:zed             installed applications
10kg to lb          inline calculation and conversion
:                   every available scope
```

Prelude is type-aware: a clipboard image previews as pixels, a file reveals in
Finder, a Session resumes in its native agent, and a process exposes its PID
before a confirmed kill action.

| Key | Action |
|---|---|
| `Enter` | Perform the focused row's stated default |
| `Ctrl+K` | Open contextual actions |
| `Ctrl+P` | Toggle Quick Look |
| `Cmd+Enter` | Reveal a file in Finder from the global panel |
| `Escape` | Go back or close Prelude |

## Local and deliberate

- Sensitive history and clipboard text are filtered; MCP details and exports
  are redacted before becoming search data.
- Destructive file, app, Skill, and inactive-Session actions confirm first and
  move the item to the Trash.
- Slow sources are cached. A failed source disappears instead of blocking the
  launcher or printing into it.
- Prelude's indexes, metadata, clipboard records, and message bus stay in its
  XDG directories on your Mac.

Check an installation with `prelude global status` and optional integrations
with `prelude doctor`.

## Documentation

- [Search scopes and query grammar](docs/SEARCH.md)
- [Actions and safety model](docs/ACTIONS.md)
- [Global panel architecture](docs/GLOBAL-HOTKEY.md)
- [Agent control plane](docs/AGENT-CONTROL-PLANE.md)
- [Build from source and contribute](CONTRIBUTING.md)

Prelude is macOS-only and licensed under [Apache-2.0](LICENSE).
