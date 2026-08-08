//! The message bus: how an agent reaches a human, and how agents reach each
//! other.
//!
//! An agent running in a terminal is a sealed box. It cannot see the other
//! agents on the machine, it cannot talk to them, and the only way it can
//! reach the person who started it is by printing into its own window — which
//! that person may not be looking at. When it needs a decision it stops and
//! waits, and the waiting is invisible. That is the "waiting 12m" the fleet
//! view already detects *from the outside*, and detecting it is only half an
//! answer: the person still has to notice the badge, go there, and read back
//! far enough to see what was asked.
//!
//! This is the other half. Four verbs, all of them things an agent can run
//! from its own shell:
//!
//! ```text
//! prelude ask  "the migration drops legacy_users. proceed?"   ← blocks
//! prelude tell "deploy is green"                              ← one-way
//! prelude say  api-gateway "I changed the auth schema"        ← agent → agent
//! prelude inbox                                               ← what came in
//! ```
//!
//! `ask` is the one that changes what an agent can be. It writes the question
//! where the launcher can see it, notifies the human wherever they are, and
//! *blocks on stdout* until an answer comes back — so the agent can be
//! written as though a person were sitting next to it. The person answers
//! from anywhere: the launcher's top row, `prelude reply` in any terminal,
//! or by hand with `prelude answer <id>`.
//!
//! **Identity is discovered, never declared.** An agent should not have to be
//! told who it is or configured with an address. `$TMUX_PANE` gives a reply
//! address for free in any pane, `$PWD` gives the project, and walking the
//! process tree finds which agent binary we are running under. So the whole
//! interface really is just the four verbs above — and no flag added here may
//! change that, because a name an agent can declare is a name it can get
//! wrong.
//!
//! **A conversation is a thread, not a pile of files.** A question, the
//! handoff it produced and the result that came back are one exchange, and
//! joining them by timestamp is guesswork. Every message therefore carries
//! the id of the message that opened its thread — its own when it is the
//! opener — and the id it is replying to. That is what lets a Task finished
//! by a second agent report back into the inbox of the first.
//!
//! **Delivery has states, not one flag.** `answer: Some("")` used to stand
//! for "handled", which made a notice "born answered" and made an inbox that
//! was drained indistinguishable from a question somebody decided. There are
//! four different facts there — it went nowhere, it was typed into a pane, it
//! was collected, it was answered — and only the last is an answer.
//!
//! **Attachments are paths, never contents.** Copying a file into a message
//! would put it in the data directory, in an inbox, and past every filter
//! that guards the original. The receiving agent has a filesystem; it is
//! handed the path and reads what it needs.
//!
//! **Storage is one JSON file per message**, under the data directory rather
//! than the cache: an unanswered question is not something to be dropped when
//! a cache is cleared. Writes go through `write_atomic`, so a reader never
//! sees half a message, and no daemon owns the bus — every operation is a
//! directory read and a file write, which is what makes it work identically
//! from a launcher, a shell, and an agent's tool call. Fields are added with
//! `#[serde(default)]` and unknown ones are ignored, so a message written by
//! either build is readable by the other.

use crate::item::{Item, Kind};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// How long `ask` waits by default before giving up.
///
/// An agent's own tool call usually has a shorter deadline than this, so the
/// number that matters is the one the caller passes. This is the backstop for
/// when nobody passed one: long enough that stepping away for a coffee does
/// not lose the answer, short enough that a forgotten `ask` in a script does
/// not wedge forever.
pub const DEFAULT_WAIT: u64 = 600;

/// How often a blocked `ask` looks for its answer. Fast enough to feel
/// instant to the person who just typed it, cheap enough to be free.
const POLL: Duration = Duration::from_millis(250);

/// Settled messages are kept this long so `inbox --all` can show what was
/// decided; open ones — an unanswered question, mail nobody has collected —
/// are never swept.
const KEEP_SETTLED: u64 = 24 * 3600;

/// How much of a message's own words survive. Generous, because a question
/// worth stopping for is often a paragraph and a list.
const TEXT_MAX: usize = 4000;

/// A handoff note, a cancellation reason, a result. Shorter than the text:
/// these also travel into the event log, whose whole virtue is that one
/// record is one small write.
const NOTE_MAX: usize = 500;

/// No plausible message attaches more files than this, and an unbounded list
/// is an unbounded line typed into somebody else's terminal.
const ATTACH_MAX: usize = 32;

