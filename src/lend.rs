//! Borrowing one agent's capability for a single run of another.
//!
//! `copy_skill` answers "I want this permanently" — it writes into the other
//! agent's skills directory and the skill is simply there from then on. This
//! module answers the far more common "just this once", and does it without
//! writing anything into a user's agent directories at all.
//!
//! Every agent turns out to have a door for exactly this, and no two of them
//! are the same shape:
//!
//! | | MCP | skill |
//! |---|---|---|
//! | claude | `--mcp-config <json>` | `--plugin-dir <dir>` |
//! | codex  | `-c mcp_servers.…=…`  | — |
//! | pi     | —                     | `--skill <path>` |
//! | opencode | —                   | — |
//!
//! A dash means the agent has no way to take one for a single run, and the
//! honest answer is to offer nothing rather than a command that will not
//! work. `copy_skill` remains the fallback for those.

use crate::exec::shq;
use std::path::{Path, PathBuf};

fn sensitive_text(value: &str) -> bool {
    if crate::secrets::looks_secret(value) {
        return true;
    }
    // Credentials in URL authority (`scheme://user:pass@host`) need no
    // keyword such as "token" to be dangerous.
    value.split_once("://").is_some_and(|(_, rest)| {
        rest.split('/').next().is_some_and(|authority| authority.contains('@'))
    })
}

/// An MCP server, in the only two shapes anyone ships.
///
/// Deliberately not a passthrough of either agent's own schema: the whole
/// point is to carry a definition from one to the other, so it holds what
/// both understand and drops what only one of them does (startup timeouts,
/// per-agent auth bookkeeping).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Mcp {
    Stdio {
        name: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: Vec<(String, String)>,
    },
    Http {
        name: String,
        url: String,
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
}

impl Mcp {
    pub fn name(&self) -> &str {
        match self {
            Mcp::Stdio { name, .. } | Mcp::Http { name, .. } => name,
        }
    }

    /// Does this definition carry anything beyond a command or a URL?
    ///
    /// Not "does it look secret" — an env block or a header set is treated as
    /// sensitive whether or not `secrets` recognises what is in it. Header
    /// and variable names for credentials are endless, and the cost of being
    /// wrong is asymmetric: staging a harmless definition in a file costs a
    /// path instead of a blob on screen, while inlining a secret one puts a
    /// key in shell history for good.
    pub fn has_sensitive_fields(&self) -> bool {
        match self {
            Mcp::Stdio { command, args, env, .. } => {
                !env.is_empty() || sensitive_text(command) || args.iter().any(|arg| sensitive_text(arg))
            }
            Mcp::Http { url, headers, .. } => !headers.is_empty() || sensitive_text(url),
        }
    }

    /// Stable comparison identity with all env/header names and values
    /// removed. A count says the definitions have private material without
    /// turning that material into indexed data.
    pub fn public_fingerprint(&self) -> String {
        let value = match self {
            Mcp::Stdio { command, args, env, .. } => {
                let command = if sensitive_text(command) { "<redacted>" } else { command };
                let args: Vec<&str> = args.iter().map(|arg| {
                    if sensitive_text(arg) { "<redacted>" } else { arg.as_str() }
                }).collect();
                serde_json::json!({
                    "type": "stdio", "command": command, "args": args,
                    "private_fields": env.len(),
                })
            }
            Mcp::Http { url, headers, .. } => serde_json::json!({
                "type": "http", "url": if sensitive_text(url) { "<redacted>" } else { url },
                "private_fields": headers.len(),
            }),
        };
        crate::capability::fingerprint(value.to_string().as_bytes())
    }

