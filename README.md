# Prelude

A Raycast-style launcher for your terminal.

Press one key. A search box appears. Type a few letters. It searches
everything you might want to run — this project's scripts, your shell
history, listening ports, running processes, installed apps, your clipboard,
your snippets — and **types the command onto your prompt.**

You press Enter to actually run it.

```
╭─ Prelude ────────────────────────────────────────────────────────────╮
│ ⌕ dev                                                        4/2280  │
│ ──────────────────────────────────────────────────────────────────── │
│   ⏎ insert   ^O run here   ^X run in shell   ^Y copy   ^K actions    │
│ ▸ pnpm dev                    script    · package.json · vite --host │
│   npm run dev                 history                                │
│   :3000 node                  port      · node      · pid 4821       │
╰──────────────────────────────────────────────────────────────────────╯
```

## What Enter does

The list holds two different kinds of thing, and they deserve different
defaults.

**Commands** — history, scripts, `$PATH`, snippets, ports, processes — are
**inserted onto your prompt, never executed.** You read them before they run.
That is what makes it safe to bind this to a key you press dozens of times a
day, and safe for the launcher to offer things like killing whatever holds
port 3000.

**Objects** — files, apps, links, sessions, results — you wanted to *use*.
Enter opens the file, launches the app, follows the link, resumes the
session. These are harmless and reversible in a way that running a shell
command is not.

The right answer also depends on where you are. Selecting a file at a shell
prompt means "open it". Selecting the same file from the popup over an
agent's input box means "here, look at this path" — so that is what you get.

`^K` always states the current default as its first entry, and
`PRELUDE_CLASSIC_ENTER=1` restores insert-everything.

## Install