/// What replaces a line we refused to store. A marker rather than silence:
/// text that quietly loses half of itself reads as a bug.
const REDACTED: &str = "[redacted]";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Where a message got to.
///
/// Stored as a word rather than a typed enum on disk: a state written by a
/// newer build must not make the whole message unreadable, and when the word
/// is missing or unknown `Msg::state` can work it out from the other fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Written to the bus. Nothing yet says anybody has seen it.
    Sent,
    /// There was nowhere to deliver it — no pane, no way in. It is waiting in
    /// the recipient's inbox, and saying so is the point: an undelivered
    /// message that looks delivered is how work goes missing.
    Undelivered,
    /// Typed into the recipient's pane, or posted to the human's notifications.
    Delivered,
    /// The recipient collected it with `inbox` or `drain`.
    Read,
    /// A person answered it.
    Answered,
    /// The sender withdrew it.
    Cancelled,
    /// Its deadline passed with no answer. Not deleted — a question nobody
    /// got to is exactly the thing that must not disappear quietly — but no
    /// longer demanding attention.
    Expired,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Sent => "sent",
            State::Undelivered => "undelivered",
            State::Delivered => "delivered",
            State::Read => "read",
            State::Answered => "answered",
            State::Cancelled => "cancelled",
            State::Expired => "expired",
        }
    }

    /// Inherent rather than `FromStr`, for the reason `task::State` gives: an
    /// unknown word is not an error worth a type, it is simply not a state.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<State> {
        [
            State::Sent,
            State::Undelivered,
            State::Delivered,
            State::Read,
            State::Answered,
            State::Cancelled,
            State::Expired,
        ]
        .into_iter()
        .find(|state| state.as_str() == s)
    }

    /// Has this reached its recipient in a way they had to look at?
    ///
    /// A line typed into a pane has been *delivered* but not *collected* —
    /// the agent may never have read that far — so it stays in the inbox
    /// until somebody says otherwise.
    fn collected(self) -> bool {
        matches!(self, State::Read | State::Answered | State::Cancelled | State::Expired)
    }

    /// Is anything still expected to happen? Only the sweep asks, and only
    /// these two answers protect a file from it: a question waiting on a
    /// person, and mail that never reached anybody.
    fn open(self) -> bool {
        matches!(self, State::Sent | State::Undelivered)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Msg {
    pub id: String,
    /// `ask` (wants an answer), `tell` (a notice), `say` (agent to agent).
    pub kind: String,
    /// Which agent sent it, or `shell` when a person ran it by hand.
    pub from: String,
    #[serde(default)]
    pub from_project: String,
    #[serde(default)]
    pub from_cwd: String,
    /// Where to type a reply back, when the sender was in a pane.
    #[serde(default)]
    pub from_pane: String,
    /// `human`, or the label of the agent this was addressed to.
    pub to: String,
    #[serde(default)]
    pub to_pane: String,
    #[serde(default)]
    pub to_cwd: String,
    /// The agent this is for, as a bare binary name — the address of last
    /// resort.
    ///
    /// A pane is exact and a directory is nearly so; this is neither, and it
    /// exists because the two above can both be empty. An agent that started
    /// outside tmux and has since `cd`'d has no pane and no matching cwd, and
    /// a result addressed only by those is a message that exists, is open,
    /// and can never be collected by anybody — so it is never swept either,
    /// and it accumulates. `for_agent` falls back to this only when there is
    /// no pane and no cwd on the message at all, so it can never widen a
    /// message that already had a real address.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub to_agent: String,
    pub text: String,
    pub ts: u64,
    /// The message that opened this exchange — its own id when it is the
    /// opener. Absent on anything written by an older build, which is why
    /// `thread_id` treats "missing" as "this message is its own thread".
    #[serde(default)]
    pub thread: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// The word from `State`. Empty on an older file, where the answer field
    /// was carrying the same information badly.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    /// Absolute time after which an unanswered question stops asking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<u64>,
    /// Why it was cancelled, or what it was reassigned to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Absolute paths the recipient may read. Never their contents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attach: Vec<String>,
    /// The Task this message is about, when it is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered: Option<u64>,
}

impl Msg {
    /// A question still waiting on a person.
    ///
    /// This is the launcher's whole view of the bus — `items()` and the Home
    /// ordering are built on it — so it means exactly what it has always
    /// meant. The two additions are the two ways a question stops asking
    /// without being answered: the sender withdrew it, or its deadline
    /// passed. Neither can happen to a message written by an older build,
    /// which has neither field.
    pub fn pending(&self) -> bool {
        self.kind == "ask"
            && self.answer.is_none()
            && !matches!(self.state(), State::Cancelled | State::Expired)
    }

    /// Where this message got to.
    ///
    /// Derived rather than only read, for two reasons. An older build wrote
    /// no state word at all and used `answer: Some("")` as its marker for
    /// "handled"; and nothing runs at a deadline, so expiry has to happen on
    /// read or it never happens.
    pub fn state(&self) -> State {
        let stored = State::from_str(&self.state).unwrap_or_else(|| self.implied_state());
        if stored.open() && self.past_deadline() { State::Expired } else { stored }
    }

    /// What the fields say, when the state word does not.
    fn implied_state(&self) -> State {
        match self.answer.as_deref() {
            None => State::Sent,
            // The old marker for "handled without an answer": a notice, a
            // line typed into a pane, an inbox that was drained.
            Some("") => State::Delivered,
            Some(_) => State::Answered,
        }
    }

    fn past_deadline(&self) -> bool {
        self.deadline.is_some_and(|d| now() > d)
    }

    /// The exchange this belongs to. A message with no thread recorded is its
    /// own thread, which is both the right answer for an opener and the right
    /// answer for a file written before threads existed.
    pub fn thread_id(&self) -> &str {
        if self.thread.is_empty() { &self.id } else { &self.thread }
    }

    /// `claude · api-gateway`, or just the agent when there is no project.
    pub fn label(&self) -> String {
        match (self.from.as_str(), self.from_project.as_str()) {
            (a, "") => a.to_string(),
            (a, p) => format!("{a} · {p}"),
        }
    }
    pub fn age(&self) -> u64 {
        now().saturating_sub(self.ts)
    }

    fn set(&mut self, state: State) {
        self.state = state.as_str().to_string();
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Ids we generate are digits and dashes; an id from a command line is
/// whatever somebody typed. This is what stops `prelude answer ../../id_rsa`
/// reading or writing outside the bus directory.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Unique even when one process posts twice in the same second — a handoff
/// and the result that follows it can be that close, and two messages sharing
/// a file is one message lost.
fn new_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!("{}-{}-{}", now(), std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed))
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// The bus, addressed by root rather than by the environment.
///
/// `sessions.rs` writes down the reason and `task.rs` and `events.rs` are
/// already built this way; this was the module still holding out.
/// `paths::data()` reads `$XDG_DATA_HOME`, the environment belongs to the
/// whole process, and `cargo test` runs its tests on several threads at once
/// — so a test that repoints it is mutating shared state underneath every
/// other test's `std::env::var_os`, which is documented undefined behaviour
/// and is not something a mutex over one module can prevent. It is worse here
/// than anywhere else, because `whoami` shells out to `ps`: a child process
/// reads the parent's whole environment at exactly the moment another thread
/// is rewriting it.
///
/// So the root is a parameter and the free functions at the end of this file
/// are thin env-free wrappers over it. One `Bus` fixes the root of everything
/// it reaches, too — the Task store it sweeps against and the event log it
/// narrates into are `Store::at` and `Log::at` on the same root — so a bus at
/// a temporary root cannot touch a real one's tasks or events either.
pub(crate) struct Bus {
    root: std::path::PathBuf,
}

/// The bus under the data directory. What every free function below uses.
fn bus() -> Bus {
    Bus::at(data_root())
}

#[cfg(not(test))]
fn data_root() -> std::path::PathBuf {
    crate::paths::data()
}

/// Under test, the root this *thread* was given, and otherwise the real one.
///
/// It exists for one seam and no other. `task::Store::finish` reports a
/// finished Task back through `crate::bus::report_task`, which is a free
/// function and therefore resolves its own root — so a test driving a
/// `task::Store::at` some temporary root would have the result posted into
/// the person's real bus. Closing that properly means handing `task::Store`
/// the bus it should report to, which is a change to `task.rs`.
///
/// Thread-local rather than a global behind a mutex, and that is the whole
/// point: `cargo test` gives each test its own thread, so nothing another
/// test does can be seen here and nothing done here can be seen there. No
/// environment is written, so no concurrent reader and no child process is
/// racing anything.
#[cfg(test)]
fn data_root() -> std::path::PathBuf {
    tests::thread_root().unwrap_or_else(crate::paths::data)
}

impl Bus {
    pub(crate) fn at(root: std::path::PathBuf) -> Bus {
        Bus { root }
    }

    fn dir(&self) -> std::path::PathBuf {
        self.root.join("bus")
    }

    fn file_of(&self, id: &str) -> std::path::PathBuf {
        self.dir().join(format!("{id}.json"))
    }

    /// The event log this bus narrates into — the same file `events::append`
    /// would use when the root is the real one.
    fn log(&self) -> crate::events::Log {
        crate::events::Log::at(self.root.join("events.jsonl"))
    }

    /// The Task store this bus sweeps against and hands work to.
    fn tasks(&self) -> crate::task::Store {
        crate::task::Store::at(self.root.clone())
    }

    /// Every message on the bus, oldest first.
    ///
    /// A file that will not parse is skipped rather than fatal: one bad write
    /// must never take a listing down.
    ///
    /// O(every message not yet swept), which is right for an explicit command and
    /// wrong for anything on a gather — `items()` goes through the pending index
    /// instead, for the reason `index()` sets out.
    ///
    /// The id breaks the tie. `ts` has one-second resolution and a handoff and the
    /// result that follows it are routinely inside the same second, so sorting on
    /// it alone left the order to whatever `read_dir` happened to hand back.
    pub(crate) fn all(&self) -> Vec<Msg> {
        let Ok(rd) = std::fs::read_dir(self.dir()) else { return Vec::new() };
        let mut out: Vec<Msg> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Msg>(&t).ok())
            .collect();
        out.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
        out
    }

    pub(crate) fn get(&self, id: &str) -> Option<Msg> {
        if !valid_id(id) {
            return None;
        }
        let t = std::fs::read_to_string(self.file_of(id)).ok()?;
        serde_json::from_str(&t).ok()
    }

    pub(crate) fn save(&self, m: &Msg) -> std::io::Result<()> {
        if !valid_id(&m.id) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a message id"));
        }
        let json = serde_json::to_vec_pretty(m)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        crate::cache::write_atomic(&self.file_of(&m.id), &json)
    }

    /// Is this message still worth keeping, whatever its age?
    ///
    /// Four reasons, and three of them were missing.
    ///
    /// **An unanswered question, always.** `State::open()` excludes `Expired`, so
    /// a day after it was *sent* a question nobody got to was deleted — which is
    /// the exact opposite of what `State::Expired`, this function and the
    /// acceptance criterion all say. Expiry means it has stopped demanding
    /// attention, not that it has stopped existing: it is still answerable, and
    /// answering it is still useful. Only a withdrawal settles a question.
    ///
    /// **Mail that never reached anybody**, unless there is nobody it could ever
    /// reach. `addressable` is what separates the two.
    ///
    /// **The message a live Task points at.** A handoff becomes `Delivered` the
    /// moment it is typed into the recipient's pane and `Read` the moment they
    /// run `inbox` — both success paths, both settled, both swept a day later.
    /// `report_task` then follows `task.message` to a file that is gone, returns
    /// `None`, and `finish` discards it: the result of a day-old handed-over task
    /// existed in no message anywhere. `task::sweep` already gets the analogous
    /// rule right — a missing record is not evidence of success — and this is the
    /// same rule from the other end.
    fn keep(m: &Msg, cutoff: u64, live: &std::collections::HashSet<String>) -> bool {
        let unanswered_question =
            m.kind == "ask" && m.answer.is_none() && m.state() != State::Cancelled;
        unanswered_question
            || (m.state().open() && addressable(m))
            || live.contains(&m.id)
            || m.ts >= cutoff
    }

    /// Drop settled messages once they are old enough to be history rather than
    /// context — see `keep` for everything that is never settled.
    ///
    /// The Task edges come from `open_tasks`, the bounded reader, rather than
    /// `all()`: a *finished* task has already reported, so the message that
    /// handed it over is ordinary history, and reading the whole task store here
    /// would put a 54 ms scan behind every `ask`.
    fn sweep(&self) {
        let cutoff = now().saturating_sub(KEEP_SETTLED);
        let live: std::collections::HashSet<String> = self.tasks().open_tasks()
            .into_iter()
            .filter_map(|task| task.message)
            .collect();
        let _lock = self.lock_index();
        let mut kept = Vec::new();
        for m in self.all() {
            match Self::keep(&m, cutoff, &live) {
                true => kept.push(m),
                false => {
                    let _ = std::fs::remove_file(self.file_of(&m.id));
                }
            }
        }
        // This is also the moment the pending set is known exactly — the
        // directory has just been read and nothing else holds the lock — so the
        // index is written from what survived rather than left to drift.
        self.write_index(&Self::pending_ids(kept.iter()));
    }

    // ---------------------------------------------------------------------------
    // The pending index
    // ---------------------------------------------------------------------------

    /// Where the launcher looks instead of at the directory.
    ///
    /// `bus::all()` was the one unbounded reader left on the gather path. Measured
    /// on a warmed store it costs about 10 µs a message — 16 ms at a thousand
    /// messages, 38 at three thousand, 60 at five — and the ceiling is permanent,
    /// because undelivered mail is never swept. The task store solved the same
    /// problem the same way and the reasoning transfers whole: `items()` wants
    /// only *pending questions*, which is bounded by how many decisions are
    /// actually outstanding rather than by how much has ever been said.
    ///
    /// A hint, never the authority — the message files are that. It is a
    /// *superset* of the ids worth showing: `ask` appends its own line, and a
    /// sweep reduces the file to what is still worth keeping. So a reader may be
    /// handed an id that has since been answered (filtered out), expired
    /// (filtered out) or swept (absent, skipped), and it can only *miss* a
    /// question if that one append failed — in which case the next sweep
    /// reconstructs the file from the directory.
    fn index(&self) -> std::path::PathBuf {
        self.dir().join("pending.idx")
    }

    /// The lock every rewrite takes, and that an append respects — `task.rs`
    /// records why at length: without it a rewrite can replace the file between
    /// another process reading the directory and writing its answer, and the
    /// question that landed in between stays invisible to the fast path until
    /// something else happens to be asked.
    fn lock_index(&self) -> Option<std::fs::File> {
        let _ = std::fs::create_dir_all(self.dir());
        crate::events::lock_exclusive(&self.dir().join("pending.lock"))
    }

    fn pending_ids<'a>(msgs: impl Iterator<Item = &'a Msg>) -> Vec<String> {
        msgs.filter(|m| m.pending()).map(|m| m.id.clone()).collect()
    }

    fn write_index(&self, ids: &[String]) {
        let mut text = String::with_capacity(ids.len() * 24);
        for id in ids {
            text.push_str(id);
            text.push('\n');
        }
        let _ = crate::cache::write_atomic(&self.index(), text.as_bytes());
    }

    fn read_index(&self) -> Option<Vec<String>> {
        let text = std::fs::read_to_string(self.index()).ok()?;
        let mut seen = std::collections::HashSet::new();
        Some(
            text.lines()
                .map(str::trim)
                .filter(|id| valid_id(id))
                .filter(|id| seen.insert(id.to_string()))
                .map(str::to_string)
                .collect(),
        )
    }

    /// One `O_APPEND` line under the lock, on `task::note_open`'s reasoning: the
    /// kernel resolves the offset and the write together, so two agents asking at
    /// once cannot land on the same bytes, and the lock is what orders this write
    /// against a rewrite rather than only against other appends.
    fn note_pending(&self, id: &str) {
        let _lock = self.lock_index();
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut handle) = options.open(self.index()) {
            use std::io::Write;
            let _ = handle.write_all(format!("{id}\n").as_bytes());
        }
    }

    /// Reconstruct the index from the directory, and leave it behind.
    ///
    /// This is the expensive read, so the one thing it must not do is happen
    /// twice: the caller that paid for the scan gets the messages back rather
    /// than walking the directory again.
    pub(crate) fn rebuild_index(&self) -> Vec<Msg> {
        let _lock = self.lock_index();
        let msgs = self.all();
        self.write_index(&Self::pending_ids(msgs.iter()));
        msgs
    }

    /// Ask another process to rebuild the index, and do not wait for it.
    ///
    /// Never under `cargo test`: the running executable there is the test binary,
    /// and re-invoking it with `_refresh bus-index` would run the suite again
    /// rather than repair anything.
    fn repair_detached(&self) {
        if cfg!(test) || self.root != crate::paths::data() || !self.dir().exists() {
            // No bus at all is not a lost index. There is nothing to reconstruct
            // and nobody to tell.
            return;
        }
        crate::cache::spawn_self(&["_refresh", "bus-index"]);
    }

    /// Narrate a message operation into the structured event log.
    ///
    /// Only when the message carries a Task edge: `events` is a Task's history,
    /// and a record belonging to no task could never be found there again. A
    /// message without one is already recorded — it is a file.
    fn record(&self, m: &Msg, kind: &str, detail: &str) {
        let Some(task) = m.task.as_deref().filter(|t| !t.is_empty()) else { return };
        let event = crate::events::Event::new(kind, task)
            .agent(&m.from)
            .project(&m.from_project)
            .run(m.run.as_deref().unwrap_or_default())
            .session(m.session.as_deref().unwrap_or_default())
            .message(&m.id)
            .detail(detail);
        // The message file is the authority; its narration failing must not undo
        // a state change that is already on disk.
        let _ = self.log().append(&event);
    }
}

// ---------------------------------------------------------------------------
// What may be stored
// ---------------------------------------------------------------------------

/// The credential rule for a message's own words.
///
/// `events::redact` is the broad filter, and it is right for a note that
/// lands in the event log: there, the word "token" in a sentence costs one
/// line of narration. Here it would cost the question itself — "should I
/// rotate the API key?" is exactly the kind of thing an agent stops to ask,
/// and replacing it with a marker leaves the person a decision with no
/// subject. So text uses `looks_secret_material`, the stronger signal
/// `capability.rs` already reaches for in the same situation, and keeps its
/// line structure, because a question is often a list.
fn clean(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if !out.is_empty() {
            out.push('\n');
        }
        if crate::secrets::looks_secret_material(line) {
            out.push_str(REDACTED);
        } else {
            out.push_str(line);
        }
        if out.chars().count() >= TEXT_MAX {
            break;
        }
    }
    out.chars().take(TEXT_MAX).collect::<String>().trim_end().to_string()
}

/// A note that will also travel into the event log, so it takes the log's
/// broader filter and the log's shorter bound.
fn clean_note(text: &str) -> String {
    crate::events::redact(text, NOTE_MAX).unwrap_or_default()
}

/// Canonical, existing, non-credential paths — and never their contents.
///
/// Existence is checked here because a path that does not resolve is a
/// message the recipient cannot act on, and finding that out an hour later in
/// another agent's conversation is the expensive way. `looks_secret` is the
/// broad filter deliberately: `~/.aws/credentials` and `secrets/prod.env` are
/// not files to name in a message that gets typed into a terminal.
///
/// A control character in a path is refused outright rather than stripped.
/// These paths are appended to a line that `deliver` types into somebody
/// else's terminal, so a newline inside one is a second submitted input with
/// no attribution on it — the same hole as an embedded newline in the text,
/// arriving by the other door. A file whose name really does contain a
/// newline cannot be named safely in a message, and refusing to attach it is
/// the honest answer.
pub fn attachments(paths: &[String]) -> Result<Vec<String>, String> {
    if paths.len() > ATTACH_MAX {
        return Err(format!("more than {ATTACH_MAX} attachments"));
    }
    let mut out = Vec::new();
    for p in paths {
        if crate::secrets::looks_secret(p) {
            return Err(format!("{p} reads as a credential path — not attaching it"));
        }
        if p.chars().any(char::is_control) {
            return Err("an attachment path with a control character in it cannot be named safely"
                .to_string());
        }
        let full = std::fs::canonicalize(p).map_err(|_| format!("no such file: {p}"))?;
        let full = full.to_string_lossy().into_owned();
        if crate::secrets::looks_secret(&full) {
            return Err(format!("{full} reads as a credential path — not attaching it"));
        }
        if full.chars().any(char::is_control) {
            return Err("an attachment path with a control character in it cannot be named safely"
                .to_string());
        }
        out.push(full);
    }
    Ok(out)
}

/// An optional stored field, filtered and bounded exactly like a note.
///
/// It used to be `trim()` and nothing more, which is the same hole `task.rs`
/// closed in its own `opt` and which was never closed here: `--task`, `--run`,
/// `--session` and `--reply-to` all arrive from a command line, are stored
/// verbatim, and are serialized straight back out by `inbox --json` and
/// `thread --json`. `--task="t sk-…"` reached the message file, and a 600 000
/// character `--task` made a 600 KB message that every reader then paid for.
/// Nothing that arrives from a command line is exempt.
fn opt(value: &str) -> Option<String> {
    crate::events::redact(value, crate::events::FIELD_MAX)
}

/// A note — a cancellation reason, a handoff line — as an optional field.
/// The log's broader filter and the log's longer bound, because this is prose
/// rather than an id.
fn note_opt(value: &str) -> Option<String> {
    crate::events::redact(value, NOTE_MAX)
}

// ---------------------------------------------------------------------------
// Who is calling
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Who {
    /// The agent binary we are running underneath, or empty at a bare shell.
    pub agent: String,
    pub pane: String,
    pub cwd: String,
    pub project: String,
}

