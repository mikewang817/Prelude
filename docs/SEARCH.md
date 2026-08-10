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

Once a non-special query is typed, fzf searches a smaller root catalogue:

- unanswered questions
- Agent, Run, Skill, and MCP inventory rows
- search-provider commands
- scope commands such as `Past Conversations` and `Search Files`
- fixed Quicklinks

Session and Config objects themselves are not in root search; their visible
scope commands lead to `s:` and `cfg:`. Large sources such as files, history,
applications, and `$PATH` commands also require their scope. For example, exact
`f` produces the `Search Files` command; Enter changes the query to `f:` rather
than matching every file containing the letter.

Clearing the query returns to the Agent home. Type `:` to list every scope.

## Scopes

| Prefix | Items selected from the cached snapshot |
|---|---|
| `a:` | questions, Agents, Runs, Skills, MCP servers, and Agent Config |
| `r:` | live Runs only |
| `s:` | Claude Code, Codex, and pi Sessions |
| `skill:` | non-archived Skills merged across Agent directories |
| `f:` | current-project files plus the explicit file index |
| `c:` | clipboard text, Finder file lists, and images |
| `h:` | up to 3,000 recent unique non-secret shell-history commands |
| `app:` | installed macOS applications |
| `cmd:` | `$PATH` executables and built-in system commands |
| `dir:` | zoxide results and directories recovered from shell history |
| `proj:` | current-project scripts, project files, and Git rows |
| `ssh:` | hosts from `~/.ssh/config` |
| `snip:` | snippets from `snippets.toml` |
| `port:` | cached listening TCP ports |
| `proc:` | processes with CPU and memory fields |
| `docker:` | running Docker containers |
| `mcp:` | non-archived Claude/Codex MCP inventory rows |
| `cfg:` | existing Agent settings and instruction files |
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

## Files and Finder tags

`f:` combines two sets:

- files below the current project root, gathered with the project source
- paths stored by the explicit `prelude index` command

Search roots come from `~/.config/prelude/roots.txt`; when absent or empty, the
built-ins are `~/App`, `~/Documents`, and `~/Desktop`. Adding or removing a root
does not rebuild the index automatically.

`prelude index` uses `fd`/`fdfind` when available, otherwise `find`, with a
maximum depth of 7. It then asks Foundation for Finder tags and stores bounded,
credential-filtered names beside each path. Ordinary terms match paths or tags;
`tag:` matches tags only:

```text
f:invoice
f:tag:work
f:tag:"Project Alpha"
```

Each indexed query returns at most 60 rows. No Spotlight, `mdfind`, `mdls`, or
Finder-tag subprocess runs on a keystroke.

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

## Providers, Quicklinks, URLs, and computed rows

A template Quicklink has a command state and a result state:

```text
g             Search Google · Enter changes the query to `g `
g rust async  real Link row · Enter opens the generated URL
```

Default templates include Google, GitHub, npm, MDN, Google Scholar, Baidu,
Bing, and DuckDuckGo. `quicklinks.toml` may also hold fixed file, folder, app,
and URL objects. Exact aliases outrank fuzzy root matches. Credential-looking
URLs are refused when creating a Quicklink.

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