    /// The name of the first field that `secrets` calls a credential.
    pub fn secret_field(&self) -> Option<String> {
        match self {
            Mcp::Stdio { command, args, .. }
                if sensitive_text(command) || args.iter().any(|arg| sensitive_text(arg)) =>
            {
                return Some("command argument".into());
            }
            Mcp::Http { url, .. } if sensitive_text(url) => return Some("URL credential".into()),
            _ => {}
        }
        let pairs: &[(String, String)] = match self {
            Mcp::Stdio { env, .. } => env,
            Mcp::Http { headers, .. } => headers,
        };
        pairs
            .iter()
            .find(|(k, v)| crate::secrets::looks_secret(k) || crate::secrets::looks_secret(v))
            .map(|(k, _)| k.clone())
    }

    /// A name safe to use as a bare dotted-path segment.
    ///
    /// codex addresses config by dotted path, so a server called
    /// `claude.ai Gmail` would be read as three levels of nesting. Borrowed
    /// servers are named fresh in the borrowing session anyway, so renaming
    /// costs nothing.
    pub fn key(&self) -> String {
        let s: String = self
            .name()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let s = s.trim_matches('_').to_string();
        if s.is_empty() { "borrowed".into() } else { s }
    }

    /// The `--mcp-config` payload claude accepts. Its help says "JSON files
    /// or strings", so a definition with nothing sensitive in it rides
    /// inline and leaves no file behind; `mcp_flags` decides which.
    /// Just the server object, without the `mcpServers` wrapper.
    ///
    /// `--mcp-config` wants a whole config file; `mcp add-json` wants one
    /// server. Same data, two shapes.
    pub fn to_body_json(&self) -> String {
        let full = self.to_claude_json();
        serde_json::from_str::<serde_json::Value>(&full)
            .ok()
            .and_then(|v| v.get("mcpServers")?.as_object()?.values().next().cloned())
            .map(|b| b.to_string())
            .unwrap_or(full)
    }

    pub fn to_claude_json(&self) -> String {
        let body = match self {
            Mcp::Stdio { command, args, env, .. } => {
                let mut m = serde_json::Map::new();
                m.insert("command".into(), command.as_str().into());
                if !args.is_empty() {
                    m.insert("args".into(), args.clone().into());
                }
                if !env.is_empty() {
                    m.insert("env".into(), json_obj(env));
                }
                serde_json::Value::Object(m)
            }
            Mcp::Http { url, headers, .. } => {
                let mut m = serde_json::Map::new();
                m.insert("type".into(), "http".into());
                m.insert("url".into(), url.as_str().into());
                if !headers.is_empty() {
                    m.insert("headers".into(), json_obj(headers));
                }
                serde_json::Value::Object(m)
            }
        };
        let mut servers = serde_json::Map::new();
        servers.insert(self.name().to_string(), body);
        let mut root = serde_json::Map::new();
        root.insert("mcpServers".into(), serde_json::Value::Object(servers));
        serde_json::Value::Object(root).to_string()
    }

    /// codex's per-invocation overrides. The value half of `-c key=value` is
    /// parsed as TOML, so each one is emitted as a TOML literal.
    pub fn to_codex_flags(&self) -> Vec<String> {
        let k = self.key();
        let mut v = Vec::new();
        let mut set = |field: &str, value: String| {
            v.push("-c".to_string());
            v.push(format!("mcp_servers.{k}.{field}={value}"));
        };
        match self {
            Mcp::Stdio { command, args, env, .. } => {
                set("command", toml_str(command));
                if !args.is_empty() {
                    set("args", toml_array(args));
                }
                if !env.is_empty() {
                    set("env", toml_table(env));
                }
            }
            Mcp::Http { url, headers, .. } => {
                set("url", toml_str(url));
                if !headers.is_empty() {
                    set("http_headers", toml_table(headers));
                }
            }
        }
        v
    }

