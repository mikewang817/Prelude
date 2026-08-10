//! Keeping the panel's list alive while nobody is looking at it.
//!
//! The global panel is a hidden Ghostty hosting one long-lived `prelude
//! _panel` loop, and `panel::once` starts the launcher *before* the press, not
//! after it — that is the whole point, and why a press reveals rather than
//! builds. But it means `gather` runs when the previous interaction ends, and
//! the list you are shown was assembled then. Dismiss at nine, press at two,
//! and you are reading the morning's machine: the clipping you copied a minute
//! ago is not in it, nor is the session you just started, and neither will be
//! until you dismiss and re-open once. The symptom people report is "it takes
//! a while to show up", which is not quite it — nothing was slow, the snapshot
//! simply predates what they are looking for.
//!
//! So a thread re-gathers behind the hidden panel and hands fzf the new list
//! through its own `--listen` socket. Three rules keep that from being felt:
//!
//! * **It never touches a panel in use.** The refresh is a `transform` sent
//!   over the socket, and a transform is evaluated by fzf with the live `{q}`
//!   and `{n}` — so `_tick` can see a typed query or a moved cursor and answer
//!   with no action at all. A background reload that moved somebody's
//!   selection would be worse than the staleness it fixes.
//! * **It does no work when nothing has changed.** A tick is a handful of
//!   `stat`s; a gather happens only when one of the files behind the list has
//!   a new mtime, or when the full interval has elapsed.
//! * **It reuses the layout it was given.** Widths and the title column are
//!   computed once and passed down — recomputing them here would land the
//!   per-keystroke helper's rows in a different column from the static ones.
//!
//! Everything here degrades to nothing. No socket, a failed bind, a POST that
//! goes nowhere: the panel behaves exactly as it did before, one snapshot per
//! interaction.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How often the thread wakes to look. Cheap enough to be frequent, because
/// the case that matters is "copy something, press the chord" — a minute-long
/// interval would answer a question nobody was still asking.
const TICK: Duration = Duration::from_secs(3);

/// A gather happens on a changed input, or this long after the last one
/// regardless — the mtime probe cannot see everything, and a slow source
/// refreshed detached lands in its cache without anyone telling us.
const FORCE_AFTER: Duration = Duration::from_secs(30);

/// `sun_path` is 104 bytes on macOS, and a socket that cannot be bound is
/// better skipped than half-configured.
const MAX_SOCKET_PATH: usize = 100;

/// Where fzf should listen, or `None` when this is not the panel.
///
/// Only the global panel gets this. The zsh widget is opened, used and closed
/// inside a few seconds; its snapshot is never older than the press that made
/// it, and a background thread there would be pure cost.
pub fn listen_socket() -> Option<PathBuf> {
    if !crate::ui::env_flag("PRELUDE_FULL_SURFACE") {
        return None;
    }
    let path = crate::paths::data().join("panel.sock");
    if path.to_string_lossy().len() > MAX_SOCKET_PATH {
        return None;
    }
    // A socket file left by a panel that died refuses the next bind.
    let _ = std::fs::remove_file(&path);
    Some(path)
}

