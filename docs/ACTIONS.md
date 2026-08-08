# The action panel

`Enter` does the default thing for the selected row. `Ctrl+K` opens the
alternatives.

The panel is deliberately not a complete inventory of everything Prelude can
possibly do. It is the short list of things a person is likely to want after
selecting this particular object.

## Interaction

The selected item's title and kind are in the border. Enter's default is a
non-selectable header:

```text
 README.md · file
 Default: Open it · Enter

 Action › Insert the full path
          Open with…
          Open in editor
          Reveal in Finder
          Copy path
          Change default app for .md files…
          Move it to the Trash…
```

The consequences are simple:

- `Esc` from a submenu returns to the action list.
- `Esc` from the action list returns to the main search.
- Copying and viewing details leave the action list open.
- Opening, inserting, running and navigating complete the launcher action.
- Destructive actions are red, last, and confirm when they cannot be undone.

The prompt says `Action ›`, not `⌘`: terminals do not receive the Command key.
The footer says `Choose`, not `Run`, because an action may copy, reveal, open,
or remove something.

## Rules

### 1. Do not repeat Enter

The main footer already says what Enter does, and the action header repeats it
for context. It is not a selectable row. Opening `Ctrl+K` means the user wants
an alternative to Enter.

The secondary action remains available where it adds a real choice, because it
has no portable terminal key of its own. Rich kinds such as skills and running
agents use a more specific label instead of a generic secondary row.

### 2. Say whether an action runs or inserts

A label must describe the observed result:

- `Open in editor` opens it.
- `Insert cd command` returns a command to the prompt.
- `Insert login command` does not claim that login already happened.
- `Insert stop command` does not claim that a container was stopped.

An explicit `Run now` action runs immediately. Irreversible operations confirm
first, with Cancel selected by default.

Files, folders, applications and URLs are not shell commands. Their defaults
call macOS Launch Services directly with separate arguments; no `open ...`
line is pasted, executed by the shell, or recorded in shell history.

### 3. Prefer intent over completeness

There is no generic checklist appended to every kind. In particular, Prelude
does not add `Ask an agent about this`, `Go to project folder`, `Run here`, or
`Copy to clipboard` merely because it can construct something plausible.
Each kind opts into actions that answer a real use case.

### 4. Preserve per-kind order

The order written for a kind is the product decision. It is stable and is not
re-ranked by frecency. The only global reorder is moving destructive actions
to the end.

### 5. The host matters

A shell prompt and an agent input box are different destinations. Default and
secondary actions are computed for the host, and rich kinds expose separate
menus. A path selected at a shell opens locally; the same path selected over
an agent is text to hand into that conversation.

Agent-host menus omit resume, lend and one-off shell commands that would
otherwise be pasted into chat as prose. Explicitly local actions such as Open
Settings, Open in Editor and Reveal in Finder execute in the popup process
rather than being typed into the conversation. URLs are the deliberate
exception to path handoff: Enter always opens the browser, while `Insert URL`
is the conversation alternative.

## Current menus

The exact rows are conditional: unavailable targets, missing paths and healthy
authentication states are omitted.

### Agent

- Start now
- Resume latest session
- Ask a one-off question and show the answer here
- Open settings

### Running agent

- Send a message without switching panes
- Show its last response
- Go to its pane full-screen
- Insert a command to change to its project
- Copy its pane address
- End the agent

Rows without a tmux address omit pane-only actions.

### Question from an agent

- Answer “go ahead”
- Answer “no”
- Show the conversation context
- Go to the agent full-screen
- Copy the question

A custom answer remains Enter's default.

### Session

- Resume now
- Show conversation details
- Start fresh in the same project
- Insert a `cd` command
- Copy the session ID

### Skill

- Run with an owner agent
- Prepare a one-off borrowed run
- Install into another agent
- Read the instructions
- Open `SKILL.md` in the editor
- Delete a copy, recoverably

Agent choices use a submenu. A submenu with one possible target is collapsed
into a direct action.

### MCP server

At a shell, Enter shows what the server exposes. Opening the owning config is
not the server's primary purpose.

Alternatives are:

- Prepare one-off use with another supported agent
- Insert an install command for review
- Insert a login command when authentication is required
- Open the owning configuration when one exists
- Copy the server name
- Insert a remove command

Definitions containing credentials are never put on a command line.

### File and config

- Insert the full path
- Open with another application
- Open in the editor
- Reveal in Finder
- Copy the path
- Change the default application for the extension
- Create a Quicklink
- Move to the Trash

Config rows omit deletion. Files always go to the Trash rather than being
unlinked, and protected paths are refused after canonicalization.

### Application

- Reveal in Finder
- Copy the application path
- Insert the `open` command
- Create a Quicklink
- Move the application to the Trash

There is no `cd into the app`: entering an application bundle is not a normal
launcher task.

### Port and process

- Inspect the process or listener
- Copy the PID
- Kill the process

The kill is red, last, and confirmed. There is no harmlessly-labelled generic
runner that reaches the same kill command.

### Container

- Insert the follow-logs command
- Insert the restart command
- Copy the container name
- Insert the stop command

The labels are explicit because these commands return to the prompt for review.

### Command rows

History, scripts, PATH commands, snippets, system commands and git commands
may offer:

- Run now
- Run and show output inside Prelude
- Copy the command

Only commands whose output belongs in a small Prelude window get the second
entry. Agent TUIs, SSH and interactive containers do not.

### Link, directory and results

These stay intentionally short:

- Link: Enter opens the default browser directly; the panel inserts or copies the URL.
- Directory: Enter opens Finder; the panel inserts `cd`, inserts the path, or copies it.
- Files, folders, links, configs and applications can create a Quicklink.
- Calculator: insert or copy the result.
- Translation: insert the translation or copy its source.
- Clipboard: copy or insert the text, and translate it locally.

A two-line action panel is an honest result, not a gap to fill.

### Search commands and Quicklink management

A search provider without an argument is a `Search` command rather than a
half-formed link. Enter keeps the launcher open and fills its alias (`g `,
`b `, and so on) into the query; its panel opens the provider configuration.
Scope commands behave the same way with `f:`, `c:` and the other scopes, but
need no action-panel entries.

A stable object without a quicklink offers `Create Quicklink…`. A resolved
quicklink instead offers `Edit Quicklink Definition`, and action-created
entries offer `Remove Quicklink…` before any action that removes the target.
Removing the name leaves the file, folder, application or URL untouched.
Hand-written template definitions are edited in the config rather than
rewritten by Prelude.

## Implementation

| File | Responsibility |
|---|---|
| `defaults.rs` | Enter and the secondary action for each host |
| `actions.rs::actions_for_host` | contextual alternatives and their order |
| `actions.rs::panel` | modal loop, submenus, header and stay/close behavior |
| `actions.rs::is_destructive` | danger styling and last-position invariant |
| `ui.rs::confirm` | confirmation with Cancel first |
| `preview.rs::text` | reusable detail text for preview and action views |

`prelude _actions '<row json>'` prints a shell-host panel without opening fzf.
Tests pin the important invariants and representative category menus.
