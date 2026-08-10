# Defaults and the action panel

The behavior in this document comes from `src/defaults.rs`, `src/actions.rs`,
`src/ui.rs`, and `src/panel.rs`.

`Enter` performs the focused row's default. `Ctrl+K` opens a modal list of
alternatives for that Item. The footer and the action-panel header both state
the current default; it is not repeated as a selectable action.

## Two surfaces

Prelude has one Item model and two handoff surfaces:

| Surface | Command handoff |
|---|---|
| zsh widget | `INSERT` replaces the current line; `RUN` replaces and submits it |
| global Ghostty panel | both `INSERT` and `RUN` copy the same command text; the panel then closes |

The global panel has no destination shell, so it cannot preserve the difference
between “insert” and “insert and press Enter.” Commands produced by custom
actions—resume, fork, editor, login, install, remove, and similar commands—obey
the same rule.

Direct object actions do not use that handoff. Files, folders, applications,
and URLs go to macOS Launch Services on both surfaces. Actions that explicitly
run inside Prelude, such as MCP inspection or `Run and show output`, also behave
the same on both surfaces.

`PRELUDE_CLASSIC_ENTER`, or the corresponding `set:` setting, is the exception:
when enabled, every default becomes a command/text insertion instead of the
per-Kind behavior below.

## Default behavior

With the default **per kind** setting:

| Kind | Enter |
|---|---|
| Question (`Msg`) | prompt for an answer and write it to the bus |
| Agent | hand over the Agent command for further editing |
| Run | hand over `cd` for the Run's working directory |
| idle Session | hand over its native resume command |
| Session attached to a live Run | hand over `cd` for that active project instead of starting a competitor |
| one-off `@agent …` or `/skill …` result | run the Agent's non-interactive form inside Prelude and show its answer |
| Skill object | hand over an invocation through the first owning Agent |
| MCP server | run the owner's `mcp get` inside Prelude and show its output |
| File / indexed file / Config | open through Prelude's remembered app or the macOS default |
| Folder | open in Finder |
| Application | launch through Launch Services |
| URL / Quicklink result | open in the default browser |
| Calculator / conversion / translation | copy the result |
| Prelude setting | carry out the setting-specific edit |
| clipboard record | hand its text or payload path back to the surface |
| History, script, `$PATH`, snippet, SSH, container, Git, port, process, system command | hand over the command for review |
| incomplete search provider or scope command | keep Prelude open and complete the query prefix |

The secondary action is generally the opposite: acting objects offer text, and
handed-over commands may offer execution. It has no dedicated key because
terminal applications do not portably deliver Command, Option, Shift+Enter, or
Ctrl+Enter. It appears as the first relevant row in `Ctrl+K` instead.

## Panel behavior

- `Escape` and `←` from a submenu return to the action list.
- `Escape` and `←` from the action list return to the main search.
- `→` opens the focused row's actions from the main search, and chooses within
  a list — the same door `Ctrl+K` and `Enter` open.
- `Escape` in the main search clears a typed query first, and closes Prelude
  only when there is nothing left to back out of.
- The arrow keys belong to the query line whenever a query exists, so they act
  on levels only while nothing is typed. `Ctrl+K` is unconditional.
- Copy, details, Skill Diff, cached MCP tools, and MCP comparison remain in the
  panel after completion.
- A canceled confirmation returns to the action list.
- Other successful actions close the launcher or return their handoff.
- Destructive entries are red and sorted last.
- Parameter choices—Agent, Skill, MCP server, or copy—use a submenu only when
  more than one choice exists.

The dedicated global panel has one shortcut outside `Ctrl+K`: Ghostty translates
`Cmd+Enter` to a private `Ctrl+G`, and Prelude opens the containing directory
only for File and indexed-File rows. The inline zsh widget never advertises it.

## Current contextual actions

Rows below are conditional on the fields present in the Item and the capability
flags in `src/agent.rs`.

### Agent

An installed Agent row may offer:

- choose one of its current Runs and hand over `cd`
- resume its newest visible Session
- browse up to 100 of its Sessions
- ask a one-off question and show the answer inside Prelude
- open its existing settings file
- run `prelude doctor agents` inside Prelude
- add or remove the Agent from Favorites
- start immediately from the zsh surface through the generic secondary action

The Agent registry, not string matching in the panel, decides invocation,
settings path, one-off ask, resume, fork, Skill, and MCP support.

### Run

A live Run offers:

- show the last response and effective context
- leave a message in the Run project's Prelude inbox
- hand over `cd` when a working directory is known
- copy the PID
- end the process after confirmation

Prelude does not focus, split, or type into another terminal. Every Run is
addressed by Agent/project/PID facts, not by a terminal pane.

### Waiting question

A question posted through `prelude ask` offers:

- answer `go ahead`
- answer `no`
- show its stored context
- copy the question

A custom answer remains Enter's default. There is no “go to conversation”
action because the bus does not retain a terminal address.

### Session

Native Sessions currently come from Claude Code, Codex, and pi JSONL files. A
Session may offer:

