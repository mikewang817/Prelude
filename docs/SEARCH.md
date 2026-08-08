# Search model

Prelude has a small home and a large catalogue. They are deliberately not
the same list.

## Empty query: agent home

The empty-query feed contains only:

- questions waiting for a person (`Msg`)
- installed agents (`Agent`)
- running agents (`Run`)
- skills (`Skill`)
- MCP servers (`Mcp`)

History, files, applications, `$PATH`, sessions and machine objects are still
gathered at startup, but are not rendered into the home feed. This keeps the
first screen about the product's main job without adding latency when the
person starts typing.

## Ordinary query: root commands

The first non-whitespace character searches a small command layer: agent-home
items, search providers, scope commands and fixed Quicklinks. It does not
expose the complete gathered catalogue to fzf. An exact `f` therefore shows
one `Search Files` command, whose Enter completion is `f:`; it does not fuzzy
match every file, command and history row containing the letter.

Large sources require their scope (`app:Zed`, `cmd:cargo`, `proj:dev`). This
keeps one-letter root queries useful and makes the transition from command to
its results explicit. Clearing the query reloads the agent home.

## Scopes

An explicit scope disables fzf's search and lets Prelude filter the cached
items itself. That matters because `c: git` cannot fuzzy-match a clipboard
row whose displayed text does not contain `c:`.

| Prefix | Contents |
|---|---|
| `a:` | agents, running agents, skills, MCP and agent config |
| `r:` | running agents, classified live |
| `s:` | all conversation sessions |
| `f:` | current-project files and the indexed roots |
| `c:` | clipboard history |
| `h:` | shell history |
| `app:` | applications |
| `cmd:` | `$PATH` and system commands |
| `dir:` | zoxide and recent `cd` targets |
| `proj:` | current-project scripts, files and Git rows |
| `ssh:` | SSH hosts |
| `snip:` | snippets |
| `port:` | listening TCP ports |
| `proc:` | processes |
| `docker:` | running containers |
| `mcp:` | MCP servers |
| `cfg:` | agent configuration |

The bare prefix is useful: `c:` lists recent clipboard entries and `f:` lists
files. `:` produces searchable scope commands; Enter on one fills its prefix
into the same search box. `/` browses skills as the name is typed, and `@`
lists agent-question commands before the question exists.

Agent sessions intentionally do not appear under `a:`. Hundreds of old
conversations turned the agent overview into a session browser; `s:` already
has that job.

## Search providers

A template Quicklink has two states:

```text
g             Search Google · g <query>
g rust async  Google: rust async · https://…
```

The first is a `Search` command, not a half-formed `Link`. Enter changes the
query to `g ` and leaves Prelude open. The second is a real `Link`; Enter opens
it through Launch Services.

Provider commands live in the root command layer, so names such as `google`
and `baidu` find them. An exact alias is an intent and wins over fuzzy matches
such as Gmail or Google Drive. Fixed Quicklinks also win on an exact alias and
resolve back to their target kind.

## Implementation constraints

- `cache::gather` still runs once, within the 40 ms budget.
- `ui::search` renders `home.txt` and the root-command `list.txt` with one
  shared layout, and stores the complete gathered items as JSON for scopes.
- Per-keystroke helpers read those files; they do not gather sources again.
- `running` remains live because cached state is actively misleading.
- `is_special` recognizes intent but never runs a calculation, subprocess or
  network request.
- Computed and scoped queries disable fzf search so their own rows cannot be
  filtered out by the syntax that produced them.
