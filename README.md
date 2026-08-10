# Prelude

**A general launcher in the terminal, with an agent control plane and a message bus.**

Prelude gives macOS one searchable place for commands, files, applications,
clipboard history, projects, settings, agents, sessions, skills and MCP
servers. It has two ways to open:

- **`Ctrl+R` at a zsh prompt** — selected command text is inserted into that
  prompt.
- **A global Ghostty panel** — selected command text is copied to the clipboard
  so you can paste it into the terminal you actually intend to use.

Files, folders, applications and URLs are objects rather than shell commands.
They open directly through macOS Launch Services on both surfaces.

```text
╭─ Prelude ──────────────────────────────────────────────────────────────╮
│ ⌕ f:tag:work                                                   1/18    │
│ ▸ proposal.pdf       file · ~/Documents/Clients/Acme · #work           │
│ ────────────────────────────────────────────────────────────────────── │
│ Open it  Enter · Folder  Cmd+Enter · Actions  Ctrl+K · Preview  Ctrl+P │
╰────────────────────────────────────────────────────────────────────────╯
```

Prelude is macOS-only and built on [fzf](https://github.com/junegunn/fzf).
Ghostty is required only for the optional global panel.

## Contents

- [Install](#install)
- [Start using Prelude](#start-using-prelude)
- [Keys and actions](#keys-and-actions)
- [Search scopes](#search-scopes)
- [Files, Finder tags and local paths](#files-finder-tags-and-local-paths)
- [Clipboard history](#clipboard-history)
- [Quicklinks and web search](#quicklinks-and-web-search)
- [Agents, sessions, skills and MCP](#agents-sessions-skills-and-mcp)
- [Agent-to-human and agent-to-agent messages](#agent-to-human-and-agent-to-agent-messages)
- [Settings](#settings)
- [Command reference](#command-reference)
- [Configuration files](#configuration-files)
- [Safety, privacy and performance](#safety-privacy-and-performance)

## Install

### Requirements

- macOS
- zsh
- fzf
- Rust toolchain
- Ghostty, only if you want the global hotkey panel

```sh
brew install fzf rust

git clone https://github.com/mikewang817/Prelude.git
cd Prelude
cargo build --release
ln -s "$PWD/target/release/prelude" /usr/local/bin/prelude
```

Any directory already on `$PATH` can be used instead of `/usr/local/bin`.

### Install the zsh widget

```sh
echo 'eval "$(prelude init zsh)"' >> ~/.zshrc
exec zsh
```

Press **`Ctrl+R`** to open Prelude at the current prompt.

Prelude takes over zsh's default incremental history key. Incremental history
moves to `Ctrl+S`, while Prelude's complete shell history is available through
`h:`. To use another launcher key, set it before the `eval` line:

```sh
export PRELUDE_KEY='^T'
eval "$(prelude init zsh)"
```

Check the installation with:

```sh
prelude doctor
```

### Install the global panel

The shell widget is available only while a prompt is active. For a launcher
that can be opened from Finder, a browser or another application, install the
dedicated Ghostty panel:

```sh
prelude global install
prelude global status
```

The default chord is `Cmd+Space`. If macOS or another application already owns
it, Prelude refuses to replace the working configuration. Choose another:

```sh
prelude global hotkey cmd+shift+space
```

You can also choose the directory in which the panel gathers project-local
sources:

```sh
prelude global directory ~/src
prelude global directory --default   # return to $HOME
```

The panel is a hidden Ghostty quick-terminal instance kept out of the Dock and
app switcher. A key press reveals an existing instance; it does not construct a
new terminal. `Escape` hides it and resets the search for the next use.

Manage it with:

```sh
prelude global start
prelude global stop
prelude global status
prelude global uninstall
```

`prelude global uninstall --reset` also removes Prelude's saved global-panel
preferences and status records.

## Start using Prelude

### 1. Open it

- At a zsh prompt: `Ctrl+R`
- From anywhere: the configured global chord

### 2. Search

An empty query is the **agent home**. It shows questions waiting for you,
installed and running agents, skills, MCP servers and recent sessions.

Start typing to search the small root command layer. Large sources are opened
through explicit scopes:

```text
f              Search Files
f:             all indexed files
f:proposal     files matching proposal
c:             clipboard history
h:git          shell history containing git
app:zed        applications matching Zed
:              every available scope
```

Selecting a scope command completes its prefix without closing Prelude.
Clearing the query returns to the agent home.

### 3. Read the footer

The footer always states what `Enter` will do to the focused row:

```text
Insert into prompt  Enter · Actions  Ctrl+K · Preview  Ctrl+P
Copy the command    Enter · Actions  Ctrl+K · Preview  Ctrl+P
Open it             Enter · Actions  Ctrl+K · Preview  Ctrl+P
```

The wording changes with both the selected object and the surface you opened.

## Two surfaces, one behavior model

Prelude never guesses which terminal window or pane should receive a command.

### Shell widget

A command selected from `Ctrl+R` is inserted into the prompt you are already
using. You can edit it and press Enter yourself.

### Global panel

A command selected from the global panel is copied to the system clipboard and
the panel closes. Paste it into the terminal and prompt you intend to use.

### Objects

Files, folders, applications and URLs act directly on both surfaces:

- file → its default application
- folder → Finder
- application → launch
- URL → default browser

No `open ...` shell command is inserted, executed or written to shell history.

## Keys and actions

| Key | Behavior |
|---|---|
| `Enter` | Perform the row's default, stated in the footer |
| `Ctrl+K` | Open the contextual action panel |
| `Ctrl+P` | Show or hide Quick Look |
| `Escape` | Leave the current panel or close Prelude |
| `Cmd+Enter` | Open a file's containing folder; global panel only |

`Cmd+Enter` is available only in the dedicated global panel. Terminal programs
do not normally receive Command-key events, so Prelude's generated Ghostty
configuration translates this one chord into a private control sequence. The
ordinary Ghostty configuration and the `Ctrl+R` widget are not modified.

Compatibility shortcuts remain available:

| Key | Behavior |
|---|---|
| `Ctrl+O` | Run here when the selected kind supports it |
| `Ctrl+X` | Run a command in the shell |
| `Ctrl+Y` | Copy |

### What Enter does

Commands are handed to you; objects act.

| Selected row | Default `Enter` behavior |
|---|---|
| Question from an agent | Answer it and unblock the agent |
| Agent | Insert its command |
| Running agent | Insert `cd` to its project |
| Session | Insert the native resume command |
| History, script, snippet, PATH command | Insert the command |
| File or config | Open with the selected/default application |
| Folder | Open in Finder |
| Application | Launch |
| URL | Open in the default browser |
| Calculator or translation result | Copy the result |
| Setting | Perform the setting's named edit |

Set `PRELUDE_CLASSIC_ENTER=1` if you want command-like insertion for every kind
that supports it instead of Prelude's per-kind defaults.

### The action panel

`Ctrl+K` contains alternatives to Enter, not a second copy of Enter. Its header
states the current default and the selectable rows name the other operations:

```text
 README.md · file
 Default: Open it · Enter

 Action › Insert the full path
          Open with…
          Open in editor
          Reveal in Finder
          Copy file
          Copy path
          Create Quicklink…
          Change default app for .md files…
          Move to Trash…
```

Actions vary by kind. Examples include:

- files: open with, reveal, copy, Quicklink, default application, Trash
- sessions: resume, fork, pin, rename, archive, export, Trash when inactive
- skills: run, lend, install, diff copies, replace, read, delete a copy
- MCP servers: inspect tools, refresh, lend, install, diff definitions, login
- processes and ports: inspect, copy PID, kill after confirmation
- clipboard objects: restore the original Finder file or image to the clipboard

Destructive actions are red, appear last and confirm where the operation cannot
be trivially reversed. File, application, skill and inactive-session deletion
means moving to the Trash, never unlinking.

### Quick Look

`Ctrl+P` replaces the result area with details and toggles back to the list.
Depending on the kind it can show:

- image pixels through Ghostty/Kitty or iTerm's native image protocol
- file path, Finder tags, size and text head
- agent executable, settings and supported operations
- running-agent state and recent conversation output
- session metadata and relationship to a live run
- skill copy fingerprints and divergence
- MCP owners, health, definition variants and discovered tools

The `c:` clipboard scope is the deliberate exception: every clipboard row gets
an automatic right-side preview while that scope is active.

## Search scopes

| Prefix | Contents |
|---|---|
| `a:` | Agent control center: agents, runs, skills, MCP and config |
| `r:` | Running agents with live state |
| `s:` | Claude Code, Codex and pi sessions |
| `skill:` | Installed skills; `/` is the invocation form |
| `mcp:` | MCP servers |
| `cfg:` | Agent configuration files |
| `proj:` | Current-project scripts, files and Git rows |
| `f:` | Current-project files and indexed roots |
| `c:` | Text, Finder files and images from clipboard history |
| `h:` | Shell history |
| `app:` | Installed macOS applications |
| `cmd:` | `$PATH` executables and system commands |
| `dir:` | zoxide and recent `cd` destinations |
| `ssh:` | SSH hosts |
| `snip:` | Saved snippets |
| `port:` | Listening TCP ports |
| `proc:` | Processes |
| `docker:` | Running containers |
| `set:` | Prelude settings |

Use `:` to browse all scopes. Bare prefixes are valid: `f:` lists files and
`c:` lists clipboard history without requiring another term.

### Agent filters

```text
a:waiting                         runs or questions waiting on you
a:working                         active runs
a:agent:claude                    exact agent
a:project:Prelude                 exact project
a:using deploy                    runs that loaded a capability
a:without deploy                  runs that did not load it
a:using "claude.ai Google Drive" quoted multi-word capability
```

### Session filters

```text
s:is:pinned       pinned, non-archived sessions
s:is:active       sessions attached to a live run
s:is:archived     archived sessions
s:is:all oauth    include archived sessions while searching
```

### Skill invocation

An incomplete `/name` browses matching skills. A complete name invokes it:

```text
/cnipa-oo             browse matching skills
/cnipa-ooa            invoke the exact skill
/cnipa-ooa extra text invoke it with arguments
```

### Ask an installed agent

```text
@claude why is port 3000 busy?
```

Prelude uses the agent's non-interactive mode and shows the answer in the
preview area. A long-lived session is still handed back to a real prompt.

### Computed queries

```text
1847*0.23          424.81
10kg to lb         22.046226 lb
100 usd to cny     currency conversion
now + 3 days       date arithmetic
en:你好              on-device translation to English
zh:hello           on-device translation to Chinese
localhost:3000     local URL
g rust async       Google search
```

## Files, Finder tags and local paths

### Choose search roots

Use the settings scope or the CLI:

```sh
prelude settings add-root ~/work
prelude settings remove-root ~/work
prelude settings roots
```

Root changes are immediate, but the file index is rebuilt explicitly:

```sh
prelude index
```

The index operation walks the configured roots and records Finder tags. It may
take a minute for a large set of roots. The `File index` setting shows when the
index is stale.

Prelude refuses unsafe broad roots such as `~`, `/`, `/Users` and
`/Applications`; recursively indexing those would walk protected application
data and can trigger misleading macOS privacy prompts naming the terminal.

### Search filenames, paths and Finder tags

```text
f:proposal                 filename, parent path or Finder tag
f:Clients Acme             all words across path and tags
f:tag:work                 Finder tags only
f:tag:"Project Alpha"      quoted multi-word Finder tag
```

File rows use a stable three-column layout: filename, kind and parent path.
Prelude gives the filename enough room first. If the parent is still too long,
its middle becomes `...` so both its root and the directories nearest the file
remain visible.

Matching Finder tags are shown on the row and in Quick Look:

```text
proposal.pdf  file · ~/Documents/Clients/Acme · #work #approved
```

Finder tags are refreshed by `prelude index`, not queried with a subprocess on
every keystroke.

### Open the containing folder

In the global panel, focus a File or Find row and press `Cmd+Enter`. Finder
opens the file's parent directory. Ordinary Enter continues to open the file
with its default application.

### Paste a local path directly

A path does not need to be indexed. Existing values in these forms become real
File, Folder or Application rows:

```text
/Users/me/Notes/file.md
~/Notes/file.md
./src/main.rs
../another-project
folder/subfolder/file
file:///Users/me/Notes/file.md
```

Quoted paths and one layer of shell escaping are accepted, including paths
pasted by completion or Finder dragging:

```text
~/Library/Mobile\ Documents/report.pdf
```

The resulting row supports normal Quick Look, Quicklinks and contextual file
or folder actions.

## Clipboard history

Open clipboard history with `c:`.

Prelude preserves clipboard types instead of flattening everything to text:

- text stays text
- one or several Finder files stay Finder file objects
- PNG/TIFF clipboard data stays an image

Rows are strictly chronological. Frecency never moves an older clipping above
a newer one. Byte-identical image publications are merged and only the newest
occurrence remains; this handles screenshot tools that update the macOS
pasteboard several times for one image.

While `c:` is active, every row automatically uses the right 55% as a preview:

- images render as pixels while preserving aspect ratio
- text shows full content and metadata
- file objects show their paths and details

`Ctrl+K` can restore a file or image as the original system clipboard object,
not merely copy its path. Private image payloads are stored as owner-only files
inside Prelude's data directory and removed as history ages out.

Clipboard and history records that look like credentials are not indexed.

## Quicklinks and web search

A Quicklink gives a stable object a short keyword. Select a file, folder,
application, config or URL and choose:

```text
Ctrl+K → Create Quicklink…
```

For example:

```text
~/Documents/Notes/README.md  →  notes
```

Typing `notes` later resolves to the original File row, with the same Enter,
Quick Look and action behavior. Prelude canonicalizes local targets before
saving them so a relative path cannot silently change meaning in another
working directory.

Managed entries are appended to `quicklinks.toml` without rewriting comments
or hand-written search templates. Removing a Quicklink never removes its
target. Credential-bearing URLs are refused.

Built-in provider examples:

```text
g                    prepare a Google query
g rust async         Google search
gs elliptic curve    Google Scholar
b Rust 异步           Baidu
bing rust async      Bing
ddg rust async       DuckDuckGo
gh prelude           GitHub search
```

A provider without an argument is a Search command. Enter completes `g ` and
keeps Prelude open for the search term.

## Agents, sessions, skills and MCP

Prelude recognizes Claude Code, Codex, pi and OpenCode through one typed Agent
registry. It uses each agent's native command syntax and reports unsupported
operations by omitting them rather than constructing commands that will fail.

### Agent home

The empty query shows:

1. questions waiting for you
2. installed agents
3. running agents
4. skills
5. MCP servers
6. recent sessions

Favorites promote an Agent, Skill or MCP server only within its existing kind;
they never move a skill above an agent or write into native agent files.

### Running agents

`r:` finds agent processes wherever they run, not just in a particular terminal
or multiplexer. Prelude combines process state with the associated conversation
file:

- a turn ending in a tool call is still working
- a turn ending in prose has handed back to the user
- batch runs without a conversation file are marked and never guessed to be
  waiting

Quick Look shows the effective project, session, state and recent response.
`Ctrl+K` can leave a message in the run's inbox or end the process after
confirmation.

Outside the launcher:

```sh
prelude fleet
prelude fleet --status
prelude watch
```

`prelude watch` sends one macOS notification when a run moves into waiting. It
does not repeatedly notify while the same run remains stopped.

### Sessions

`s:` merges conversations from Claude Code, Codex and pi. Session actions can:

- resume or fork through the native CLI
- pin, rename or archive using Prelude-only metadata
- start fresh in the same project
- resume with a borrowed Skill or MCP server where the agent supports it
- export raw JSONL or a redacted portable Markdown transcript
- reveal the native file
- move an inactive native session to the Trash

A Session already owned by a live Run does not start a competing resume.
Prelude hands over that run's project instead. Trash re-checks the live fleet
and accepts only canonical conversation files inside known native session
roots.

### Skills

Prelude inventories skills from:

```text
~/.claude/skills
~/.agents/skills
~/.codex/skills
~/.pi/agent/skills
~/.config/opencode/skills
```

Copies with the same name are merged into one row. Background fingerprints
cover the complete skill tree while ignoring VCS/cache metadata. Quick Look
shows whether copies are identical, divergent, unknown or private-unknown.

Skill actions can run with an owner agent, prepare a one-run loan, install into
another agent, show a recursive diff, replace a divergent copy after
re-verification, read instructions or move one selected copy to the Trash.
Credential-like source material is never copied automatically.

A skill can also be handed to an agent without CLI support for borrowed skills:

```text
Read /path/to/SKILL.md and follow it.
```

### MCP servers

MCP health comes from each agent's own CLI rather than from guessing at config
files. Prelude stores redacted fingerprints and display summaries, never
complete command arguments, environment variables, headers or credential-like
URLs.

For supported stdio servers, a cached background probe performs MCP initialize
and paginated `tools/list`. Quick Look can show bounded tool names and
descriptions. Hosted or HTTP servers say when tool inventory is unsupported
because Prelude does not have the owner's authentication.

Depending on the owner and receiving agent, actions can refresh status, inspect
tools, lend for one run, insert a reviewed install/login/remove command, or
compare redacted definitions. An unavailable or unsafe transfer target is not
shown.

### Relationship graph

```sh
prelude control
prelude control --json
```

The graph connects Agents, Runs, Sessions, Skills and MCP servers with stable
ids. It deliberately excludes full process prompts and complete MCP
definitions because either can contain credentials.

## Agent-to-human and agent-to-agent messages

Prelude is also a small local message bus. An agent can stop at a decision,
ask the person and receive the answer on stdout:

```sh
answer=$(prelude ask "The migration drops legacy_users. Proceed?")
```

The question becomes the first row of the launcher and can trigger a macOS
notification. `prelude ask` blocks until answered or until its timeout:

| Exit | Meaning |
|---|---|
| `0` | answered; stdout contains the answer |
| `3` | timed out; the unanswered question remains in the inbox |

Useful forms:

```sh
prelude ask --timeout=300 "Proceed?"
id=$(prelude ask --no-wait "Proceed?")
prelude answer-of "$id"
prelude tell "Deployment finished"
```

Agents can address peers by project, agent name or pid:

```sh
prelude fleet --json
prelude say api-gateway "I changed the auth schema; rebase before editing"
prelude inbox
prelude drain
```

Ambiguous recipients are refused instead of risking a message in the wrong
conversation.

Teach an agent about the interface once:

```sh
prelude init agent >> CLAUDE.md
```

The generated instructions explain `ask`, `tell`, `say`, `inbox`, JSON output
and when to use each operation.

## Settings

Open `set:` to manage Prelude without memorizing files or environment
variables. Every setting row shows its effective value and source: default,
saved file or environment override.

Typical rows include:

```text
Search roots
File index
Global hotkey
Panel directory
Launcher key at a shell
Inline height
Quick Look
What Enter does
Open-with rules
Snippets
Quicklinks
Favorites
```

Enter performs the setting's primary edit. `Ctrl+K` shows alternatives such as
resetting, opening the authoritative file, revealing it in Finder, removing a
root or showing index details.

The CLI uses the same validation and write paths:

```sh
prelude settings
prelude settings --json
prelude settings get height
prelude settings get height --json
prelude settings set height 75%
prelude settings set preview off
prelude settings set hotkey cmd+shift+space
prelude settings reset height
prelude settings reset all
prelude settings check
prelude settings check --json
prelude settings path quicklinks
prelude settings roots
prelude settings add-root ~/work
prelude settings remove-root ~/work
```

Environment variables remain per-invocation overrides and therefore win over
saved values:

| Variable | Effect |
|---|---|
| `PRELUDE_KEY='^T'` | zsh launcher key; default `^R` |
| `PRELUDE_HEIGHT=80%` | inline launcher height |
| `PRELUDE_NO_PREVIEW=1` | disable Quick Look |
| `PRELUDE_CLASSIC_ENTER=1` | insert command-like values instead of per-kind defaults |
| `PRELUDE_DEBUG=1` | print source and fzf fallback diagnostics |

Invalid file or environment values are ignored at runtime and reported by
`prelude settings check`. Editing `settings.toml` through Prelude preserves
comments and unknown future keys.

## Command reference

Run `prelude --help` for the live list.

### Launcher and human-facing commands

| Command | Purpose |
|---|---|
| `prelude` | Open search |
| `prelude reply` | Answer the oldest question waiting on a person |
| `prelude fleet` | List running agents and state |
| `prelude fleet --status` | One status-bar line |
| `prelude watch` | Notify when an agent begins waiting |
| `prelude control [--json]` | Agent/Run/Session/Skill/MCP graph |
| `prelude agents [--json]` | Agent overview as data |
| `prelude sessions [--json]` | Sessions as data |
| `prelude skills [--json]` | Skills and owners as data |

### Global panel

| Command | Purpose |
|---|---|
| `prelude global install` | Install and start the Ghostty panel |
| `prelude global status [--json]` | Panel, chord and integration diagnostics |
| `prelude global hotkey [CHORD]` | Read or change the global chord |
| `prelude global directory [PATH]` | Read or change the panel directory |
| `prelude global start` | Start the installed panel |
| `prelude global stop` | Stop it |
| `prelude global uninstall [--reset]` | Remove it |

### Agent message bus

| Command | Purpose |
|---|---|
| `prelude ask TEXT` | Ask the person and wait |
| `prelude tell TEXT` | Notify without waiting |
| `prelude say WHO TEXT` | Send to another agent |
| `prelude inbox [--json]` | Read messages for the current agent/project |
| `prelude inbox --human` | Questions waiting on a person |
| `prelude drain` | Mark inbox messages collected |
| `prelude answer ID TEXT` | Answer a specific question |
| `prelude answer-of ID` | Collect an asynchronous answer |

### Setup and diagnostics

| Command | Purpose |
|---|---|
| `prelude init zsh` | Print zsh integration |
| `prelude init agent` | Print agent message-bus instructions |
| `prelude index` | Rebuild file paths and Finder tags |
| `prelude doctor` | Diagnose the complete setup |
| `prelude doctor agents` | Agent installation and login diagnostics |
| `prelude doctor sessions` | Session-record diagnostics |
| `prelude doctor skills` | Skill-copy and fingerprint diagnostics |
| `prelude doctor mcp` | MCP health, tools and privacy diagnostics |
| `prelude bench` | Measure gather latency |
| `prelude build-translate` | Build the Apple Translation helper |

`doctor --json` provides machine-readable findings. `doctor --repair` asks
separately before each supported repair and only modifies Prelude-owned
records.

## Configuration files

Prelude follows XDG directories.

| Path | Purpose |
|---|---|
| `$XDG_CONFIG_HOME/prelude/settings.toml` | Shell key, height, Quick Look and Enter behavior |
| `$XDG_CONFIG_HOME/prelude/global.toml` | Global chord and panel directory |
| `$XDG_CONFIG_HOME/prelude/roots.txt` | File-index roots |
| `$XDG_CONFIG_HOME/prelude/open.toml` | Per-extension default applications |
| `$XDG_CONFIG_HOME/prelude/snippets.toml` | Snippets |
| `$XDG_CONFIG_HOME/prelude/quicklinks.toml` | Quicklinks and web providers |
| `$XDG_CONFIG_HOME/prelude/favorites.txt` | Agent, Skill and MCP favorites |
| `$XDG_DATA_HOME/prelude/frecency.tsv` | Selection history |
| `$XDG_DATA_HOME/prelude/sessions.json` | Session names, pins and archive state |
| `$XDG_DATA_HOME/prelude/exports/` | Private session exports |
| `$XDG_DATA_HOME/prelude/clipboard.jsonl` | Clipboard metadata |
| `$XDG_DATA_HOME/prelude/clipboard/` | Private clipboard image payloads |
| `$XDG_DATA_HOME/prelude/bus/` | Questions and messages that must survive cache clearing |
| `$XDG_CACHE_HOME/prelude/` | File index and source caches |

List files are owned by the feature that writes them. Settings commands do not
silently rewrite snippets, roots, Quicklinks, favorites or open-with rules.

## On-device translation

`en:` and `zh:` use Apple's Translation framework. Build the helper once:

```sh
prelude build-translate
```

This requires Xcode's `swiftc`. The helper is packaged as an ad-hoc-signed
local `.app`; no developer account is required. Translation stays on-device,
though the model should not be treated as authoritative for legal or technical
language.

## Safety, privacy and performance

### Commands remain reviewable

Command lines are inserted into a prompt or copied to the clipboard. Prelude
does not silently type into another terminal, choose a multiplexer pane or add
a destructive command to shell history.

### Objects bypass the shell

Files, folders, applications and URLs are passed directly to Launch Services.
They are not rendered as `open ...` commands.

### Deletion is recoverable

Files, applications, skill copies and inactive native sessions move to the
Trash. Protected roots and paths outside known ownership boundaries are
refused after canonicalization. Confirmations select Cancel first.

### Credentials are not search material

History, clipboard text, skill fingerprints, MCP summaries, Finder tag names
and exported transcripts pass through credential filters appropriate to their
source. Complete MCP definitions and process prompts do not survive in
launcher Items or caches.

### Sources degrade to nothing

A failing source returns no rows rather than blocking, panicking or printing
into the launcher. Slow sources use bounded background caches.

### Latency target

Prelude's gather budget is 40 ms:

```sh
prelude bench
```

fzf invokes small Prelude helpers whenever the query changes, so the hot path
avoids network calls, directory walks and metadata subprocesses. Finder tags
are collected only during explicit indexing; MCP tools and Skill fingerprints
are refreshed behind cache tiers.

## Platform

Prelude currently supports macOS only. It depends on macOS interfaces for
applications, Launch Services, Finder objects, pasteboard types, notifications,
process inspection and on-device translation.

Image previews use Ghostty/Kitty or iTerm native protocols first, with Chafa as
a fallback.

## More documentation

- [Search model and filter grammar](docs/SEARCH.md)
- [Action behavior and safety rules](docs/ACTIONS.md)
- [Global panel architecture and acceptance record](docs/GLOBAL-HOTKEY.md)
- [Agent control plane](docs/AGENT-CONTROL-PLANE.md)
- [Contributing](CONTRIBUTING.md)

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
