//! Quick Look for the selected item.
//!
//! 90% of titles are under 17 columns, so a wide terminal left most of the
//! panel empty while Chinese skill descriptions were cut at eight characters.
//! Narrowing the title column is not the answer — fzf matches against
//! *displayed* text, so truncating titles would stop you finding a long
//! command by its tail. The space goes to detail instead.

use crate::ansi::*;
use crate::item::{Item, Kind};
use crate::paths::tilde;
use std::io::Write;

/// How many of a Task's events the panel renders.
///
/// The tail, because that is where a task's story ends and the reason anybody
/// opened this is what happened last. Generous enough to hold a whole ordinary
/// task — opened, started, a few notes, finished — and small enough that a
/// task with hundreds of retries costs the same as one with four.
const HISTORY_SHOWN: usize = 40;

pub fn show(it: &Item) {
    if image_path(it).is_some_and(show_image) {
        return;
    }
    println!("{}", text(it));
}

fn image_path(it: &Item) -> Option<&str> {
    let path = match it.kind {
        Kind::File | Kind::Find => it.get("path"),
        Kind::Clip if matches!(it.get("clip_kind"), "image" | "files") => it.get("path"),
        _ => return None,
    };
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic"
            | "heif" | "avif" | "svg"
    )
    .then_some(path)
}

