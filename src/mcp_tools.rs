//! Slow, cached MCP tool inventory.
//!
//! Agent list/get commands do not expose actual tools. For enabled stdio
//! servers Prelude performs the MCP initialize + tools/list handshake itself
//! in this background source. Env values are transient child-process input;
//! only filtered names/descriptions and a timestamp survive.

use crate::item::{Item, Kind};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

fn clean(value: &str, max: usize) -> String {
    crate::width::flatten(value).trim().chars().take(max).collect()
}

pub(crate) fn parse_tools_response(value: &serde_json::Value) -> Vec<Tool> {
    value.get("result")
        .and_then(|result| result.get("tools"))
        .and_then(|tools| tools.as_array())
        .map(|tools| {
            tools.iter().filter_map(|tool| {
                let name = clean(tool.get("name")?.as_str()?, 120);
                if name.is_empty() || crate::secrets::looks_secret_material(&name) {
                    return None;
                }
                let raw = tool.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let description = if crate::secrets::looks_secret(raw) {
                    String::new()
                } else {
                    clean(raw, 300)
                };
                Some(Tool { name, description })
            }).take(200).collect()
        })
        .unwrap_or_default()
}

fn receive_id(
    receiver: &mpsc::Receiver<serde_json::Value>,
    id: u64,
    timeout: Duration,
) -> Result<serde_json::Value, &'static str> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err("timeout");
        }
        let value = receiver.recv_timeout(left).map_err(|_| "server closed")?;
        if value.get("id").and_then(|value| value.as_u64()) == Some(id) {
            return Ok(value);
        }
    }
}

fn stdio_tools(
    command: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
) -> Result<Vec<Tool>, &'static str> {
    let mut process = Command::new(command);
    process.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        process.env(key, value);
    }
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        process.current_dir(cwd);
    }
    // Its own process group. An MCP server is routinely a launcher — `npx`
    // starting `node`, `uvx` starting python — and the grandchild inherits
    // these pipes. Killing only the child left it holding stdout open, so the
    // reader thread below joined on a pipe that would never close and this
    // whole refresh process hung, permanently, holding nothing anybody wanted.
    crate::exec::own_process_group(&mut process);
    let mut child = process.spawn().map_err(|_| "could not start server")?;
    let stdout = child.stdout.take().ok_or("no server output")?;
    let stderr = child.stderr.take();
    // Drain stderr without retaining it: startup errors can echo env values.
    let stderr_thread = stderr.map(|stderr| std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            if line.is_err() { break; }
        }
    }));
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str(&line) {
                if sender.send(value).is_err() { break; }
            }
        }
    });
    let result = (|| {
        let input = child.stdin.as_mut().ok_or("no server input")?;
        let initialize = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "prelude", "version": env!("CARGO_PKG_VERSION")}
            }
        });
        writeln!(input, "{initialize}").map_err(|_| "initialize write failed")?;
        input.flush().map_err(|_| "initialize write failed")?;
        let initialized = receive_id(&receiver, 1, Duration::from_secs(15))?;
        if initialized.get("error").is_some() {
            return Err("initialize rejected");
        }
        writeln!(input, "{}", serde_json::json!({
            "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
        })).map_err(|_| "initialized write failed")?;
        let mut id = 2u64;
        let mut cursor: Option<String> = None;
        let mut tools = Vec::new();
        loop {
            let params = cursor.as_ref().map_or_else(
                || serde_json::json!({}),
                |cursor| serde_json::json!({"cursor": cursor}),
            );
            writeln!(input, "{}", serde_json::json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/list", "params": params
            })).map_err(|_| "tools/list write failed")?;
            input.flush().map_err(|_| "tools/list write failed")?;
            let response = receive_id(&receiver, id, Duration::from_secs(20))?;
            if response.get("error").is_some() {
                return Err("tools/list rejected");
            }
            tools.extend(parse_tools_response(&response));
            cursor = response.get("result").and_then(|result| result.get("nextCursor"))
                .and_then(|cursor| cursor.as_str()).map(str::to_string);
            if cursor.is_none() || tools.len() >= 200 || id >= 11 {
                tools.truncate(200);
                return Ok(tools);
            }
            id += 1;
        }
    })();
    // The tree, not the process: everything holding these pipes has to go
    // before the reader threads can finish.
    crate::exec::kill_tree(child.id() as i32);
    let _ = child.kill();
    let _ = child.wait();
    drop(receiver);
    let _ = reader.join();
    if let Some(thread) = stderr_thread { let _ = thread.join(); }
    result
}

fn cached_item(
    agent: &str,
    name: &str,
    status: &str,
    checked_at: u64,
    tools: &[Tool],
    error: &str,
) -> Item {
    Item::new(format!("{agent}:{name}"), Kind::Mcp)
        .put("agent", agent)
        .put("name", name)
        .put("status", status)
        .put("checked_at", checked_at.to_string())
        .put("tools", serde_json::to_string(tools).unwrap_or_default())
        .put("error", error)
}

