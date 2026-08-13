# Search model

This document describes the query behavior implemented in `src/compute.rs`,
`src/cache.rs`, and the source modules under `src/sources/`.

Prelude does not send its complete catalogue to fzf for every query. It builds
three files when the launcher opens:

- `home.txt`: the empty-query Agent home
- `list.txt`: the small root-search catalogue
- `search-items.json`: the complete gathered snapshot used by explicit scopes

The per-keystroke helper filters those files. It does not gather sources again.

## Empty query: Agent home

The home contains, in Kind order:

1. unanswered questions posted with `prelude ask`
2. installed Agent launch rows
3. live Agent Runs
4. non-archived Skills
5. non-archived MCP servers
6. up to the 15 newest visible Sessions

Files, applications, commands, history, clipboard records, ports, processes,
containers, settings, and search commands are gathered but do not appear on an
empty query.

The list is already sorted before `home_items` filters it. Kind decides the
band; source rank, Favorites, and frecency only reorder objects inside the same
Kind. A Favorite cannot lift a Skill above an Agent or an MCP server above a
Run.

Archived Sessions normally leave the home. If an archived Session is attached
to a live Run, it becomes visible while that Run exists; the archive flag is
not cleared. Archived Skills and MCP servers remain hidden until explicitly
requested.

## Ordinary root search

Once a non-special query is typed, fzf searches a smaller root catalogue.
Every ordinary result uses one semantic three-column layout—**name**, **type**,
then **context**—so static Agent/Skill/Search rows, local files and folders,
Quicklinks, and the web fallback share one type column. Explicit scopes retain
specialized layouts such as the wider name/path treatment in `f:` and `dir:`.

The root catalogue contains:

- unanswered questions
- Agent, Run, Skill, and MCP inventory rows
- scope commands such as `Past Conversations` and `Files & Folders`
- every Quicklink, fixed and template alike

Session and Config objects themselves are not in root search; their visible
scope commands lead to `s:` and `cfg:`. History, clipboard rows, and `$PATH`
commands require their scope.

Local objects are the deliberate exception, in two tiers. Any ordinary query of
at least two characters adds up to five matching applications, then up to ten
matching files or folders; exact `f` still opens the longer `f:` view, and
`app:` still lists every installed application. Both tiers sit below the
catalogue, so an Agent or a Quicklink keyword still leads its own name.
Applications sit above files because between two objects of one name, the
application is what a launcher is usually being asked for — `Chrome` means the
browser far more often than it means an icon inside somebody's `node_modules`.

An application matches only where the query is genuinely part of its name.
Files accept a forgiving subsequence; applications do not, because their block
is printed first and a stretched match would otherwise outrank a file whose
name the query actually spells.

Local matching uses the object's own name and Finder tags. A parent path is
shown only as context and does not make a child match. Thus `OpenGhostty`
returns `OpenGhosttyFromAnyFolder`, not every `main.swift` beneath it. A query
containing `/`, such as `OpenGhostty/main`, explicitly opts into ordered path
component matching.

Clearing the query returns to the Agent home. Type `:` to list every scope.

Two keys carry their shell meaning into the launcher. `Ctrl+R` moves the typed
text into the `h:` history scope and back out again — so pressing it twice at
a shell is still incremental history search, now over the deduplicated,
secret-filtered history rows. `Tab` completes the focused row when completion
is what Enter would do anyway: scope commands (`f` → `f:`) and search
providers (`g` → `g `). On any other row Tab does nothing.

## The web search under every query

Because root search deliberately excludes most large sources, an ordinary
sentence such as `git commit` can still match nothing local. Every query therefore
carries one computed web-search row, which is what makes a result of `0/58`
impossible:

```text
git commit    · Search Google · www.google.com    Enter opens the browser
```

Three rules define it:

- Its displayed text is the query itself, so fzf matches it by construction.
- It is emitted after the catalogue, so `--tiebreak=index` keeps it below
  anything that scored the same. It leads only when nothing else matched.
- It is absent inside a scope, and for the bare `:` and `/` browsers. A scope
  is a statement about where to look.