    /// Read one out of codex's `mcp list --json`.
    ///
    /// Returns None for anything neither shape covers, which is the same
    /// contract the sources have: a format that changed underfoot yields
    /// nothing rather than a half-built definition.
    pub fn from_codex(name: &str, transport: &serde_json::Value) -> Option<Mcp> {
        let name = name.to_string();
        match transport.get("type").and_then(|t| t.as_str())? {
            "stdio" => Some(Mcp::Stdio {
                name,
                command: transport.get("command")?.as_str()?.to_string(),
                args: str_list(transport.get("args")),
                env: pairs(transport.get("env")),
            }),
            t if t.contains("http") || t == "sse" => Some(Mcp::Http {
                name,
                url: transport.get("url")?.as_str()?.to_string(),
                headers: pairs(transport.get("http_headers")),
            }),
            _ => None,
        }
    }

    /// Read one out of `claude mcp get <name>`.
    ///
    /// Returns None for claude.ai-hosted servers, and that is the important
    /// case rather than an edge one: they carry no command and no borrowable
    /// URL, because their credentials live with the Claude account rather
    /// than on this machine. Offering to lend one would produce a command
    /// that connects to nothing.
    pub fn from_claude_get(name: &str, text: &str) -> Option<Mcp> {
        let mut command = String::new();
        let mut args: Vec<String> = Vec::new();
        let mut url = String::new();
        for line in text.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            match k.trim().to_ascii_lowercase().as_str() {
                "command" => command = v.to_string(),
                "args" => args = v.split_whitespace().map(str::to_string).collect(),
                "url" => url = v.to_string(),
                // Reconstructed above; `Scope: claude.ai config` is exactly
                // the server that cannot be lent.
                _ => {}
            }
        }
        // A URL line reappears as part of `Status:` on some builds, so the
        // command form is checked first.
        if !command.is_empty() {
            return Some(Mcp::Stdio { name: name.into(), command, args, env: Vec::new() });
        }
        if url.starts_with("http") {
            return Some(Mcp::Http { name: name.into(), url, headers: Vec::new() });
        }
        None
    }
}

/// The lendable definition of the server a row stands for.
///
/// Complete definitions are never retained in launcher Items or caches:
/// command arguments, env and headers can all carry credentials. Resolve
/// from the owning CLI here, on an explicit keystroke, where a subprocess is
/// affordable and private data remains transient.
pub fn resolve(it: &crate::item::Item) -> Result<Mcp, String> {
    if let Ok(m) = serde_json::from_str::<Mcp>(it.get("def")) {
        return Ok(m);
    }
    let (agent, name) = (it.get("agent"), it.get("name"));
    // A row cached before this machine knew to keep definitions carries no
    // `def`, and the cache may not refresh for minutes. Rather than explain
    // that, ask again.
    if agent == "codex" {
        let text = crate::exec::run(&["codex", "mcp", "list", "--json"], std::time::Duration::from_secs(20));
        return serde_json::from_str::<Vec<serde_json::Value>>(&text)
            .ok()
            .and_then(|list| {
                list.into_iter()
                    .find(|s| s.get("name").and_then(|v| v.as_str()) == Some(name))
                    .and_then(|s| Mcp::from_codex(name, s.get("transport")?))
            })
            .ok_or_else(|| format!("codex no longer reports a server called {name}"));
    }
    if agent != "claude" {
        return Err(format!("don't know how to read {agent}'s server definitions"));
    }
    let text = crate::exec::run(&["claude", "mcp", "get", name], std::time::Duration::from_secs(20));
    Mcp::from_claude_get(name, &text).ok_or_else(|| {
        format!("{name} is hosted by claude.ai — its credentials live with the \
                 Claude account, not on this machine, so there is nothing to lend")
    })
}

