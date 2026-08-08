//! What an agent *said* it was doing, in order, in its own words.
//!
//! `running.rs` works the state out from the outside: a session file that has
//! not grown for three minutes is probably waiting on somebody. That
//! inference is the best available when nothing else speaks, and it is wrong
//! in the expensive direction — an agent halfway through a build is exactly
//! as quiet as one that has stopped to ask a question, and a badge that cries
//! wolf is worth less than no badge.
//!
//! This is the channel that removes the guess. `prelude task progress …` is
//! four words an agent runs from its own shell, and what comes out the other
//! end is a statement rather than a clock reading. That is why the plan puts
//! a structured Agent event above conversation evidence and above process
//! clocks, and `running::latest_events` is where a Run view collects it —
//! once for the whole fleet, from the tail, rather than once per run.
//!
//! **One file, appended to, never rewritten in place.** Several agents share
//! it and none of them locks it to append, so every record is serialized in
//! full — the trailing newline included — and handed to one `write_all` on a
//! descriptor opened `O_APPEND`. The kernel then resolves the offset and the
//! write together, so two writers cannot land on the same bytes. What they
//! must never do is arrive in two pieces: half a record from one writer
//! spliced into half of another's destroys both.
//!
//! **Every stored string is bounded, not only `detail`.** That used to be the
//! rule for `detail` alone, and the identity fields around it — agent,
//! project, run, session, message — were whatever the caller passed. One
//! 600 KB `--project` was therefore one 600 KB record, larger than the whole
//! `KEEP` window, and the next append's trim looked for a record boundary
//! inside a slice that contained none and wrote the file back empty. Five
//! records became zero. A bound on every field is what makes "one record is
//! one small write" true rather than aspirational, and `trim` now refuses to
//! keep nothing regardless.
//!
//! **Every stored string is also credential-filtered.** `detail` is free text
//! an agent typed, but so is a project name, an agent name, a run id and a
//! message id when they come off a command line: `--project "p sk-…"` used to
//! land verbatim. All of them go through `secrets::looks_secret` line by
//! line, the same rule history and the clipboard live under. A full prompt or
//! command line never comes here at all — a Task carries a *reference*.
//!
//! **Nothing reads this file per keystroke.** It is a whole-file parse, on
//! the same footing as `bus::all()`: fine for an explicit command and for a
//! gather that has already decided to pay for it, never for the transform
//! binding that runs on every key.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// How much of an agent's note survives, in display columns. Long enough for
/// a sentence that explains a failure, short enough that the record stays one
/// small write.
pub const DETAIL_MAX: usize = 500;

/// How much of an identity field survives — an agent name, a project, a run
/// or session id, a message id. Every one of these is a name or a key, so a
/// bound this generous only ever bites something that was not one.
pub(crate) const FIELD_MAX: usize = 200;

/// The byte ceiling a display bound implies, per column.
///
/// Four is the widest a single UTF-8 character gets, so this never cuts
/// ordinary text that already fits its column bound — it exists because a
/// column bound is not a size bound at all: zero-width characters cost
/// columns nothing and bytes plenty, and "one small write" is a claim about
/// bytes. The worst a record can now cost is `DETAIL_MAX` plus six
/// `FIELD_MAX` fields at four bytes a column, about 6.8 KB — two orders of
/// magnitude below `KEEP`, which is the property `trim` depends on.
const BYTES_PER_COL: usize = 4;

/// Trim once the log passes this. At the ~250 bytes a task event costs that
/// is around two thousand records — months of a busy fleet — and still a file
/// `all()` parses in single-digit milliseconds. It is a bound, not a
/// retention policy: the Task records are the authority and this is their
/// narration, so the oldest half is simply dropped.
const CAP: u64 = 512 * 1024;

/// What a trim keeps. Half the cap rather than all of it, so trimming happens
/// once every few thousand appends instead of on every append past the line.
const KEEP: usize = 256 * 1024;