/// Work out who is running this command, without being told.
///
/// The three signals are all free. `$TMUX_PANE` is exported into every pane,
/// so an agent calling us from one hands over a reply address without knowing
/// it did. `$PWD` names the project. And the process tree says which agent we
/// are underneath — an agent's tool call runs `sh -c "prelude …"`, so the
/// agent is our grandparent rather than our parent, and the walk has to climb
/// rather than look once.
pub fn whoami() -> Who {
    let cwd = crate::paths::cwd()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let project = cwd.rsplit('/').next().unwrap_or("").to_string();
    Who {
        agent: enclosing_agent().unwrap_or_default(),
        pane: std::env::var("TMUX_PANE").unwrap_or_default(),
        cwd,
        project,
    }
}

/// Climb the process tree looking for an agent binary.
///
/// One bulk `ps` and an in-memory walk: asking `ps` once per level would be
/// six subprocesses for an answer that is already in the first one's output.
fn enclosing_agent() -> Option<String> {
    let out = crate::exec::run(&["ps", "-Ao", "pid=,ppid=,comm="], Duration::from_secs(5));
    let mut parent: std::collections::HashMap<u32, (u32, String)> = std::collections::HashMap::new();
    for line in out.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(ppid)) = (it.next(), it.next()) else { continue };
        let comm = it.collect::<Vec<_>>().join(" ");
        let base = comm.rsplit('/').next().unwrap_or(&comm).to_string();
        if let (Ok(p), Ok(pp)) = (pid.parse(), ppid.parse()) {
            parent.insert(p, (pp, base));
        }
    }
    let names: Vec<&str> = crate::sources::sessions::AGENTS.iter().map(|a| a.name).collect();
    let mut pid = std::process::id();
    // Bounded: a broken tree with a cycle must not spin forever, and no real
    // agent is more than a handful of processes above its own tool call.
    for _ in 0..12 {
        let (pp, comm) = parent.get(&pid)?;
        if let Some(n) = names.iter().find(|n| *n == comm) {
            return Some((*n).to_string());
        }
        if *pp <= 1 {
            return None;
        }
        pid = *pp;
    }
    None
}

// ---------------------------------------------------------------------------
// What a sender may say about a message, beyond its words
// ---------------------------------------------------------------------------

/// Everything optional about a message.
///
/// Deliberately none of it is identity: there is no `--from` and there never
/// will be, because a name a sender can declare is a name it can get wrong,
/// and a message attributed to the wrong agent is worse than no message.
/// These are edges and deadlines — facts about the *work*, which only the
/// caller knows.
#[derive(Clone, Debug, Default)]
pub struct Opts {
    /// Absolute paths, checked and canonicalized at send time.
    pub attach: Vec<String>,
    /// Seconds from now, or 0 for no deadline.
    pub expires: u64,
    pub task: String,
    pub run: String,
    pub session: String,
    /// The exchange to join. Empty opens a new one.
    pub thread: String,
    pub reply_to: String,
}

impl Bus {
    /// `--attach=PATH --expires=N --task=ID --run=ID --session=ID --reply-to=ID`.
    ///
    /// The `=` form only, and never a separate value word: the agent-facing verbs
    /// take their text last and unquoted, so a flag that swallows the next word
    /// would eat the first word of the message. That is the same trap `lend.rs`
    /// records for `--mcp-config`.
    pub(crate) fn parse_opts(&self, flags: &[&str]) -> Result<Opts, String> {
        let mut o = Opts::default();
        for f in flags {
            let Some((name, value)) = f.split_once('=') else { continue };
            match name {
                "--attach" => o.attach.push(value.to_string()),
                "--expires" => {
                    o.expires = value
                        .parse()
                        .map_err(|_| format!("{value} is not a number of seconds"))?
                }
                "--task" => o.task = value.to_string(),
                "--run" => o.run = value.to_string(),
                "--session" => o.session = value.to_string(),
                "--reply-to" => o.reply_to = value.to_string(),
                _ => {}
            }
        }
        // Replying joins the thread the other message is in, without the caller
        // having to know its name.
        if o.thread.is_empty() {
            if let Some(parent) = self.get(&o.reply_to) {
                o.thread = parent.thread_id().to_string();
            }
        }
        Ok(o)
    }
}

/// The half of a message that does not depend on who it is for.
fn base(kind: &str, me: &Who, opts: &Opts) -> Result<Msg, String> {
    let attach = attachments(&opts.attach)?;
    let id = new_id();
    // Bounded like every other id here. Nothing on a command line sets it
    // today, but it is written into every message in an exchange, so an
    // unbounded one would be unbounded once per message rather than once.
    let thread = match opt(&opts.thread) {
        None => id.clone(),
        Some(t) => t,
    };
    Ok(Msg {
        id,
        kind: kind.to_string(),
        from: if me.agent.is_empty() { "shell".into() } else { me.agent.clone() },
        from_project: me.project.clone(),
        from_cwd: me.cwd.clone(),
        from_pane: me.pane.clone(),
        ts: now(),
        thread,
        reply_to: opt(&opts.reply_to),
        // Saturating, because `--expires=18446744073709551615` otherwise wraps
        // in release and the question is born already expired, and panics
        // under the overflow checks the test profile turns on — against this
        // module's own rule that nothing here ever panics. A deadline pinned
        // at the end of time is the honest reading of "never expire".
        deadline: (opts.expires > 0).then(|| now().saturating_add(opts.expires)),
        attach,
        task: opt(&opts.task),
        run: opt(&opts.run),
        session: opt(&opts.session),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// The verbs
// ---------------------------------------------------------------------------

impl Bus {
    /// `prelude ask "…"` — put a question to the human and wait for the answer.
    ///
    /// stdout carries the answer and nothing else, so `ans=$(prelude ask …)` is
    /// the whole integration. Everything else goes to stderr. Exit 0 when
    /// answered, 3 when it timed out — distinct, so a script can tell "they said
    /// no" from "nobody was there".
    ///
    /// The edges and the deadline a caller may know about arrive in `opts`; the
    /// caller with none passes `Opts::default()`, which is what `parse_opts` of
    /// no flags returns, so there is no second entry point to keep in step.
    ///
    /// `--expires` is not the same number as `--timeout`: the timeout is how long
    /// *this process* blocks, and the deadline is how long the question is worth
    /// answering. A `--no-wait` question blocks for none of it and can still go
    /// stale, which is the case the deadline exists for.
    pub(crate) fn ask_with(
        &self,
        me: &Who,
        text: &str,
        wait: u64,
        no_wait: bool,
        opts: Opts,
    ) -> i32 {
        let mut m = match base("ask", me, &opts) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("prelude: {e}");
                return 2;
            }
        };
        m.to = "human".into();
        m.text = clean(text);
        m.set(State::Sent);
        if let Err(e) = self.save(&m) {
            eprintln!("prelude: could not post the question: {e}");
            return 2;
        }
        self.record(&m, "msg.ask", &m.text);
        // The one place a pending question is created, and so the one place the
        // index has to grow. The sweep that follows rewrites it from the
        // directory anyway; this line is what keeps the fast path correct if that
        // sweep, or the process, does not get that far.
        self.note_pending(&m.id);
        self.sweep();
        post(&format!("{} asks", m.label()), &m.text);
        // Nudge every attached tmux client so a status bar showing the count
        // updates now rather than at its next poll.
        crate::exec::run(&["tmux", "refresh-client", "-S"], Duration::from_secs(1));

        if no_wait {
            eprintln!("prelude: asked — collect the answer with  prelude answer-of {}", m.id);
            println!("{}", m.id);
            return 0;
        }

        eprintln!("prelude: waiting for an answer ({}s)…", wait);
        let deadline = std::time::Instant::now() + Duration::from_secs(wait);
        while std::time::Instant::now() < deadline {
            if let Some(cur) = self.get(&m.id) {
                if let Some(a) = cur.answer {
                    println!("{a}");
                    return 0;
                }
                // Withdrawn or gone stale: waiting on it is waiting on nothing.
                match cur.state() {
                    State::Cancelled => {
                        eprintln!("prelude: the question was cancelled");
                        return 3;
                    }
                    State::Expired => {
                        eprintln!(
                            "prelude: the question expired unanswered — it is still {}",
                            m.id
                        );
                        return 3;
                    }
                    _ => {}
                }
            }
            std::thread::sleep(POLL);
        }
        eprintln!(
            "prelude: no answer within {wait}s — it is still in the inbox as {}",
            m.id
        );
        3
    }

    /// `prelude answer-of <id>` — collect a `--no-wait` answer later.
    pub(crate) fn answer_of(&self, id: &str) -> i32 {
        match self.get(id) {
            None => {
                eprintln!("prelude: no message {id}");
                2
            }
            Some(m) => match m.answer {
                Some(a) => {
                    println!("{a}");
                    0
                }
                None => {
                    // Why there is no answer is worth a line: "nobody has got to
                    // it" and "you withdrew it" are different problems.
                    match m.state() {
                        State::Cancelled => eprintln!("prelude: {id} was cancelled"),
                        State::Expired => eprintln!("prelude: {id} expired unanswered"),
                        _ => {}
                    }
                    3
                }
            },
        }
    }

    /// `prelude tell "…"` — a notice, with nothing to wait for.
    pub(crate) fn tell_with(&self, me: &Who, text: &str, opts: Opts) -> i32 {
        let mut m = match base("tell", me, &opts) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("prelude: {e}");
                return 2;
            }
        };
        m.to = "human".into();
        m.text = clean(text);
        // A notice needs no reply. It is delivered the moment the notification
        // goes out — never "answered", which is a word reserved for a decision
        // somebody actually made.
        m.set(State::Delivered);
        let _ = self.save(&m);
        self.record(&m, "msg.tell", &m.text);
        post(&m.label(), &m.text);
        0
    }

    /// `prelude say <target> <text>` — one agent to another.
    ///
    /// Delivery is direct when the target has a pane: the text is typed into it
    /// and submitted, because unlike a person at a keyboard the sender here is
    /// deliberately sending a message and there is nobody to press Enter for it.
    /// An agent with no pane cannot be typed into at all, so the message waits in
    /// its inbox for the next time it looks — marked undelivered, so that "it is
    /// in their inbox" is never mistaken for "they have it".
    pub(crate) fn say_with(&self, me: &Who, target: &str, text: &str, opts: Opts) -> i32 {
        if target == "human" {
            return self.tell_with(me, text, opts);
        }
        self.say_to(me, &crate::sources::running::live(), target, text, opts)
    }

    /// `say`, against a fleet the caller already has. The split exists so the
    /// exact-recipient rule can be tested without a machine full of agents.
    fn say_to(&self, me: &Who, runs: &[Item], target: &str, text: &str, opts: Opts) -> i32 {
        let hit = match Self::one(runs, target) {
            Ok(h) => h,
            Err(code) => return code,
        };
        let mut m = match base("say", me, &opts) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("prelude: {e}");
                return 2;
            }
        };
        m.text = clean(text);
        Self::address(&mut m, &hit);
        match self.hand_over(&mut m, me) {
            true => eprintln!("prelude: delivered to {} ({})", m.to, hit.get("addr")),
            false => {
                eprintln!("prelude: {} is not in a tmux pane — left in its inbox instead", m.to)
            }
        }
        0
    }

    /// Exactly one recipient, or an explanation and an exit code.
    ///
    /// Guessing between two agents is worse than refusing: the message would land
    /// in the wrong conversation and read as the human's own words.
    fn one(runs: &[Item], target: &str) -> Result<Item, i32> {
        let hits = resolve(runs, target);
        match hits.len() {
            1 => Ok(hits[0].clone()),
            0 => {
                eprintln!("prelude: nothing running matches {target:?}");
                eprintln!("prelude: try one of:");
                for r in runs.iter().take(12) {
                    eprintln!("  {} · {}  ({})", r.get("agent"), r.get("project"), r.get("addr"));
                }
                Err(2)
            }
            _ => {
                eprintln!("prelude: {target:?} matches more than one — be specific:");
                for r in &hits {
                    eprintln!("  {} · {}  ({})", r.get("agent"), r.get("project"), r.get("addr"));
                }
                Err(2)
            }
        }
    }

    fn address(m: &mut Msg, hit: &Item) {
        m.to = format!("{} · {}", hit.get("agent"), hit.get("project"));
        m.to_pane = hit.get("pane").to_string();
        m.to_cwd = hit.get("cwd").to_string();
        m.to_agent = hit.get("agent").to_string();
    }

    /// Type it into the recipient's pane if there is one, save it either way, and
    /// say which of the two happened.
    ///
    /// Saving first is deliberate: a message that reached the pane but never
    /// reached the disk cannot be answered, threaded or reported against, and the
    /// tmux call is the part most likely to fail.
    fn hand_over(&self, m: &mut Msg, me: &Who) -> bool {
        let from = if me.agent.is_empty() { "you".to_string() } else { me.label_of() };
        m.set(State::Undelivered);
        let _ = self.save(m);
        if m.to_pane.is_empty() {
            self.record(m, "msg.undelivered", &m.to);
            return false;
        }
        deliver(&m.to_pane, &from, &m.text, &m.attach);
        m.set(State::Delivered);
        let _ = self.save(m);
        self.record(m, "msg.delivered", &m.to);
        true
    }
}

