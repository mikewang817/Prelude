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

Quick Look for a Run is `running::effective_context` and nothing of its own:
Agent, project, branch, Session, state, start time, model, the capabilities
this Run confirmed against what its Agent merely has installed, and then what
the conversation last said.

`effective_context` has exactly one caller, and it is this panel. `prelude
fleet` renders its own columns and shows no branch, model or capabilities, and
`control.rs` re-derives what it needs from the same underlying helpers —
`branch_label`, `model_of`, `confirmed_capabilities` — rather than from the
list. Those helpers are the shared part, and they are shared precisely because
two surfaces deriving one fact separately is how they start disagreeing about
it. Anything a second surface needs from a Run belongs in a helper both call,
not in a second reading of the row.

### Question from an agent

- Answer “go ahead”
- Answer “no”
- Show the conversation context
- Go to the agent full-screen
- Copy the question

A custom answer remains Enter's default.

### Session

- Enter resumes an idle Session; an active Session goes to its tmux pane, or inserts its project when no pane address exists, rather than starting a competing resume
- Resume now, only when no active Run owns it
- Fork it through the native Claude, Codex or pi CLI; offer nothing where the Agent has no known fork verb
- Resume it with a Skill, or with an MCP server, that its Agent does not own — for one run only, and absent where that Agent has no one-run flag
- Pin or unpin it
- Rename it, or restore the Agent's native title
- Archive it without touching the native conversation; archived Sessions appear under `s:is:archived`
- Show conversation details
- Start fresh in the same project
- Insert a `cd` command
- Export the untouched JSONL into Prelude's private exports directory
- Export a portable Markdown transcript beside it
- Reveal the authoritative native file
- Copy the session ID
- Move an inactive native conversation to the Trash, after confirmation

Archive is Prelude metadata and is reversible. Trash is offered only for an
inactive Session, re-finds the fleet at action time, accepts only canonical
JSONL files below the known Claude, Codex and pi Session roots, and never
unlinks the file.

The two exports are not alternatives. The raw JSONL is authoritative and is
what you hand back to the Agent that wrote it; the Markdown one is what you
send to a person, redacted and free of tool-call plumbing. Both land in
Prelude's own private exports directory.

Resuming with a borrowed capability follows the same rule as forking: three of
the eight Agent/capability pairings have no one-run syntax at all, and a
command assembled for one of those would look right on the prompt and fail
after the launcher had closed. Those Agents are offered nothing. Neither entry
appears against a live Run, for the reason `Resume now` does not: it would
start a competitor. The capability itself is chosen after the verb, in a
picker of its own, so drawing the panel never costs a walk of every Skill
directory.

The picker lists only what the Session's Agent does not already own, because
that is what borrowing is: a claude Session offered a one-run borrow of
claude's own nine Skills is a nine-row question with no answer in it.
`~/.agents/skills` is a location rather than an Agent — `missing_agents`
reports a Skill that lives only there as missing from every Agent — so a shared
Skill stays in the list. With nothing left to offer the action says so instead
of opening an empty picker.

### Skill

- Run with an owner agent
- Prepare a one-off borrowed run
- Install into another agent
- Compare divergent copies with a recursive, read-only Diff
- Replace one divergent copy from another only after showing that Diff; the old target moves to the Trash
- Read the instructions
- Open `SKILL.md` in the editor
- Open all copies, only where there is more than one
- Delete a copy, recoverably

Replacement re-hashes source and target and refuses if either changed after
the comparison. A source with credential-like material is never copied.
Agent choices use a submenu. A submenu with one possible target is collapsed
into a direct action.

A Skill merged across four Agents is four directories behind one row, and
`dir` is only ever the first of them. `Open all copies` is that target made
explicit, so it appears only where there is more than one — with a single copy
`Open` already is it, and a panel whose fifth row repeats its fourth teaches
you not to read it. Quick Look states the same thing as data: a modification
time per copy, so a divergence says which way round to replace, and a warning
line for any symlink that is broken or resolves outside the Skill.

### MCP server

At a shell, Enter shows what the server exposes. Opening the owning config is
not the server's primary purpose.

Alternatives are:

- Show cached actual tools when the background MCP handshake succeeded
- Test Claude connection health now, or refresh Codex's owner-reported status
- Refresh an enabled stdio tool inventory explicitly
- Show owner-reported configuration details
- Prepare one-off use with another supported agent
- Insert an install command for review
- Compare structurally redacted definitions when several Agents own the same server
- Prepare a remove-and-install replacement command only after showing that comparison
- Insert a login command when authentication is required
- Open the owning configuration when one exists
- Copy the server name
- Insert a remove command

Complete MCP definitions are never retained in an Item or cache. Definition
fingerprints omit env/header values and credential-bearing arguments. A
replacement command is inserted for review, never run automatically, and is
refused when the source cannot be represented without private fields.
Account-hosted servers without a transferable local definition offer no
borrow, install or replacement action. HTTP/hosted tool inventory is labelled
unsupported when owner authentication is unavailable. Current Claude and Codex
CLI help exposes no server-level Enable/Disable verb, so Prelude does not invent
one.

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
- Directory: Enter opens Finder; the panel copies the Finder object, inserts `cd`, or copies the path.
- Files, folders, links, configs and applications can create a Quicklink.
- Files, folders and applications can be copied as real Finder objects, separately from copying path text.
- Calculator: insert or copy the result.
- Translation: insert the translation or copy its source.
- Clipboard text: copy or insert it, and translate it locally.
- Clipboard files/images: insert their path(s), restore the original pasteboard object, open or reveal it.

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
| `preview.rs` | full-area Quick Look text and image rendering; reusable detail text for action views |

`prelude _actions '<row json>'` prints a shell-host panel without opening fzf.
Tests pin the important invariants and representative category menus.
