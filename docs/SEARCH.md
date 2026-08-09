# Search model

Prelude has a small home and a large catalogue. They are deliberately not
the same list.

## Empty query: agent home

The home is the agent inventory: the things this launcher exists to manage,
on one screen, because looking at it *is* how you manage it.

- questions waiting for a person (`Msg`)
- installed agents, as launch entries (`Agent`)
- running agents (`Run`)
- their skills (`Skill`) and MCP servers (`Mcp`)
- the conversations you have had with them (`Session`)

History, files, applications, `$PATH`, clipboard and machine objects are still
gathered at startup, but are not rendered into the home feed — that is what
stops the first screen being a list of two thousand files.

This was briefly an attention list instead, where healthy Skills and servers
were pushed behind `/name` and `mcp:` so that only exceptions reached the home.
It reads well as a principle and was wrong in practice: a launcher you open to
see what you have is not improved by hiding what you have, and the panel went
quiet exactly when nothing was broken, which is most of the time.

Sessions are the one kind counted rather than filtered. There are hundreds of
them, so `gather` puts only the newest `sessions::IN_MAIN_LIST` in the list at
all and `s:` owns the rest.

### Home order

`cache::by_rank`, exactly as everywhere else: the kind band decides and
frecency orders within it. A question blocking somebody leads, then the
agents, what they are running, Skills, MCP, and the recent conversations
underneath. Agent, Skill and MCP favourites receive a bonus only after source
rank and frecency and still cannot cross their kind band. `home_items` is a
filter and nothing more — it leaves the order it was handed alone — because
one ordering rule for the launcher was enough.

## Ordinary query: root commands

The first non-whitespace character searches a small command layer: agent-home
items, search providers, scope commands and fixed Quicklinks. Agent management
is discoverable by ordinary names — Agent Control Center, Running Agents, Past
Conversations, Skills, MCP Servers and Agent Config — and selecting one fills
its prefix into the same box. Prefixes are accelerators, not prerequisite
knowledge. It does not
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
| `skill:` | all installed Skills; `/` remains the invocation accelerator |
| `f:` | current-project files and the indexed roots; `set:` decides which roots |
| `c:` | text, Finder files and images, strictly newest first |
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
| `set:` | Prelude's own settings, each with its current value |

The bare prefix is useful: `c:` lists recent clipboard entries and `f:` lists
files. `:` produces searchable scope commands; Enter on one fills its prefix
into the same search box. `@` lists agent-question commands before the
question exists.

`/` has the two states a provider has, and for the same reason. While the name
is *incomplete* it browses: `/cnipa-oo` lists every Skill whose name contains
that, as Skill rows. The moment it is a name — `/cnipa-ooa`, with or without
arguments after it — the query has stopped describing a search and started
being an invocation, so the single row is the run that invokes it and the
Skill row is gone. That is deliberate; nothing else in the launcher shows two
rows for one intent. A Skill's own row — with its copies, its integrity and
its `^K` panel — is reached by `skill:`, `/`, `a:`, or ordinary search.
`/` never lists MCP servers at all: they are not invoked by name, and `mcp:`
is their scope.

Agent sessions intentionally do not appear under `a:`. Hundreds of old
conversations turned the agent overview into a session browser; `s:` already
has that job.

Control filters are explicit words inside `a:`:

| Query | Meaning |
|---|---|
| `a:waiting` | anything waiting — a run gone quiet, or a question blocked on you |
| `a:working`, `a:dead` | the rest of a Run's own state vocabulary |
| `a:claude Prelude` | an agent name and a project together |
| `a:agent:claude`, `a:project:Prelude` | the exact forms; a project is never widened into a substring |
| `a:using deploy` | runs that explicitly loaded that Skill or MCP server |
| `a:without deploy` | runs that did not |

`using` and `without` read the capability a run *confirmed* — the flags it was
started with — and never the installed inventory: "claude has forty skills"
and "this run loaded one" are different facts. Only a `Run` can answer them,
so both filters restrict the result to runs; letting `without` fall through to
skills, servers and agents would return the whole scope minus one row and call
it an answer. Two `using` words are *and*ed, because two capabilities are two
independent facts about one run; `without` is the negation of that, so a run
is kept only when it loaded none of them. `state:`, `agent:` and `project:`
are *or*s within themselves — one run cannot be in two projects.

A capability name may contain spaces, so the value is quote-aware exactly as
`s:` is: `a:using "claude.ai Google Drive"` and `a:using:"claude.ai Google
Drive"` both hold together, while unquoted the first word goes to the keyword
and the rest become needles. A Skill name is an identifier and never needs
this; an MCP server's name is whatever its owner called it, and several of the
common ones have spaces.

`project:` reads all three places a project is written down: the `project`
field a Run carries, its working directory, and the `projects` array
an Agent carries one entry of per live run. Reading only the first two hid the
agent working in the very project asked about while listing its run.

A filter-shaped word that named nothing — `a:using` with nothing after it,
`a:agent:`, `a:state:banana` — is searched for literally, on `s:`'s rule: a
list that visibly collapses is a question you can see went wrong, while a
filter that quietly matches everything looks exactly like one that worked.

Every one of these filters runs against the one cached snapshot the launcher
already wrote. No Agent CLI, no directory walk, no relationship join
— `scoped_rows` is called from the per-keystroke helper.

Session filters are explicit words inside that scope:

| Query | Meaning |
|---|---|
| `s:is:pinned` | pinned, non-archived conversations |
| `s:is:active` | conversations attached to a live Run |
| `s:is:archived` | archived conversations only |
| `s:is:all` | include archived conversations in ordinary text search |

Archived Sessions are excluded from bare `s:` and from the small recent set
in the general catalogue. Renamed Sessions still match their native title.

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