/// Type a line into another agent's pane, and press Enter for it.
///
/// The attribution is not decoration: without it the receiving agent reads a
/// peer's message as its owner's, and answers a question nobody asked. And
/// attachments go over as paths — the recipient reads the file itself, which
/// is the whole reason contents are never copied into a message.
///
/// **One message is one line.** `clean` deliberately keeps the stored text's
/// line structure, because a question worth stopping for is often a list —
/// but a line typed into a pane is submitted the moment it reaches a newline,
/// so a two-line message arrived as *two* submitted inputs and only the first
/// carried the attribution. The second read as the owner's own words, which
/// is precisely what the attribution exists to prevent, and `hand_over` still
/// reported it all as delivered. So the delivered form is flattened, and the
/// remaining control characters go with it: an escape sequence typed into a
/// TUI is not text at all. The stored message keeps its lines; this is the
/// wire format, and only this.
fn delivered_line(from: &str, text: &str, attach: &[String]) -> String {
    let mut line = format!("[via prelude, from {from}] {text}");
    if !attach.is_empty() {
        line.push_str(&format!(" (attached, read them yourself: {})", attach.join(" ")));
    }
    crate::width::flatten(&line).chars().filter(|c| !c.is_control()).collect()
}

fn deliver(pane: &str, from: &str, text: &str, attach: &[String]) {
    let line = delivered_line(from, text, attach);
    let d = Duration::from_secs(5);
    crate::exec::run(&["tmux", "send-keys", "-t", pane, "-l", &line], d);
    crate::exec::run(&["tmux", "send-keys", "-t", pane, "Enter"], d);
}

impl Who {
    fn label_of(&self) -> String {
        match (self.agent.as_str(), self.project.as_str()) {
            ("", p) => p.to_string(),
            (a, "") => a.to_string(),
            (a, p) => format!("{a} · {p}"),
        }
    }
}

/// Match a free-text target against the running fleet.
///
/// Deliberately several ways at once — an agent writing `prelude say
/// api-gateway …` should not have to know whether that is a project, a pane
/// or a session name. Exact matches win outright so that a project literally
/// called `claude` cannot be shadowed by every claude on the machine.
pub fn resolve(runs: &[Item], target: &str) -> Vec<Item> {
    let t = target.trim().to_lowercase();
    let exact: Vec<Item> = runs
        .iter()
        .filter(|r| {
            r.get("addr").eq_ignore_ascii_case(&t)
                || r.get("pane").eq_ignore_ascii_case(&t)
                || r.get("project").eq_ignore_ascii_case(&t)
                || r.get("pid") == t
        })
        .cloned()
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    runs.iter()
        .filter(|r| {
            r.get("agent").eq_ignore_ascii_case(&t)
                || r.get("project").to_lowercase().contains(&t)
                || r.get("cwd").to_lowercase().contains(&t)
        })
        .cloned()
        .collect()
}

impl Bus {
    /// `prelude answer <id> <text>` — the return path.
    pub(crate) fn answer(&self, id: &str, text: &str) -> i32 {
        let Some(mut m) = self.get(id) else {
            eprintln!("prelude: no message {id}");
            return 2;
        };
        if m.answer.is_some() {
            eprintln!("prelude: {id} was already answered");
            return 2;
        }
        if m.state() == State::Cancelled {
            eprintln!("prelude: {id} was cancelled by whoever asked it");
            return 2;
        }
        // An expired question can still be answered — the deadline says it stopped
        // demanding attention, not that the answer stopped being useful — but the
        // person deserves to know nobody may be listening any more.
        if m.state() == State::Expired {
            eprintln!("prelude: {id} had expired; answering it anyway");
        }
        m.answer = Some(clean(text));
        m.answered = Some(now());
        m.set(State::Answered);
        if let Err(e) = self.save(&m) {
            eprintln!("prelude: could not save the answer: {e}");
            return 2;
        }
        self.record(&m, "msg.answered", m.answer.as_deref().unwrap_or_default());
        crate::exec::run(&["tmux", "refresh-client", "-S"], Duration::from_secs(1));
        eprintln!("prelude: answered {} — {}", m.label(), m.id);
        0
    }

    /// `prelude cancel <id> [reason]` — withdraw a question that no longer needs
    /// an answer.
    ///
    /// Not gated on being the sender. Identity here is discovered rather than
    /// proved, so a check would refuse the person cancelling from the next window
    /// along as readily as it would refuse a stranger — and there are no
    /// strangers: the bus is one user's own data directory.
    pub(crate) fn cancel(&self, id: &str, reason: &str) -> i32 {
        let Some(mut m) = self.get(id) else {
            eprintln!("prelude: no message {id}");
            return 2;
        };
        if m.answer.is_some() {
            eprintln!("prelude: {id} was already answered");
            return 2;
        }
        if m.state() == State::Cancelled {
            eprintln!("prelude: {id} was already cancelled");
            return 2;
        }
        m.set(State::Cancelled);
        m.reason = note_opt(reason);
        if let Err(e) = self.save(&m) {
            eprintln!("prelude: could not cancel it: {e}");
            return 2;
        }
        self.record(&m, "msg.cancelled", m.reason.as_deref().unwrap_or_default());
        // The status bar counts pending questions; this one has stopped being one.
        crate::exec::run(&["tmux", "refresh-client", "-S"], Duration::from_secs(1));
        eprintln!("prelude: cancelled {id}");
        0
    }

    /// `prelude reassign <id> <target>` — point a message at a different agent.
    ///
    /// The original is not rewritten. It is cancelled with a reason naming its
    /// replacement, and the replacement carries `reply_to` back to it in the same
    /// thread — so the exchange reads as what happened rather than as though the
    /// second agent had been the recipient all along.
    pub(crate) fn reassign(&self, me: &Who, id: &str, target: &str) -> i32 {
        self.reassign_to(me, &crate::sources::running::live(), id, target)
    }

    fn reassign_to(&self, me: &Who, runs: &[Item], id: &str, target: &str) -> i32 {
        let Some(mut old) = self.get(id) else {
            eprintln!("prelude: no message {id}");
            return 2;
        };
        if old.kind == "ask" {
            eprintln!("prelude: {id} is a question for a person — answer or cancel it");
            return 2;
        }
        if old.state().collected() {
            eprintln!("prelude: {id} has already been {} — too late", old.state().as_str());
            return 2;
        }
        let hit = match Self::one(runs, target) {
            Ok(h) => h,
            Err(code) => return code,
        };
        let opts = Opts {
            thread: old.thread_id().to_string(),
            reply_to: old.id.clone(),
            task: old.task.clone().unwrap_or_default(),
            run: old.run.clone().unwrap_or_default(),
            session: old.session.clone().unwrap_or_default(),
            attach: old.attach.clone(),
            ..Default::default()
        };
        let mut m = match base(&old.kind, me, &opts) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("prelude: {e}");
                return 2;
            }
        };
        m.text = old.text.clone();
        Self::address(&mut m, &hit);
        let delivered = self.hand_over(&mut m, me);

        let was = old.to.clone();
        old.set(State::Cancelled);
        old.reason = Some(format!("reassigned to {} as {}", m.to, m.id));
        let _ = self.save(&old);
        self.record(&old, "msg.reassigned", &format!("from {was} to {}", m.to));
        eprintln!(
            "prelude: {id} reassigned from {was} to {} as {}{}",
            m.to,
            m.id,
            if delivered { "" } else { " (waiting in its inbox)" }
        );
        println!("{}", m.id);
        0
    }

    /// `prelude handoff <target> <task> [note]` — give another agent the work.
    ///
    /// The Task keeps its id, its title and its history — `task::assign` is the
    /// same operation as the first assignment — and the message that carries it
    /// joins whatever thread the Task already came out of. That thread is how the
    /// result finds its way back: `report_task` replies into it when the work
    /// finishes, so the agent that handed the work over hears about it without
    /// having to poll anything.
    ///
    /// It refuses on anything but exactly one resolved recipient, exactly as
    /// `say` does. A handoff into the wrong conversation is worse than a failed
    /// one: the wrong agent starts work nobody asked it for, and the right one
    /// never hears about it.
    pub(crate) fn handoff(&self, me: &Who, target: &str, task_id: &str, note: &str) -> i32 {
        self.handoff_to(me, &crate::sources::running::live(), target, task_id, note)
    }

    fn handoff_to(&self, me: &Who, runs: &[Item], target: &str, task_id: &str, note: &str) -> i32 {
        // The recipient is resolved before the Task is touched: refusing after
        // reassigning it would leave the work owned by an agent nobody told.
        let hit = match Self::one(runs, target) {
            Ok(h) => h,
            Err(code) => return code,
        };
        let agent = hit.get("agent").to_string();
        let task = match self.tasks().assign(task_id, &agent) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("prelude: {e}");
                return 2;
            }
        };
        // Join the exchange this work already belongs to, when it has one.
        let thread = task
            .message
            .as_deref()
            .and_then(|id| self.get(id))
            .map(|m| m.thread_id().to_string())
            .unwrap_or_default();
        let opts = Opts {
            task: task.id.clone(),
            reply_to: task.message.clone().unwrap_or_default(),
            thread,
            ..Default::default()
        };
        let mut m = match base("say", me, &opts) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("prelude: {e}");
                return 2;
            }
        };
        let note = clean_note(note);
        m.text = match note.as_str() {
            "" => format!("handing over task {}: {}", task.id, task.title),
            n => format!("handing over task {}: {} — {n}", task.id, task.title),
        };
        Self::address(&mut m, &hit);
        let delivered = self.hand_over(&mut m, me);
        self.record(&m, "msg.handoff", &format!("to {} · {}", agent, hit.get("project")));
        self.supersede(&task, &m);
        // The Task now points at the message that handed it over, which is what
        // `report_task` follows home.
        if let Err(e) = self.tasks().link(&task.id, "", "", &m.id) {
            eprintln!("prelude: handed over, but could not record the message edge: {e}");
        }
        eprintln!(
            "prelude: task {} handed to {}{}",
            task.id,
            m.to,
            if delivered { "" } else { " (waiting in its inbox)" }
        );
        println!("{}", m.id);
        0
    }

    /// Withdraw the handoff this one replaces.
    ///
    /// A Task has exactly one `message` edge and a second handoff moves it, so
    /// the first recipient was left holding an instruction nobody would ever
    /// withdraw: `Undelivered` in its inbox for ever, telling it to do work that
    /// somebody else has been given, while only the second agent is sent the
    /// result. Both agents are told to do the job. `task::retry` names this
    /// hazard in its own doc — two agents on one job, and no way for either to
    /// know — and `reassign` already cancels the message it replaces; this is the
    /// same act on the path that had been missing it.
    ///
    /// Deliberately narrow. Only a previous *handoff of this task* is withdrawn:
    /// the first time, `task.message` is the question the work came out of, and
    /// cancelling that would withdraw a person's question and break the thread
    /// the result has to come home to.
    fn supersede(&self, task: &crate::task::Task, replacement: &Msg) {
        let Some(previous) = task
            .message
            .as_deref()
            .filter(|id| *id != replacement.id)
            .and_then(|id| self.get(id))
        else {
            return;
        };
        let was_a_handoff = previous.kind == "say"
            && previous.to != "human"
            && previous.task.as_deref() == Some(task.id.as_str());
        if !was_a_handoff || matches!(previous.state(), State::Cancelled | State::Answered) {
            return;
        }
        let mut previous = previous;
        let was = previous.to.clone();
        previous.set(State::Cancelled);
        previous.reason =
            Some(format!("superseded by the handoff to {} as {}", replacement.to, replacement.id));
        let _ = self.save(&previous);
        self.record(&previous, "msg.superseded", &format!("from {was} to {}", replacement.to));
    }

    /// A finished Task, reported back to whoever handed it over.
    ///
    /// `task.rs` calls this from `finish`, so a Task that reaches `done` or
    /// `failed` answers the message that started it without the agent doing the
    /// work having to know who to tell. The reply lands in the same thread and in
    /// the original sender's inbox — a person's if a person opened it, the
    /// sender's pane if an agent did.
    ///
    /// Returns the id of the message it posted, or `None` when nothing handed
    /// this Task over: work nobody delegated has nobody to report to.
    pub(crate) fn report_task(&self, me: &Who, task: &crate::task::Task) -> Option<String> {
        use crate::task::State as TaskState;
        if !matches!(task.state, TaskState::Done | TaskState::Failed) {
            return None;
        }
        let origin = self.get(task.message.as_deref().unwrap_or_default())?;
        let outcome = if task.state == TaskState::Done { "done" } else { "failed" };
        let body = task
            .result
            .as_deref()
            .or(task.reason.as_deref())
            .unwrap_or_default();

        // A person gets a notice; an agent gets a line it can act on. `inbox`
        // sorts the two by exactly this field, so getting it wrong makes the
        // result unreachable from the window it was sent to.
        let to_human = origin.from == "shell" || origin.to == "human";
        let opts = Opts {
            task: task.id.clone(),
            thread: origin.thread_id().to_string(),
            reply_to: origin.id.clone(),
            run: task.run.clone().unwrap_or_default(),
            session: task.session.clone().unwrap_or_default(),
            ..Default::default()
        };
        let mut m = base(if to_human { "tell" } else { "say" }, me, &opts).ok()?;
        m.text = match body {
            "" => format!("task {} {outcome}: {}", task.id, task.title),
            b => format!("task {} {outcome}: {} — {}", task.id, task.title, clean_note(b)),
        };
        if to_human {
            m.to = "human".into();
            m.set(State::Delivered);
            let _ = self.save(&m);
            post(&format!("{} · task {outcome}", m.label()), &m.text);
        } else {
            // Back the way it came: the sender's own pane and project, and — when
            // it had neither — the agent itself. An originator that started
            // outside tmux and has since `cd`'d has no pane and no matching cwd,
            // and a result addressed only by those was invisible to `for_agent`,
            // which guards both matches on being non-empty. The message existed,
            // was open, and was therefore never swept: nobody could read it and
            // nothing could remove it.
            m.to = origin.label();
            m.to_pane = origin.from_pane.clone();
            m.to_cwd = origin.from_cwd.clone();
            m.to_agent = origin.from.clone();
            self.hand_over(&mut m, me);
        }
        self.record(&m, "msg.result", &m.text);
        Some(m.id)
    }
}