/// What replaces a line that looked like a credential. A marker rather than
/// silence: "something was here and we refused to store it" is information,
/// and an event that quietly loses half its text reads as a bug.
const REDACTED: &str = "[redacted]";

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Event {
    pub ts: u64,
    /// The event's own id. Unique within a process even inside one second,
    /// because a task that starts and finishes immediately emits two.
    pub id: String,
    /// `task.start`, `task.queue`, `task.progress`, `task.wait`, `task.done`,
    /// `task.fail`, `task.cancel`, `task.assign`, `task.link`, `task.retry`,
    /// `task.orphaned`. A reader must tolerate kinds it does not know.
    pub kind: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// The originating question or message, when this work came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Credential-filtered, bounded free text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Event {
    /// `kind` is one of this module's own literals and is bounded only as
    /// defence in depth. `task` is a key — the thing `for_task` matches on —
    /// and goes through the same filter as everything else: an id that trips
    /// it is a credential somebody put in a `--task=` flag, and filing that
    /// event under `[redacted]` where nobody can look it up again is the
    /// right answer rather than a lost lookup.
    pub fn new(kind: &str, task: &str) -> Self {
        Event {
            ts: crate::bus::now(),
            id: next_id(),
            kind: bounded(kind, FIELD_MAX),
            task: some(task).unwrap_or_default(),
            ..Default::default()
        }
    }
    pub fn agent(mut self, v: &str) -> Self {
        self.agent = some(v);
        self
    }
    pub fn run(mut self, v: &str) -> Self {
        self.run = some(v);
        self
    }
    pub fn session(mut self, v: &str) -> Self {
        self.session = some(v);
        self
    }
    pub fn project(mut self, v: &str) -> Self {
        self.project = some(v);
        self
    }
    pub fn message(mut self, v: &str) -> Self {
        self.message = some(v);
        self
    }
    /// Free text from an agent, and therefore filtered before it is kept.
    pub fn detail(mut self, v: &str) -> Self {
        self.detail = redact(v, DETAIL_MAX);
        self
    }
}

/// An identity field, filtered and bounded exactly like free text.
///
/// It used to be `trim()` and nothing else, which is how a credential in
/// `--project` and a 600 KB `--agent` both reached the log. There is no field
/// here whose value did not arrive from a command line at some point, so
/// there is no field here that is exempt.
fn some(v: &str) -> Option<String> {
    redact(v, FIELD_MAX)
}

/// Bound a stored string twice over: display columns, because `width::dtrunc`
/// is the only correct way to cut a string that will be printed in a column,
/// and bytes, because a column bound alone bounds nothing.
pub(crate) fn bounded(text: &str, cols: usize) -> String {
    let out = crate::width::dtrunc(text, cols);
    let cap = cols.saturating_mul(BYTES_PER_COL);
    if out.len() <= cap {
        return out;
    }
    let mut end = cap;
    while end > 0 && !out.is_char_boundary(end) {
        end -= 1;
    }
    out[..end].to_string()
}

fn next_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "e{}-{}-{}",
        crate::bus::now(),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// The credential policy for everything an agent hands us as prose.
///
/// Line by line, because a note is usually one useful sentence and one pasted
/// error message, and throwing the whole thing away because the error quoted
/// an `Authorization:` header loses the part that mattered. A line that trips
/// `looks_secret` is replaced whole rather than edited: knowing *where* in a
/// line the secret starts is exactly the thing we cannot do reliably, and a
/// partial redaction that leaves the last eight characters of a key is not a
/// redaction.
pub fn redact(text: &str, max: usize) -> Option<String> {
    let mut out = String::new();
    // The early break is on bytes rather than characters because the loop
    // used to count the whole accumulator once per line, which is quadratic
    // in exactly the case that matters — a very large field arriving as many
    // short lines. `bounded` below does the real cut; this only stops the
    // work once there is provably enough of it.
    let ceiling = max.saturating_mul(BYTES_PER_COL);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        if crate::secrets::looks_secret(line) {
            out.push_str(REDACTED);
        } else {
            out.push_str(line);
        }
        if out.len() >= ceiling {
            break;
        }
    }
    let out = bounded(out.trim(), max);
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

// ---------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------

/// The log, addressed by path rather than by the environment.
///
/// `sessions.rs` records the reason: `paths::data()` reads `$XDG_DATA_HOME`,
/// the environment belongs to the whole process, and `cargo test` runs tests
/// on several threads at once — so a test that repoints it is mutating shared
/// state underneath every other test's `std::env::var_os`. Taking the root as
/// a parameter, with the env-free public API on top, is how that module
/// solved it and is how this one does too.
pub(crate) struct Log {
    path: PathBuf,
}