/// Flags that attach a borrowed MCP server to one run of `agent`.
///
/// Additive on purpose. claude's `--strict-mcp-config` would make the
/// borrowed server the *only* one, which is not what borrowing means — you
/// want your own servers plus this one.
///
/// A server's env block is the one part of this that must never reach a
/// command line. It routinely holds an API key, and the borrowed command is
/// handed to the shell prompt, from where it goes into shell history — which
/// this launcher then reads back and ranks. claude takes a file, so anything
/// carrying env or headers is written to one, mode 0600, and only its path
/// appears. codex has no file form, so the same server is refused rather
/// than leaked.
pub fn mcp_flags(agent: &str, m: &Mcp) -> Result<Vec<String>, String> {
    match agent {
        // `--mcp-config` is variadic — it keeps eating bare words until the
        // next option. Written with a space, a prompt typed after the
        // borrowed command is swallowed as another config file and claude
        // dies with "MCP config file not found: <your prompt>". The `=` form
        // takes exactly one value and stops, which is what makes the command
        // safe to type after.
        "claude" => {
            let json = m.to_claude_json();
            if !m.has_sensitive_fields() {
                return Ok(vec![format!("--mcp-config={json}")]);
            }
            let p = private_file(&format!("{}.json", m.key()), json.as_bytes())
                .map_err(|e| format!("could not stage the server definition: {e}"))?;
            Ok(vec![format!("--mcp-config={}", p.to_string_lossy())])
        }
        "codex" => {
            if m.has_sensitive_fields() {
                let k = m.secret_field().unwrap_or_else(|| "private fields".into());
                return Err(format!(
                    "{} carries {k}, and codex can only be handed a server inline — \
                     it would end up in your shell history. Lend it to claude \
                     instead, or add it to codex permanently with `codex mcp add`",
                    m.name()
                ));
            }
            Ok(m.to_codex_flags())
        }
        _ => Err(format!("{agent} cannot take an MCP server for one run")),
    }
}

/// Flags that attach a borrowed skill to one run of `agent`.
///
/// pi takes a path to anyone's skill directory directly. claude wants a
/// plugin, so one gets synthesised — see `plugin_shim`.
pub fn skill_flags(agent: &str, dir: &Path, name: &str) -> Result<Vec<String>, String> {
    match agent {
        "pi" => Ok(vec!["--skill".into(), dir.to_string_lossy().into_owned()]),
        "claude" => {
            let shim = plugin_shim(dir, name)
                .map_err(|e| format!("could not stage {name} as a plugin: {e}"))?;
            Ok(vec!["--plugin-dir".into(), shim.to_string_lossy().into_owned()])
        }
        _ => Err(format!("{agent} cannot load a skill it does not own")),
    }
}

/// A file only this user can read, staged in Prelude's own cache.
fn private_file(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = crate::paths::cache().join("borrow");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    let tmp = path.with_extension("tmp");
    // The mode is set as the file is created, not after: a chmod afterwards
    // leaves a window in which the key is world-readable.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Agents that can borrow a skill they do not own.
pub fn can_borrow_skill(agent: &str) -> bool {
    matches!(agent, "pi" | "claude")
}

/// Agents that can borrow an MCP server.
pub fn can_borrow_mcp(agent: &str) -> bool {
    matches!(agent, "claude" | "codex")
}

/// A one-skill plugin whose skill is a symlink to the original.
///
/// `--plugin-dir` is claude's only session-scoped way in, and it wants a
/// plugin rather than a loose skill directory. The shim lives in Prelude's
/// cache and links rather than copies, so the borrowed skill stays a view of
/// its owner's copy: edit the original and the next borrowed run sees the
/// edit. Nothing under a user's agent directories is touched.
pub fn plugin_shim(dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let root = crate::paths::cache().join("borrow").join(name);
    let meta = root.join(".claude-plugin");
    let skills = root.join("skills");
    std::fs::create_dir_all(&meta)?;
    std::fs::create_dir_all(&skills)?;
    let manifest = serde_json::json!({
        "name": format!("borrowed-{name}"),
        "version": "0.0.0",
        "description": format!("{name}, borrowed by Prelude for one session"),
    });
    crate::cache::write_atomic(&meta.join("plugin.json"), manifest.to_string().as_bytes())?;
    let link = skills.join(name);
    // The owner may have moved since the last borrow, so the link is always
    // rebuilt. remove_file is what unlinks a symlink; remove_dir_all would
    // follow it and delete the original skill.
    if std::fs::symlink_metadata(&link).is_ok() {
        std::fs::remove_file(&link)?;
    }
    std::os::unix::fs::symlink(dir, &link)?;
    Ok(root)
}

/// The whole command: cd there, start the agent with the borrowed thing
/// attached, hand it the prompt.
///
/// All three agents that can borrow take a prompt positionally, so unlike
/// `start_cmd` there is no per-agent subcommand to thread through — the ones
/// that need one (opencode) cannot borrow in the first place.
pub fn borrow_cmd(agent: &str, flags: &[String], cwd: Option<&str>, prompt: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(d) = cwd.filter(|d| !d.is_empty()) {
        s.push_str(&format!("cd {} && ", shq(d)));
    }
    s.push_str(agent);
    for f in flags {
        s.push(' ');
        s.push_str(&shq(f));
    }
    if let Some(p) = prompt.filter(|p| !p.is_empty()) {
        s.push(' ');
        s.push_str(&shq(p));
    }
    s
}

fn json_obj(pairs: &[(String, String)]) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (k, v) in pairs {
        m.insert(k.clone(), v.as_str().into());
    }
    serde_json::Value::Object(m)
}

