# Prelude

<p align="center">
  <strong>The command center for terminal-first work on macOS.</strong><br>
  Search your Mac. Control your coding agents. Keep your hands on the keyboard.
</p>

<p align="center">
  <a href="#install"><strong>Install</strong></a> ·
  <a href="#one-search-box-your-whole-workspace"><strong>Explore features</strong></a> ·
  <a href="docs/SEARCH.md"><strong>Search guide</strong></a> ·
  <a href="CONTRIBUTING.md"><strong>Contribute</strong></a>
</p>

![Prelude agent home showing agents, runs, skills, MCP servers, and recent sessions](docs/assets/prelude-home.png)

Prelude turns one shortcut into a live index of your work: **commands, files,
apps, clipboard history, projects, agents, sessions, skills, and MCP servers**.
Find what you need, see exactly what `Enter` will do, and move on.

It is not another terminal dashboard. It is the layer between you and all the
things you reach for while building software.

## Why Prelude

| | |
|---|---|
| **One launcher, two surfaces** | Press `Ctrl+R` at a zsh prompt, or open a global Ghostty panel from anywhere on your Mac. |
| **An AI agent control plane** | See Claude Code, Codex, pi, and OpenCode together—running jobs, waiting questions, conversations, skills, and MCP servers. |
| **Commands stay reviewable** | Prelude inserts commands into your prompt or copies them from the global panel. It never guesses which terminal should execute them. |
| **Objects just open** | Files, folders, apps, and URLs go directly to macOS Launch Services instead of becoming shell commands. |
| **Every action is contextual** | `Enter` does the obvious thing, `Ctrl+K` shows alternatives, and the footer tells you what will happen before you act. |
| **Built for instant recall** | The gather path targets under 40 ms. Slow work is cached, and no network request or directory walk runs on every keystroke. |

## One search box. Your whole workspace.

```text
f:tag:work          files with the Finder tag “work”
c:                  text, images, and Finder objects from your clipboard
h:git rebase        complete shell history
app:zed             installed applications
s:is:pinned         pinned Claude, Codex, and pi conversations
a:waiting           agents waiting for your input
/cnipa-ooa          invoke an installed skill
@claude explain …   ask an installed agent
10kg to lb          calculate and convert inline
:                   browse every available scope
```

Large collections stay behind explicit scopes, so the home screen remains
useful instead of becoming a wall of thousands of files. Clear the query and
Prelude returns to your agent home.

### Your agents, at a glance

Prelude gives local coding agents a shared operational surface:

- see what is **working, waiting, or no longer running**
- resume native conversations without hunting for session IDs
- answer an agent question from the top row of the launcher
- inspect and move skills between supported agents
- compare MCP ownership, health, tools, and redacted definitions
- send local agent-to-human and agent-to-agent messages

```sh
prelude fleet
prelude watch
prelude ask "The migration drops legacy_users. Proceed?"
prelude say api-gateway "I changed the auth schema; rebase before editing"
```

No separate server is required. Identity comes from the process tree and the
current project; messages remain on the local machine.

### More than fuzzy search

Prelude understands the type of every result. That means a clipboard image can
preview as pixels, a file can reveal itself in Finder, a session can resume in
its native agent, and a process can expose its PID and a confirmed kill action.

| Key | Action |
|---|---|
| `Enter` | Perform the focused row's stated default |
| `Ctrl+K` | Open contextual actions |
| `Ctrl+P` | Toggle Quick Look |
| `Cmd+Enter` | Reveal a file in Finder from the global panel |
| `Escape` | Go back or close Prelude |

## Install

### Requirements

- macOS
- zsh
- [fzf](https://github.com/junegunn/fzf)
- Rust toolchain
- [Ghostty](https://ghostty.org/) only for the optional global panel

### 1. Build Prelude

```sh
brew install fzf rust
git clone https://github.com/mikewang817/Prelude.git
cd Prelude
cargo install --path .
```

### 2. Add it to zsh

```sh
echo 'eval "$(prelude init zsh)"' >> ~/.zshrc
exec zsh
```

Press **`Ctrl+R`** at any prompt. Prelude moves zsh's incremental history search
to `Ctrl+S`; your complete history remains available through `h:`.

### 3. Optional: launch from anywhere

```sh
prelude global install
```

The global panel is a hidden Ghostty quick terminal. Its default shortcut is
**`Cmd+Space`**, and you can change it without editing configuration files:

```sh
prelude global hotkey cmd+shift+space
prelude global status
```

Finish by checking the complete setup:

```sh
prelude doctor
```

## Designed for trust

- **Local-first:** indexes, clipboard records, agent state, and the message bus
  live in Prelude's XDG directories on your Mac.
- **Credential-aware:** secret-looking history, clipboard text, MCP details,
  tags, and exports are filtered or redacted rather than becoming search data.
- **Recoverable:** destructive file, app, skill, and inactive-session actions
  move items to the Trash and confirm first.
- **Failure-tolerant:** an unavailable source disappears instead of blocking or
  printing errors into the launcher.

## Go deeper

The README is the front door. The detailed behavior and design records live
here:

- [Search scopes and query grammar](docs/SEARCH.md)
- [Actions and safety model](docs/ACTIONS.md)
- [Global panel setup and architecture](docs/GLOBAL-HOTKEY.md)
- [Agent control plane](docs/AGENT-CONTROL-PLANE.md)
- [Contributing](CONTRIBUTING.md)

Prelude is macOS-only, built in Rust on top of
[fzf](https://github.com/junegunn/fzf), and licensed under
[Apache-2.0](LICENSE).