pub(crate) fn log() -> Log {
    Log::at(file())
}

fn file() -> PathBuf {
    crate::paths::data().join("events.jsonl")
}

impl Log {
    pub(crate) fn at(path: PathBuf) -> Log {
        Log { path }
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Add one record. The only way this file is ever written.
    pub(crate) fn append(&self, event: &Event) -> Result<(), String> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| "no data directory".to_string())?;
        ensure_private_dir(dir)?;
        let mut line = serde_json::to_string(event)
            .map_err(|e| format!("could not encode the event: {e}"))?;
        line.push('\n');

        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut handle = options
            .open(&self.path)
            .map_err(|e| format!("could not open the event log: {e}"))?;
        handle
            .write_all(line.as_bytes())
            .map_err(|e| format!("could not write the event: {e}"))?;
        drop(handle);
        trim(&self.path);
        Ok(())
    }

    /// Every event, oldest first.
    ///
    /// A line that will not parse is skipped rather than fatal — a source
    /// degrades to nothing, and a log truncated by a full disk must not take
    /// a launcher down with it.
    pub(crate) fn all(&self) -> Vec<Event> {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return Vec::new() };
        text.lines()
            .filter_map(|line| serde_json::from_str::<Event>(line).ok())
            .collect()
    }

    pub(crate) fn for_task(&self, id: &str) -> Vec<Event> {
        if id.is_empty() {
            return Vec::new();
        }
        self.all().into_iter().filter(|e| e.task == id).collect()
    }

}

/// `0700`, because everything Prelude keeps under the data directory is about
/// what the person and their agents are doing and none of it is other users'
/// business.
///
/// The mode is applied *at creation*, never to a directory that is already
/// there. Setting it unconditionally meant that appending one event silently
/// re-moded the shared `$XDG_DATA_HOME/prelude` — which also holds `bus/`,
/// `frecency.tsv` and the clipboard — and undid a person's own `chmod` on a
/// directory Prelude did not create. `bus.rs` has never done this.
pub(crate) fn ensure_private_dir(path: &Path) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|e| format!("could not create {}: {e}", path.display()))
}

/// A temporary name nothing else can be using. The pid is not enough on its
/// own: two threads of one process write task records concurrently in the
/// tests, and a shared temp name is a truncated file for one of them.
fn temp_of(path: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Temp file, `0600`, fsync, rename — the idiom `sessions.rs` uses for its
/// metadata, and for the same reason: a reader must never see half a record,
/// and a record must never be readable by anyone else.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or_else(|| "no parent directory".to_string())?;
    ensure_private_dir(dir)?;
    let tmp = temp_of(path);
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut handle = options
        .open(&tmp)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    handle
        .write_all(bytes)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    handle
        .sync_all()
        .map_err(|e| format!("could not sync {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("could not replace {}: {e}", path.display()))
}

/// An exclusive advisory lock on a file of our own, released when it closes.
///
/// Declared rather than depended on: `flock` is two lines of `extern "C"` and
/// this codebase does not add a crate for two lines.
///
/// Shared with `task.rs`, which locks its open-task index around every rewrite
/// for the same reason this locks the log around a trim: one process reads a
/// file whole and replaces it, and a concurrent writer that lands in between
/// is silently lost. One declaration, not two.
#[cfg(unix)]
pub(crate) fn lock_exclusive(path: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    const LOCK_EX: i32 = 2;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)
        .ok()?;
    // SAFETY: `file` owns the descriptor for the whole of this call and is
    // returned to the caller, so it cannot be closed underneath the lock.
    if unsafe { flock(file.as_raw_fd(), LOCK_EX) } != 0 {
        return None;
    }
    Some(file)
}

#[cfg(not(unix))]
pub(crate) fn lock_exclusive(_path: &Path) -> Option<std::fs::File> {
    None
}

/// Split a log into whole records, each slice carrying its own terminating
/// newline. A file that does not end in one has a final fragment — an append
/// caught mid-flight, or a write cut short by a full disk — and it is returned
/// as its own slice rather than glued onto the record before it.
fn records(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        match bytes[start..].iter().position(|b| *b == b'\n') {
            Some(offset) => {
                out.push(&bytes[start..start + offset + 1]);
                start += offset + 1;
            }
            None => {
                out.push(&bytes[start..]);
                break;
            }
        }
    }
    out
}