impl Bus {
    /// Questions still waiting on a human, oldest first — the true answer, at
    /// whatever it costs.
    ///
    /// The explicit-command reader: `prelude reply`, the tmux status count,
    /// `doctor`. It must never say "nothing is waiting on you" because a file
    /// went missing, so when the index is gone it scans — and leaves the rebuilt
    /// index behind, so the scan is paid once rather than by every reader for
    /// ever. The launcher does not come through here; see `items`.
    pub(crate) fn pending(&self) -> Vec<Msg> {
        let msgs = match self.read_index() {
            Some(ids) => ids.iter().filter_map(|id| self.get(id)).collect(),
            None => self.rebuild_index(),
        };
        let mut out: Vec<Msg> = msgs.into_iter().filter(Msg::pending).collect();
        out.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// The same set, for a caller that has forty milliseconds for everything.
    ///
    /// **This reader never scans, not even once.** `task::home_tasks` records the
    /// trade in full and it is the same one: a foreground rebuild here is not a
    /// slow launch but a launch that has spent its whole budget before the first
    /// subprocess reports, so the repair is handed to a detached process exactly
    /// as the SLOW sources hand over theirs and this launch shows no Msg rows.
    /// One launch shows nothing and every launch after it shows everything, which
    /// beats every launch being over budget — and nothing is lost meanwhile,
    /// because `prelude inbox`, `prelude reply` and the status bar all read
    /// through `pending` and answer truthfully whether or not the index exists.
    fn pending_fast(&self) -> Vec<Msg> {
        let Some(ids) = self.read_index() else {
            self.repair_detached();
            return Vec::new();
        };
        // Filtered again on the way out, and not only for tidiness: the index is
        // a hint that may be a day out of date, so a line in it is a candidate
        // rather than a row.
        let mut out: Vec<Msg> =
            ids.iter().filter_map(|id| self.get(id)).filter(Msg::pending).collect();
        out.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// `prelude reply` — answer the oldest waiting question from any terminal.
    pub(crate) fn reply(&self) -> i32 {
        let p = self.pending();
        let Some(m) = p.first() else {
            eprintln!("prelude: nothing is waiting on you");
            return 0;
        };
        eprintln!("{} asks:", m.label());
        eprintln!("  {}", m.text);
        let Some(line) = crate::ui::prompt_line(&format!(" answer {} ", m.label())) else {
            return 130;
        };
        answer(&m.id, &line)
    }

    /// Every message in one exchange, opener first and every reply after the
    /// message it answers.
    pub(crate) fn thread_of(&self, id: &str) -> Vec<Msg> {
        let Some(seed) = self.get(id) else { return Vec::new() };
        let thread = seed.thread_id().to_string();
        let msgs = self.all().into_iter().filter(|m| m.thread_id() == thread).collect();
        Self::in_reply_order(msgs, &thread)
    }

    /// Order one exchange by the edge that says what answered what.
    ///
    /// `thread --json` is the form an agent reads to pick up handed-over work, so
    /// "oldest first" has to be an order rather than an aspiration — and it was
    /// not one. A question, the handoff it produced and the result that came back
    /// are routinely written inside the same second, `ts` has one-second
    /// resolution, and there was no tiebreak: six identical runs produced three
    /// different orderings, some of them putting the question after the result.
    ///
    /// The data to do it properly is already on every message. `reply_to` is a
    /// tree, so this is a depth-first walk of it from the opener — which puts a
    /// reply immediately after what it replies to, and its own replies after that
    /// — with `(ts, id)` deciding only between siblings. A visited set makes a
    /// cycle in a hand-edited file finite rather than fatal, and anything the
    /// walk cannot reach (a `reply_to` pointing outside the thread, or at a
    /// message since swept) is appended in the same tiebreak order rather than
    /// dropped: this is a listing, and losing a message from it is worse than
    /// showing one out of place.
    fn in_reply_order(mut msgs: Vec<Msg>, thread: &str) -> Vec<Msg> {
        msgs.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
        let present: std::collections::HashSet<&str> = msgs.iter().map(|m| m.id.as_str()).collect();
        let mut children: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        let mut roots: Vec<usize> = Vec::new();
        for (i, m) in msgs.iter().enumerate() {
            match m
                .reply_to
                .as_deref()
                .filter(|parent| *parent != m.id && present.contains(*parent))
            {
                Some(parent) => children.entry(parent.to_string()).or_default().push(i),
                None => roots.push(i),
            }
        }
        // The opener leads: the message the thread is named after, when it is
        // still here, and otherwise the oldest root.
        if let Some(at) = roots.iter().position(|i| msgs[*i].id == thread) {
            let opener = roots.remove(at);
            roots.insert(0, opener);
        }

        let mut order = Vec::with_capacity(msgs.len());
        let mut seen = vec![false; msgs.len()];
        let mut stack: Vec<usize> = roots.into_iter().rev().collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            order.push(i);
            for child in children.get(&msgs[i].id).into_iter().flatten().rev() {
                stack.push(*child);
            }
        }
        for (i, was) in seen.iter().enumerate() {
            if !was {
                order.push(i);
            }
        }
        let mut slots: Vec<Option<Msg>> = msgs.into_iter().map(Some).collect();
        order.into_iter().filter_map(|i| slots[i].take()).collect()
    }
}

/// How many messages of one exchange are printed.
///
/// A thread is the one listing here whose length is decided by other people's
/// messages rather than by the caller: an exchange is a question, a handoff
/// and a result, and a long one is a dozen — but nothing stops an agent in a
/// loop replying into the same thread for an hour. `thread --json` is what
/// another agent parses to pick work up, and handing it an unbounded document
/// is the same mistake as an unbounded field. The opener is always kept,
/// because it is what the exchange is *about*; the newest fill the rest.
const THREAD_MAX: usize = 200;

impl Bus {
    /// `prelude thread <id> [--json]` — the whole exchange, in order.
    ///
    /// The `--json` half is the one that matters: an agent picking up work that
    /// was handed to it needs the question, the handoff and the attachments as
    /// fields, not as a transcript.
    pub(crate) fn thread_cmd(&self, id: &str, json: bool) -> i32 {
        let msgs = self.thread_of(id);
        let total = msgs.len();
        let msgs = bound_thread(msgs);
        if total > msgs.len() {
            eprintln!(
                "prelude: {total} messages in this exchange — showing the opener and the newest {}",
                msgs.len() - 1
            );
        }
        if json {
            println!("{}", serde_json::to_string_pretty(&msgs).unwrap_or_else(|_| "[]".into()));
            return if msgs.is_empty() { 2 } else { 0 };
        }
        if msgs.is_empty() {
            eprintln!("prelude: no message {id}");
            return 2;
        }
        for m in &msgs {
            print_msg(m);
        }
        0
    }
}

fn bound_thread(mut msgs: Vec<Msg>) -> Vec<Msg> {
    if msgs.len() <= THREAD_MAX {
        return msgs;
    }
    let tail = msgs.split_off(msgs.len() - (THREAD_MAX - 1));
    let mut out = vec![msgs.swap_remove(0)];
    out.extend(tail);
    out
}

impl Bus {
    /// `prelude inbox` — what has come in for whoever is asking.
    ///
    /// From an agent this is how it collects messages left by peers that could
    /// not be typed into it. From a person it is the list of questions waiting on
    /// them. Which of the two you get is decided by `whoami` rather than by a
    /// flag, because an agent should not have to know which it is.
    ///
    /// `--human` overrides that. A person working *inside* an agent's terminal
    /// is still a person, and the heuristic — correctly — reads their shell as
    /// the agent's. Without the override the questions would be unreachable from
    /// exactly the window they are most likely to be sitting in.
    pub(crate) fn inbox(&self, me: &Who, json: bool, all_of_them: bool, as_human: bool) -> i32 {
        let msgs = self.all();
        let human = as_human || me.agent.is_empty();
        let mine: Vec<Msg> = if human {
            // A person: everything addressed to a human.
            msgs.into_iter()
                .filter(|m| m.to == "human" && (all_of_them || m.pending()))
                .collect()
        } else {
            for_agent(me, &msgs, all_of_them)
        };

        // Listing an agent's own inbox *is* collecting it: the message has been
        // put in front of the agent, and showing it again on the next poll is how
        // one instruction gets carried out twice. A person's questions are not
        // marked, because reading a question is not answering it and they must
        // keep asking until somebody decides.
        if !human {
            self.mark_read(&mine);
        }

        if json {
            println!("{}", serde_json::to_string_pretty(&mine).unwrap_or_else(|_| "[]".into()));
            return 0;
        }
        if mine.is_empty() {
            println!("nothing waiting");
            return 0;
        }
        for m in &mine {
            print_msg(m);
        }
        0
    }
}

fn print_msg(m: &Msg) {
    let mark = if m.pending() { "?" } else { "·" };
    println!("{mark} {}  {}  [{}]", m.label(), m.text, m.id);
    let state = m.state();
    if state != State::Delivered && state != State::Sent {
        println!("  ({})", state.as_str());
    }
    for path in &m.attach {
        println!("  attached {path}");
    }
    if let Some(a) = m.answer.as_ref().filter(|a| !a.is_empty()) {
        println!("  → {a}");
    }
    if let Some(r) = m.reason.as_ref().filter(|r| !r.is_empty()) {
        println!("  → {r}");
    }
}

/// What was left for this agent: this pane, this project, or — when the
/// message has neither of those — this agent.
///
/// Without `all`, only what it has not collected yet — which now includes a
/// message that was *delivered* into its pane but never acknowledged, because
/// a line typed into a TUI is not proof anybody read it.
///
/// The third clause is the address of last resort and is deliberately the
/// narrowest possible widening: it applies only to a message that carries no
/// pane and no cwd at all, so nothing that already had a real address can be
/// picked up by the wrong agent. Without it a result written for an
/// originator with no tmux and no surviving cwd was invisible to every reader
/// — open, and therefore never swept, so it accumulated for ever while the
/// agent that was waiting for it heard nothing.
pub fn for_agent(me: &Who, msgs: &[Msg], all_of_them: bool) -> Vec<Msg> {
    msgs.iter()
        .filter(|m| {
            let for_me = (!me.pane.is_empty() && m.to_pane == me.pane)
                || (!me.cwd.is_empty() && m.to_cwd == me.cwd)
                || (m.to_pane.is_empty()
                    && m.to_cwd.is_empty()
                    && !me.agent.is_empty()
                    && m.to_agent == me.agent);
            for_me && m.kind == "say" && (all_of_them || !m.state().collected())
        })
        .cloned()
        .collect()
}

/// Is there anywhere at all this message could be collected from?
///
/// A `say` with no pane, no directory and no agent has no reader — not now,
/// and not after any amount of waiting. Distinguishing that from ordinary
/// undelivered mail is what stops the second kind being swept and the first
/// kind accumulating for ever.
fn addressable(m: &Msg) -> bool {
    m.to == "human" || !(m.to_pane.is_empty() && m.to_cwd.is_empty() && m.to_agent.is_empty())
}

impl Bus {
    /// Mark messages collected.
    ///
    /// Read, not answered. The two were the same flag once, which meant an agent
    /// that had merely picked its mail up looked exactly like a person who had
    /// made a decision — and `pending()`, the launcher's whole view of the bus,
    /// is built on that difference.
    fn mark_read(&self, msgs: &[Msg]) -> usize {
        let mut n = 0;
        for m in msgs {
            // Re-read rather than trusting the listing: an answer may have landed
            // between building it and marking it, and a stale copy would write the
            // answer back out of existence.
            let Some(mut cur) = self.get(&m.id) else { continue };
            if cur.state().collected() {
                continue;
            }
            cur.set(State::Read);
            if self.save(&cur).is_ok() {
                self.record(&cur, "msg.read", &cur.to);
                n += 1;
            }
        }
        n
    }