The provider follows the `g` Quicklink, so re-pointing that keyword at Baidu,
Kagi, or an intranet search moves this row with it. Deleting the keyword falls
back to Google. The row is an ordinary Link: `Ctrl+K` offers its URL as text
and can save it as a Quicklink.

## Scopes

| Prefix | Items selected from the cached snapshot |
|---|---|
| `a:` | questions, Agents, Runs, Skills, MCP servers, and Agent Config |
| `r:` | live Runs only |
| `s:` | Claude Code, Codex, and pi Sessions |
| `skill:` | non-archived Skills merged across Agent directories |
| `f:` | current-project files plus indexed files and folders |
| `c:` | clipboard text, Finder file lists, and images |
| `h:` | up to 3,000 recent unique non-secret shell-history commands |
| `app:` | installed macOS applications |
| `cmd:` | `$PATH` executables and built-in system commands |
| `dir:` | indexed folders, promoted by zoxide and recent `cd` use |
| `proj:` | current-project scripts, project files, and Git rows |
| `ssh:` | hosts from `~/.ssh/config` |
| `snip:` | snippets from `snippets.toml` |
| `port:` | cached listening TCP ports |
| `proc:` | processes with CPU and memory fields |
| `docker:` | running Docker containers |
| `mcp:` | non-archived Claude/Codex MCP inventory rows |
| `cfg:` | existing Agent settings and instruction files |
| `ql:` | every Quicklink, including entries that do not resolve |
| `set:` | Prelude settings with their effective values and sources |

A bare prefix lists the scope. Scoped queries disable fzf's own fuzzy filter;
Prelude applies the term against the cached Items so syntax such as `c:` is not
required to appear in a row's displayed text.

### Agent filters

`a:` accepts these structured words:

| Query | Meaning |
|---|---|
| `a:waiting` | waiting Runs and unanswered questions |
| `a:working` | working Runs |
| `a:dead` | dead Runs if one exists in the supplied snapshot; the live source normally drops exited processes |
| `a:claude Prelude` | free-text Agent/project search |
| `a:agent:claude` | exact Agent filter |
| `a:project:Prelude` | exact project name or full path |
| `a:state:waiting` | explicit Run-state filter |
| `a:using deploy` | Runs that explicitly loaded the named Skill or MCP capability |
| `a:without deploy` | Runs that loaded none of the named capabilities |

`using` and `without` inspect capability names extracted from that Run's launch
arguments. They do not mean “installed for this Agent.” Multiple `using` values
are ANDed; repeated values inside `agent:`, `project:`, or `state:` are ORed.
Capability names with spaces can be quoted:

```text
a:using "claude.ai Google Drive"
a:using:"claude.ai Google Drive"
```

Sessions are deliberately excluded from `a:` and remain under `s:`. Archived
Skills and MCP servers are also excluded. Unknown filter-shaped words are kept
as literal search needles, which visibly collapses the result instead of
silently widening it.

### Session filters

Session discovery currently reads native Claude Code, Codex, and pi JSONL
files. `s:` returns at most 80 matching rows and supports:

| Query | Meaning |
|---|---|
| `s:is:pinned` | pinned visible Sessions |
| `s:is:active` | Sessions linked to live Runs, including archived ones |
| `s:is:archived` | Sessions carrying the archive flag, even if active |
| `s:is:all` | include archived Sessions in ordinary text search |
| `s:agent:codex` | exact Agent |
| `s:project:Prelude` | exact directory basename or full path |
| `s:since:30m` | modified within 30 minutes |
| `s:since:24h` | modified within 24 hours |
| `s:since:7d` | modified within 7 days |
| `s:since:2w` | modified within 2 weeks |

Filters and ordinary words can be combined. Project values are quote-aware.
Several `agent:` or `project:` values are ORed; different filter categories are
ANDed. A duration requires a positive number and `m`, `h`, `d`, or `w` (long
forms such as `days` also work). Invalid forms such as `since:7` or
`is:yesterday` are searched literally and therefore normally return nothing.

A local Session rename does not remove the native title from search.

### Capability archive filters