/// Keep the newest records once the file passes `CAP`.
///
/// Three things happen here that did not used to.
///
/// **The cut is on record boundaries, from the end, never on an offset.** The
/// trim used to take everything after the first newline at or after
/// `len - KEEP`, which works only while every record is small compared with
/// the window. One oversized record — a 600 KB `--project` from a build before
/// the fields were bounded — occupies the whole window, and then that single
/// newline is the *end* of the giant: everything before it, which is every
/// live record in the file, was thrown away. A guard against `from` running
/// past the end of the file made the failure look survivable, but it only held
/// while the giant was the file's last line; one more append moved it inside
/// the window and six records became one. Walking records backwards from the
/// end has no window to occupy.
///
/// **An oversized record is dropped wherever it sits.** A single line longer
/// than the whole keep window can never be part of an honest trim: keeping it
/// means keeping nothing else, and it is by construction a record no bound in
/// this module can produce. So it is skipped and its neighbours survive, which
/// is what makes an already-poisoned log survivable — the live narration
/// around the poison is what somebody wants back.
///
/// **It still refuses to keep nothing.** If every record in the file is
/// oversized there is nothing honest to keep, and the trim is abandoned rather
/// than writing the file back empty. The log then stays over `CAP` until
/// enough ordinary records have been appended to give it something to keep.
///
/// **Only one process trims at a time.** Appends stay lock-free — that is the
/// file's whole virtue — but every appender that saw the file over `CAP` used
/// to perform its own full read and rename, so a burst multiplied the window
/// rather than sharing it: eight threads appending 480 records lost 17 of
/// them, 3.5%, all of them the newest. An exclusive `flock` on a sidecar
/// file, taken only here and re-checking the size once acquired, collapses a
/// burst to one trim. What remains is the honest residue: a writer holding an
/// `O_APPEND` descriptor across *that one* rename keeps writing to the
/// replaced inode and loses its line. That is a real loss and it is a line of
/// narration, never a Task record; the alternative is a lock on the append
/// path, and the whole point of this file is that appending needs none.
fn trim(path: &Path) {
    let over_cap = || std::fs::metadata(path).is_ok_and(|meta| meta.len() > CAP);
    if !over_cap() {
        return;
    }
    let _lock = lock_exclusive(&path.with_extension("lock"));
    // Somebody else may have trimmed it while we waited for the lock.
    if !over_cap() {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else { return };
    let mut keep: Vec<&[u8]> = Vec::new();
    let mut total = 0usize;
    for record in records(&bytes).into_iter().rev() {
        if record.len() > KEEP {
            continue;
        }
        if total + record.len() > KEEP {
            break;
        }
        total += record.len();
        keep.push(record);
    }
    if keep.is_empty() {
        return;
    }
    keep.reverse();
    let _ = write_private(path, &keep.concat());
}

/// The whole log, for a caller that has not been given a root. `doctor
/// agents` walks it looking for events that name a task nobody has a record
/// of; `running.rs` deliberately does not come through here, because it wants
/// the tail rather than the file.
pub fn all() -> Vec<Event> {
    log().all()
}

pub fn for_task(id: &str) -> Vec<Event> {
    log().for_task(id)
}

#[cfg(test)]
pub(crate) mod testing {
    //! One door into a private store, and it is a directory rather than an
    //! environment variable.
    //!
    //! `root()` makes a temporary directory and hands it back; a test drives
    //! `Log::at`, `task::Store::at` or `bus::Bus::at` with it. It touches no
    //! shared state, so any number of tests may hold one at once.
    //!
    //! There used to be a second door — `store()`, which repointed
    //! `$XDG_DATA_HOME` for the whole process under a mutex — for the modules
    //! that had not been given a root yet. It is gone, and deliberately: the
    //! environment belongs to the process while `cargo test` runs its tests on
    //! several threads, so writing it races every other test's `var_os` and
    //! every subprocess's inherited copy of it. That is undefined behaviour,
    //! and a mutex over the tests that *write* it cannot make it defined
    //! because it does not hold the ones that read.

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// A private directory that is removed when the guard drops. No
    /// environment variable is touched, so this is safe to hold on any number
    /// of threads at once.
    pub(crate) struct Root {
        pub(crate) path: PathBuf,
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn fresh(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "prelude-test-{name}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test store directory");
        root
    }

    pub(crate) fn root(name: &str) -> Root {
        Root { path: fresh(name) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_in(name: &str) -> (testing::Root, Log) {
        let root = testing::root(name);
        let log = Log::at(root.path.join("events.jsonl"));
        (root, log)
    }

    #[test]
    fn a_credential_never_reaches_the_log() {
        let (_root, log) = log_in("event-secret");
        let key = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01";
        log.append(
            &Event::new("task.progress", "t1")
                .detail(&format!("deploying\nexport API_KEY={key}\ndone")),
        )
        .expect("append");

        let raw = std::fs::read_to_string(log.path()).expect("log");
        assert!(!raw.contains(key), "the stored log kept a credential: {raw}");
        assert!(raw.contains("deploying"), "the harmless lines must survive");
        assert!(raw.contains(REDACTED), "a dropped line must leave a marker");
    }

    /// Finding 2's other half: an identity field is a command-line string too.
    #[test]
    fn every_event_field_is_credential_filtered() {
        let (_root, log) = log_in("event-fields");
        let key = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01";
        log.append(
            &Event::new("task.start", &format!("t {key}"))
                .agent(&format!("a {key}"))
                .project(&format!("p {key}"))
                .run(&format!("r {key}"))
                .session(&format!("s {key}"))
                .message(&format!("m {key}"))
                .detail(&format!("d {key}")),
        )
        .expect("append");
        let raw = std::fs::read_to_string(log.path()).expect("log");
        assert!(!raw.contains(key), "an event field kept a credential: {raw}");
    }

    /// Finding 1: every field is bounded, so no single record can be a
    /// meaningful fraction of `KEEP`.
    #[test]
    fn every_event_field_is_bounded() {
        let (_root, log) = log_in("event-bounds");
        let huge = "a".repeat(600_000);
        log.append(
            &Event::new("task.start", &huge)
                .agent(&huge)
                .project(&huge)
                .run(&huge)
                .session(&huge)
                .message(&huge)
                .detail(&huge),
        )
        .expect("append");
        let size = std::fs::metadata(log.path()).expect("log").len();
        assert!(size < 8 * 1024, "a record grew to {size} bytes");
        assert!((size as usize) < KEEP / 8, "a record is a large fraction of the keep window");
        assert_eq!(log.all().len(), 1, "the record must still parse");
    }

    /// Finding 1, exactly as it was reproduced: five records and 645 bytes
    /// became zero records and zero bytes the moment one oversized record was
    /// appended.
    ///
    /// The oversized record is built by hand, because no field this module
    /// accepts can produce one any more — that is the first half. The second
    /// half is where the giant *sits*. A prefix cut has a window an oversized
    /// line can occupy, and while the giant is the file's last line the guard
    /// against cutting past the end hides it; one more append moves it inside
    /// the window and every record before it goes. So the giant is written
    /// mid-file here, with live records on both sides of it, and both sides
    /// have to survive.
    #[test]
    fn an_oversized_record_is_dropped_and_its_neighbours_survive() {
        let (_root, log) = log_in("event-poison");

        // A log an older Prelude poisoned: three ordinary records, one 600 KB
        // record from before the fields were bounded, three more ordinary
        // ones. Written by hand, because no door here accepts the giant.
        let mut seeded = String::new();
        let record = |n: usize| {
            let event = Event::new("task.progress", "t1").detail(&format!("record {n}"));
            serde_json::to_string(&event).expect("encode") + "\n"
        };
        for n in 0..3 {
            seeded.push_str(&record(n));
        }
        let mut giant = Event::new("task.start", "poison");
        giant.project = Some("a".repeat(600_000));
        seeded.push_str(&(serde_json::to_string(&giant).expect("encode") + "\n"));
        for n in 3..6 {
            seeded.push_str(&record(n));
        }
        write_private(log.path(), seeded.as_bytes()).expect("seed");
        assert_eq!(log.all().len(), 7, "seeded: six live records around one giant");

        // The next ordinary append is what triggers the trim.
        log.append(&Event::new("task.done", "newest").detail("the last word")).expect("append");

        let kept = log.all();
        let details: Vec<String> = kept.iter().filter_map(|e| e.detail.clone()).collect();
        for n in 0..6 {
            assert!(
                details.iter().any(|d| d == &format!("record {n}")),
                "the trim discarded a live record: {details:?}"
            );
        }
        assert_eq!(kept.last().map(|e| e.task.as_str()), Some("newest"));
        assert!(
            !kept.iter().any(|e| e.task == "poison"),
            "the oversized record has no honest place in the window"
        );
        let size = std::fs::metadata(log.path()).expect("log").len();
        assert!(size <= CAP, "the log is still unbounded: {size} bytes");

        // The same input through the ordinary door costs nothing at all,
        // because every field is bounded before it is written.
        let (_root, log) = log_in("event-poison-bounded");
        for n in 0..5 {
            log.append(&Event::new("task.progress", "t1").detail(&format!("record {n}")))
                .expect("append");
        }
        log.append(&Event::new("task.start", "poison").project(&"a".repeat(600_000)))
            .expect("append");
        assert_eq!(log.all().len(), 6, "a bounded record cost the log nothing");
    }

    /// A log made of nothing but oversized records has no honest place to cut,
    /// so the trim is abandoned rather than writing it back empty. That is the
    /// property the old guard was reaching for and only ever held by accident.
    #[test]
    fn a_log_of_nothing_but_giants_is_never_emptied() {
        let (_root, log) = log_in("event-all-giants");
        let mut seeded = String::new();
        for n in 0..3 {
            let mut giant = Event::new("task.start", &format!("poison{n}"));
            giant.project = Some("a".repeat(600_000));
            seeded.push_str(&(serde_json::to_string(&giant).expect("encode") + "\n"));
        }
        write_private(log.path(), seeded.as_bytes()).expect("seed");
        trim(log.path());
        assert_eq!(log.all().len(), 3, "the log was emptied rather than left over CAP");
    }

    /// Half the writers emit the largest record the bounds allow, because
    /// "one record is one small write" is the property that keeps two
    /// appenders from splicing halves of each other's lines together, and it
    /// is worth exercising at the bound rather than well inside it.
    #[test]
    fn interleaved_appends_all_survive_and_stay_parseable() {
        let (_root, log) = log_in("event-append");
        let writers = 8;
        let each = 25;
        let wide = "wide ".repeat(DETAIL_MAX);
        std::thread::scope(|scope| {
            let log = &log;
            let wide = wide.as_str();
            for w in 0..writers {
                scope.spawn(move || {
                    for n in 0..each {
                        let event = Event::new("task.progress", &format!("t{w}"));
                        let event = if w % 2 == 0 {
                            event.detail(&format!("{n}"))
                        } else {
                            event
                                .agent(wide)
                                .project(wide)
                                .run(wide)
                                .session(wide)
                                .message(wide)
                                .detail(wide)
                        };
                        log.append(&event).expect("append");
                    }
                });
            }
        });

        let raw = std::fs::read_to_string(log.path()).expect("log");
        let lines = raw.lines().count();
        let events = log.all();
        assert_eq!(lines, writers * each, "a write was lost or split");
        assert_eq!(events.len(), lines, "a line failed to parse: appends interleaved");
        for w in 0..writers {
            assert_eq!(log.for_task(&format!("t{w}")).len(), each);
        }
    }

    /// Finding 3: a burst of appends against a log that is already over `CAP`.
    ///
    /// Without the lock every appender ran its own read-and-rename and the
    /// windows multiplied — 17 of 480 records lost, and always the newest.
    /// The residual loss is one rename's worth, so this asserts a ceiling
    /// rather than perfection, and separately that nothing was duplicated,
    /// torn, or left as a partial line at the head.
    #[test]
    fn a_burst_of_appends_over_the_cap_keeps_almost_everything() {
        let (_root, log) = log_in("event-burst");
        let mut seeded = String::new();
        let mut n = 0u64;
        while seeded.len() as u64 <= CAP {
            let event = Event::new("task.progress", "old").detail(&format!("record {n}"));
            seeded.push_str(&serde_json::to_string(&event).expect("encode"));
            seeded.push('\n');
            n += 1;
        }
        write_private(log.path(), seeded.as_bytes()).expect("seed");

        // The burst has to be wide enough that the ceiling below is wider than
        // the loss this design admits, or the test asserts something stricter
        // than the code promises and fails whenever the machine is busy. Only
        // one trim can happen — the first drops the file to `KEEP` and the
        // burst never puts it back over `CAP` — and one rename can cost at
        // most one line per writer, because a writer is inside one append at a
        // time. So the honest bound is eight lost lines however long the burst
        // is, and 1% of sixty apiece was four: a run that lost six was inside
        // the design and outside the assertion. 1% of two hundred apiece is
        // sixteen, which is the same claim with room for the whole of it.
        let writers = 8;
        let each = 200;
        std::thread::scope(|scope| {
            let log = &log;
            for w in 0..writers {
                scope.spawn(move || {
                    for i in 0..each {
                        log.append(&Event::new("task.progress", "new").detail(&format!("{w}-{i}")))
                            .expect("append");
                    }
                });
            }
        });

        let raw = std::fs::read_to_string(log.path()).expect("log");
        assert!(
            raw.lines().all(|line| serde_json::from_str::<Event>(line).is_ok()),
            "a line was torn"
        );
        let fresh: Vec<String> = log
            .all()
            .into_iter()
            .filter(|e| e.task == "new")
            .filter_map(|e| e.detail)
            .collect();
        let unique: std::collections::HashSet<&str> = fresh.iter().map(String::as_str).collect();
        assert_eq!(unique.len(), fresh.len(), "a record was duplicated");
        let sent = writers * each;
        assert!(
            fresh.len() * 100 >= sent * 99,
            "{} of {sent} newest records survived the trim",
            fresh.len()
        );
    }

    #[test]
    fn the_log_is_bounded_and_keeps_the_newest_records() {
        let (_root, log) = log_in("event-cap");
        let mut filler = String::new();
        let mut n = 0u64;
        while filler.len() as u64 <= CAP {
            let event = Event::new("task.progress", "old").detail(&format!("record {n}"));
            filler.push_str(&serde_json::to_string(&event).expect("encode"));
            filler.push('\n');
            n += 1;
        }
        let before = filler.lines().count();
        write_private(log.path(), filler.as_bytes()).expect("seed");

        log.append(&Event::new("task.done", "newest").detail("the last word")).expect("append");

        let size = std::fs::metadata(log.path()).expect("log").len();
        assert!(size <= CAP, "the log is unbounded: {size} bytes");
        let kept = log.all();
        assert!(kept.len() < before, "nothing was trimmed");
        assert!(!kept.is_empty(), "everything was trimmed");
        assert_eq!(kept.last().map(|e| e.task.as_str()), Some("newest"));
        // The oldest survivor must be a whole record, not the tail of one.
        assert!(kept.first().is_some_and(|e| e.kind == "task.progress"));
    }

    #[test]
    fn a_malformed_line_is_skipped_rather_than_fatal() {
        let (_root, log) = log_in("event-broken");
        log.append(&Event::new("task.start", "t1")).expect("append");
        let mut raw = std::fs::read_to_string(log.path()).expect("log");
        raw.push_str("{not json at all\n");
        write_private(log.path(), raw.as_bytes()).expect("write");
        log.append(&Event::new("task.done", "t1")).expect("append");

        let kinds: Vec<String> = log.all().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec!["task.start".to_string(), "task.done".to_string()]);
    }

    /// Finding 5: creating the store must not re-mode a directory somebody
    /// else owns — the data root holds `bus/`, the clipboard and frecency.
    #[test]
    #[cfg(unix)]
    fn an_existing_directory_keeps_its_own_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = testing::root("event-mode");
        let shared = root.path.join("prelude");
        std::fs::create_dir_all(&shared).expect("shared");
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let log = Log::at(shared.join("events.jsonl"));
        log.append(&Event::new("task.start", "t1")).expect("append");

        let mode = std::fs::metadata(&shared).expect("shared").permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "appending re-moded a directory it did not create");

        // A directory Prelude does create is private from the start.
        let mine = shared.join("tasks");
        ensure_private_dir(&mine).expect("create");
        let mode = std::fs::metadata(&mine).expect("mine").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "a directory Prelude creates must be private");
    }
}