- resume now when no live Run owns it
- fork through the native CLI for Claude Code, Codex, or pi
- resume with a non-owned Skill or MCP server for one Run, only where the
  target Agent has supported one-run syntax
- pin or unpin
- apply a local name or restore the native title
- archive or restore through Prelude metadata
- show conversation details
- start a fresh Agent in the same project
- hand over `cd`
- export a private raw JSONL copy
- export a readable, redacted Markdown transcript
- reveal the native Session file
- copy the native Session id
- move an inactive recognized native Session to the Trash

An active Session does not offer resume, capability-assisted resume, archive,
or Trash. Before trashing, Prelude refreshes the fleet and refuses any exact or
ambiguous live relationship. The path must canonicalize to a `.jsonl` file
under a known Claude Code, Codex, or pi Session root.

Session metadata is stored in Prelude's private `sessions.json`; rename, pin,
and archive do not modify the native conversation. An archived Session attached
to a live Run is visible until the Run exits, while retaining its archive flag.

Raw and Markdown exports are intentionally different. The raw file is copied
byte-for-byte for the owning Agent. The Markdown transcript omits tool-call
arguments and harness material and applies credential redaction for a person to
read.

### Skill

Skills are merged by frontmatter/folder name across five roots: Claude Code,
shared `~/.agents`, Codex, pi, and OpenCode. A Skill row may offer:

- run through an Agent that owns a copy
- hand over the bare `/name` command
- hand over `Read <SKILL.md> and follow it.` for any Agent
- prepare a supported one-run loan to an installed Agent that lacks it
- copy the complete tree permanently into another supported Agent
- compare divergent copies with `diff -ru`
- replace one divergent copy from another after a fresh hash check, Diff, and
  confirmation; the old copy goes to the Trash
- page the frontmatter description
- hand over an editor command for `SKILL.md`
- open every copy when more than one directory exists
- delete one named Agent copy to the Trash
- archive or restore the merged Skill in Prelude
- add or remove it from Favorites

One-run Skill borrowing currently exists for Claude Code and pi. Codex and
OpenCode can still be pointed at the Skill file; all four registry Agents can
receive a permanent Skill copy.

Full-tree identity includes scripts, references, and symlinks while excluding
VCS/cache metadata. A missing hash produces `unknown`; matching public hashes
produce `identical`; different hashes produce `divergent`; redacted private
content prevents an equality claim. Sensitive or incompletely read sources are
not copied, lent, or used for replacement.

Archive is a Prelude view overlay. It preserves every native copy and every
Favorite, removes the Skill from ordinary inventory and slash invocation, and
is reversible from `skill:is:archived`.

### MCP server

MCP inventory currently comes from Claude Code and Codex owner CLIs. Enter runs
the owner's `mcp get` inside Prelude. Conditional alternatives include:

- show cached tools
- refresh owner-reported status; Claude is labelled `Test connection now`
- refresh stdio tool inventory with an explicit MCP handshake
- prepare a supported one-run borrow for another Agent
- hand over an owner-CLI install command for another Agent
- compare redacted owner variants and their structural public-definition Diff
- prepare a remove-and-install replacement command after showing that comparison
- hand over a login command for auth/failure states
- open the owner configuration when a path exists
- copy the server name
- hand over the owner-CLI remove command
- archive or restore all variants sharing the normalized capability id
- add or remove it from Favorites

Claude Code and Codex have one-run and permanent MCP syntax in the current
registry. pi and OpenCode do not. An owner-account server marked non-portable
has no borrow/install/replacement target. Sensitive definitions are never
inlined into a command: Claude may receive a private `0600` staged file; a
target that only accepts an unsafe inline form is refused.

Complete MCP definitions are resolved from the owner CLI only on the explicit
action that needs them. Ordinary Items, list caches, and Control JSON retain
transport, status, bounded tools, a redacted public shape, and a semantic
fingerprint—not env/header values or credential-bearing arguments.

Only enabled stdio servers are started for background tool inventory. HTTP and
hosted servers report `unsupported` when Prelude cannot reuse owner auth. The
current Claude/Codex CLI syntax has no verified server-level Enable/Disable
verb, so Prelude offers none.

MCP archive is not Disable: it edits only
`$XDG_DATA_HOME/prelude/capabilities.json`. Native definitions and connections
remain unchanged. Restore from `mcp:is:archived`.

### Prelude setting

`set:` rows show the effective value and its source. Enter is specific to the
row: add a root, rebuild the index, prompt for a key/chord/directory/height,
toggle Quick Look or Enter mode, or open a list-backed file.

The action panel may additionally:

- remove and inspect search roots
- show index status
- reset scalar preferences to their actual defaults
- start or restart the global panel
- create/open the owning settings file
- hand over an editor command, reveal the file, or copy its path

Resetting the global hotkey means `Cmd+Shift+Space`; resetting the panel
directory means `$HOME`. `reset all` affects only key, height, Quick Look, and
Enter mode. It never removes roots, snippets, Quicklinks, Favorites, or
open-with rules. A valid environment override remains effective over a saved
value and is named on the row.