/// Watch the inputs behind the list, and hand fzf a new one when they move.
pub fn keep_current(socket: PathBuf, widths: Vec<usize>, tw: usize, cols: usize) {
    std::thread::spawn(move || {
        let me = std::env::current_exe().unwrap_or_default().to_string_lossy().into_owned();
        let mut last_seen = fingerprint();
        // `Instant`, not `SystemTime`. This is a *duration since*, and the
        // wall clock is not one: NTP stepping the clock backwards — which
        // happens on any laptop that has been asleep — makes `elapsed` return
        // an error or a smaller number than it should, and the forced gather
        // that exists precisely to catch what the mtime probe cannot see gets
        // postponed for as long as the correction was large.
        let mut last_gather = std::time::Instant::now();
        loop {
            std::thread::sleep(TICK);
            // `^K` is a modal: fzf exits, the action panel runs, and the list
            // is rebuilt when it returns. The socket is gone for that whole
            // time and comes back — so its absence is a reason to skip a tick,
            // never a reason to stop. Ending here left every session that had
            // opened the action panel once with no live refresh at all, which
            // is both a silent loss and the hardest kind to notice. The thread
            // ends when the process does.
            if !socket.exists() {
                continue;
            }
            let now = fingerprint();
            let due = last_gather.elapsed() >= FORCE_AFTER;
            if now == last_seen && !due {
                continue;
            }
            // Ask before spending anything. Somebody in the middle of typing
            // is not going to accept a reload, and gathering for them anyway
            // is the whole cost of this thread paid for a result that will be
            // thrown away — every three seconds, for as long as they type.
            //
            // Nothing is recorded when the answer is no: `last_seen` keeps the
            // old fingerprint, so this stays due and lands on the first tick
            // after they stop. Marking it seen here was the bug this replaces
            // — the change was consumed by a refresh that never happened.
            if !panel_is_idle(&socket) {
                continue;
            }
            rebuild(&widths, tw, cols);
            // `_tick` still makes the final call, and has to: the person can
            // start typing during the gather, and only fzf can answer that
            // question at the instant the reload would take effect.
            let posted = post(
                &socket,
                &format!("transform:{} _tick {{q}} {{n}}", crate::exec::shq(&me)),
            );
            if posted.is_err() {
                // The panel went away mid-refresh — `^K` is a modal that takes
                // the socket with it. Leave the fingerprint unrecorded so the
                // next tick tries again rather than treating a POST into the
                // void as a delivered refresh.
                continue;
            }
            last_seen = now;
            last_gather = std::time::Instant::now();
        }
    });
}

/// Re-render the three files the launcher reads: the root list fzf reloads
/// from, the home list an empty query shows, and the catalogue the scopes
/// parse.
fn rebuild(widths: &[usize], tw: usize, cols: usize) {
    let items = crate::cache::gather();
    if items.is_empty() {
        return;
    }
    let root = crate::compute::root_items(&items);
    let home = crate::compute::home_items(&items);
    let root_feed = crate::render::render_with(&root, cols, Some(widths), Some(tw));
    let home_feed = crate::render::render_with(&home, cols, Some(widths), Some(tw));
    let cache = crate::paths::cache();
    let mut snapshot = vec![
        (cache.join("list.txt"), root_feed.as_bytes().to_vec()),
        (cache.join("home.txt"), home_feed.as_bytes().to_vec()),
    ];
    if let Ok(json) = serde_json::to_vec(&items) {
        snapshot.push((cache.join("search-items.json"), json));
    }
    crate::cache::write_group(&snapshot);
}

/// What the list is made of, as one number.
///
/// Modification times only — no reads, no parsing, no subprocess. This runs
/// every `TICK` forever on a laptop, so it has to cost nothing when the answer
/// is "nothing happened", which is almost always.
fn fingerprint() -> u64 {
    let data = crate::paths::data();
    let cache = crate::paths::cache();
    let config = crate::paths::config();
    let watched: [PathBuf; 11] = [
        // What a person changes and expects to see immediately.
        data.join("clipboard.jsonl"),
        config.join("quicklinks.toml"),
        // A question an agent is blocked on is the most urgent row there is.
        data.join("bus"),
        // The slow tier, refreshed detached: its caches land without anyone
        // being told, which is the other half of "one launch behind".
        cache.join("sessions.json"),
        cache.join("mcp.json"),
        cache.join("fleet.json"),
        cache.join("ports.json"),
        cache.join("procs.json"),
        cache.join("dirs.json"),
        data.join("sessions.json"),
        data.join("capabilities.json"),
    ];
    let mut sum: u64 = 0;
    for (i, path) in watched.iter().enumerate() {
        sum = sum.wrapping_mul(31).wrapping_add(stamp(path).wrapping_add(i as u64));
    }
    sum
}

fn stamp(path: &Path) -> u64 {
    let Ok(meta) = path.metadata() else { return 0 };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    modified.wrapping_mul(31).wrapping_add(meta.len())
}

/// fzf's HTTP server, over its Unix socket. Hand-rolled because the request is
/// four lines and a body, and a dependency on the launch path is paid at every
/// startup — see the note on `web_url`.
fn post(socket: &Path, action: &str) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let request = format!(
        "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{action}",
        action.len()
    );
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    let mut answer = [0u8; 64];
    let _ = stream.read(&mut answer);
    Ok(())
}

