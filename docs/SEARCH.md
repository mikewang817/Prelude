# Search model

Prelude has a small home and a large catalogue. They are deliberately not
the same list.

## Empty query: agent home

The home is what is *outstanding*, not an inventory. The empty-query feed
contains:

- questions waiting for a person (`Msg`)
- open tasks (`Task`)
- running agents (`Run`)
- installed agents, as launch entries (`Agent`)
- exceptions: an MCP server that cannot account for itself, and a Skill whose
  copies have diverged or whose tree has a broken or escaping symlink or an
  unreadable file

History, files, applications, `$PATH`, sessions and machine objects are still
gathered at startup, but are not rendered into the home feed. Nor is healthy
inventory: a skill that is fine is a keystroke away through `/name`, `a:` and
ordinary search, a server that answers through `mcp:`, `a:` and ordinary
search, and forty of them in front of three running agents is a list nobody
reads.

A row is dropped when it *accounts for itself*, which is not the same as
answering:

| Kind | Dropped | Shown |
|---|---|---|
| `Mcp` | `health` is `ok` or `disabled` | `failed`, `auth`, `unknown`, an unrecognised word, or no `health` field at all |
| `Skill` | `integrity` is `single`, `identical`, `unknown` or `private-unknown`, **and** no copy has a broken or escaping symlink or an unreadable file | `divergent`, any of those tree faults, or no `integrity` field at all |

`disabled` is somebody's decision, and `prelude doctor mcp` says so — it
records a note rather than an issue and reports the server `ok`. A permanent
home row for it would be Prelude contradicting its own diagnostic for ever.
What stays is the server that can say neither.

`unknown` integrity means at least one copy has no fingerprint, so nothing can
be said about whether they match. In the launcher the usual cause is that the
background `skill-hashes` source has not reached that copy yet, which is true
of every skill on a machine's first launch; the other cause, a tree that could
not be read, is caught directly by the `unreadable` count rather than through
the word. `doctor skills` words the state identically and names the cause it
can be sure of: it hashes as it reports, so a missing fingerprint there is
always a copy that could not be read.

### Home order

Home rows are ordered by what is outstanding rather than by kind. The plan
names seven slots; `home_rank` inserts two more, so the full order is:

1. explicit unanswered human questions
2. failed tasks
3. completed tasks awaiting review
4. waiting agents
   - tasks that reported themselves waiting
5. working agents
   - tasks that reported themselves working
6. queued tasks, and anything terminal that is not a failure
7. agent launch entries, then inventory exceptions

A task that reported itself waiting or working sits beside its agent-state
counterpart, because a task saying it is blocked and a run that has gone quiet
are the same fact reported two ways — below it rather than above, because the
machine's own reading of a run outranks a claim the task made about itself
some time ago.

This is deliberately *not* `cache::by_rank`. That comparison settles the kind
band before it looks at any score, which is what stops learned ranking lifting
one kind over another in search. The home interleaves two kinds by state — a
failed task above the waiting runs, a queued one below the working ones — and
those two orderings point in opposite directions, so no single band per kind
can express both. `home_rank` is that order, applied in `home_items` after the
general list is built; the sort is stable, so within one slot the source's own
ranking and frecency still decide.

A completed task stays in slot 3 until it is acknowledged, and acknowledgement
is an explicit act that removes it from the home's source — never inferred
from the row having been looked at. Focus and Quick Look move across rows
without a decision being made, and a task is routinely opened while it is
still running, so "opening acknowledges it" would silently drop the completion
notice the slot exists to show.

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
| `a:` | agents, open tasks, running agents, skills, MCP and agent config |
| `r:` | running agents, classified live |
| `s:` | all conversation sessions |
| `f:` | current-project files and the indexed roots |
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
its `^K` panel — is reached by the prefix, by `a:`, or by ordinary search.
`/` never lists MCP servers at all: they are not invoked by name, and `mcp:`
is their scope.

Agent sessions intentionally do not appear under `a:`. Hundreds of old
conversations turned the agent overview into a session browser; `s:` already
has that job. Open tasks do appear there, and have no scope of their own:
they are bounded by outstanding work rather than by everything ever done, so
they cannot swamp the control scope the way conversations did, and `a:waiting`
and `a:failed` already ask the task-shaped questions inside the scope that
owns the control plane. A `t:` prefix would be a second door onto the same
handful of rows, at the cost of a permanent root-command row and a prefix
nothing else can use.

Control filters are explicit words inside `a:`:

| Query | Meaning |
|---|---|
| `a:waiting` | anything waiting — a run gone quiet, a task that said so, a question blocked on you |
| `a:failed` | failed tasks |
| `a:queued`, `a:working`, `a:done`, `a:cancelled` | the rest of the `task::State` vocabulary |
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
field a Run or Task carries, its working directory, and the `projects` array
an Agent carries one entry of per live run. Reading only the first two hid the
agent working in the very project asked about while listing its run.

A filter-shaped word that named nothing — `a:using` with nothing after it,
`a:agent:`, `a:state:banana` — is searched for literally, on `s:`'s rule: a
list that visibly collapses is a question you can see went wrong, while a
filter that quietly matches everything looks exactly like one that worked.

Every one of these filters runs against the one cached snapshot the launcher
already wrote. No Agent CLI, no tmux, no directory walk, no relationship join
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

- `cache::gather` still runs once, within the 40 ms budget. The Task source
  reads `task::home_tasks`, which is bounded by outstanding work rather than
  by everything ever done: 0.07 ms for a real store, 0.9 ms for fifty open
  tasks among five thousand records. Reading that same store whole is 54 ms,
  which is why nothing on this path calls `task::all`. Nor does it call
  `task::open_tasks`, which is the reader that *may* walk the directory: that
  one is for `task list`, `task show` and `doctor`, where a complete answer is
  worth a scan and there is no per-keystroke budget to blow.
- `ui::search` renders `home.txt` and the root-command `list.txt` with one
  shared layout, and stores the complete gathered items as JSON for scopes.
- Per-keystroke helpers read those files; they do not gather sources again.
- `running` remains live because cached state is actively misleading.
- `is_special` recognizes intent but never runs a calculation, subprocess or
  network request.
- Computed and scoped queries disable fzf search so their own rows cannot be
  filtered out by the syntax that produced them.