fn codex_inventory(into: &mut Vec<Item>, checked_at: u64) {
    if crate::exec::require("codex").is_none() {
        return;
    }
    // Same partition rule as the MCP inventory: an agent that could not be
    // asked keeps its cached tools rather than losing them to an empty list.
    let probe = crate::exec::capture(
        &["codex", "mcp", "list", "--json"],
        Duration::from_secs(20),
    );
    let text = probe.stdout_text();
    let Ok(servers) = serde_json::from_str::<Vec<serde_json::Value>>(&text) else {
        // A clean exit whose output will not parse is not "no servers". This
        // returned silently, so a malformed answer here — beside a claude
        // that answered — replaced every cached codex tool list with nothing.
        crate::exec::note_incomplete("codex");
        return;
    };
    let mut rows = Vec::new();
    let out = &mut rows;
    for server in servers {
        let name = server.get("name").and_then(|value| value.as_str()).unwrap_or("");
        if name.is_empty() || crate::secrets::looks_secret(name) {
            continue;
        }
        if !server.get("enabled").and_then(|value| value.as_bool()).unwrap_or(true) {
            out.push(cached_item("codex", name, "disabled", checked_at, &[], ""));
            continue;
        }
        let Some(transport) = server.get("transport") else { continue };
        if transport.get("type").and_then(|value| value.as_str()) != Some("stdio") {
            out.push(cached_item("codex", name, "unsupported", checked_at, &[], "http tools require owner authentication"));
            continue;
        }
        let Some(command) = transport.get("command").and_then(|value| value.as_str()) else {
            out.push(cached_item("codex", name, "failed", checked_at, &[], "missing stdio command"));
            continue;
        };
        let args: Vec<String> = transport.get("args").and_then(|value| value.as_array())
            .map(|args| args.iter().filter_map(|value| value.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let env: Vec<(String, String)> = transport.get("env").and_then(|value| value.as_object())
            .map(|env| env.iter().filter_map(|(key, value)| {
                value.as_str().map(|value| (key.clone(), value.to_string()))
            }).collect()).unwrap_or_default();
        let cwd = transport.get("cwd").and_then(|value| value.as_str());
        match stdio_tools(command, &args, &env, cwd) {
            Ok(tools) => out.push(cached_item("codex", name, "ok", checked_at, &tools, "")),
            Err(error) => out.push(cached_item("codex", name, "failed", checked_at, &[], error)),
        }
    }
    if crate::sources::agents::trusted(&probe, "codex", rows.len()) {
        into.extend(rows);
    }
}

fn claude_inventory(into: &mut Vec<Item>, checked_at: u64) {
    if crate::exec::require("claude").is_none() {
        return;
    }
    let probe = crate::exec::capture(&["claude", "mcp", "list"], Duration::from_secs(30));
    let text = probe.stdout_text();
    let mut rows = Vec::new();
    let out = &mut rows;
    for line in text.lines() {
        let line = line.trim();
        let Some((name, rest)) = line.split_once(": ") else { continue };
        if name.is_empty() || name.starts_with("claude.ai ") || crate::secrets::looks_secret(name) {
            if !name.is_empty() && name.starts_with("claude.ai ") {
                out.push(cached_item("claude", name, "unsupported", checked_at, &[], "owner-account tools are not exposed by the CLI"));
            }
            continue;
        }
        if rest.starts_with("http://") || rest.starts_with("https://") {
            out.push(cached_item("claude", name, "unsupported", checked_at, &[], "http tools require owner authentication"));
            continue;
        }
        let detail = crate::exec::run(&["claude", "mcp", "get", name], Duration::from_secs(20));
        let Some(crate::lend::Mcp::Stdio { command, args, env, .. }) =
            crate::lend::Mcp::from_claude_get(name, &detail)
        else {
            out.push(cached_item("claude", name, "unsupported", checked_at, &[], "no transferable stdio definition"));
            continue;
        };
        match stdio_tools(&command, &args, &env, None) {
            Ok(tools) => out.push(cached_item("claude", name, "ok", checked_at, &tools, "")),
            Err(error) => out.push(cached_item("claude", name, "failed", checked_at, &[], error)),
        }
    }
    if crate::sources::agents::trusted(&probe, "claude", rows.len()) {
        into.extend(rows);
    }
}

pub fn inventory() -> Vec<Item> {
    let checked_at = crate::frecency::now() as u64;
    let mut out = Vec::new();
    claude_inventory(&mut out, checked_at);
    codex_inventory(&mut out, checked_at);
    out
}

pub fn attach_cached(items: &mut [Item]) {
    let cached = crate::cache::read_cached("mcp-tools");
    for item in items {
        let found = cached.iter().find(|tools| {
            tools.get("agent") == item.get("agent") && tools.get("name") == item.get("name")
        });
        match found {
            Some(tools) => {
                item.data.insert("tools_status".into(), tools.get("status").to_string());
                item.data.insert("tools_checked_at".into(), tools.get("checked_at").to_string());
                item.data.insert("tools".into(), tools.get("tools").to_string());
                item.data.insert("tools_error".into(), tools.get("error").to_string());
            }
            None => {
                item.data.insert("tools_status".into(), "pending".into());
            }
        }
    }
}