### File, indexed file, and Config

File rows may offer:

- hand over the full path
- open once with another application
- hand over an `$EDITOR` command
- reveal in Finder
- copy a real Finder file object
- copy the path as text
- remember an application per extension
- create or manage a Quicklink
- move the file to the Trash

Config rows use the same open actions but omit Trash. The default opens through
Launch Services; `$EDITOR` is an explicit command-producing alternative.

### Folder, application, and link

- Folder: default opens Finder; alternatives copy a Finder object, hand over
  `cd`, copy the path, and create or manage a Quicklink.
- Application: default launches it; alternatives reveal it, copy the app or its
  path, hand over an `open` command, create or manage a Quicklink, and move the
  app to the Trash.
- Link: default opens the browser; alternatives hand over/copy the URL, create
  a fixed Quicklink, and save it as a `{q}` search keyword.

### Quicklink

Any file, folder, application, or URL row offers `Create Quicklink…`. A URL
additionally offers `Save As A Search Keyword…`, which asks for the keyword and
then for the URL with `{q}` already placed where the search term most likely
was; the guess is editable before anything is written. The result row a
template produces is an ordinary Link and may itself be saved either way.

A row that already is a Quicklink offers, without opening any file:

- rename the keyword
- rename what the row says
- point it somewhere else
- remove the Quicklink
- open `quicklinks.toml`

Removal removes only the keyword; the target remains untouched. It reaches
hand-written entries as well as Prelude's own — Prelude's carry markers so
removing one leaves every hand-written line byte-for-byte, and an unmarked
entry is bounded by its own section header.

An entry whose target has gone leads with `Point It Somewhere Else…` rather
than stating the fault and offering nothing. `prelude quicklink` performs each
of these without a terminal, and `prelude quicklink check` reports every entry
that will not resolve.

A keyword is refused when it collides with a scope command, when it is already
taken, or when it cannot be typed as a keyword — at the moment it is named,
which is the only moment the reason can be given.

### Port, process, and container

Ports and processes offer inspection, PID copy, and process termination.
Termination runs `kill` directly and confirms first.

A running Docker container offers command text for:

- `docker logs -f`
- `docker restart`
- `docker stop`

It also copies the container name. These commands are handed over rather than
run automatically.

### Command-like rows

History, project scripts, `$PATH`, snippets, system commands, and Git rows can
offer `Run and show output` inside Prelude. The zsh surface may additionally
offer `Run now`; the global panel suppresses that row because “insert” and
“run” are identical clipboard bytes there.

SSH and interactive Agent commands do not receive an output-in-Prelude action.
Snippets expand `{{name}}` to visible `<name>` placeholders before handoff.

### Clipboard and computed results

- Text clips: hand back the full text; alternatives restore/copy it or translate
  it to English/Chinese.
- Finder clips: restore the original file-list pasteboard object; alternatives
  open/reveal the first file and copy paths as text.
- Image clips: restore the original image; alternatives open/reveal its private
  payload and copy the payload path.
- Translation: copy the translated result by default; the action panel can copy
  the original.
- Calculator/conversion: copy the result by default; the opposite inserts or
  copies it according to the surface, with no extra panel rows.

## Safety boundaries

- External objects are passed to `/usr/bin/open` as separate arguments, never
  assembled into an `open …` shell line for their default action.
- Protected roots, `$HOME`, system trees, and container directories are refused
  by `paths::is_protected` before Trash operations.
- Skill deletion accepts only a canonical direct child of one of the five known
  Skill roots.
- Session Trash accepts only an inactive canonical native JSONL under a known
  Session root and refreshes live relationships immediately before moving it.
- Skill replacement shows Diff, hashes source and target again, refuses changed
  or sensitive sources, trashes the old target, and removes a failed half-copy.
- Process/Run termination confirms with Cancel selected first.
- Other recoverable removal actions implement their own confirmation and move
  to the Trash; Docker stop and MCP remove are only handed-over commands.

## Implementation map

| Code | Responsibility |
|---|---|
| `defaults.rs::on_enter` | per-Kind Enter decision |
| `defaults.rs::on_secondary` | default opposite, including surface suppression |
| `actions.rs::actions_for` | contextual alternatives and order |
| `actions.rs::panel` | modal loop, submenus, stay/close behavior |
| `actions.rs::apply` | action execution and handoff |
| `actions.rs::is_destructive` | red styling and last-position rule |
| `actions.rs::needs_confirming` plus action-specific guards | confirmations |
| `ui.rs::perform` | defaults that act or emit `INSERT`/`RUN` |
| `panel.rs` | clipboard collapse for the global surface |
| `preview.rs` | Quick Look and reusable detail text |

`prelude _actions '<rendered row>'` prints the current panel without launching
fzf. Tests assert that Enter is not repeated, destructive actions remain last,
and clipboard-only handoff does not expose a duplicate shell-run alternative.
