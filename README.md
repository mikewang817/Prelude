# Prelude

<p align="center">
  <strong>A macOS launcher for terminal work and local coding agents.</strong><br>
  Search what you use, see what your agents are doing, and keep shell commands reviewable.
</p>

<p align="center">
  <a href="#install"><strong>Install</strong></a> ·
  <a href="#how-prelude-is-different"><strong>Why Prelude</strong></a> ·
  <a href="#agent-control-plane"><strong>Agents</strong></a> ·
  <a href="docs/SEARCH.md"><strong>Search guide</strong></a>
</p>

![Prelude showing skills and recent agent sessions](docs/assets/prelude.png)

Prelude is a Rust program built on [fzf](https://github.com/junegunn/fzf). It
indexes commands, files, apps, clipboard history, projects, Agent sessions,
and Skills, while keeping large collections behind explicit
scopes. Its gather path has a 40 ms budget.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/mikewang817/Prelude/main/install.sh | bash
```

The installer supports Apple Silicon and Intel Macs. It downloads the latest
checksum-verified Prelude release, reuses a compatible `fzf` or installs a
private copy, installs the official Ghostty app in `~/Applications` when
needed, adds managed blocks to `~/.zshrc` and Ghostty's ordinary configuration,
installs the global panel as a LaunchAgent, and generates the `prelude://`
handler in `~/Applications`. The Ghostty block translates `Ctrl+Enter`,
`Ctrl+Shift+Enter`, and `Ctrl+Option+Enter`; uninstall removes all of it.

On first install, enable Ghostty in **System Settings → Privacy & Security →
Accessibility** when prompted. Prelude checks Ghostty's actual global event-tap
result before reporting the shortcut ready.

- **`Cmd+Shift+Space`** reveals the global panel from anywhere.
- **`Ctrl+R`** opens the same full launcher in zsh shells started after installation.

Run the same installer command again to upgrade or repair the integration.
Check it with `prelude global status`.

## How Prelude is different

Most launchers are organized around applications, files, and web searches.
Prelude includes those objects, but its interaction model is built for shell
commands and coding agents.

| Typical desktop launcher | Prelude |
|---|---|
| Searches apps, files, and links | Also indexes shell history, `$PATH`, project scripts, clipboard objects, sessions, Skills, and live Agent processes |
| Executes a command as soon as it is selected | Copies command text for review from either launcher entry point |
| Treats results as interchangeable text | Opens files, folders, apps, and URLs directly through macOS while keeping shell commands editable |
| Adds AI as another prompt box | Shows Skills, past conversations, live Agent state, and waiting questions |
| Integrates with one assistant at a time | Uses one typed registry for Claude Code, Codex, pi, and OpenCode |

There is one launcher surface with two ways in:

- **`Ctrl+R` in zsh** and the **global Ghostty chord** show the same full layout,
  catalogue, labels, actions and directory shortcuts.
- Commands are copied from either entry point; neither launcher edits or
  submits a shell line. External objects act directly through macOS.
- Both stand in the directory selected by `prelude global directory`, so
  project-derived rows cannot change merely because a different key opened the
  launcher.

`Enter` states its current behavior in the footer; `Ctrl+K` opens the
alternatives. This avoids a launcher guessing which terminal, tab, pane, or
working directory you meant.

## Agent control plane

Prelude manages local Agent facts rather than replacing the Agents themselves.
It currently knows Claude Code, Codex, pi, and OpenCode.

From the launcher you can:

- list installed Agents and every matching process running on the machine
- classify live Runs as working or waiting from process and Session evidence
- answer questions posted with `prelude ask`
- browse and resume native Claude Code, Codex, and pi Sessions
- pin, rename, archive, fork, export, reveal, or safely trash a Session
- merge Skills by name across Agent directories and compare their complete trees
- copy a Skill permanently or prepare a supported one-run loan
- archive Skills in Prelude without changing native Agent files
- inspect the versioned Agent/Run/Session/Skill graph with `prelude control --json`

Support is capability-specific. For example, Claude and pi can borrow a Skill
for one run; OpenCode Sessions are
not currently discovered. Prelude omits an action when the owning CLI has no
known syntax instead of constructing a command that only looks plausible.

The local message bus is file-backed and requires no Prelude server:

```sh
prelude fleet
prelude watch
prelude ask "The migration drops legacy_users. Proceed?"
prelude tell "Migration finished"
prelude say api-gateway "I changed the auth schema; rebase before editing"
prelude inbox --json
```

`ask` waits for an answer on stdout. `say` always leaves a message in the
matched Run's project inbox; it never types into another terminal. Ambiguous
recipients are refused.

## Search

An empty query is a compact Agent home. Type `:` to see every scope.

```text
a:waiting                 Runs or questions waiting for input
s:agent:claude since:24h  recent Claude Code Sessions
s:is:pinned               pinned Sessions
skill:is:archived         archived Skills, ready to restore
/cnipa-ooa                run an installed Skill and show its answer
@claude explain this      ask an installed Agent and show its answer
Prelude                    files and folders named Prelude
f:tag:work                 indexed files and folders carrying a Finder tag
c:                        clipboard text, Finder objects, and images
ql:                       keywords you saved yourself
h:git rebase              recent, filtered shell history
app:zed                   installed applications
10kg to lb                unit conversion
:                         every available scope
```

Ordinary queries also add the local objects whose own names match: up to five
applications, then up to ten files or folders. So `Chrome` offers Google Chrome
before it offers anything named after it. Names and Finder tags match; parent
paths are display context, so searching
`OpenGhostty` returns the folder named `OpenGhosttyFromAnyFolder`, not every
`main.swift` below it. Large collections such as history, clipboard rows and
`$PATH` commands stay behind their scopes.
Whatever you type also carries a web search on the last row, which stops an
unmatched query from being an empty box:

```text
git commit                the query itself · Search Google · Enter opens it
```

It sits under everything the machine could answer with, so it leads only when
nothing else matched, and it stays out of scopes — a scope is you saying where
to look, and "or the web" is not an answer to that.

Which providers appear, and in what order, is the **When nothing matches** row
in `set:`. It is a list of Quicklink keywords, so re-pointing `g` moves your
web searches and adding a second keyword adds a second row:

```sh
prelude settings set fallbacks "g, ddg"    # Google first, then DuckDuckGo
```

A keyword has to be a `{q}` template, because a fixed target has nowhere to put
the query; one that is not is skipped and reported by `prelude settings check`.
If nothing in your list resolves, the built-in search is offered anyway — a
query must never dead-end, which is the whole reason this row exists.

Choose where this search looks from `set:` → **Search folders**. Enter opens a
folder manager: add with the native macOS chooser, show an existing root in
Finder, or remove it from search without deleting anything. Prelude starts with
`~/App`, `~/Documents`, and `~/Desktop`; after the first edit, your list is
fully authoritative and rebuilds automatically.

See [the search guide](docs/SEARCH.md) for the complete query grammar.

## Quicklinks

A Quicklink is a keyword you type to reach one thing. `Ctrl+K` on any file,
folder, application, or URL creates one; `ql:` lists every keyword you have.

```text
notes                     a folder, an app, a file or a URL you named
jira api timeout          a {q} template, with the term filled in
ql:                       browse, rename, re-point or remove them
```

Prelude ships with keywords for general search (`g`, `gh`, `npm`, `mdn`, `gs`,
`b`, `bing`, `ddg`), for looking things up while writing code (`so`, `crates`,
`docsrs`, `pypi`, `pkg`, `caniuse`, `explain`, `hn`), and for working with
agents (`hf`, `arxiv`, `ccdocs`, `mcpdocs`). They arrive in versioned blocks:
one that clashes with a keyword you already use is skipped, and one you delete
stays deleted.

Keywords are matched without regard to case and may be written in any
language. A saved keyword outranks anything Prelude merely found, so it leads
the list well before you finish typing it. Prelude refuses a keyword a scope
command has already spent, refuses to overwrite an existing one, and refuses a
URL that looks like it carries a credential.

```sh
prelude --version                 what you are running, and whether the panel agrees
prelude update --check            is there a newer release
prelude update                    verify the checksum, swap the binary, restart the panel
prelude update --rollback         put the previous binary back
```

```sh
prelude quicklink list
prelude quicklink add notes ~/Documents/notes
prelude quicklink add jira 'https://jira.example.com/issues?jql={q}' Jira
prelude quicklink rename notes n
prelude quicklink check
```

## Keys

| Key | Action |
|---|---|
| `Enter` | Perform the focused row's stated default |
| `Tab` | Complete the focused scope command or search keyword |
| `Ctrl+R` | Search shell history with the text already typed; again to come back |
| `Ctrl+K` | Open contextual actions |
| `Ctrl+P` | Toggle Quick Look when enabled |
| `Ctrl+Enter` | Reveal a file or folder in Finder |
| `Ctrl+Shift+Enter` | Open Ghostty in the file's parent or the selected folder |
| `Ctrl+Option+Enter` | Copy the absolute path and close Prelude |
| `→` | In `set:`, increase/enable/add/change the focused setting; otherwise open Actions when nothing is typed |
| `←` | In `set:`, decrease/disable/remove/reset the focused setting; otherwise go back one level |
| `Escape` | Go back one level; close Prelude at the outermost one |

`Ctrl+R` pressed twice at a shell is therefore still incremental history
search: the first press opens Prelude, the second moves what you type into the
`h:` scope.

## Names and Favorites

A Quicklink names a target you type in. An **alias** names something Prelude
already has — an Agent, a Skill, an application, or a saved
Quicklink — so you reach it by a word of your own instead of by whatever it is
called:

```sh
prelude alias add browser "Google Chrome"
prelude alias list
prelude alias remove browser
```

The name then leads the list the moment you finish typing it, and appears on
the row it belongs to so you learn it by seeing it. `set:` → **Aliases** is the
same thing with a manager instead of a shell.

Only things with an identity that outlives a search can be named. A session, a
file and a history entry have none, and naming one would mean storing a path.
A name is refused the moment you type it — never later — if a scope command,
an Agent's own name or an existing keyword has already spent it, because a name
that is accepted and then silently unreachable is the worst of the three
outcomes.

**Favorites** are the lighter version: no name, just promotion. `Ctrl+K` →
*Add to Favorites* on an Agent, Skill, application or saved
Quicklink lifts it above its neighbours, and `set:` → **Favorites** manages the
set. Promotion stays inside a row's own kind, so a pinned application never
outranks an Agent.

## A hotkey per command

`prelude://` links act. There is no launcher in between:

```sh
open 'prelude://run?alias=browser'        # opens Google Chrome
```

Bind that to a chord in whatever hotkey tool you already use and you have a
hotkey per command. `prelude global install` generates the handler at
`~/Applications/Prelude Link.app` and claims the scheme; `prelude global
uninstall` removes and unregisters it. `prelude global status` says whether it
is there.

A link may only name an alias you created yourself, and may only act on rows
the launcher would act on — files, folders, applications and URLs, which go to
macOS. It will not copy text, start an agent or run a server, because any web
page can navigate to a `prelude://` URL and none of those is something a link
gets to cause. Anything it does not recognise does nothing and says so.

A scope is not an object, so this cannot open the clipboard history: only
things with a stable name are reachable this way.

## Boundaries

- Prelude is macOS-only and uses Ghostty for both entry points; the shell
  shortcut additionally requires zsh.
- Prelude's own indexes, preferences, clipboard records, capability metadata,
  and message bus stay in its XDG directories. Outside them it writes three
  things, all during explicit setup: managed blocks in `~/.zshrc` and Ghostty's
  configuration, the LaunchAgent, and the `prelude://` handler in
  `~/Applications`. `prelude global uninstall` removes all of them. Agent CLIs, web
  searches, and currency conversion use the network only when explicitly asked.
- The update check is the one exception, and it is stated rather than buried:
  at most twelve times an hour, Prelude asks GitHub's releases API for a
  version number, falling back to the `releases/latest` redirect. It is
  unauthenticated, sends nothing about you and carries no identifier.
  `prelude settings set update off` stops
  that automatic check; `notify` (the default) only tells you, `download`
  stages a verified archive, and `apply` installs it — never mid-session, only
  as the panel next starts. `prelude update` is a request you typed, so it
  works under every setting including `off`.
- Secret-looking history, clipboard text, messages, tags, and
  exported transcripts are filtered or redacted.
- File, application, Skill-copy, and inactive-Session removal goes through the
  Trash, and Prelude says where it put it. Irreversible process termination
  confirms first.
- A `prelude://` link may only name an alias you created and may only act on
  objects macOS opens. It cannot reach the clipboard, an agent or a shell.
- A failed source degrades to an empty or cached result instead of blocking the
  launcher indefinitely.

## Documentation

- [Search scopes and query grammar](docs/SEARCH.md)
- [Defaults, actions, and safety](docs/ACTIONS.md)
- [Global panel architecture and lifecycle](docs/GLOBAL-HOTKEY.md)
- [Agent control plane model and support matrix](docs/AGENT-CONTROL-PLANE.md)
- [Build from source and contribute](CONTRIBUTING.md)

Prelude is licensed under [Apache-2.0](LICENSE).