/// Render an image inside fzf's Quick Look area. Chafa gives the best result
/// and handles animation and scaling; when it is not installed, Ghostty/
/// Kitty and iTerm still get their native inline-image protocol. Every other
/// terminal falls back to the ordinary path and metadata view.
fn show_image(path: &str) -> bool {
    if !std::path::Path::new(path).is_file() {
        return false;
    }
    let cols = std::env::var("FZF_PREVIEW_COLUMNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(80)
        .saturating_sub(2)
        .max(1);
    let rows = std::env::var("FZF_PREVIEW_LINES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(24)
        .saturating_sub(2)
        .max(1);
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default().to_ascii_lowercase();
    let term = std::env::var("TERM").unwrap_or_default().to_ascii_lowercase();

    if crate::exec::which("chafa").is_some() {
        let format = if program.contains("iterm") {
            "iterm"
        } else if program.contains("ghostty") || term.contains("kitty") {
            "kitty"
        } else {
            "symbols"
        };
        if std::process::Command::new("chafa")
            .args([
                "--animate=off",
                "--exact-size=off",
                "--format", format,
                "--size", &format!("{cols}x{rows}"),
                path,
            ])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return true;
        }
    }

    if program.contains("ghostty") || term.contains("kitty") {
        let encoded = base64(path.as_bytes());
        print!("\x1b_Ga=T,f=100,t=f,q=2,C=1,c={cols},r={rows};{encoded}\x1b\\");
        let _ = std::io::stdout().flush();
        return true;
    }
    if program.contains("iterm") {
        if let Ok(bytes) = std::fs::read(path) {
            print!(
                "\x1b]1337;File=inline=1;width={cols};height={rows};preserveAspectRatio=1:{}\x07",
                base64(&bytes)
            );
            let _ = std::io::stdout().flush();
            return true;
        }
    }
    false
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = *chunk.get(1).unwrap_or(&0);
        let c = *chunk.get(2).unwrap_or(&0);
        out.push(ALPHABET[(a >> 2) as usize] as char);
        out.push(ALPHABET[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 { ALPHABET[(c & 63) as usize] as char } else { '=' });
    }
    out
}

/// Where a finished task stands with the person it was done for.
///
/// Three answers, and the empty one matters as much as the others: a task that
/// is still running has nothing to be dismissed yet, and a label with "not
/// applicable" after it is a line nobody needed. A finished task past its
/// review window has stopped appearing on the home by itself, so saying it is
/// still there would be a lie — it is simply undismissed history.
///
/// `Done | Failed`, not `finished()`. A cancelled task never asked for
/// attention in the first place — `task::awaiting_review` excludes it because
/// cancelling *is* the decision — so "dismissed · not yet" against one was the
/// panel reporting that a question nobody had been asked was still
/// outstanding. The action panel gates `Dismiss it` on the same two states.
fn dismissable(task: &crate::task::Task) -> bool {
    matches!(task.state, crate::task::State::Done | crate::task::State::Failed)
}

fn dismissal(task: &crate::task::Task) -> String {
    let now = crate::bus::now();
    match task.acked_at {
        Some(ts) => crate::sources::user::ago(ts as f64),
        None if crate::task::awaiting_review(task, now) => "not yet · still on the home".into(),
        None if dismissable(task) => "not yet".into(),
        None => String::new(),
    }
}

/// Render the same detail view as text so the action panel can page it and
/// then return to the actions instead of closing the launcher.
pub fn text(it: &Item) -> String {
    let (color, label) = it.kind.style();
    let mut out = vec![format!("{color}{label}{RESET}"), String::new()];
    fn kv(out: &mut Vec<String>, k: &str, v: &str) {
        if !v.is_empty() {
            out.push(format!("{DIM}{k:<16}{RESET}{v}"));
        }
    }
    /// A block of label/value pairs whose labels do not all fit the fixed
    /// sixteen columns above — "installed for this agent" is twenty-four.
    ///
    /// The column is measured once over the whole block and passed to every
    /// row, for the same reason the list's columns are: two rows that each
    /// measure themselves land in different places. Empty values are dropped
    /// rather than printed as a label with nothing after it — "nothing says
    /// so" is not a fact worth a line.
    fn kv_block(out: &mut Vec<String>, pairs: &[(&'static str, String)]) {
        let width = pairs
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(label, _)| crate::width::dwidth(label))
            .max()
            .unwrap_or(0)
            + 2;
        for (label, value) in pairs {
            if value.is_empty() {
                continue;
            }
            out.push(format!("{DIM}{}{RESET}{value}", crate::width::pad_to(label, width, false)));
        }
    }

    match it.kind {
        // A question, in full, plus the conversation it came out of. The row
        // has to fit the question on one line; this does not, and a decision
        // you are being asked to make deserves the whole sentence and the
        // few exchanges that led to it.
        Kind::Msg => {
            kv(&mut out, "from", it.get("agent"));
            kv(&mut out, "project", &tilde(it.get("cwd")));
            kv(&mut out, "at", it.get("pane"));
            out.push(String::new());
            out.extend(it.get("text").lines().map(str::to_string));
            let pane = it.get("pane");
            if !pane.is_empty() {
                let screen = crate::exec::run(
                    &["tmux", "capture-pane", "-p", "-t", pane, "-S", "-30"],
                    std::time::Duration::from_secs(2),
                );
                let screen = screen.trim_end();
                if !screen.is_empty() {
                    out.push(String::new());
                    out.push(format!("{DIM}what it was doing{RESET}"));
                    out.extend(screen.lines().map(str::to_string));
                }
            }
        }
        // The one preview that answers the question you actually had. A row
        // saying "waiting 12m" tells you something is stuck; only its screen
        // tells you what it asked. One capture for the selected pane, not
        // for all eighty of them.
        Kind::Run => {
            // One source for the effective context, shared with `prelude
            // fleet` and the control table. This view used to build its own
            // key/value block, which is how two readings of one run start
            // disagreeing about which session it is in.
            let mut pairs = crate::sources::running::effective_context(it);
            // The three identifiers the shared context leaves out, because
            // they are addresses rather than facts about the work — and this
            // is the view somebody reaches for in order to go there. They sit
            // before the last thing the run said, which is always the final
            // pair and reads as the end of the block.
            let before_last = pairs.len().saturating_sub(1);
            for (n, pair) in [
                ("at", it.get("addr").to_string()),
                ("pid", it.get("pid").to_string()),
                ("run", it.get("run_id").to_string()),
            ]
            .into_iter()
            .enumerate()
            {
                pairs.insert(before_last + n, pair);
            }
            kv_block(&mut out, &pairs);
            // What it last said, from its conversation file — which exists
            // whether or not it is in a terminal Prelude can see into. The
            // pane's screen is better when there is one, since it shows the
            // half-finished line too.
            let pane = it.get("pane");
            let screen = if pane.is_empty() {
                String::new()
            } else {
                crate::exec::run(
                    &["tmux", "capture-pane", "-p", "-t", pane, "-S", "-40"],
                    std::time::Duration::from_secs(2),
                )
            };
            let screen = screen.trim_end();
            if !screen.is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}its screen{RESET}"));
                out.extend(screen.lines().map(str::to_string));
            } else {
                let tail = crate::sources::running::transcript_tail(it.get("session"), 8);
                if !tail.is_empty() {
                    out.push(String::new());
                    out.push(format!("{DIM}what it last said{RESET}"));
                    out.extend(tail);
                }
            }
        }
        // The record, and then everything anybody said about it. A task row
        // carries only what a list column can hold; this is where the
        // dependencies, the timestamps, the result and the edges live — and
        // the store is the authority for all of them, so it is read here
        // rather than reconstructed from the row.
        Kind::Task => {
            let id = it.get("task_id");
            let record = crate::task::get(id);
            let ago = |ts: u64| crate::sources::user::ago(ts as f64);
            match &record {
                Some(task) => {
                    // Which dependencies are still in the way is the question;
                    // listing the ids and leaving the reader to look each one
                    // up is the row's answer, not this one's.
                    //
                    // One `get` per dependency, not two. Quick Look is bound
                    // to a preview window that re-runs on every keystroke and
                    // every cursor move, and this used to read each record
                    // once for its label and once again to decide whether it
                    // counted as unfinished — two hundred dependencies were
                    // four hundred file opens, per key.
                    let mut states: Vec<(&str, Option<crate::task::State>)> =
                        Vec::with_capacity(task.deps.len());
                    let mut known: std::collections::HashMap<&str, Option<crate::task::State>> =
                        std::collections::HashMap::new();
                    for dep in &task.deps {
                        let state = *known
                            .entry(dep.as_str())
                            .or_insert_with(|| crate::task::get(dep).map(|other| other.state));
                        states.push((dep.as_str(), state));
                    }
                    let deps: Vec<String> = states
                        .iter()
                        .map(|(dep, state)| match state {
                            Some(state) if state.finished() => format!("{dep} · {}", state.as_str()),
                            Some(state) => format!("{dep} · {} · still to do", state.as_str()),
                            None => format!("{dep} · no such task · still to do"),
                        })
                        .collect();
                    let unfinished = states
                        .iter()
                        .filter(|(_, state)| !state.is_some_and(|state| state.finished()))
                        .count();
                    let needs = match (deps.is_empty(), unfinished) {
                        (true, _) => String::new(),
                        (false, 0) => deps.join("\n"),
                        (false, n) => format!("{n} still unfinished\n{}", deps.join("\n")),
                    };
                    kv_block(&mut out, &[
                        ("title", task.title.clone()),
                        ("id", task.id.clone()),
                        ("state", task.state.as_str().to_string()),
                        ("project", task.project.clone()),
                        ("where", tilde(&task.cwd)),
                        ("agent", task.agent.clone()),
                        ("run", task.run.clone().unwrap_or_default()),
                        ("session", task.session.clone().unwrap_or_default()),
                        ("message", task.message.clone().unwrap_or_default()),
                        ("prompt", task.prompt_ref.clone().unwrap_or_default()),
                        ("retry of", task.retry_of.clone().unwrap_or_default()),
                        ("opened", ago(task.created_at)),
                        ("started", task.started_at.map(ago).unwrap_or_default()),
                        ("last touched", ago(task.updated_at)),
                        ("finished", task.finished_at.map(ago).unwrap_or_default()),
                        // Whether anybody has said they have seen how it went.
                        // Acknowledgement is an explicit act, never inferred
                        // from this very panel having been opened — so the
                        // line reads the same however long you look at it.
                        ("dismissed", dismissal(task)),
                        ("result", task.result.clone().unwrap_or_default()),
                        ("reason", task.reason.clone().unwrap_or_default()),
                    ]);
                    if !needs.is_empty() {
                        out.push(String::new());
                        out.push(format!("{DIM}needs{RESET}"));
                        out.extend(needs.lines().map(str::to_string));
                    }
                }
                // The store is the authority, so a row whose record has been
                // swept says exactly that rather than re-displaying the row.
                None => {
                    kv_block(&mut out, &[
                        ("title", it.title.clone()),
                        ("id", id.to_string()),
                        ("state", it.get("state").to_string()),
                        ("project", it.get("project").to_string()),
                        ("agent", it.get("agent").to_string()),
                    ]);
                    out.push(String::new());
                    out.push("no record under that id — it has been finished and swept".into());
                }
            }
            // Newest last, the way a conversation reads, and the way every
            // other list in this file is ordered — and bounded, the way
            // `transcript_tail` bounds a conversation. A long-running task
            // accumulates events without limit, and this is a preview: it is
            // re-rendered on every keystroke while Quick Look is open, and
            // nobody scrolls back through five hundred lines of it to reach
            // the record above.
            let mut history = crate::events::for_task(id);
            let total = history.len();
            if total > HISTORY_SHOWN {
                history.drain(..total - HISTORY_SHOWN);
            }
            if !history.is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}history{RESET}"));
                if total > history.len() {
                    out.push(format!(
                        "{DIM}{} earlier events · prelude task show {id} --json has them all{RESET}",
                        total - history.len(),
                    ));
                }
                for event in history {
                    let detail = event.detail.unwrap_or_default();
                    out.push(format!(
                        "{} {}{}",
                        crate::width::pad_to(&crate::sources::user::ago(event.ts as f64), 12, false),
                        event.kind,
                        if detail.is_empty() { String::new() } else { format!("  {detail}") },
                    ));
                }
            }
        }
        Kind::Skill => {
            kv(&mut out, "agents", it.get("agent"));
            kv(&mut out, "integrity", it.get("integrity"));
            kv(&mut out, "fingerprint", it.get("fingerprint"));
            kv(&mut out, "file", &tilde(it.get("file")));
            let copies = crate::capability::copies(it);
            if !copies.is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}installed copies{RESET}"));
                for copy in copies {
                    let hash = copy.fingerprint.strip_prefix("fnv1a64-v1:").unwrap_or(&copy.fingerprint);
                    let warning = if copy.sensitive_files > 0 {
                        format!(" · {} sensitive file(s) redacted", copy.sensitive_files)
                    } else if copy.unreadable > 0 {
                        format!(" · {} unreadable", copy.unreadable)
                    } else {
                        String::new()
                    };
                    // When each copy was last touched. Two copies with
                    // different fingerprints say they have diverged; only the
                    // dates say which way round to replace them.
                    let when = match crate::sources::user::ago(copy.modified as f64) {
                        when if when.is_empty() => String::new(),
                        when => format!(" · {when}"),
                    };
                    out.push(format!(
                        "{} {} · {} files · {} bytes{when}{warning}",
                        crate::width::pad_to(&copy.agent, 10, false), hash, copy.files, copy.bytes,
                    ));
                    out.push(format!("           {}", tilde(&copy.dir)));
                    // A broken link is a copy that will not work and an
                    // escaping one is a copy that cannot be copied
                    // completely; both are worth a line where a hash is not.
                    // Worded by `capability::link_faults`, which is also what
                    // `doctor skills` will say — two descriptions of one
                    // condition eventually disagree about it.
                    for fault in crate::capability::link_faults(&copy) {
                        out.push(format!("           {RED}⚠ {}{RESET}", fault.detail));
                    }
                }
            }
            if !it.get("desc").is_empty() {
                out.push(String::new());
                out.push(it.get("desc").to_string());
            }
        }
        Kind::Mcp => {
            kv(&mut out, "agent", it.get("agent"));
            kv(&mut out, "name", it.get("name"));
            kv(&mut out, "config", &tilde(it.get("config")));
            kv(&mut out, "transport", it.get("transport"));
            let health_at = it.get("health_checked_at").parse::<f64>().unwrap_or(0.0);
            if health_at > 0.0 {
                kv(&mut out, "health checked", &crate::sources::user::ago(health_at));
            }
            kv(&mut out, "tools", it.get("tools_status"));
            let tools_at = it.get("tools_checked_at").parse::<f64>().unwrap_or(0.0);
            if tools_at > 0.0 {
                kv(&mut out, "tools checked", &crate::sources::user::ago(tools_at));
            }
            kv(&mut out, "comparison", it.get("comparison"));
            let variants = crate::capability::mcp_variants(it);
            let variant_count = variants.len();
            if !variants.is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}availability{RESET}"));
                for variant in variants {
                    let hash = variant.fingerprint.strip_prefix("fnv1a64-v1:")
                        .unwrap_or(&variant.fingerprint);
                    let identity = if variant.summary.is_empty() {
                        hash.to_string()
                    } else {
                        format!("{} · {hash}", variant.summary)
                    };
                    out.push(format!(
                        "{:<10} {:<10} {}{}",
                        variant.agent,
                        variant.health,
                        identity,
                        if !variant.portable {
                            " · owner-account only"
                        } else if variant.sensitive {
                            " · private fields omitted"
                        } else {
                            ""
                        },
                    ));
                    if !variant.tools.is_empty() {
                        out.push(format!("           {DIM}tools · checked {}{RESET}",
                            crate::sources::user::ago(variant.tools_checked_at as f64)));
                        for tool in variant.tools {
                            let detail = if tool.description.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", tool.description)
                            };
                            out.push(format!("             {}{detail}", tool.name));
                        }
                    } else if !variant.tools_status.is_empty() {
                        out.push(format!("           {DIM}tools · {}{RESET}", variant.tools_status));
                    }
                }
                if variant_count > 1 {
                    out.push(String::new());
                    out.push(format!("{DIM}public definition diff{RESET}"));
                    out.extend(crate::capability::mcp_definition_diff(it));
                }
            }
        }
        Kind::Proc => {
            kv(&mut out, "pid", it.get("pid"));
            kv(&mut out, "cpu", &format!("{}%", it.get("cpu")));
            kv(&mut out, "memory", it.get("mem"));
            if !it.get("cmd").is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}command{RESET}"));
                out.push(it.get("cmd").to_string());
            }
        }
        Kind::Port => {
            kv(&mut out, "port", it.get("port"));
            kv(&mut out, "process", it.get("proc"));
            kv(&mut out, "pid", it.get("pid"));
        }
        Kind::File | Kind::Find => {
            let p = it.get("path");
            kv(&mut out, "path", &tilde(p));
            if let Ok(m) = std::fs::metadata(p) {
                kv(&mut out, "size", &format!("{} bytes", group(m.len())));
            }
            if let Ok(text) = std::fs::read_to_string(p) {
                out.push(String::new());
                out.push(format!("{DIM}head{RESET}"));
                out.extend(text.lines().take(20).map(str::to_string));
            }
        }
        Kind::Clip => out.push(it.get("full").to_string()),
        Kind::App => kv(&mut out, "path", &tilde(it.get("path"))),
        Kind::Link => kv(&mut out, "url", it.get("url")),
        Kind::Search => {
            kv(&mut out, "type", it.get("completion"));
            kv(&mut out, "about", it.get("desc"));
        },
        Kind::Agent => {
            for (k, v) in [("skills", 0), ("mcp", 1), ("sessions", 2)] {
                kv(&mut out, k, it.fields.get(v).map(String::as_str).unwrap_or(""));
            }
            let runs = it.get("run_count").parse::<usize>().unwrap_or(0);
            let waiting = it.get("waiting_count").parse::<usize>().unwrap_or(0);
            if runs > 0 {
                kv(
                    &mut out,
                    "running",
                    &format!("{runs}{}", if waiting > 0 { format!(" · {waiting} waiting") } else { String::new() }),
                );
                let projects: Vec<String> = serde_json::from_str(it.get("projects")).unwrap_or_default();
                kv(&mut out, "projects", &projects.join(", "));
            }
            out.push(String::new());
            out.push(format!("{DIM}start it{RESET}"));
            out.push(it.get("agent").to_string());
        }
        Kind::Session => {
            kv(&mut out, "agent", it.get("agent"));
            kv(&mut out, "id", it.get("id"));
            let status = [
                (it.get("pinned") == "true").then_some("pinned"),
                (it.get("archived") == "true").then_some("archived"),
            ].into_iter().flatten().collect::<Vec<_>>().join(" · ");
            kv(&mut out, "status", &status);
            kv(&mut out, "native title", it.get("native_title"));
            kv(&mut out, "where", &tilde(it.get("cwd")));
            kv(&mut out, "file", &tilde(it.get("file")));
            if !it.get("active_run").is_empty() {
                kv(&mut out, "active", it.get("active_state"));
                kv(&mut out, "run", it.get("active_run"));
                kv(&mut out, "at", it.get("active_addr"));
                // Archiving hides a conversation; it does not close it. A row
                // that says "archived" and is visible anyway has a reason,
                // and the reason belongs here rather than in a bug report.
                if it.get("archived") == "true" {
                    out.push(format!(
                        "{DIM}{:<16}{RESET}{}",
                        "note",
                        "archived, but something resumed it — it is listed again until that run ends",
                    ));
                }
            }
            if !it.get("opening").is_empty() {
                out.push(String::new());
                out.push(format!("{DIM}opened with{RESET}"));
                out.push(it.get("opening").to_string());
            }
        }
        Kind::Config => {
            kv(&mut out, "agent", it.get("agent"));
            let p = it.get("path");
            kv(&mut out, "path", &tilde(p));
            if let Ok(text) = std::fs::read_to_string(p) {
                out.push(String::new());
                out.push(format!("{DIM}head{RESET}"));
                out.extend(text.lines().take(24).map(str::to_string));
            }
        }
        Kind::Snippet => {
            kv(&mut out, "name", it.get("name"));
            out.push(String::new());
            out.push(crate::ui::fill_placeholders(&it.cmd));
        }
        Kind::Container => {
            kv(&mut out, "name", it.get("name"));
            kv(&mut out, "image", it.get("image"));
        }
        Kind::Ssh => kv(&mut out, "host", it.get("host")),
        Kind::Dir | Kind::Script | Kind::Git | Kind::History | Kind::Path
        | Kind::Sys | Kind::Calc | Kind::Translate => {
            if !it.subtitle.is_empty() {
                out.push(it.subtitle.clone());
                out.push(String::new());
            }
            if let Some(cwd) = &it.cwd {
                kv(&mut out, "in", &tilde(cwd));
            }
        }
    }
    if !matches!(it.kind, Kind::Clip | Kind::Link | Kind::Search) {
        out.push(String::new());
        out.push(format!("{DIM}runs{RESET}"));
        out.push(it.cmd.clone());
    }
    out.join("\n")
}

fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