/// Ask fzf what it is showing, before deciding to spend a gather on it.
///
/// The order used to be the other way round: rebuild — the most expensive
/// thing this thread does, a full gather plus two renders plus four hundred
/// kilobytes of catalogue — and *then* ask `_tick` whether the panel could be
/// touched. When somebody was typing, the answer was no, and all of that work
/// had already happened. It happened every `TICK` for as long as they kept
/// typing.
///
/// fzf's listen socket answers `GET /` with its live state, so the question
/// can be asked first for the price of one connection on a Unix socket.
///
/// `None` means the question could not be asked — no socket, an older fzf, a
/// reply in a shape this does not recognise — and the caller then behaves
/// exactly as it did before rather than refusing to refresh. Everything here
/// degrades to nothing.
fn panel_state(socket: &Path) -> Option<(String, i64)> {
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .ok()?;
    stream.flush().ok()?;
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    parse_state(&String::from_utf8_lossy(&answer))
}

/// The query and cursor row out of fzf's state reply, or nothing if this is
/// not a reply we understand.
fn parse_state(reply: &str) -> Option<(String, i64)> {
    let body = reply.split("\r\n\r\n").nth(1)?;
    let state: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    Some((
        state.get("query")?.as_str()?.to_string(),
        state.get("position")?.as_i64()?,
    ))
}

/// Is the panel sitting untouched, as a hidden one waiting to be revealed is?
///
/// Unknown is *busy*. A refresh skipped costs a person nothing they can see;
/// a reload under their cursor moves their selection, which is worse than the
/// staleness it was fixing.
fn panel_is_idle(socket: &Path) -> bool {
    match panel_state(socket) {
        Some((query, position)) => query.is_empty() && position == 0,
        // The question could not be asked. Fall through to the old order and
        // let `_tick` make the call, which it still does either way.
        None => true,
    }
}

/// May the panel be redrawn under the person right now?
///
/// Only when they have not begun using it: no query typed, and the cursor
/// still on the row fzf drew first. A hidden panel waiting to be revealed is
/// in exactly that state; a panel somebody is reading is not, and it is left
/// alone until they dismiss it, at which point the next launcher gathers
/// anyway.
///
/// Nothing is the safe answer, and it is the answer to anything unexpected —
/// an index fzf phrased in a way this does not recognise included.
pub(crate) fn may_redraw(query: &str, index: &str) -> bool {
    query.is_empty() && matches!(index.trim(), "" | "0")
}

/// Prints an fzf action, or nothing at all.
pub fn tick(query: &str, index: &str) -> i32 {
    if !may_redraw(query, index) {
        return 0;
    }
    let me = std::env::current_exe().unwrap_or_default();
    let me = crate::exec::shq(&me.to_string_lossy());
    let home = crate::paths::cache().join("home.txt");
    println!("reload({me} _dynamic '' {})", crate::exec::shq(&home.to_string_lossy()));
    0
}

#[cfg(test)]
mod tests {
    use super::parse_state;

    /// The panel is asked what it is showing *before* a gather is spent on
    /// it. This is the reply fzf 0.74 actually sends, captured from a live
    /// socket — the whole point of asking is lost if the answer is misread.
    #[test]
    fn fzfs_state_reply_is_read_for_the_query_and_the_cursor() {
        let busy = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 223\r\n\r\n\
            {\"reading\":false,\"progress\":100,\"query\":\"tw\",\"position\":0,\"sort\":true,\
            \"totalCount\":3,\"matchCount\":1,\"current\":{\"index\":1,\"text\":\"two\"}}\n";
        assert_eq!(parse_state(busy), Some(("tw".to_string(), 0)));

        let idle = "HTTP/1.1 200 OK\r\n\r\n{\"query\":\"\",\"position\":0}";
        assert_eq!(parse_state(idle), Some((String::new(), 0)));

        let moved = "HTTP/1.1 200 OK\r\n\r\n{\"query\":\"\",\"position\":4}";
        assert_eq!(parse_state(moved), Some((String::new(), 4)));

        // Anything else is "could not ask", which falls back to the old
        // order rather than refusing to refresh for the rest of the session.
        for unknown in [
            "",
            "HTTP/1.1 404 Not Found\r\n\r\n",
            "HTTP/1.1 200 OK\r\n\r\nnot json",
            "HTTP/1.1 200 OK\r\n\r\n{\"query\":\"\"}",
            "{\"query\":\"\",\"position\":0}",
        ] {
            assert_eq!(parse_state(unknown), None, "{unknown:?}");
        }
    }
}