Requires [fzf](https://github.com/junegunn/fzf) and a Rust toolchain.

```sh
git clone https://github.com/YOUR_NAME/prelude
cd prelude
cargo build --release
ln -s "$PWD/target/release/prelude" /usr/local/bin/prelude   # or anywhere on $PATH

echo 'eval "$(prelude init zsh)"' >> ~/.zshrc
exec zsh
```

Press **Ctrl-R**. Run `prelude doctor` to check the setup.

> Ctrl-R replaces zsh's incremental history search, which moves to Ctrl-S.
> Prelude is a superset of it. Override with `PRELUDE_KEY='^T'` set before the
> `eval` line. Ctrl-Space is deliberately not used: macOS binds it to
> "Select the previous input source" and the OS eats it first.

## Using it inside an agent conversation

The Ctrl-R widget only exists at a shell prompt. Inside a coding agent's TUI
the agent owns the keyboard, so nothing bound in your shell can reach you.

tmux sits *above* whatever runs in the pane, so a tmux binding works
everywhere:

```sh
prelude init tmux >> ~/.tmux.conf
tmux source-file ~/.tmux.conf
```

**prefix + r** now opens the launcher in a floating popup over whatever you
are doing, and picking something **types it into the pane underneath** —
into the agent's input box, vim, a REPL, an ssh session — without submitting
it. `^O` instead runs the command inside the popup, so the agent never sees
it at all.

## Keys

| Key | Action |
|---|---|
| `^R` | Open the launcher |
| `⏎` | The obvious thing for what you selected — see below |
| `^O` | Run it here, inside the launcher; the window stays open |
| `^X` | Run it in the shell below |
| `^Y` | Copy; the window stays open |
| `^K` | Action panel — the verbs for the selected item |
| `^P` | Toggle the detail pane |
| `esc` | Close |

## What it searches

| Source | Where it comes from |
|---|---|
| **Project scripts** | `package.json`, `Makefile`, `justfile`, `Cargo.toml`, `pyproject.toml`, `docker-compose.yml` — found by walking up from `$PWD`, with the runner picked from your lockfile |
| **Ports** | Listening TCP ports — "what's on :3000, kill it" |
| **Processes** | Heaviest by CPU and memory |
| **Containers** | Running Docker containers |
| **Clipboard** | The last things you copied |
| **Snippets** | `snippets.toml`, with `{{placeholder}}` blanks |
| **Agent skills** | Merged across Claude Code, Codex, pi, opencode |
| **MCP servers** | Merged across every agent that has any |
| **Apps** | Every installed `.app` |
| **SSH hosts** | `~/.ssh/config` |
| **Files** | The current project, plus an indexed set of roots via `f:name` |
| **History** | Deduped, newest first |
| **Folders** | zoxide's database, or `cd` targets mined from history |
| **Git** | Branches read straight off `.git/refs` |
| **`$PATH`** | Every executable, ranked lowest |

Plus rows computed from what you type: arithmetic, unit and currency
conversion, date arithmetic, on-device translation, and quicklinks.

```
10kg to lb        →  22.046226 lb        1847*0.23     →  424.81
1gb to mb         →  1,024 mb            now + 3 days  →  2026-08-10 …
100 usd to cny    →  676.04 CNY          1699999999    →  2023-11-15 …
en:你好            →  Hello               g rust async  →  opens Google
```

## The action panel

`^K` gives a result verbs appropriate to *what it is*, not just "insert":

| Kind | Actions |
|---|---|
| **port** | Insert kill · Kill now · Show what's using it · Copy pid |
| **process** | Insert kill · Kill now · Show full command · Copy pid |
| **container** | Shell into · Follow logs · Stop · Restart |
| **skill / mcp** | Insert name · Show description · Open in editor · cd to it |
| **file** | Insert path · Open in `$EDITOR` · Copy absolute path |
| **snippet** | Insert and fill blanks · Edit snippets file |

That is the difference between a command picker and a launcher: a port is not
text to insert, it is something you kill or inspect.

## One surface for every agent

**Resume a past conversation** without hunting for a uuid. Sessions from
Claude Code, Codex and pi are merged, newest first, with Claude's
AI-generated titles where available:

```
⌕ s:inkquest
▸ Develop InkQuest math problem app     session · claude · ~/App/InkQuest · 2h ago
  InkQuest                              session · pi     · ~/App/InkQuest · 1d ago
```

The most recent 15 appear in the main list; `s:` searches all of them.

**Copy a skill to an agent that lacks it.** Prelude knows which agents have a
skill, so it knows which do not:

```
▸ my-skill    skill · claude, shared · missing: codex, pi
     ^K  →  Copy it to codex / Copy it to pi / Copy it to all missing
```

**Start a session from the launcher** with `@claude refactor this`, which
opens an agent in the current directory with that prompt.

**Jump to any agent config** — `CLAUDE.md`, `AGENTS.md`, `settings.json`,
`config.toml`, `.mcp.json` — as a first-class row.



Prelude reads skills and MCP servers from every agent CLI you have installed
and merges them into one list. A skill present in several agents is shown
once, labelled with which agents have it, rather than duplicated or silently
deduped.

Skills come from `~/.claude/skills`, `~/.agents/skills`, `~/.codex/skills`,
`~/.pi/agent/skills` and `~/.config/opencode/skills`; MCP servers from
`~/.codex/config.toml`, `~/.claude.json` and any project-local `.mcp.json`.
`prelude doctor` prints the inventory.

## It learns

Every selection is recorded in `frecency.tsv` — a plain text file you can read
and edit. What you actually pick floats up, with recent use weighted far above
old use.

## Latency

Latency is the whole product. A launcher that takes 250ms to appear feels
broken.

```
$ prelude bench
gather: 2364 items  min 20.4ms  median 22.1ms  max 28.6ms
budget: 40ms  ->  OK
```

Startup is ~1.7ms, which matters because fzf re-invokes the binary on every
keystroke to decide what to show.

Sources are tiered by cost:

- **Local** (history, scripts, skills, snippets, git, apps) — file reads only.
- **Fast external** (zoxide, docker, `fd`, `ps`) — run concurrently against a
  50ms deadline; a straggler falls back to its cached result and finishes
  writing a fresh one while you read the list.
- **Slow external** (`lsof` for ports, ~65ms and not improvable — restricting
  it to one user is *worse*) — always served from cache, refreshed detached.
  Safe to be stale because the generated command is `kill $(lsof -ti tcp:3000)`,
  which re-resolves the pid at run time rather than trusting the cached one.
- **`$PATH`** (~250ms to scan) — cached, refreshed detached. Only the very
  first run pays full price.

## Configuration

| Variable | Effect |
|---|---|
| `PRELUDE_KEY='^T'` | Change the hotkey (default `^R`) |
| `PRELUDE_HEIGHT=80%` | How much of the terminal the inline view uses |
| `PRELUDE_NO_POPUP=1` | Never open a tmux popup; render inline |
| `PRELUDE_PREVIEW_MIN=150` | Terminal width at which the detail pane appears |
| `PRELUDE_NO_PREVIEW=1` | Never show the detail pane |
| `PRELUDE_DEBUG=1` | Report source failures and fzf fallbacks on stderr |

Files, all under the usual XDG locations:

| Path | What |
|---|---|
| `$XDG_CONFIG_HOME/prelude/snippets.toml` | Your snippets |
| `$XDG_CONFIG_HOME/prelude/quicklinks.toml` | Your quicklinks |
| `$XDG_CONFIG_HOME/prelude/roots.txt` | Which folders `f:` indexes |
| `$XDG_DATA_HOME/prelude/frecency.tsv` | What you pick, so it learns |
| `$XDG_DATA_HOME/prelude/clipboard.jsonl` | Clipboard history |
| `$XDG_CACHE_HOME/prelude/` | Source caches |

## Translation

`en:文字` or `zh:text` translates using Apple's on-device models — offline,
nothing leaves your machine. Build the helper once:

```sh
prelude build-translate     # needs Xcode's swiftc
```

Apple's `Translation` framework is only vended through a SwiftUI view modifier
and refuses to serve a bare CLI binary — the XPC call to `translationd` hangs
forever with no error. It works as soon as the caller has real app-bundle
identity, so this compiles a tiny Swift helper and wraps it in an
ad-hoc-signed `.app`. No developer account needed.

Two quirks, both handled: auto-detection hangs indefinitely on very short
input, so the source language is pinned by script instead; and the model is
fine for casual text but unreliable for legal or technical register.

## Platform

**macOS only, for now.** The core is portable, but ports, processes, apps,
clipboard, the system commands and translation all use macOS-specific
interfaces. Linux support is welcome but not present.

## Notes on the awkward parts

**zsh metafies its history file.** Any byte in `0x80-0x9f` is stored as `0x83`
followed by `byte ^ 32`. Read it as plain UTF-8 and every multi-byte character
comes back mangled, and the replacement characters have the wrong display
width — which silently breaks column alignment. Prelude undoes it first.

**East Asian *Ambiguous* width is a real fork.** `·` `—` `“”` `→` are one
column in most Western terminals and two in CJK-configured ones, and `·` is
the separator used on every row. Rather than infer it from `$LANG`,
`prelude doctor` prints one, asks the terminal where the cursor ended up
(`ESC[6n`), and caches the answer.

**fzf matches against displayed text.** A computed row can therefore never
fuzzy-match the query that produced it — you would type `en:…` and watch your
own answer get filtered out. Prelude uses a `change:transform` binding to
disable fzf's filtering for computed queries and re-enable it otherwise.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports with a screenshot of
the list rendering wrong are the most useful thing you can send.

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