/// TOML basic string. serde_json's escaping is a subset of TOML's for the
/// characters that actually turn up here, and saves hand-rolling one.
fn toml_str(s: &str) -> String {
    serde_json::Value::from(s).to_string()
}

fn toml_array(v: &[String]) -> String {
    let inner: Vec<String> = v.iter().map(|s| toml_str(s)).collect();
    format!("[{}]", inner.join(","))
}

fn toml_table(v: &[(String, String)]) -> String {
    let inner: Vec<String> = v.iter().map(|(k, x)| format!("{}={}", toml_str(k), toml_str(x))).collect();
    format!("{{{}}}", inner.join(","))
}

fn str_list(v: Option<&serde_json::Value>) -> Vec<String> {
    v.and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn pairs(v: Option<&serde_json::Value>) -> Vec<(String, String)> {
    v.and_then(|o| o.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// The command that installs a borrowed server into another agent **for
/// good**, using that agent's own `mcp add` rather than editing its config.
///
/// Borrowing lasts one run; this is the other half of slot 7. Both CLIs know
/// their own format, so writing `~/.claude.json` or `~/.codex/config.toml`
/// ourselves would be inventing a job someone else already does correctly —
/// and doing it to a file that holds far more than MCP servers.
///
/// The command is handed over unrun, like every other command line here.
///
/// **A server whose env holds a credential is refused for both**, which is
/// the difference from lending: claude can be *lent* one because the
/// definition goes into a 0600 file, but `mcp add` takes it inline, so there
/// is no form of this that keeps the key off the command line — and off the
/// shell history this launcher reads back.
pub fn install_cmd(agent: &str, m: &Mcp) -> Result<String, String> {
    if m.has_sensitive_fields() {
        let field = m.secret_field().unwrap_or_else(|| "private fields".into());
        return Err(format!(
            "{} carries {field} — installing it would put private data on your command line",
            m.name()
        ));
    }
    let name = m.key();
    match (agent, m) {
        ("claude", _) => Ok(format!(
            "claude mcp add-json {} {}",
            crate::exec::shq(&name),
            crate::exec::shq(&m.to_body_json())
        )),
        ("codex", Mcp::Http { url, .. }) => Ok(format!(
            "codex mcp add {} --url {}",
            crate::exec::shq(&name),
            crate::exec::shq(url)
        )),
        ("codex", Mcp::Stdio { command, args, .. }) => {
            let mut s = format!("codex mcp add {} -- {}", crate::exec::shq(&name), crate::exec::shq(command));
            for a in args {
                s.push(' ');
                s.push_str(&crate::exec::shq(a));
            }
            Ok(s)
        }
        _ => Err(format!("{agent} has no way to add an MCP server from the command line")),
    }
}