| Query | Meaning |
|---|---|
| `skill:is:archived` | archived Skills only |
| `skill:is:all` | archived and visible Skills |
| `mcp:is:archived` | archived MCP capability rows only |
| `mcp:is:all` | archived and visible MCP capability rows |

Archive is a Prelude view overlay. It does not move Skill directories, disable
MCP servers, or edit Agent files. MCP owner variants sharing one normalized
capability id archive together. Archived capabilities also leave `a:`, the
home, root search, slash invocation, and Session borrow pickers.

## Agent and Skill invocation

`@` and `/` are computed providers, not ordinary fuzzy prefixes.

```text
@                     browse installed Agent question commands
@claude explain this  run the Agent's non-interactive ask form in Prelude
/                     browse visible Skills
/review               run the complete Skill invocation in Prelude
/review pull request  pass arguments after the Skill name
```

An incomplete Skill name browses matching Skill objects. Once the name exactly
matches, Prelude produces one `Session` start row whose answer is shown inside
Prelude; it no longer shows a second Skill row for the same intent. Archived
Skills are neither browsed nor invoked.

`@name prompt` resolves only installed Agents. An exact name wins; otherwise
the first matching built-in name in registry order resolves. The one-off forms
are:

- `claude -p`
- `codex exec --skip-git-repo-check`
- `pi --print`
- `opencode run`

These explicit asks may use the Agent provider's network. They are not part of
the per-keystroke inventory gather.

## Files, folders, and Finder tags

Ordinary search combines a small result from two sets:

- live files below the current project root
- files and folders stored by Prelude's shared index

`f:` shows a longer combined list; `dir:` restricts it to folders and merges in
zoxide/recent-`cd` evidence for ranking. Empty folders are indexed too.

Search folders are managed in `set:` → **Search folders**. `→` adds through the
native macOS folder chooser, `←` selects one to remove, and Enter opens the full
manager to inspect or modify the list. Removal never touches the folder or its
contents. The CLI exposes the same operations as `prelude settings roots`,
`add-root`, and `remove-root`.