    /// Mark agent-directed messages as collected, so an agent polling its inbox
    /// does not act on the same instruction twice.
    pub(crate) fn drain(&self, me: &Who) -> i32 {
        let msgs = self.all();
        let n = self.mark_read(&for_agent(me, &msgs, false));
        eprintln!("prelude: collected {n}");
        0
    }
}

// ---------------------------------------------------------------------------
// Reaching the human
// ---------------------------------------------------------------------------

/// One line through the notification centre.
///
/// `display notification` is fire-and-forget and needs no permission prompt,
/// which is why it wins over anything with a reply button: a notification
/// that requires the user to have installed something is a notification that
/// does not arrive.
pub fn post(title: &str, body: &str) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape(&crate::width::dtrunc(body, 200)),
        escape(title)
    );
    crate::exec::run(&["osascript", "-e", &script], Duration::from_secs(5));
}

/// Into an AppleScript string literal. Quotes and backslashes are the whole
/// grammar; everything else passes through.
pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------------------
// In the launcher
// ---------------------------------------------------------------------------

impl Bus {
    /// Waiting questions, as rows.
    ///
    /// These outrank everything else on the machine, including a stuck agent: a
    /// run that has gone quiet *might* want you, while one of these has said so.
    pub(crate) fn items(&self) -> Vec<Item> {
        self.pending_fast()
            .into_iter()
            .map(|m| {
                let age = crate::sources::running::short_dur(m.age());
                Item::new(m.text.clone(), Kind::Msg)
                    .title(format!("{} asks", m.label()))
                    .fields([
                        m.from_project.clone(),
                        format!("asked {age} ago"),
                        crate::width::flatten(&m.text),
                    ])
                    .put("id", m.id.clone())
                    .put("agent", m.from.clone())
                    .put("text", m.text.clone())
                    .put("project", m.from_project.clone())
                    .put("pane", m.from_pane.clone())
                    .put("cwd", m.from_cwd.clone())
                    .put("path", m.from_cwd.clone())
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// The env-free API on top
// ---------------------------------------------------------------------------
//
// Everything above takes its root, and its sender, as arguments. These are
// what the CLI, the launcher and the other modules call: they resolve the one
// from the data directory and the other from the process tree, and nothing
// else in the crate has to know that either question was asked.

pub fn all() -> Vec<Msg> {
    bus().all()
}

/// Reconstruct the pending index from the directory. Reached through
/// `prelude _refresh bus-index`.
pub fn rebuild_index() -> Vec<Msg> {
    bus().rebuild_index()
}

pub fn parse_opts(flags: &[&str]) -> Result<Opts, String> {
    bus().parse_opts(flags)
}

pub fn ask_with(text: &str, wait: u64, no_wait: bool, opts: Opts) -> i32 {
    bus().ask_with(&whoami(), text, wait, no_wait, opts)
}

pub fn answer_of(id: &str) -> i32 {
    bus().answer_of(id)
}

pub fn tell_with(text: &str, opts: Opts) -> i32 {
    bus().tell_with(&whoami(), text, opts)
}

pub fn say_with(target: &str, text: &str, opts: Opts) -> i32 {
    bus().say_with(&whoami(), target, text, opts)
}

pub fn answer(id: &str, text: &str) -> i32 {
    bus().answer(id, text)
}

pub fn cancel(id: &str, reason: &str) -> i32 {
    bus().cancel(id, reason)
}

pub fn reassign(id: &str, target: &str) -> i32 {
    bus().reassign(&whoami(), id, target)
}

pub fn handoff(target: &str, task_id: &str, note: &str) -> i32 {
    bus().handoff(&whoami(), target, task_id, note)
}

/// A finished Task, reported back to whoever handed it over. `task::finish`
/// is the only caller, which is why this one is worth its own line: it is the
/// one place another module re-enters the bus.
pub fn report_task(task: &crate::task::Task) -> Option<String> {
    bus().report_task(&whoami(), task)
}

pub fn pending() -> Vec<Msg> {
    bus().pending()
}

pub fn reply() -> i32 {
    bus().reply()
}

pub fn thread_cmd(id: &str, json: bool) -> i32 {
    bus().thread_cmd(id, json)
}

pub fn inbox(json: bool, all_of_them: bool, as_human: bool) -> i32 {
    bus().inbox(&whoami(), json, all_of_them, as_human)
}

pub fn drain() -> i32 {
    bus().drain(&whoami())
}

pub fn items() -> Vec<Item> {
    bus().items()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing;

    // -----------------------------------------------------------------------
    // A private bus, and how these tests reach one
    // -----------------------------------------------------------------------

    thread_local! {
        /// The root `data_root` hands to the env-free wrappers *on this
        /// thread*, and the only reason it exists: `task::Store::finish`
        /// reports a finished Task back through `crate::bus::report_task`,
        /// which resolves its own root, so without this a test driving a
        /// `task::Store::at` a temporary root would have the result posted
        /// into the person's real bus.
        static ROOT: std::cell::RefCell<Option<std::path::PathBuf>> =
            const { std::cell::RefCell::new(None) };
    }

    pub(super) fn thread_root() -> Option<std::path::PathBuf> {
        ROOT.with(|root| root.borrow().clone())
    }

    /// A temporary data root — a bus, a Task store and an event log under one
    /// directory, removed when the guard drops.
    ///
    /// No environment variable is touched: `events::testing::root` only makes
    /// a directory. So any number of these may be held at once on any number
    /// of threads, which is the whole point of the exercise — `set_var`
    /// alongside another thread's `var_os` is undefined behaviour, and a
    /// mutex around the tests that repoint it cannot make it defined.
    struct TestRoot {
        path: std::path::PathBuf,
        _dir: testing::Root,
    }

    impl TestRoot {
        fn tasks(&self) -> crate::task::Store {
            crate::task::Store::at(self.path.clone())
        }
        fn events(&self) -> crate::events::Log {
            crate::events::Log::at(self.path.join("events.jsonl"))
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            ROOT.with(|root| *root.borrow_mut() = None);
        }
    }

    fn fixture(name: &str) -> (TestRoot, Bus) {
        let dir = testing::root(name);
        let path = dir.path.clone();
        ROOT.with(|root| *root.borrow_mut() = Some(path.clone()));
        (TestRoot { path: path.clone(), _dir: dir }, Bus::at(path))
    }

    /// A known sender, injected rather than discovered.
    ///
    /// `whoami` climbs the process tree with `ps`, which is right in
    /// production and wrong here: it costs a subprocess per call, and what it
    /// finds depends on whatever started the suite — including, inside tmux, a
    /// `$TMUX_PANE` that would make `deliver` type these test messages into
    /// the pane running the tests. Every verb takes its sender as a parameter,
    /// so a test says who it is instead of asking the machine.
    fn caller() -> Who {
        Who {
            agent: "claude".into(),
            pane: String::new(),
            cwd: "/tmp/prelude-test/Prelude".into(),
            project: "Prelude".into(),
        }
    }

    /// A fleet row of the shape `running::live` produces. No pane, so nothing
    /// here ever reaches tmux: delivery is exercised through the inbox, which
    /// is the half that has to survive a machine with no terminal multiplexer
    /// at all.
    fn run(agent: &str, project: &str) -> Item {
        Item::new(format!("{agent} {project}"), Kind::Run)
            .put("agent", agent)
            .put("project", project)
            .put("addr", format!("w:{project}"))
            .put("pane", "")
            .put("cwd", format!("/tmp/{project}"))
    }

    fn spec(title: &str) -> crate::task::New {
        crate::task::New { title: title.into(), agent: "claude".into(), ..Default::default() }
    }

    #[test]
    fn a_thread_survives_ask_handoff_and_result() {
        let (root, bus) = fixture("bus-thread");
        // The question that started it all.
        let question = {
            let me = caller();
            let mut m = base("ask", &me, &Opts::default()).expect("base");
            m.to = "human".into();
            m.text = "who should take the migration?".into();
            m.set(State::Sent);
            bus.save(&m).expect("save");
            m
        };
        assert_eq!(question.thread_id(), question.id, "an opener is its own thread");

        let task = root.tasks().start(crate::task::New {
            message: question.id.clone(),
            ..spec("migrate legacy_users")
        })
        .expect("task");

        let runs = vec![run("codex", "api-gateway")];
        let note = "you have the context";
        assert_eq!(bus.handoff_to(&caller(), &runs, "api-gateway", &task.id, note), 0);

        let handed = root.tasks().get(&task.id).expect("task");
        assert_eq!(handed.agent, "codex", "the handoff reassigns the task");
        let handoff_msg = bus.get(handed.message.as_deref().expect("message edge")).expect("msg");
        assert_eq!(handoff_msg.thread_id(), question.id, "the handoff joined the thread");
        assert_eq!(handoff_msg.reply_to.as_deref(), Some(question.id.as_str()));
        assert_eq!(handoff_msg.task.as_deref(), Some(task.id.as_str()));

        // Finishing handed-over work *is* the report: `Store::finish` posts
        // it, so an agent that was given a task answers whoever gave it to
        // them without knowing that it has to. Calling `report_task` here as
        // well would put a second result in the thread.
        root.tasks().done(&task.id, "migrated 3 tables").expect("done");
        let result = bus.all()
            .into_iter()
            .rfind(|m| m.text.contains("migrated 3 tables"))
            .expect("a handed-over task reports back");
        assert_eq!(result.thread_id(), question.id, "the result came home to the thread");

        // One exchange, in order, under one id.
        let thread = bus.thread_of(&result.id);
        let ids: Vec<&str> = thread.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "question, handoff, result: {ids:?}");
        assert!(thread.iter().all(|m| m.thread_id() == question.id));
        assert_eq!(
            ids,
            vec![question.id.as_str(), handoff_msg.id.as_str(), result.id.as_str()],
            "and in that order, whatever the one-second clock says",
        );

        // And the event log is the structured half of the same story.
        let kinds: Vec<String> =
            root.events().for_task(&task.id).into_iter().map(|e| e.kind).collect();
        for expected in ["task.assign", "msg.handoff", "task.done", "msg.result"] {
            assert!(kinds.contains(&expected.to_string()), "{expected} missing from {kinds:?}");
        }
    }

    /// A message written before any of this existed still parses, still reads
    /// as pending, and is still never swept.
    #[test]
    fn an_old_message_file_still_works() {
        let (_root, bus) = fixture("bus-old");
        let old = r#"{
            "id": "1700000000-42",
            "kind": "ask",
            "from": "claude",
            "from_project": "Prelude",
            "from_cwd": "/tmp/Prelude",
            "from_pane": "%3",
            "to": "human",
            "to_pane": "",
            "to_cwd": "",
            "text": "proceed?",
            "ts": 1700000000
        }"#;
        crate::cache::write_atomic(&bus.file_of("1700000000-42"), old.as_bytes()).expect("seed");

        let m = bus.get("1700000000-42").expect("an old file must still parse");
        assert!(m.pending(), "an unanswered question is still pending");
        assert_eq!(m.state(), State::Sent, "no state word means nothing has happened to it");
        assert_eq!(m.thread_id(), m.id, "a message with no thread is its own thread");
        assert!(m.attach.is_empty() && m.task.is_none());
        assert_eq!(bus.pending().len(), 1);

        // The old marker for "handled" is read as delivery, not as an answer.
        let handled = r#"{"id":"1700000001-42","kind":"tell","from":"claude","to":"human",
            "text":"deploy is green","ts":1700000001,"answer":"","answered":1700000001}"#;
        crate::cache::write_atomic(&bus.file_of("1700000001-42"), handled.as_bytes())
            .expect("seed");
        let t = bus.get("1700000001-42").expect("parse");
        assert_eq!(t.state(), State::Delivered, "a notice was never answered by anybody");
        assert!(!t.pending());

        // Sweeping is unchanged by any of it: the question survives, the
        // settled notice does not.
        bus.sweep();
        assert!(bus.get("1700000000-42").is_some(), "an unanswered question is never swept");
        assert!(bus.get("1700000001-42").is_none(), "a day-old notice is history");
    }

    /// Finding 1: a handoff is `Delivered` the moment it reaches the
    /// recipient's pane and `Read` the moment they run `inbox` — both success
    /// paths, and both settled. A day later the sweep took the file, and the
    /// result of the work existed in no message anywhere.
    #[test]
    fn a_day_old_handoff_survives_while_its_task_is_live() {
        let (root, bus) = fixture("bus-live-task");
        let me = caller();
        let mut handoff = base("say", &me, &Opts::default()).expect("base");
        handoff.text = "please take this".into();
        handoff.to = "codex · api-gateway".into();
        handoff.to_cwd = "/tmp/api-gateway".into();
        handoff.to_agent = "codex".into();
        handoff.from = "claude".into();
        handoff.from_cwd = me.cwd.clone();
        // Collected by the agent it was for, and older than the sweep window.
        handoff.set(State::Read);
        handoff.ts = now().saturating_sub(KEEP_SETTLED + 60);
        bus.save(&handoff).expect("save");

        let task = root.tasks()
            .start(crate::task::New { message: handoff.id.clone(), ..spec("rebuild the index") })
            .expect("task");
        handoff.task = Some(task.id.clone());
        bus.save(&handoff).expect("save");

        bus.sweep();
        assert!(bus.get(&handoff.id).is_some(), "the sweep took a live task's own message");

        // …which is the whole point: the completion still finds its way home.
        root.tasks().done(&task.id, "rebuilt, 40k rows").expect("done");
        assert!(
            bus.all().into_iter().any(|m| m.text.contains("rebuilt, 40k rows")),
            "the result of a day-old handed-over task was lost",
        );

        // And once the task is finished the handoff is ordinary history.
        bus.sweep();
        assert!(bus.get(&handoff.id).is_none(), "a finished task's handoff is not kept for ever");
    }

    /// Finding 6: the launcher reads an index of pending questions, never the
    /// directory. `bus::all()` costs about 10 µs a message and the store has
    /// no ceiling, because undelivered mail is never swept.
    #[test]
    fn the_launcher_reads_an_index_rather_than_the_whole_bus() {
        let (_root, bus) = fixture("bus-index");
        let me = caller();
        for n in 0..200 {
            let mut m = base("ask", &me, &Opts::default()).expect("base");
            m.to = "human".into();
            m.text = format!("question {n}");
            m.set(State::Sent);
            bus.save(&m).expect("save");
        }

        // No index yet: the gather path shows nothing and hands the repair to
        // another process rather than walking two hundred files inside a
        // forty millisecond budget.
        assert!(!bus.index().exists());
        assert!(bus.items().is_empty(), "the gather path walked the directory");
        // The explicit reader answers truthfully whatever the index says, and
        // leaves the scan it paid for behind.
        assert_eq!(bus.pending().len(), 200);
        assert!(bus.index().exists(), "the reader that paid for the scan threw it away");
        assert_eq!(bus.items().len(), 200);

        // And it really is the index that is read. A question whose append
        // was lost is a question the fast path cannot see — a hint may be
        // short, it may never be wrong about a line it does have — and the
        // next sweep reconciles it from the directory.
        let mut late = base("ask", &me, &Opts::default()).expect("base");
        late.to = "human".into();
        late.text = "the one that got away".into();
        late.set(State::Sent);
        bus.save(&late).expect("save");
        assert_eq!(bus.items().len(), 200, "items() is reading the directory, not the index");
        bus.sweep();
        assert_eq!(bus.items().len(), 201, "a sweep must reconstruct what an append lost");

        // A question that stops being one leaves the fast path even while the
        // index still names it: the index is a superset, so every line is a
        // candidate rather than a row.
        assert_eq!(bus.answer(&late.id, "go on"), 0);
        assert_eq!(bus.items().len(), 200);
    }

    /// Finding 4: every edge on a message arrives from a command line, and
    /// `inbox --json` serializes all of them straight back out.
    #[test]
    fn an_edge_from_a_command_line_is_filtered_and_bounded() {
        let (_root, bus) = fixture("bus-edges");
        let key = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01";
        let huge = "a".repeat(600_000);
        let flags = [
            format!("--task=t {key}"),
            format!("--run=r {key}"),
            format!("--session=s {huge}"),
            format!("--reply-to=x {key}"),
        ];
        let opts = bus.parse_opts(&flags.iter().map(String::as_str).collect::<Vec<_>>())
            .expect("opts");

        let runs = vec![run("codex", "api-gateway")];
        assert_eq!(bus.say_to(&caller(), &runs, "api-gateway", "have a look", opts), 0);
        let m = bus.all().pop().expect("a message");
        let raw = std::fs::read_to_string(bus.file_of(&m.id)).expect("record");
        assert!(!raw.contains(key), "a message edge kept a credential: {raw}");
        assert!(raw.len() < 8 * 1024, "one message file grew to {} bytes", raw.len());
        // What `inbox --json` and `thread --json` would hand an agent is the
        // same bounded thing, because it is the same record.
        let json = serde_json::to_string(&m).expect("encode");
        assert!(!json.contains(key) && json.len() < 8 * 1024);
    }

    /// Finding 9: `--expires` is a number somebody typed, and `now() + it`
    /// wrapped in release and panicked under the test profile's overflow
    /// checks — against this module's rule that nothing here ever panics.
    #[test]
    fn an_absurd_deadline_does_not_wrap_into_the_past() {
        let (_root, bus) = fixture("bus-expires");
        let opts = bus.parse_opts(&["--expires=18446744073709551615"]).expect("opts");
        let me = caller();
        let m = base("ask", &me, &opts).expect("base");
        assert_eq!(m.deadline, Some(u64::MAX));
        assert!(!m.past_deadline(), "the question was born already expired");
        assert!(m.pending());
    }

    /// Finding 8: a question, the handoff it produced and the result that came
    /// back are routinely written inside one second, and `ts` has one-second
    /// resolution — so six identical runs produced three different orderings.
    /// The ids here sort in exactly the wrong order, so nothing but the
    /// `reply_to` chain can produce the right one.
    #[test]
    fn a_thread_is_ordered_by_its_reply_chain() {
        let (_root, bus) = fixture("bus-order");
        let me = caller();
        let at = now();
        let mut question = base("ask", &me, &Opts::default()).expect("base");
        question.id = "z-question".into();
        question.thread = question.id.clone();
        question.to = "human".into();
        question.text = "who should take the migration?".into();
        question.ts = at;
        question.set(State::Sent);

        let mut handoff = base("say", &me, &Opts::default()).expect("base");
        handoff.id = "b-handoff".into();
        handoff.thread = question.id.clone();
        handoff.reply_to = Some(question.id.clone());
        handoff.to = "codex · api-gateway".into();
        handoff.text = "handing it over".into();
        handoff.ts = at;

        let mut result = base("say", &me, &Opts::default()).expect("base");
        result.id = "a-result".into();
        result.thread = question.id.clone();
        result.reply_to = Some(handoff.id.clone());
        result.to = "claude · Prelude".into();
        result.text = "migrated 3 tables".into();
        result.ts = at;

        for m in [&result, &handoff, &question] {
            bus.save(m).expect("save");
        }

        let ordered: Vec<String> = bus.thread_of(&result.id).into_iter().map(|m| m.id).collect();
        assert_eq!(
            ordered,
            vec!["z-question".to_string(), "b-handoff".into(), "a-result".into()],
            "an exchange must read opener, handoff, result whatever the clock says",
        );
        // The same order from any member of it, six times over: this is the
        // property that was not deterministic at all.
        for _ in 0..6 {
            for seed in ["z-question", "b-handoff", "a-result"] {
                let ids: Vec<String> = bus.thread_of(seed).into_iter().map(|m| m.id).collect();
                assert_eq!(ids, ordered, "the order depends on where you enter the thread");
            }
        }

        // A reply pointing at a message that is not here is still listed —
        // this is a listing, and losing one is worse than showing it late.
        let mut stray = base("say", &me, &Opts::default()).expect("base");
        stray.id = "c-stray".into();
        stray.thread = question.id.clone();
        stray.reply_to = Some("gone".into());
        stray.text = "an orphan reply".into();
        stray.ts = at;
        bus.save(&stray).expect("save");
        assert_eq!(bus.thread_of("z-question").len(), 4);
    }

    /// Finding 12: an exchange is three messages, and nothing stops an agent
    /// in a loop replying into one for an hour.
    #[test]
    fn a_long_exchange_is_bounded_and_keeps_its_opener() {
        let long: Vec<Msg> = (0..THREAD_MAX + 50)
            .map(|n| Msg { id: format!("m{n:04}"), ..Default::default() })
            .collect();
        let bounded = bound_thread(long);
        assert_eq!(bounded.len(), THREAD_MAX);
        assert_eq!(
            bounded.first().map(|m| m.id.as_str()),
            Some("m0000"),
            "the opener is what the exchange is about",
        );
        assert_eq!(bounded.last().map(|m| m.id.as_str()), Some("m0249"));
        let short: Vec<Msg> =
            (0..3).map(|n| Msg { id: format!("s{n}"), ..Default::default() }).collect();
        assert_eq!(bound_thread(short).len(), 3, "an ordinary exchange is untouched");
    }

    /// Finding 10: a Task has one `message` edge, and a second handoff moved
    /// it — so the first recipient held a live instruction for ever while only
    /// the second was sent the result.
    #[test]
    fn a_second_handoff_withdraws_the_first() {
        let (root, bus) = fixture("bus-handoff-twice");
        let task = root.tasks().start(spec("rebuild the index")).expect("task");
        let runs = vec![run("codex", "api-gateway"), run("claude", "docs")];

        assert_eq!(bus.handoff_to(&caller(), &runs, "api-gateway", &task.id, ""), 0);
        let first = root.tasks().get(&task.id).expect("task").message.expect("edge");
        assert_eq!(bus.handoff_to(&caller(), &runs, "docs", &task.id, ""), 0);
        let second = root.tasks().get(&task.id).expect("task").message.expect("edge");
        assert_ne!(first, second, "the edge did not move");

        let old = bus.get(&first).expect("the first handoff survives as a record");
        assert_eq!(
            old.state(),
            State::Cancelled,
            "the first recipient was left holding work somebody else was given",
        );
        assert!(old.reason.as_deref().unwrap_or_default().contains(&second));

        // And it has left that agent's inbox, which is the fact that matters.
        let them = Who {
            agent: "codex".into(),
            cwd: "/tmp/api-gateway".into(),
            ..Default::default()
        };
        assert!(
            for_agent(&them, &bus.all(), false).iter().all(|m| m.id != first),
            "a superseded handoff is still being offered to the first agent",
        );
    }

    /// The other half of finding 10, and the reason it has to be narrow: the
    /// *first* time, `task.message` is the question the work came out of, and
    /// withdrawing that would withdraw a person's question and break the
    /// thread the result has to come home to.
    #[test]
    fn a_first_handoff_leaves_the_question_it_came_from_alone() {
        let (root, bus) = fixture("bus-handoff-question");
        let runs = vec![run("codex", "api-gateway")];
        let me = caller();
        let mut question = base("ask", &me, &Opts::default()).expect("base");
        question.to = "human".into();
        question.text = "who should take the migration?".into();
        question.set(State::Sent);
        bus.save(&question).expect("save");
        let task = root.tasks()
            .start(crate::task::New { message: question.id.clone(), ..spec("migrate") })
            .expect("task");
        assert_eq!(bus.handoff_to(&caller(), &runs, "api-gateway", &task.id, ""), 0);
        assert_eq!(bus.get(&question.id).expect("question").state(), State::Sent);
        assert!(bus.get(&question.id).expect("question").pending());
    }

    /// Finding 11: with no tmux, cwd is the only address — and an originator
    /// that has `cd`'d since the handoff has neither.
    #[test]
    fn a_result_reaches_an_originator_with_no_pane_and_no_cwd() {
        let (root, bus) = fixture("bus-addressless");
        let me = caller();
        let mut handoff = base("say", &me, &Opts::default()).expect("base");
        handoff.text = "please take this".into();
        handoff.to = "codex · api-gateway".into();
        handoff.to_cwd = "/tmp/api-gateway".into();
        handoff.to_agent = "codex".into();
        handoff.from = "claude".into();
        handoff.from_pane = String::new();
        handoff.from_cwd = String::new();
        handoff.set(State::Undelivered);
        bus.save(&handoff).expect("save");

        let task = root.tasks()
            .start(crate::task::New { message: handoff.id.clone(), ..spec("rebuild") })
            .expect("task");
        root.tasks().done(&task.id, "rebuilt, 40k rows").expect("done");

        let result = bus.all()
            .into_iter()
            .rfind(|m| m.text.contains("rebuilt, 40k rows"))
            .expect("a reported result");
        assert!(result.to_pane.is_empty() && result.to_cwd.is_empty());
        assert_eq!(result.to_agent, "claude", "the result has no address at all");
        let them = Who { agent: "claude".into(), ..Default::default() };
        assert!(
            for_agent(&them, &bus.all(), false).iter().any(|m| m.id == result.id),
            "a result with no pane and no cwd was invisible to every inbox",
        );

        // A message nobody could ever collect must not accumulate for ever…
        let mut orphan = base("say", &me, &Opts::default()).expect("base");
        orphan.text = "nobody can read this".into();
        orphan.to = "codex · gone".into();
        orphan.set(State::Undelivered);
        orphan.ts = now().saturating_sub(KEEP_SETTLED + 60);
        bus.save(&orphan).expect("save");
        // …while ordinary undelivered mail, which has somewhere to go, is
        // still never swept however old it is.
        let mut waiting = base("say", &me, &Opts::default()).expect("base");
        waiting.text = "still waiting to be collected".into();
        waiting.to = "codex · api-gateway".into();
        waiting.to_cwd = "/tmp/api-gateway".into();
        waiting.to_agent = "codex".into();
        waiting.set(State::Undelivered);
        waiting.ts = now().saturating_sub(KEEP_SETTLED + 60);
        bus.save(&waiting).expect("save");

        bus.sweep();
        assert!(bus.get(&orphan.id).is_none(), "an unreachable message accumulated for ever");
        assert!(bus.get(&waiting.id).is_some(), "undelivered mail with a reader was swept");
    }

    /// Finding 2: `clean` keeps the stored text's lines on purpose — a
    /// question worth stopping for is often a list — but a line typed into a
    /// pane is submitted at the first newline.
    #[test]
    fn a_delivered_message_is_exactly_one_attributed_line() {
        let (root, _bus) = fixture("bus-deliver");
        let line = delivered_line(
            "claude · Prelude",
            "the migration drops legacy_users\nrun it? yes\rno",
            &[],
        );
        assert_eq!(line.lines().count(), 1, "a message arrived as several inputs: {line:?}");
        assert!(!line.chars().any(char::is_control), "a control character reached the pane");
        assert!(line.starts_with("[via prelude, from claude · Prelude] "));
        assert!(line.contains("run it? yes no"));
        // The stored message keeps its structure; only the wire form is flat.
        assert_eq!(clean("first line\nsecond line").lines().count(), 2);

        // The same hole by the other door: an attachment path is appended to
        // that line, so a newline in one is a second submitted input too.
        let dir = root.path.join("fixtures");
        std::fs::create_dir_all(&dir).expect("fixtures");
        let odd = dir.join("plan\nrm -rf .md");
        std::fs::write(&odd, b"the plan").expect("write");
        assert!(
            attachments(&[odd.to_string_lossy().into_owned()]).is_err(),
            "a path with a newline in it cannot be named safely in a message",
        );
    }

    #[test]
    fn an_expired_question_stops_asking_but_is_not_deleted() {
        let (_root, bus) = fixture("bus-expiry");
        let me = caller();
        let mut m = base("ask", &me, &Opts { expires: 60, ..Default::default() }).expect("base");
        m.to = "human".into();
        m.text = "still worth answering?".into();
        m.set(State::Sent);
        bus.save(&m).expect("save");
        assert!(m.pending(), "a question inside its deadline is pending");

        // Move the deadline into the past rather than the clock forward.
        let mut stale = bus.get(&m.id).expect("msg");
        stale.deadline = Some(now().saturating_sub(1));
        bus.save(&stale).expect("save");

        let stale = bus.get(&m.id).expect("still on disk");
        assert_eq!(stale.state(), State::Expired);
        assert!(!stale.pending(), "an expired question must not keep demanding attention");
        assert!(bus.pending().is_empty(), "and must not reach the launcher or the status bar");
        assert!(bus.items().is_empty());
        assert!(bus.get(&m.id).is_some(), "expired is not deleted");

        // Finding 5: and it is not deleted a day later either. `State::open`
        // excludes `Expired`, so the sweep took an unanswered question
        // twenty-four hours after it was *sent* — contradicting this test's
        // own subject, `State::Expired`'s doc and `sweep`'s doc at once.
        // Expiry means it stopped demanding attention, not that it stopped
        // existing; only a withdrawal settles a question.
        let mut old = bus.get(&m.id).expect("msg");
        old.ts = now().saturating_sub(KEEP_SETTLED + 60);
        bus.save(&old).expect("save");
        bus.sweep();
        assert!(bus.get(&m.id).is_some(), "an unanswered question was swept when it expired");

        // It is still answerable, and answering settles it.
        assert_eq!(bus.answer(&m.id, "yes, go on"), 0);
        assert_eq!(bus.get(&m.id).expect("msg").state(), State::Answered);
    }

    #[test]
    fn drain_marks_read_rather_than_answered() {
        let (_root, bus) = fixture("bus-drain");
        let me = caller();
        let mut m = base("say", &me, &Opts::default()).expect("base");
        m.kind = "say".into();
        m.text = "I changed the auth schema".into();
        m.to = "codex · Prelude".into();
        m.to_cwd = me.cwd.clone();
        m.set(State::Undelivered);
        bus.save(&m).expect("save");

        assert_eq!(for_agent(&me, &bus.all(), false).len(), 1, "uncollected mail is waiting");
        assert_eq!(bus.drain(&caller()), 0);

        let after = bus.get(&m.id).expect("still on disk");
        assert_eq!(after.state(), State::Read, "collected, not decided");
        assert!(after.answer.is_none(), "draining an inbox is not answering anything");
        assert!(after.answered.is_none());
        assert!(for_agent(&me, &bus.all(), false).is_empty(), "and not offered twice");
        assert_eq!(for_agent(&me, &bus.all(), true).len(), 1, "--all still shows it");
    }

    /// Undelivered is a fact worth keeping: an agent with no pane is not an
    /// agent that got the message.
    #[test]
    fn a_message_with_nowhere_to_go_waits_and_says_so() {
        let (_root, bus) = fixture("bus-undelivered");
        let runs = vec![run("codex", "api-gateway")];
        let sent = bus.say_to(&caller(), &runs, "api-gateway", "the schema moved", Opts::default());
        assert_eq!(sent, 0);
        let m = bus.all().pop().expect("a message");
        assert_eq!(m.state(), State::Undelivered);
        assert!(m.state().open(), "undelivered mail is never swept");
        bus.sweep();
        assert_eq!(bus.all().len(), 1);
    }

    #[test]
    fn an_attachment_is_a_path_that_exists_and_is_not_a_credential() {
        let (root, bus) = fixture("bus-attach");
        let dir = root.path.join("fixtures");
        std::fs::create_dir_all(&dir).expect("fixtures");
        let ok = dir.join("plan.md");
        std::fs::write(&ok, b"the plan").expect("write");
        let creds = dir.join("api_key.txt");
        std::fs::write(&creds, b"anything").expect("write");

        let good = attachments(&[ok.to_string_lossy().into_owned()]).expect("a plain file");
        assert_eq!(good.len(), 1);
        assert!(good[0].starts_with('/'), "canonicalized: {:?}", good[0]);

        let refused = attachments(&[creds.to_string_lossy().into_owned()]);
        assert!(refused.is_err(), "a credential-looking path must be refused");
        let missing = attachments(&[dir.join("nothing-here").to_string_lossy().into_owned()]);
        assert!(missing.is_err(), "a path that does not exist must be refused");

        // Refused at send time, before anything is written.
        let runs = vec![run("codex", "api-gateway")];
        let opts = Opts { attach: vec![creds.to_string_lossy().into_owned()], ..Default::default() };
        assert_eq!(bus.say_to(&caller(), &runs, "api-gateway", "look at this", opts), 2);
        assert!(bus.all().is_empty(), "a refused message must not reach the bus");

        // And the contents are never copied — only the path is stored.
        let opts =
            Opts { attach: vec![ok.to_string_lossy().into_owned()], ..Default::default() };
        assert_eq!(bus.say_to(&caller(), &runs, "api-gateway", "look at this", opts), 0);
        let m = bus.all().pop().expect("a message");
        assert_eq!(m.attach.len(), 1);
        let raw = std::fs::read_to_string(bus.file_of(&m.id)).expect("record");
        assert!(!raw.contains("the plan"), "a message must never copy a file's contents");
    }

    #[test]
    fn a_handoff_is_never_delivered_to_a_guess() {
        let (root, bus) = fixture("bus-handoff-exact");
        let task = root.tasks().start(spec("rebuild the index")).expect("task");
        let runs = vec![run("claude", "api-gateway"), run("claude", "api-gateway-tests")];

        // Two claudes: a bare agent name is ambiguous and must be refused.
        assert_eq!(bus.handoff_to(&caller(), &runs, "claude", &task.id, ""), 2);
        // A substring that spans both projects is likewise ambiguous.
        assert_eq!(bus.handoff_to(&caller(), &runs, "api", &task.id, ""), 2);
        // And nothing matching is not a reason to pick the only agent running.
        assert_eq!(bus.handoff_to(&caller(), &runs, "nothing-like-this", &task.id, ""), 2);
        assert!(bus.all().is_empty(), "a refused handoff must post nothing");
        assert_eq!(
            root.tasks().get(&task.id).expect("task").agent,
            "claude",
            "a refused handoff must not reassign the work"
        );

        // An exact project name wins outright, even as a prefix of another's.
        assert_eq!(bus.handoff_to(&caller(), &runs, "api-gateway", &task.id, ""), 0);
        assert_eq!(bus.all().len(), 1);
    }

    #[test]
    fn a_result_lands_in_the_inbox_of_whoever_handed_the_work_over() {
        let (root, bus) = fixture("bus-report");
        let me = caller();
        // The handoff, as though this agent had sent it from this directory.
        let mut handoff = base("say", &me, &Opts::default()).expect("base");
        handoff.text = "please take this".into();
        handoff.to = "codex · api-gateway".into();
        handoff.to_cwd = "/tmp/api-gateway".into();
        handoff.from = "claude".into();
        handoff.from_cwd = me.cwd.clone();
        handoff.set(State::Undelivered);
        bus.save(&handoff).expect("save");

        let task = root.tasks().start(crate::task::New {
            message: handoff.id.clone(),
            ..spec("rebuild the index")
        })
        .expect("task");
        // Posted by `Store::finish`, not by this test: that is the whole
        // point of the edge, and a second explicit call would double it.
        root.tasks().done(&task.id, "rebuilt, 40k rows").expect("done");
        let result = bus.all()
            .into_iter()
            .rfind(|m| m.text.contains("rebuilt, 40k rows"))
            .expect("a reported result");
        let id = result.id.clone();
        assert_eq!(result.thread_id(), handoff.thread_id(), "same thread");
        assert_eq!(result.to_cwd, me.cwd, "addressed back to where it came from");
        assert!(result.text.contains("rebuilt, 40k rows"));
        // And it is what the sender's own inbox hands back.
        let waiting = for_agent(&me, &bus.all(), false);
        assert!(waiting.iter().any(|m| m.id == id), "the result is in the originator's inbox");

        // Work nobody handed over has nobody to report to.
        let solo = root.tasks().start(spec("my own idea")).expect("task");
        let solo = root.tasks().done(&solo.id, "done").expect("done");
        assert!(bus.report_task(&caller(), &solo).is_none());
    }

    #[test]
    fn cancelling_and_reassigning_keep_the_record() {
        let (_root, bus) = fixture("bus-cancel");
        let me = caller();
        let mut q = base("ask", &me, &Opts::default()).expect("base");
        q.to = "human".into();
        q.text = "shall I force-push?".into();
        q.set(State::Sent);
        bus.save(&q).expect("save");
        assert_eq!(bus.pending().len(), 1);

        assert_eq!(bus.cancel(&q.id, "worked it out myself"), 0);
        let cancelled = bus.get(&q.id).expect("still on disk");
        assert_eq!(cancelled.state(), State::Cancelled);
        assert!(!cancelled.pending(), "a withdrawn question stops asking");
        assert_eq!(bus.answer(&q.id, "no"), 2, "and cannot be answered afterwards");
        assert_eq!(bus.cancel(&q.id, "again"), 2);

        // Reassignment re-points a message and records both ends.
        let runs = vec![run("codex", "api-gateway"), run("pi", "docs")];
        let text = "take the schema work";
        assert_eq!(bus.say_to(&caller(), &runs, "api-gateway", text, Opts::default()), 0);
        let first = bus.all().into_iter().find(|m| m.kind == "say").expect("say");
        assert_eq!(bus.reassign_to(&caller(), &runs, &first.id, "docs"), 0);

        let old = bus.get(&first.id).expect("the original survives");
        assert_eq!(old.state(), State::Cancelled);
        assert!(old.reason.as_deref().unwrap_or_default().contains("reassigned to"));
        let new = bus.all()
            .into_iter()
            .find(|m| m.reply_to.as_deref() == Some(first.id.as_str()))
            .expect("the replacement");
        assert_eq!(new.thread_id(), first.thread_id(), "one exchange, both ends");
        assert!(new.to.contains("docs"));
        assert_eq!(new.text, first.text);
        // Ambiguity is refused here too.
        assert_eq!(bus.reassign_to(&caller(), &runs, &new.id, "nothing-like-this"), 2);
    }

    /// The credential rule, on the one field that is nothing but user prose.
    #[test]
    fn a_credential_never_reaches_a_message() {
        let (_root, bus) = fixture("bus-secret");
        let key = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01";
        let runs = vec![run("codex", "api-gateway")];
        assert_eq!(
            bus.say_to(&caller(),
                &runs,
                "api-gateway",
                &format!("rotating now\nexport API_KEY={key}\nthen restarting"),
                Opts::default()
            ),
            0
        );
        let m = bus.all().pop().expect("a message");
        let raw = std::fs::read_to_string(bus.file_of(&m.id)).expect("record");
        assert!(!raw.contains(key), "a message kept a credential: {raw}");
        assert!(m.text.contains("rotating now") && m.text.contains("then restarting"));
        assert!(m.text.contains(REDACTED), "a dropped line must leave a marker");
        // The ordinary question that merely mentions the subject survives whole.
        assert_eq!(clean("should I rotate the API key?"), "should I rotate the API key?");
    }

    #[test]
    fn an_id_from_a_command_line_cannot_escape_the_bus() {
        let (_root, bus) = fixture("bus-ids");
        assert!(bus.get("../../etc/passwd").is_none());
        assert_eq!(bus.answer("../../etc/passwd", "no"), 2);
        assert_eq!(bus.cancel("../../etc/passwd", ""), 2);
        assert!(bus.thread_of("../../etc/passwd").is_empty());
        // Two messages posted in the same second by one process are two files.
        assert_ne!(new_id(), new_id());
    }
}