The Settings panel itself is organized into **Search**, **Launcher**,
**Behavior**, and **Library**, with columns for the setting, current value, and
what it affects. Storage source, defaults, environment overrides, and backing
file paths remain available in Details and `prelude settings --json` rather
than leading every row. Every setting has contextual `←` and `→` controls,
whose selected-row meanings appear in the footer; see
[Actions](ACTIONS.md#prelude-setting) for the complete mapping.

The backing file is `~/.config/prelude/roots.txt`. When it does not exist,
Prelude starts with `~/App`, `~/Documents`, and `~/Desktop`; once the person has
managed the list, that file is authoritative, including an intentionally empty
list. Adding or removing a folder starts a background rebuild automatically.
The previous index stays searchable until the new generation is atomically
complete. `prelude index` remains an explicit repair command, not a normal setup
step.

The index walks to a maximum depth of 7 without following symlinks and respects
ignore files. It asks Foundation for Finder tags and stores bounded,
credential-filtered names beside each path. Ordinary terms match object names
or tags; `tag:` matches tags only:

```text
f:invoice
f:tag:work
f:tag:"Project Alpha"
```

Ordinary search returns at most 10 filesystem rows; `f:` and `dir:` return at
most 100. Exact names rank first, then prefixes, substrings, tags, and fuzzy
subsequences; folders get a small tie-break advantage. No Spotlight, `mdfind`,
`mdls`, or Finder-tag subprocess runs on a keystroke.

An explicitly typed existing local path does not require the index. Absolute,
`~/`, `./`, `../`, `file:///`, and slash-bearing relative paths are recognized.
Prelude tries the literal text before one layer of shell unescaping, then emits
a File, Folder, or Application object with its normal Enter, Quick Look, and
Quicklink behavior. Bare `/` remains the Skill browser.

## Clipboard

`clipd` watches `NSPasteboard.changeCount` and records type, timestamp, and a
content fingerprint. The scope is chronological; frecency does not move an old
clipboard row above a newer one.

- text remains text and passes through the secret filter
- Finder file lists remain file-list objects
- PNG/TIFF data is stored as a private image payload under Prelude's data
  directory; no OCR is performed

Inside `c:`, focusing any clipboard row automatically opens a right-side
preview. Image previews use Ghostty/Kitty's native transfer protocol when
available, then Chafa as a fallback. `Ctrl+P` remains the ordinary manual Quick
Look toggle outside that contextual behavior.

Entering a scope replaces the list, and the first row of the new list is
described as soon as it arrives — its footer and, in `c:`, its preview. This
holds without touching the cursor.

## Quicklinks, URLs, and computed rows

A Quicklink is a keyword the person saved. It has two shapes. A *fixed*
Quicklink points at one file, folder, application, or URL; a *template*
Quicklink carries `{q}` and takes a search term, so it has a command state and
a result state:

```text
notes         the folder that keyword names · Enter opens it
g             Search Google · Enter changes the query to `g `
g rust async  real Link row · Enter opens the generated URL
```

Every Quicklink is a row in `ql:`, including entries that do not resolve —
that scope states what is wrong with a broken entry and carries the same
rename, re-point, and remove actions as a working one.

The built-in set covers general web search (`g`, `gh`, `npm`, `mdn`, `gs`,
`b`, `bing`, `ddg`), the places a programmer looks things up (`so`, `crates`,
`docsrs`, `pypi`, `pkg`, `caniuse`, `explain`, `hn`), and the ones an agent
user does (`hf`, `arxiv`, `ccdocs`, `mcpdocs`). Built-ins arrive in versioned
blocks: each block is offered once, an entry whose keyword you already use is
skipped rather than overwritten, and a default you delete is not restored. No
built-in keyword is a scope prefix or an Agent's name.

Quicklinks sort in a band of their own, above every scope command and below
the Agent rows, so a partially typed keyword leads the list rather than
sinking to the bottom of it with the kind of object it points at. Exact
aliases outrank fuzzy root matches, and an exact key outranks a name another
entry happens to carry.

An exact keyword leads the list without replacing it: everything else matching
the query follows underneath, so completing `github` keeps the `Search GitHub`
that was already on screen instead of deleting it.

Both shapes are labelled `quicklink`. The kind column names what a row is, not
what Enter does to it, and `search` is reserved for the scope commands — a
scope command is built in and leads into Prelude's own index, while a template
is a line in your `quicklinks.toml` that leads to the web. The two happen to
behave alike on Enter because both need an argument.

Keys are folded to lower case on both sides of a comparison, so an entry
written `[Design]` by hand is reached by typing `design`. A key may use
letters and digits of any script plus `-` and `_`; anything the search box has
already spent — a scope prefix, `:`, `/`, `@`, whitespace — is refused when
the Quicklink is named rather than accepted and then left unreachable.
Credential-looking URLs are refused, in a fixed target and in a template.

`Ctrl+K` creates, renames, re-points, re-labels, and removes them; removal
reaches hand-written entries as well as Prelude's own. `prelude quicklink`
does the same from a script, and `prelude quicklink check` reports every entry
that will not resolve.

Prelude also recognizes explicit `http://` and `https://` URLs, unambiguous
hostnames, localhost/IP addresses, and `.local`/`.test` hosts. It refuses URL
credentials and gives plausible local filenames precedence over speculative
domains.

Computed rows include:

- arithmetic and time expressions
- unit conversion such as `10kg to lb` through the system `units` command
- currency conversion through a one-day exchange-rate cache; this is the one
  computed search that may fetch the network
- translation with `en: text` or `zh: text` after `prelude build-translate`

Translation uses Apple's local Translation framework helper. A missing language
must be downloaded in System Settings.

## Performance contract

- `cache::gather` has a 40 ms external deadline.
- Fast subprocess sources run concurrently; a source missing the deadline falls
  back to its previous cache while the worker refreshes that cache.
- Session inventory, MCP status, Skill hashes, MCP tools, ports, and fleet
  identity live behind cache tiers.
- Run liveness and waiting state are refreshed with syscalls on each gather.
- Scope filtering and intent recognition do not start Agent CLIs or repeat the
  Agent/Session relationship join.
