//! A Task: the piece of work an agent was asked to do, which outlives the
//! process doing it.
//!
//! A Run is a process and a Session is a transcript. Neither survives the
//! agent exiting, and neither can say what it was *for* — `running.rs` can
//! tell you that `claude` has been quiet in this project for eleven minutes,
//! and nothing on the machine knows whether that is the migration you asked
//! for at ten o'clock, whether it finished, or whether the agent that was
//! doing it died halfway through. The Task is where that fact lives, and it
//! is the only record here that Prelude owns outright: an Agent's Session
//! files, processes and CLI output are authoritative and Prelude must not
//! rewrite them, but nobody else is keeping this.
//!
//! **The id is immutable, and reserved rather than chosen.** Renaming,
//! reassigning and retrying all keep it, because it is the thing a message, a
//! Run and a Session all point at. It comes from a timestamp and a pid, and
//! the file is created with `O_EXCL` — so two tasks started in the same
//! second by the same process, or by two processes at once, cannot collide.
//! Checking for the file and then writing it would leave exactly that race.
//! The reservation is a *guard*: an id taken and never written is an empty
//! file every reader skips forever, so giving it back is the default and
//! keeping it is the deliberate act.
//!
//! **Storage is one JSON file per task under the data directory**, `0700`
//! with `0600` files, written temp-then-rename, on the same reasoning as the
//! bus: a queued task is not something to lose when a cache is cleared, and a
//! reader must never see half a record. Reading the whole directory is a
//! handful of small files, like `bus::all()`; that is cheap enough for an
//! explicit command, and it is *not* cheap enough for a gather once the
//! directory has thousands of finished tasks in it — five thousand records
//! cost 80 ms, twice the entire gather budget. So finished tasks are swept,
//! and the launcher path reads `open_tasks`, which is bounded by how much
//! work is actually outstanding rather than by how much has ever been done.
//!
//! **Every stored string is a thing a person or an agent typed**, so all of
//! them go through the same credential filter as history and the clipboard —
//! not only the title and the result. A project name, an agent name, a run
//! id, a session id, a message id and a working directory all arrive from a
//! command line, and `--project "p sk-…"` used to land verbatim in both the
//! record and the event log. A prompt never comes here at all: `prompt_ref`
//! is a *reference* — a message id or a path — because the one thing a prompt
//! reliably contains is everything.

use crate::events::{self, Event};
use crate::item::{Item, Kind};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

/// A subject line, not a description. Long enough to name the work.
const TITLE_MAX: usize = 160;

/// A result or a failure reason. Longer, because "why" is worth more than
/// "what" and a stack trace's first lines are often the answer.
const RESULT_MAX: usize = 800;

/// A working directory. Far more generous than a name, because this is a real
/// path and a truncated one is not the same directory; a session in a deep
/// iCloud project is already 127 columns before anybody tries to be difficult.
const CWD_MAX: usize = 1024;

/// Reservation gives up after this many collisions in one second. Reaching it
/// means something is very wrong with the store, and spinning is worse than
/// saying so.
const ID_ATTEMPTS: u32 = 10_000;

/// How long a finished task is kept before it is swept.
///
/// Much longer than the day `bus.rs` gives an answered message, because a
/// finished Task is the record of what happened rather than a delivery
/// receipt: `retry_of` points back at one by id, and `prelude task list
/// --all` is where somebody asks what went wrong last week. Thirty days
/// outlives any piece of work still being asked about, and holds a fleet
/// closing a hundred tasks a day to a few thousand files — which an explicit
/// `task list` reads in tens of milliseconds and the gather path never reads
/// at all.
const KEEP_FINISHED: u64 = 30 * 24 * 3600;

/// How long a finished task keeps asking to be looked at.
///
/// A day, on `bus.rs`'s reasoning for an answered message: the home is what is
/// outstanding *now*, and a completion nobody has dismissed by the next
/// morning has stopped being news and become history. History has a place —
/// `prelude task list --all` and `task show` — and it is not the first screen.
///
/// It is deliberately far shorter than `KEEP_FINISHED`. The record survives
/// thirty days either way; this is only how long it keeps a *row*.
const AWAITING_REVIEW: u64 = 24 * 3600;

/// How many finished tasks of one terminal state the home will carry at once.
///
/// Age alone is not a bound. A fleet that fails a hundred jobs overnight
/// produces a hundred rows every one of which is within the window, and a
/// hundred rows is not a notice, it is a log — the thing the home exists to
/// not be. Counted per state rather than over the pair, so a burst of
/// completions cannot push the failures off the screen and a burst of failures
/// cannot hide the completions.
const AWAITING_SHOWN: usize = 8;

const ORPHAN_REASON: &str = "the run or session it was attached to is gone";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Accepted, not started. Possibly waiting on a dependency.
    #[default]
    Queued,
    Working,
    /// Blocked on somebody else — a person's answer, another agent, a review.
    Waiting,
    Done,
    Failed,
    Cancelled,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Queued => "queued",
            State::Working => "working",
            State::Waiting => "waiting",
            State::Done => "done",
            State::Failed => "failed",
            State::Cancelled => "cancelled",
        }
    }

    /// Inherent rather than `FromStr`: an unknown word is not an error worth
    /// a type, it is simply not a state, and every caller here wants an
    /// `Option` back.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<State> {
        [
            State::Queued,
            State::Working,
            State::Waiting,
            State::Done,
            State::Failed,
            State::Cancelled,
        ]
        .into_iter()
        .find(|state| state.as_str() == s)
    }

    /// Finished states are terminal. A finished task is never reopened —
    /// `retry` makes a new one, so the record of what happened stays intact.
    pub fn finished(self) -> bool {
        matches!(self, State::Done | State::Failed | State::Cancelled)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub project: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub state: State,
    /// The agent this is assigned to, or empty when nobody has it yet.
    #[serde(default)]
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The question or message this work came out of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// A message id or a file path. Never prompt text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_ref: Option<String>,
    /// Task ids this one waits on. Ids, and nothing else — a dependency that
    /// is not `valid_id` is refused rather than stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Why it failed, was cancelled, or was declared orphaned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The task this one is a second attempt at. The original keeps its id
    /// and its record; this edge is the only thing that joins them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// When somebody said they had seen how this finished.
    ///
    /// A timestamp rather than a flag, because "when" is a question the event
    /// trail can already answer about everything else here and this would
    /// otherwise be the one state change with no clock on it.
    ///
    /// It is only ever set by `ack`, which is an explicit act. It is never
    /// inferred from a row having been looked at: focus and Quick Look cross
    /// rows without any decision being made, and a task is routinely opened
    /// while it is still running — treating that as acknowledgement would
    /// silently drop the completion notice that arrives an hour later. The
    /// failure to avoid is a missing row, so the default is to keep asking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acked_at: Option<u64>,
}

/// What a caller knows when it opens a task. Empty means absent, so the CLI
/// can hand its flags straight over without an `Option` per field.
#[derive(Clone, Debug, Default)]
pub struct New {
    pub title: String,
    pub project: String,
    pub cwd: String,
    pub agent: String,
    pub run: String,
    pub session: String,
    pub message: String,
    pub prompt_ref: String,
    pub deps: Vec<String>,
    pub retry_of: String,
}

impl New {
    /// Fill in who and where from the environment.
    ///
    /// Identity is discovered, never declared — the same rule the bus lives
    /// under, and the reason `prelude task start "…"` needs no configuration.
    /// It is deliberately *not* inside `create`: the store stays a pure
    /// function of what it was handed, so a test never shells out to `ps`.
    pub fn here(title: &str) -> Self {
        let me = crate::bus::whoami();
        New {
            title: title.to_string(),
            project: me.project,
            cwd: me.cwd,
            agent: me.agent,
            ..Default::default()
        }
    }
}

/// Ids we generate are alphanumeric and dashes; ids from a command line are
/// whatever somebody typed. This is what stops `../../.ssh/id_rsa` being read
/// or written as a task — and, since a record carries its own id in its
/// contents, what stops one being *returned* as a task id either.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// An optional stored field, filtered and bounded exactly like a title.
///
/// It used to be `trim()` and nothing more, which is how `--run "r sk-…"`
/// reached the record verbatim and how a 600 KB `--project` reached the event
/// log and emptied it. Nothing that arrives from a command line is exempt.
fn opt(value: &str) -> Option<String> {
    events::redact(value, events::FIELD_MAX)
}

/// A place on disk — a working directory, or the project name taken from its
/// last component — filtered as a *path* rather than as prose.
///
/// `events::redact` applies `secrets::looks_secret`, which is deliberately
/// broad because it decides what is kept out of shell history — and "token"
/// anywhere in a line is enough. A path is not a line of prose: a project
/// legitimately called `.../token-service` or `.../secrets` was therefore
/// stored as `[redacted]`, and `items_as` then handed the launcher a `cwd`
/// that is not a directory at all. Every row for that project lost its
/// project, and `cd` from one of them would have gone nowhere.
///
/// Both fields, because `project` is the same string: `New::here` fills it
/// from `whoami().project`, which is the last component of the cwd. The fix
/// went into `cwd` alone at first, and a task started in `token-service`
/// therefore kept its directory and still showed `[redacted]` as its project
/// in the launcher row, in `task list` and in every project filter. The test
/// asserted only on `cwd`, which is why it did not say so.
///
/// `capability.rs` already has the right shape for this: credential-like
/// *paths* are recognised with `looks_secret_material` plus the two path
/// rules — a `.env` file and anything spelled `credential` — and a path that
/// trips it is refused rather than rewritten. Refusing is the whole point. A
/// task in `~/.aws/credentials.d` keeps its title, its state and its history
/// and carries no `cwd`; nothing anywhere is handed a directory that does not
/// exist. `~/.aws` itself is an ordinary directory and keeps both fields —
/// the rule is about what the path *says*, not about which tool owns it.
///
/// The bound is a parameter because the two fields have different ones: a
/// directory in a deep iCloud project is already 127 columns before anybody
/// tries to be difficult, while a project name is a name.
fn path_field(value: &str, max: usize) -> String {
    let path = value.trim();
    if path.is_empty() {
        return String::new();
    }
    let low = path.to_ascii_lowercase();
    if crate::secrets::looks_secret_material(path)
        || low.ends_with("/.env")
        || low.contains("credential")
    {
        return String::new();
    }
    // Truncating is the one thing that must not happen here: half a path is a
    // different directory, and half a project name matches no project, which
    // is the bug this function exists to close.
    if crate::width::dwidth(path) > max {
        return String::new();
    }
    path.to_string()
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// The task store, addressed by root rather than by the environment.
///
/// `sessions.rs` records why: `paths::data()` reads `$XDG_DATA_HOME`, the
/// environment belongs to the whole process, and `cargo test` runs tests on
/// several threads at once — so a test that repoints it is mutating shared
/// state underneath every other test's `std::env::var_os`, which is
/// documented undefined behaviour and not something a mutex over *this*
/// module can prevent. Taking the root as a parameter, with the env-free
/// public API on top, is how that module solved it. Nothing structurally
/// stops a future test from forgetting a guard; nothing structurally lets it
/// reach the person's real data either.
pub struct Store {
    root: PathBuf,
}

pub fn store() -> Store {
    Store::at(crate::paths::data())
}

/// A taken id, given back if it is dropped without being kept.
///
/// An id is taken by creating its file, so an id taken and never written is
/// an empty file every reader skips forever — the id is burnt and the litter
/// is permanent. Making "gave up" the default means every early return
/// between reserving and saving, including the `?` on `save` itself, hands
/// the id back without anybody having to remember to.
struct Reservation<'a> {
    store: &'a Store,
    id: String,
    kept: bool,
}

impl Reservation<'_> {
    fn keep(mut self) {
        self.kept = true;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_file(self.store.file_of(&self.id));
        }
    }
}

impl Store {
    pub fn at(root: PathBuf) -> Store {
        Store { root }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("tasks")
    }

    fn file_of(&self, id: &str) -> PathBuf {
        self.dir().join(format!("{id}.json"))
    }

    fn log(&self) -> events::Log {
        events::Log::at(self.root.join("events.jsonl"))
    }

    fn ensure_dir(&self) -> Result<(), String> {
        events::ensure_private_dir(&self.dir())
    }

    /// Take an id by creating its file, not by looking to see whether it is
    /// free.
    ///
    /// `create_new` is the whole trick: the kernel decides who won, so a
    /// second task started in the same second — by this process or another
    /// one — gets the next suffix rather than silently overwriting the first.
    /// The file is left empty for the moment it takes `save` to rename a
    /// complete record over it, and an empty file is skipped by every reader
    /// here.
    fn reserve(&self) -> Result<Reservation<'_>, String> {
        let stamp = crate::bus::now();
        let pid = std::process::id();
        self.ensure_dir()?;
        for n in 0..ID_ATTEMPTS {
            let id =
                if n == 0 { format!("t{stamp}-{pid}") } else { format!("t{stamp}-{pid}-{n}") };
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(self.file_of(&id)) {
                Ok(_) => return Ok(Reservation { store: self, id, kept: false }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("could not create a task: {e}")),
            }
        }
        Err("could not find a free task id".into())
    }

    /// The id inside the record is checked here rather than trusted.
    ///
    /// `get` validates the *filename*; the id in the file's contents was
    /// simply believed, so a hand-written record claiming
    /// `"id": "../../../../tmp/pwn"` came back out of `all()` verbatim and
    /// would have become the command line of a launcher Item. Not reachable
    /// from the CLI — defence in depth, on the cheapest possible line.
    fn save(&self, task: &Task) -> Result<(), String> {
        if !valid_id(&task.id) {
            return Err(format!("{} is not a task id", task.id));
        }
        let bytes =
            serde_json::to_vec_pretty(task).map_err(|e| format!("could not encode the task: {e}"))?;
        events::write_private(&self.file_of(&task.id), &bytes)
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        if !valid_id(id) {
            return None;
        }
        let text = std::fs::read_to_string(self.file_of(id)).ok()?;
        let task: Task = serde_json::from_str(&text).ok()?;
        // A record's id is the name it is filed under, or it is not a record.
        (task.id == id).then_some(task)
    }

    /// Every task, oldest first. A file that will not parse — including the
    /// empty one a reservation leaves for a moment — is skipped rather than
    /// fatal.
    ///
    /// O(every task ever created and not yet swept), which is right for
    /// `prelude task list` and wrong for anything on a gather. Use
    /// `open_tasks` there.
    pub fn all(&self) -> Vec<Task> {
        let Ok(entries) = std::fs::read_dir(self.dir()) else { return Vec::new() };
        let mut out: Vec<Task> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|x| x == "json"))
            .filter_map(|path| {
                let stem = path.file_stem()?.to_str()?.to_string();
                let text = std::fs::read_to_string(&path).ok()?;
                let task: Task = serde_json::from_str(&text).ok()?;
                (valid_id(&task.id) && task.id == stem).then_some(task)
            })
            .collect();
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        out
    }

    // -----------------------------------------------------------------------
    // The open index
    // -----------------------------------------------------------------------

    /// Where a reader looks instead of at the directory.
    ///
    /// A hint, never the authority — the per-task files are that. It is a
    /// *superset* of the ids still worth showing: every new task appends its
    /// own line, and a rewrite reduces it to the ones `indexed` keeps — open
    /// work, plus finished work nobody has dismissed yet. So a reader may be
    /// handed an id that has since finished (filtered out) or been swept
    /// (absent, skipped), and it can only *miss* a task if that one append
    /// failed — in which case the next rewrite reconstructs the file from the
    /// directory. A missing index is not an empty one; what each reader does
    /// about that is `open_tasks` and `home_tasks` below.
    fn index(&self) -> PathBuf {
        self.dir().join("open.idx")
    }

    /// The lock every *rewrite* of the index takes, and that a new task's
    /// append respects.
    ///
    /// `events.rs` leaves its appends lock-free because that file is appended
    /// to on every operation and a lock on that path would be the whole cost.
    /// This one is different: the index is appended to exactly once per task,
    /// by a person or an agent typing a command, so the lock is free — and
    /// without it a rebuild cannot be made safe at all. A rewrite reads the
    /// directory and replaces the file; an append that lands between those two
    /// moments is a task the new file has never heard of, and it stays
    /// invisible to the fast path until something else finishes. That is
    /// precisely the missing row this whole mechanism exists to prevent.
    fn lock_index(&self) -> Option<std::fs::File> {
        let _ = self.ensure_dir();
        events::lock_exclusive(&self.dir().join("open.lock"))
    }

    /// Replace the index with exactly these ids. Temp-then-rename, like every
    /// other file here, so a reader never sees half a list.
    fn write_index(&self, ids: &[&str]) {
        let mut text = String::with_capacity(ids.len() * 24);
        for id in ids {
            text.push_str(id);
            text.push('\n');
        }
        let _ = events::write_private(&self.index(), text.as_bytes());
    }

    /// Reconstruct the index from the directory, and leave it behind.
    ///
    /// This is the expensive read — O(every task ever created and not yet
    /// swept), 54 ms over five thousand records — so the one thing it must not
    /// do is happen twice. It used to: the fallback in `open_tasks` did the
    /// whole scan and then threw the answer away, and since `sweep` only
    /// rewrites the index when a task *finishes*, a store where nothing
    /// finished never got it back. A lost index was lost for good and every
    /// launch paid the scan again.
    ///
    /// Returns everything it read, so the caller that asked for the open set
    /// does not walk the directory a second time to get it.
    fn rebuild_index(&self) -> Vec<Task> {
        let _lock = self.lock_index();
        let tasks = self.all();
        let now = crate::bus::now();
        let keep: Vec<&str> =
            home_set(tasks.iter(), now).into_iter().map(|task| task.id.as_str()).collect();
        self.write_index(&keep);
        tasks
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

    /// One `O_APPEND` line, on the same reasoning as the event log: the
    /// kernel resolves the offset and the write together, so two processes
    /// opening a task at once cannot land on the same bytes.
    ///
    /// Under `lock_index` as well, which the event log's appends are not. The
    /// offset rule only orders this write against *other appends*; it says
    /// nothing about a rewrite replacing the file underneath one, and losing
    /// this line to that is losing the task from every fast read until
    /// something else happens to finish.
    fn note_open(&self, id: &str) {
        let _lock = self.lock_index();
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut handle) = options.open(self.index()) {
            let _ = handle.write_all(format!("{id}\n").as_bytes());
        }
    }

    /// Drop finished tasks old enough to be history, and rewrite the index.
    ///
    /// Three things are never swept. An open task, obviously. A finished task
    /// that an open one still lists in `deps`: `ready` reads a missing
    /// dependency as unfinished — deliberately, because a missing record is
    /// not evidence of success — so sweeping a done prerequisite would block
    /// its dependant forever. And a finished task nobody has dismissed while
    /// it is still inside its review window, because deleting the record would
    /// delete the notice with it. Thirty days is so much longer than a day
    /// that the last of those can never bite today; it is written down anyway
    /// so the two windows stay independent numbers rather than one number
    /// silently depending on the other.
    ///
    /// This is O(the whole store) and runs only where a task *becomes*
    /// finished or is dismissed, which is an explicit command somebody typed.
    /// The read path pays none of it.
    fn sweep(&self) {
        let _lock = self.lock_index();
        let tasks = self.all();
        let now = crate::bus::now();
        let cutoff = now.saturating_sub(KEEP_FINISHED);
        let needed: std::collections::HashSet<&str> = tasks
            .iter()
            .filter(|task| !task.state.finished())
            .flat_map(|task| task.deps.iter().map(String::as_str))
            .collect();
        let keep: Vec<&str> =
            home_set(tasks.iter(), now).into_iter().map(|task| task.id.as_str()).collect();
        for task in &tasks {
            if !task.state.finished() {
                continue;
            }
            let settled = task.finished_at.unwrap_or(task.updated_at);
            if settled < cutoff
                && !needed.contains(task.id.as_str())
                && !awaiting_review(task, now)
            {
                let _ = std::fs::remove_file(self.file_of(&task.id));
            }
        }
        self.write_index(&keep);
    }

    /// Open tasks, oldest first — the true answer, at whatever it costs.
    ///
    /// This is the explicit-command reader: `task list`, `doctor`, a Quick
    /// Look asking what a run is doing. It must never say "no work
    /// outstanding" because a file went missing, so when the index is gone it
    /// scans — and, crucially, leaves the rebuilt index behind, so the scan is
    /// paid once rather than by every reader forever.
    ///
    /// The launcher does not come through here; see `home_tasks`.
    pub fn open_tasks(&self) -> Vec<Task> {
        let mut out: Vec<Task> = match self.read_index() {
            Some(ids) => ids
                .iter()
                .filter_map(|id| self.get(id))
                .filter(|task| !task.state.finished())
                .collect(),
            None => {
                self.rebuild_index().into_iter().filter(|task| !task.state.finished()).collect()
            }
        };
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// What the launcher gathers: open work, plus finished work that has not
    /// been dismissed — bounded by age *and* by count.
    ///
    /// **This reader never scans, not even once.** A gather has forty
    /// milliseconds for everything it does and the scan alone measures 54, so
    /// a foreground rebuild here is not a slow launch, it is a launch that has
    /// already spent its whole budget before the first subprocess reports.
    /// Instead the repair is handed to a detached process exactly as the SLOW
    /// sources hand over theirs, and this launch shows no Task rows. That is
    /// the honest trade and it is the same one `ports` and `sessions` make:
    /// one launch shows nothing and every launch after it shows everything,
    /// which beats every launch being over budget. Nothing is lost meanwhile —
    /// `task list`, `task show` and `doctor` all read through `open_tasks` and
    /// answer truthfully whether or not the index exists.
    pub fn home_tasks(&self) -> Vec<Task> {
        let Some(ids) = self.read_index() else {
            self.repair_detached();
            return Vec::new();
        };
        // Filtered again on the way out, and not only for tidiness: the index
        // is a hint that may be a day out of date, so a line in it is a
        // candidate rather than a row.
        let found: Vec<Task> = ids.iter().filter_map(|id| self.get(id)).collect();
        home_set(found.iter(), crate::bus::now()).into_iter().cloned().collect()
    }

    /// Ask another process to rebuild the index, and do not wait for it.
    ///
    /// Guarded on being the real store rather than on being outside a test: a
    /// `Store` addressed at some other root *cannot* be repaired this way,
    /// because the helper resolves its own root from the environment and has
    /// no idea which directory it was meant to look at. Spawning for one would
    /// be a process that rebuilds the wrong index.
    fn repair_detached(&self) {
        if !self.dir().exists() {
            // No store at all is not a lost index. There is nothing to
            // reconstruct and nobody to tell.
            return;
        }
        if self.root != crate::paths::data() {
            return;
        }
        crate::cache::spawn_self(&["_refresh", "task-index"]);
    }

    // -----------------------------------------------------------------------
    // Events
    // -----------------------------------------------------------------------

    /// Every operation below appends exactly one of these, and the event
    /// carries the edges the task had *at that moment* — reassigning a task
    /// later must not rewrite who was working on it an hour ago.
    fn record(&self, task: &Task, kind: &str, detail: &str) {
        let event = Event::new(kind, &task.id)
            .agent(&task.agent)
            .project(&task.project)
            .run(task.run.as_deref().unwrap_or_default())
            .session(task.session.as_deref().unwrap_or_default())
            .message(task.message.as_deref().unwrap_or_default())
            .detail(detail);
        // The task record is the authority; its narration failing must not
        // undo a state change that is already on disk.
        let _ = self.log().append(&event);
    }

    pub fn for_task(&self, id: &str) -> Vec<Event> {
        self.log().for_task(id)
    }

    // -----------------------------------------------------------------------
    // Operations
    // -----------------------------------------------------------------------

    fn create(&self, spec: New, state: State, kind: &str, detail: &str) -> Result<Task, String> {
        let title = events::redact(&spec.title, TITLE_MAX)
            .ok_or_else(|| "a task needs a title".to_string())?;
        // A dependency is a task id and nothing else: not a path, and not a
        // credential either. `valid_id` alone would still admit
        // `sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01`, which is alphanumeric and a
        // dash — so the same filter every other field goes through has to
        // agree, and a dep it would have altered is refused rather than
        // silently rewritten into something no task will ever match.
        for dep in &spec.deps {
            if !valid_id(dep) || opt(dep).as_deref() != Some(dep.as_str()) {
                return Err(format!("{dep} is not a task id"));
            }
        }
        let now = crate::bus::now();
        let held = self.reserve()?;
        let task = Task {
            id: held.id.clone(),
            title,
            project: path_field(&spec.project, events::FIELD_MAX),
            cwd: path_field(&spec.cwd, CWD_MAX),
            state,
            agent: opt(&spec.agent).unwrap_or_default(),
            run: opt(&spec.run),
            session: opt(&spec.session),
            message: opt(&spec.message),
            prompt_ref: events::redact(&spec.prompt_ref, TITLE_MAX),
            deps: spec.deps,
            result: None,
            reason: None,
            retry_of: opt(&spec.retry_of),
            created_at: now,
            updated_at: now,
            started_at: (state == State::Working).then_some(now),
            finished_at: None,
            acked_at: None,
        };
        // `?` here drops the reservation unkept, which gives the id back.
        self.save(&task)?;
        held.keep();
        self.note_open(&task.id);
        self.record(&task, kind, detail);
        Ok(task)
    }

    /// Open a task and start working on it now.
    pub fn start(&self, spec: New) -> Result<Task, String> {
        self.create(spec, State::Working, "task.start", "")
    }

    /// Accept a task without starting it. Where a dependency, or a queue of
    /// work waiting for a free agent, is represented.
    pub fn queue(&self, spec: New) -> Result<Task, String> {
        self.create(spec, State::Queued, "task.queue", "")
    }

    /// A task that can still be worked on, or an explanation of why not.
    fn open(&self, id: &str) -> Result<Task, String> {
        let task = self.get(id).ok_or_else(|| format!("no task {id}"))?;
        if task.state.finished() {
            return Err(format!("{id} is already {}", task.state.as_str()));
        }
        Ok(task)
    }

    pub fn progress(&self, id: &str, note: &str) -> Result<Task, String> {
        let mut task = self.open(id)?;
        // Reporting progress *is* the claim that work is happening; a queued
        // or waiting task that says so has started.
        task.state = State::Working;
        if task.started_at.is_none() {
            task.started_at = Some(crate::bus::now());
        }
        touch(&mut task);
        self.save(&task)?;
        self.record(&task, "task.progress", note);
        Ok(task)
    }

    /// Blocked on somebody else. Distinct from silence, which is what
    /// `running.rs` has to guess from when nobody says this.
    pub fn wait(&self, id: &str, note: &str) -> Result<Task, String> {
        let mut task = self.open(id)?;
        task.state = State::Waiting;
        touch(&mut task);
        self.save(&task)?;
        self.record(&task, "task.wait", note);
        Ok(task)
    }

    pub fn done(&self, id: &str, result: &str) -> Result<Task, String> {
        self.finish(id, State::Done, "task.done", result)
    }

    pub fn fail(&self, id: &str, reason: &str) -> Result<Task, String> {
        self.finish(id, State::Failed, "task.fail", reason)
    }

    pub fn cancel(&self, id: &str, reason: &str) -> Result<Task, String> {
        self.finish(id, State::Cancelled, "task.cancel", reason)
    }

    fn finish(&self, id: &str, state: State, kind: &str, text: &str) -> Result<Task, String> {
        let mut task = self.open(id)?;
        let text = events::redact(text, RESULT_MAX);
        task.state = state;
        match state {
            State::Done => task.result = text.clone(),
            _ => task.reason = text.clone(),
        }
        task.finished_at = Some(crate::bus::now());
        touch(&mut task);
        self.save(&task)?;
        self.record(&task, kind, text.as_deref().unwrap_or_default());
        // A task has just left the open set, so this is the one moment the
        // index goes stale and the one moment something new can be swept.
        self.sweep();
        // …and the one moment whoever handed this over can be told. It
        // self-filters to Done and Failed and to a task that carries the
        // message that delegated it, so work nobody delegated costs one
        // `get("")` and nothing else. Never fatal: the record on disk is the
        // authority, and failing to narrate it must not undo it.
        let _ = crate::bus::report_task(&task);
        Ok(task)
    }

    /// Dismiss a finished task: somebody has seen how it went.
    ///
    /// The record stays, complete, with its result and its event trail. All
    /// this changes is whether it is still *asking* — which is why it is not
    /// destructive, is not confirmed, and is offered nowhere near a live task.
    /// A task that is still running is not asking to be dismissed; it is
    /// asking to be finished, and saying so is better than quietly writing an
    /// acknowledgement of a result that does not exist yet.
    ///
    /// Idempotent, and the first answer is the one kept: "when was this seen"
    /// has one true answer and a second `ack` is not new information.
    pub fn ack(&self, id: &str) -> Result<Task, String> {
        let mut task = self.get(id).ok_or_else(|| format!("no task {id}"))?;
        if !task.state.finished() {
            return Err(format!(
                "{id} is still {} — finish it before dismissing it",
                task.state.as_str()
            ));
        }
        if task.acked_at.is_some() {
            return Ok(task);
        }
        task.acked_at = Some(crate::bus::now());
        touch(&mut task);
        self.save(&task)?;
        self.record(&task, "task.ack", "");
        // It has just left the set the home reads, so this is the moment the
        // index goes stale — and the moment an old record that was being held
        // back only because nobody had seen it becomes ordinary history again.
        self.sweep();
        Ok(task)
    }

    /// Hand a task to an agent, or to a different one. Reassignment is the
    /// same operation as the first assignment — the id, the title and the
    /// history all stay, which is the whole reason the id is immutable.
    pub fn assign(&self, id: &str, agent: &str) -> Result<Task, String> {
        let agent = opt(agent).ok_or_else(|| "assign it to whom?".to_string())?;
        let mut task = self.open(id)?;
        let previous = std::mem::replace(&mut task.agent, agent);
        // A reassigned task is no longer being worked on by the run that had
        // it. Saying so is honest; leaving a dead run attached is what makes
        // orphans that nothing can explain.
        if previous != task.agent {
            task.run = None;
            task.session = None;
        }
        touch(&mut task);
        self.save(&task)?;
        let detail = if previous.is_empty() { String::new() } else { format!("was {previous}") };
        self.record(&task, "task.assign", &detail);
        Ok(task)
    }

    /// Attach edges as they become known — a run that picked the task up, the
    /// session it is being done in, the question it came from.
    pub fn link(&self, id: &str, run: &str, session: &str, message: &str) -> Result<Task, String> {
        let mut task = self.open(id)?;
        if let Some(value) = opt(run) {
            task.run = Some(value);
        }
        if let Some(value) = opt(session) {
            task.session = Some(value);
        }
        if let Some(value) = opt(message) {
            task.message = Some(value);
        }
        touch(&mut task);
        self.save(&task)?;
        self.record(&task, "task.link", "");
        Ok(task)
    }

    /// Try again: a *new* task with the same title, project, dependencies and
    /// origin, and a `retry_of` edge back.
    ///
    /// Never a mutation of the original. What failed at three o'clock stays
    /// failed at three o'clock, with its reason and its event trail intact —
    /// otherwise the only record of a repeated failure is one row saying the
    /// third attempt worked.
    ///
    /// Only ever of a *finished* task. Retrying one that is still running
    /// produces a second queued clone of work already in progress: two agents
    /// on one job, and no way for either to know. Cancel it first, which is
    /// the sentence that says so out loud.
    pub fn retry(&self, id: &str) -> Result<Task, String> {
        let original = self.get(id).ok_or_else(|| format!("no task {id}"))?;
        if !original.state.finished() {
            return Err(format!(
                "{id} is still {} — cancel it before retrying it",
                original.state.as_str()
            ));
        }
        self.create(
            New {
                title: original.title.clone(),
                project: original.project.clone(),
                cwd: original.cwd.clone(),
                agent: original.agent.clone(),
                message: original.message.clone().unwrap_or_default(),
                prompt_ref: original.prompt_ref.clone().unwrap_or_default(),
                deps: original.deps.clone(),
                retry_of: original.id.clone(),
                ..Default::default()
            },
            State::Queued,
            "task.retry",
            &format!("retry of {}", original.id),
        )
    }

    /// Declare a task orphaned: its run or session went away without it ever
    /// being finished.
    ///
    /// Detection is separate from this on purpose — `orphans` only reports,
    /// and nothing calls this automatically. An agent that crashed and an
    /// agent that was `kill -9`d by the person sitting there look identical
    /// from here, and silently marking work failed is exactly the kind of
    /// guess the plan forbids.
    pub fn mark_orphaned(&self, id: &str) -> Result<(), String> {
        let mut task = self.open(id)?;
        task.state = State::Failed;
        task.reason = Some(ORPHAN_REASON.to_string());
        task.finished_at = Some(crate::bus::now());
        touch(&mut task);
        self.save(&task)?;
        self.record(&task, "task.orphaned", ORPHAN_REASON);
        self.sweep();
        // An orphan reaches `Failed` without going through `finish`, so it
        // needs the same line: an agent that handed work over and then lost
        // the run doing it must still hear that it will not be coming back.
        let _ = crate::bus::report_task(&task);
        Ok(())
    }
}

fn touch(task: &mut Task) {
    task.updated_at = crate::bus::now();
}

/// A finished task that is still asking to be looked at.
///
/// `Cancelled` is deliberately never one of these. Cancelling *is* the
/// decision — somebody stopped the work on purpose, from a panel that asked
/// them first — so a row saying "this thing you cancelled has been cancelled"
/// would be the launcher reporting the person's own keystroke back to them.
/// Done and Failed are the two that arrive while nobody is looking.
pub(crate) fn awaiting_review(task: &Task, now: u64) -> bool {
    matches!(task.state, State::Done | State::Failed)
        && task.acked_at.is_none()
        && task.finished_at.unwrap_or(task.updated_at) > now.saturating_sub(AWAITING_REVIEW)
}

/// Open work, plus the newest few undismissed results per terminal state.
///
/// One function, used twice, and the second use is the one that matters. It is
/// obviously what `home_tasks` shows. It is *also* what the index carries —
/// because the index is what a reader pays for, one `get` per line, and a
/// bound applied only at the end of that read is not a bound at all. A fleet
/// closing a hundred tasks an hour has thousands of results inside the
/// twenty-four hour window; an index that listed every one of them would hand
/// the launcher thousands of file reads to throw away, which is the exact cost
/// the index exists to avoid wearing a different name.
///
/// The index used to hold only open ids, which is the other half of why slots
/// 2 and 3 of the home could never be filled: a finished task was dropped from
/// it the instant it finished, so the one reader that must not walk the
/// directory could never see one.
///
/// Counted per state, so a burst of completions cannot push the failures out
/// and a burst of failures cannot hide the completions. A result beyond the
/// count is still a *record* — `awaiting_review` keeps `sweep` off it and
/// `task list --all` still finds it; it simply stops holding a row.
fn home_set<'a>(tasks: impl Iterator<Item = &'a Task>, now: u64) -> Vec<&'a Task> {
    let (mut open, mut failed, mut done) = (Vec::new(), Vec::new(), Vec::new());
    for task in tasks {
        if !task.state.finished() {
            open.push(task);
        } else if !awaiting_review(task, now) {
            continue;
        } else if task.state == State::Failed {
            failed.push(task);
        } else {
            done.push(task);
        }
    }
    for bucket in [&mut failed, &mut done] {
        bucket
            .sort_by_key(|task| std::cmp::Reverse(task.finished_at.unwrap_or(task.updated_at)));
        bucket.truncate(AWAITING_SHOWN);
    }
    open.append(&mut failed);
    open.append(&mut done);
    open.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    open
}

// ---------------------------------------------------------------------------
// The env-free API on top
// ---------------------------------------------------------------------------

pub fn get(id: &str) -> Option<Task> {
    store().get(id)
}

pub fn all() -> Vec<Task> {
    store().all()
}

/// Open tasks only. What anything on a launcher path wants: bounded by
/// outstanding work rather than by everything ever done.
pub fn open_tasks() -> Vec<Task> {
    store().open_tasks()
}

/// Reconstruct the open-task index from the directory.
///
/// Reached through `prelude _refresh task-index`, which is what `home_tasks`
/// spawns when it finds the index gone. Nothing else needs it: every other
/// path that rewrites the index does so because it already knows something
/// changed.
pub fn rebuild_index() {
    store().rebuild_index();
}

pub fn ack(id: &str) -> Result<Task, String> {
    store().ack(id)
}

pub fn done(id: &str, result: &str) -> Result<Task, String> {
    store().done(id, result)
}

pub fn cancel(id: &str, reason: &str) -> Result<Task, String> {
    store().cancel(id, reason)
}

pub fn retry(id: &str) -> Result<Task, String> {
    store().retry(id)
}

// `assign` and `link` had wrappers here too, and `bus.rs` was their only
// caller. A handoff now reassigns the task through the store at the bus's own
// root — one root for the message, the event and the Task record — so the two
// env-free doors had nobody left to walk through them. `Store::assign` and
// `Store::link` are unchanged.

impl Task {
    /// Can this be worked on now?
    ///
    /// A dependency that is not in the set at all counts as unfinished: a
    /// missing record is not evidence of success, and treating it as one
    /// would start the second half of a migration because the first half's
    /// file was deleted.
    pub fn ready(&self, all: &[Task]) -> bool {
        if self.state.finished() {
            return false;
        }
        self.deps.iter().all(|dep| {
            all.iter().any(|other| other.id == *dep && other.state == State::Done)
        })
    }

    fn age(&self) -> u64 {
        crate::bus::now().saturating_sub(self.updated_at)
    }
}

/// Tasks whose Run or Session has disappeared while they were still open.
///
/// Reported, never repaired. `live_runs` are Run ids from the fleet and
/// `live_sessions` are Session ids that still exist; a task with neither edge
/// is not an orphan, because nothing was ever attached for it to lose.
pub fn orphans<'a>(
    tasks: &'a [Task],
    live_runs: &[String],
    live_sessions: &[String],
) -> Vec<&'a Task> {
    tasks
        .iter()
        .filter(|task| matches!(task.state, State::Working | State::Waiting))
        .filter(|task| {
            let run_gone = task
                .run
                .as_ref()
                .is_some_and(|run| !live_runs.iter().any(|live| live == run));
            let session_gone = task
                .session
                .as_ref()
                .is_some_and(|session| !live_sessions.iter().any(|live| live == session));
            run_gone || session_gone
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

/// The three columns every view of a task shares. There is deliberately no
/// flattened `TaskRow` beside this: `task list` prints through `line_of` and
/// the launcher goes through `items_as`, so a third shape would only be a
/// third thing to keep in step.
fn fields_of(task: &Task) -> Vec<String> {
    let age = crate::sources::running::short_dur(task.age());
    vec![
        task.project.clone(),
        format!("{} · {age}", task.state.as_str()),
        task.agent.clone(),
    ]
}

/// Launcher rows for open tasks.
///
/// The kind is a parameter rather than `Kind::Task` inline, because it is the
/// one thing here a caller can legitimately disagree about — the Agent Home
/// wants the same rows under its own band — and taking it as an argument keeps
/// this a pure view over the store.
///
/// This is the gather-path entry, so it reads `home_tasks` rather than `all`:
/// a store with five thousand finished records in it costs 54 ms to read
/// whole, which is more than the entire gather budget, and almost none of
/// those records would have survived the filter anyway. `home_tasks` is
/// bounded by outstanding work plus a day's worth of undismissed results, and
/// never walks the directory at all.
///
/// `cmd` is `prelude task show <id>` rather than the title, because `finish`
/// dedupes on `(kind, cmd)` — two tasks called "fix the build" are two tasks,
/// and a title as the command line would collapse them into one row.
///
/// There is no `acked` field on the row and there should not be: `home_tasks`
/// only ever hands over a finished task that nobody has dismissed, so a flag
/// saying so would be a constant. The panel reads the same fact from the state
/// word, and `ack` is idempotent, so a row that has gone stale in somebody's
/// terminal cannot do anything worse than agree.
pub fn items_as(kind: Kind) -> Vec<Item> {
    store()
        .home_tasks()
        .into_iter()
        .map(|task| {
            let item = Item::new(format!("prelude task show {}", task.id), kind)
                .title(task.title.clone())
                .fields(fields_of(&task))
                .put("task_id", task.id.clone())
                .put("state", task.state.as_str())
                .put("project", task.project.clone())
                .put("agent", task.agent.clone())
                .put("path", task.cwd.clone())
                .rank(rank_of(&task));
            let item = match task.run.as_deref() {
                Some(run) => item.put("run_id", run),
                None => item,
            };
            let item = match task.session.as_deref() {
                Some(session) => item.put("session_id", session),
                None => item,
            };
            if task.cwd.is_empty() { item } else { item.cwd(task.cwd.clone()) }
        })
        .collect()
}

/// Where a task sits within its own kind.
///
/// State first, on the ordering Milestone 7 specifies for the Agent Home —
/// failed above completed above waiting above working above queued — and
/// recency only inside that. Recency is expressed against *now* rather than
/// as a raw timestamp so it stays monotonic in age; a clock value would wrap
/// its band every day and put an old task on top.
fn rank_of(task: &Task) -> f64 {
    let band = match task.state {
        State::Failed => 500.0,
        State::Done => 400.0,
        State::Waiting => 300.0,
        State::Working => 200.0,
        State::Queued | State::Cancelled => 100.0,
    };
    band + 99.0 / (1.0 + task.age() as f64 / 3600.0)
}

// ---------------------------------------------------------------------------
// The CLI
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Opts {
    json: bool,
    all: bool,
    project: String,
    agent: String,
    run: String,
    session: String,
    message: String,
    prompt_ref: String,
    needs: Vec<String>,
    words: Vec<String>,
}

const VALUE_FLAGS: &[&str] =
    &["--project", "--agent", "--run", "--session", "--message", "--prompt-ref", "--needs"];

/// The verbs whose payload is prose rather than a fixed list of words.
const FREE_TEXT: &[&str] = &["progress", "wait", "done", "fail", "cancel"];

impl Opts {
    /// Two rules, because there are two shapes of command here.
    ///
    /// `task start migrate users --project api` puts the flag *after* the
    /// text and reads perfectly well, so `start` — and the verbs whose
    /// payload is a fixed number of words — recognise a known flag wherever
    /// it appears, and treat an unknown `--word` as prose.
    ///
    /// Every other verb's payload is a sentence somebody wrote, and a
    /// sentence contains dashes: `task fail T1 could not resolve --needs more
    /// time` used to store "could not resolve time", and `task done T1 it
    /// printed --json badly` used to switch stdout to JSON. There the rule is
    /// the bus's — CLAUDE.md: *the flag split stops at the first non-flag
    /// word* — applied at each of the three places a flag can legitimately
    /// stand: before the verb, between the verb and the id, and between the
    /// id and the prose.
    fn parse(args: &[&str]) -> Result<Opts, String> {
        let verb = args.iter().copied().find(|a| !a.starts_with("--")).unwrap_or("");
        if FREE_TEXT.contains(&verb) {
            Ok(Opts::split(args))
        } else {
            Opts::anywhere(args)
        }
    }

    /// Known flags wherever they appear; everything else is text.
    fn anywhere(args: &[&str]) -> Result<Opts, String> {
        let mut opts = Opts::default();
        let mut i = 0;
        while i < args.len() {
            let arg = args[i];
            if arg == "--json" {
                opts.json = true;
            } else if arg == "--all" {
                opts.all = true;
            } else if let Some((name, value)) =
                arg.split_once('=').filter(|(name, _)| VALUE_FLAGS.contains(name))
            {
                opts.set(name, value);
            } else if VALUE_FLAGS.contains(&arg) {
                let value = args.get(i + 1).ok_or_else(|| format!("{arg} needs a value"))?;
                opts.set(arg, value);
                i += 1;
            } else {
                opts.words.push(arg.to_string());
            }
            i += 1;
        }
        Ok(opts)
    }

    /// Flags, verb, flags, id, flags, and then prose all the way to the end.
    fn split(args: &[&str]) -> Opts {
        let mut opts = Opts::default();
        let mut i = 0;
        for _ in 0..2 {
            i = opts.leading(args, i);
            if let Some(word) = args.get(i) {
                opts.words.push(word.to_string());
                i += 1;
            }
        }
        i = opts.leading(args, i);
        opts.words.extend(args[i.min(args.len())..].iter().map(|s| s.to_string()));
        opts
    }

    /// Consume `--json`, `--all` and `--name=value` from `args[i..]`, and
    /// stop at the first word that is not one of those.
    ///
    /// Never the `--name value` form. A flag that swallows the next word eats
    /// the first word of the prose that follows it, which is the trap
    /// `bus::parse_opts` records for exactly this reason and which
    /// `lend.rs` records for `--mcp-config`.
    fn leading(&mut self, args: &[&str], mut i: usize) -> usize {
        while let Some(arg) = args.get(i) {
            if *arg == "--json" {
                self.json = true;
            } else if *arg == "--all" {
                self.all = true;
            } else if let Some((name, value)) =
                arg.split_once('=').filter(|(name, _)| VALUE_FLAGS.contains(name))
            {
                self.set(name, value);
            } else {
                break;
            }
            i += 1;
        }
        i
    }

    fn set(&mut self, name: &str, value: &str) {
        match name {
            "--project" => self.project = value.to_string(),
            "--agent" => self.agent = value.to_string(),
            "--run" => self.run = value.to_string(),
            "--session" => self.session = value.to_string(),
            "--message" => self.message = value.to_string(),
            "--prompt-ref" => self.prompt_ref = value.to_string(),
            "--needs" => {
                self.needs = value
                    .split(',')
                    .map(str::trim)
                    .filter(|dep| !dep.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            _ => {}
        }
    }
}

const USAGE: &str =
    "prelude: task start|progress|wait|done|fail|cancel|assign|retry|ack|list|show";

/// `prelude task …`.
pub fn cli(args: &[&str]) -> i32 {
    store().cli(args)
}

impl Store {
    /// Exit 0 when it worked and 2 for a bad request — no such task, already
    /// finished — the same split `bus::answer` uses, so an agent can tell
    /// "that is not a task" from "the store is broken".
    pub fn cli(&self, args: &[&str]) -> i32 {
        let opts = match Opts::parse(args) {
            Ok(opts) => opts,
            Err(e) => {
                eprintln!("prelude: {e}");
                return 2;
            }
        };
        let words: Vec<&str> = opts.words.iter().map(String::as_str).collect();
        let Some((verb, rest)) = words.split_first() else {
            eprintln!("{USAGE}");
            return 2;
        };
        match (*verb, rest) {
            ("start", title) => self.start_cmd(&opts, &title.join(" ")),
            ("progress", [id, note @ ..]) if !note.is_empty() => {
                self.report(self.progress(id, &note.join(" ")), &opts)
            }
            // The other four accept an empty note; this one cannot, and
            // falling through to the generic usage line said so about the
            // wrong thing entirely.
            ("progress", [_id]) => {
                eprintln!("prelude: say what you are doing — prelude task progress ID NOTE");
                2
            }
            ("wait", [id, note @ ..]) => self.report(self.wait(id, &note.join(" ")), &opts),
            ("done", [id, result @ ..]) => self.report(self.done(id, &result.join(" ")), &opts),
            ("fail", [id, reason @ ..]) => self.report(self.fail(id, &reason.join(" ")), &opts),
            ("cancel", [id, reason @ ..]) => self.report(self.cancel(id, &reason.join(" ")), &opts),
            ("assign", [id, agent]) => self.report(self.assign(id, agent), &opts),
            ("retry", [id]) => self.opened(self.retry(id), &opts),
            // Not a state change so much as an answer to one: the record is
            // untouched, it simply stops asking to be looked at.
            ("ack", [id]) => self.report(self.ack(id), &opts),
            ("list", []) => self.list_cmd(&opts),
            ("show", [id]) => self.show_cmd(&opts, id),
            _ => {
                eprintln!("{USAGE}");
                2
            }
        }
    }

    /// stdout carries the id and nothing else, so `T=$(prelude task start
    /// "…")` is the whole integration — the same door `prelude ask --no-wait`
    /// opens.
    fn start_cmd(&self, opts: &Opts, title: &str) -> i32 {
        if title.trim().is_empty() {
            eprintln!("prelude: start what?");
            return 2;
        }
        let mut spec = New::here(title);
        if !opts.project.is_empty() {
            spec.project = opts.project.clone();
        }
        if !opts.agent.is_empty() {
            spec.agent = opts.agent.clone();
        }
        spec.run = opts.run.clone();
        spec.session = opts.session.clone();
        spec.message = opts.message.clone();
        spec.prompt_ref = opts.prompt_ref.clone();
        spec.deps = opts.needs.clone();
        // A task with an unmet dependency has not started; it is queued.
        let result = if spec.deps.is_empty() { self.start(spec) } else { self.queue(spec) };
        self.opened(result, opts)
    }

    /// A verb that *creates* a task answers with its id and nothing else, so
    /// `T=$(prelude task start "…")` and `T=$(prelude task retry "$T")` are
    /// both the whole integration. Everything a person would want to read
    /// goes to stderr, exactly as `ask` does.
    ///
    /// `--json` is the machine mode and replaces that with the whole record
    /// on stdout, consistently with every other verb here: an agent that
    /// wants fields reads `.id` out of them, and one that wants the id alone
    /// asks for no flag at all.
    fn opened(&self, result: Result<Task, String>, opts: &Opts) -> i32 {
        match result {
            Err(e) => {
                eprintln!("prelude: {e}");
                2
            }
            Ok(task) => {
                if opts.json {
                    print_json(&task);
                } else {
                    eprintln!(
                        "prelude: task {} {} — report with  prelude task progress {} \"…\"",
                        task.id,
                        task.state.as_str(),
                        task.id
                    );
                    println!("{}", task.id);
                }
                0
            }
        }
    }

    fn report(&self, result: Result<Task, String>, opts: &Opts) -> i32 {
        match result {
            Err(e) => {
                eprintln!("prelude: {e}");
                2
            }
            Ok(task) => {
                if opts.json {
                    print_json(&task);
                } else {
                    println!("{}", line_of(&task, &[]));
                }
                0
            }
        }
    }

    fn list_cmd(&self, opts: &Opts) -> i32 {
        let tasks = self.all();
        let mut shown: Vec<&Task> =
            tasks.iter().filter(|task| opts.all || !task.state.finished()).collect();
        shown.sort_by_key(|task| std::cmp::Reverse(task.updated_at));
        if opts.json {
            println!("{}", serde_json::to_string_pretty(&shown).unwrap_or_else(|_| "[]".into()));
            return 0;
        }
        if shown.is_empty() {
            println!("no open tasks");
            return 0;
        }
        for task in shown {
            println!("{}", line_of(task, &tasks));
        }
        0
    }

    fn show_cmd(&self, opts: &Opts, id: &str) -> i32 {
        let Some(task) = self.get(id) else {
            eprintln!("prelude: no task {id}");
            return 2;
        };
        let events = self.for_task(&task.id);
        if opts.json {
            let record = serde_json::json!({ "task": task, "events": events });
            println!("{}", serde_json::to_string_pretty(&record).unwrap_or_else(|_| "{}".into()));
            return 0;
        }
        println!("{}", line_of(&task, &self.all()));
        if !task.deps.is_empty() {
            println!("  needs     {}", task.deps.join(", "));
        }
        for (label, value) in [
            ("run", task.run.as_deref()),
            ("session", task.session.as_deref()),
            ("message", task.message.as_deref()),
            ("prompt", task.prompt_ref.as_deref()),
            ("retry of", task.retry_of.as_deref()),
            ("result", task.result.as_deref()),
            ("reason", task.reason.as_deref()),
        ] {
            if let Some(value) = value {
                println!("  {} {value}", crate::width::pad_to(label, 9, false));
            }
        }
        for event in events {
            let detail = event.detail.unwrap_or_default();
            println!(
                "  {} {}{}",
                event.ts,
                event.kind,
                if detail.is_empty() { String::new() } else { format!("  {detail}") }
            );
        }
        0
    }
}

/// One readable line. `blocked` rather than `queued` when a dependency is the
/// reason nothing is happening — that is the question a person scanning this
/// list is actually asking.
///
/// Padded by display width, never by character count. `who` is built with
/// `·`, which is the East Asian *Ambiguous* character CLAUDE.md names by
/// hand, and a project in CJK put the title column seven places out.
fn line_of(task: &Task, all: &[Task]) -> String {
    let state = if task.state == State::Queued && !all.is_empty() && !task.ready(all) {
        "blocked"
    } else {
        task.state.as_str()
    };
    let who = match (task.agent.as_str(), task.project.as_str()) {
        ("", "") => String::new(),
        (agent, "") => agent.to_string(),
        ("", project) => project.to_string(),
        (agent, project) => format!("{agent} · {project}"),
    };
    format!(
        "{} {} {}  [{}]",
        crate::width::pad_to(state, 9, false),
        crate::width::pad_to(&who, 24, false),
        task.title,
        task.id
    )
}

fn print_json(task: &Task) {
    println!("{}", serde_json::to_string_pretty(task).unwrap_or_else(|_| "{}".into()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::testing;

    /// A private store that touches no environment variable, so any number of
    /// these run at once — the `sessions.rs` rule.
    fn store_in(name: &str) -> (testing::Root, Store) {
        let root = testing::root(name);
        let store = Store::at(root.path.clone());
        (root, store)
    }

    fn spec(title: &str) -> New {
        New {
            title: title.into(),
            agent: "claude".into(),
            project: "Prelude".into(),
            ..Default::default()
        }
    }

    #[test]
    fn ids_are_unique_within_one_second_and_never_change() {
        let (_root, store) = store_in("task-ids");
        let first = store.start(spec("migrate users")).expect("first");
        let second = store.start(spec("migrate users")).expect("second");
        assert_ne!(first.id, second.id, "two tasks in the same second collided");
        assert_eq!(store.all().len(), 2, "one overwrote the other on disk");

        let assigned = store.assign(&first.id, "codex").expect("assign");
        let progressed = store.progress(&first.id, "halfway").expect("progress");
        let finished = store.done(&first.id, "shipped").expect("done");
        for later in [&assigned, &progressed, &finished] {
            assert_eq!(later.id, first.id, "the id must survive every operation");
        }
    }

    /// The `O_EXCL` reservation, under the load it exists for.
    #[test]
    fn concurrent_reservations_are_all_distinct() {
        let (_root, store) = store_in("task-race");
        let threads = 64;
        let ids: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|n| {
                    let store = &store;
                    scope.spawn(move || store.start(spec(&format!("job {n}"))).expect("start").id)
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("thread")).collect()
        });
        let unique: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(unique.len(), threads, "two threads were given the same id");
        assert_eq!(store.all().len(), threads, "a record was overwritten");
    }

    #[test]
    fn every_transition_reaches_the_event_log() {
        let (_root, store) = store_in("task-events");
        let task = store.start(spec("reindex")).expect("start");
        store.progress(&task.id, "reading files").expect("progress");
        store.wait(&task.id, "needs a decision").expect("wait");
        store.assign(&task.id, "codex").expect("assign");
        store.done(&task.id, "reindexed").expect("done");

        let kinds: Vec<String> =
            store.for_task(&task.id).into_iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec!["task.start", "task.progress", "task.wait", "task.assign", "task.done"],
            "one operation, one event, in order"
        );
        assert!(
            store.for_task(&task.id).iter().all(|e| e.task == task.id),
            "an event must name the task it belongs to"
        );
        // Terminal really is terminal.
        assert!(store.progress(&task.id, "more").is_err(), "a finished task cannot progress");
    }

    #[test]
    fn a_dependency_blocks_until_it_is_done() {
        let (_root, store) = store_in("task-deps");
        let first = store.start(spec("build the schema")).expect("first");
        let second = store
            .queue(New { deps: vec![first.id.clone()], ..spec("migrate onto it") })
            .expect("second");

        assert!(!second.ready(&store.all()), "an unfinished dependency must block");
        assert!(first.ready(&store.all()), "a task with no dependencies is ready");

        store.done(&first.id, "done").expect("finish the dependency");
        assert!(second.ready(&store.all()), "a done dependency unblocks it");

        // A dependency that is not in the set at all is not evidence of
        // success.
        let orphaned_dep = store
            .queue(New { deps: vec!["t-nonexistent".into()], ..spec("hopeful") })
            .expect("queued");
        assert!(!orphaned_dep.ready(&store.all()));

        // And a dependency that is not an id at all is refused outright.
        assert!(
            store.queue(New { deps: vec!["../../etc/passwd".into()], ..spec("sneaky") }).is_err(),
            "a dependency can only ever be a task id"
        );
    }

    #[test]
    fn retry_makes_a_new_task_and_leaves_the_original_alone() {
        let (_root, store) = store_in("task-retry");
        let original = store.start(spec("publish")).expect("start");

        // Finding 12: a live task retried is two agents on one job.
        assert!(store.retry(&original.id).is_err(), "a running task must not be retried");

        store.fail(&original.id, "registry refused it").expect("fail");

        let again = store.retry(&original.id).expect("retry");
        assert_ne!(again.id, original.id);
        assert_eq!(again.retry_of.as_deref(), Some(original.id.as_str()));
        assert_eq!(again.title, original.title);
        assert_eq!(again.state, State::Queued);

        let kept = store.get(&original.id).expect("the original is still there");
        assert_eq!(kept.state, State::Failed);
        assert_eq!(kept.reason.as_deref(), Some("registry refused it"));
        assert!(kept.retry_of.is_none(), "the original is nobody's retry");
    }

    #[test]
    fn orphans_are_open_tasks_whose_run_has_gone() {
        let (_root, store) = store_in("task-orphans");
        let live =
            store.start(New { run: "claude:100:1".into(), ..spec("still going") }).expect("live");
        let lost =
            store.start(New { run: "claude:200:1".into(), ..spec("its agent died") }).expect("lost");
        let ended = store
            .start(New { run: "claude:300:1".into(), ..spec("finished properly") })
            .expect("ended");
        store.done(&ended.id, "").expect("done");
        let detached = store.start(spec("no run edge at all")).expect("detached");

        let tasks = store.all();
        let runs = vec!["claude:100:1".to_string()];
        let found: Vec<&str> =
            orphans(&tasks, &runs, &[]).into_iter().map(|t| t.id.as_str()).collect();
        assert_eq!(found, vec![lost.id.as_str()], "only the open task with a dead run");
        assert!(!found.contains(&live.id.as_str()));
        assert!(!found.contains(&ended.id.as_str()), "a done task is not an orphan");
        assert!(!found.contains(&detached.id.as_str()), "nothing was attached to lose");

        // A session that no longer exists counts too.
        let sessionless = store
            .start(New { session: "claude:abc".into(), ..spec("its session was trashed") })
            .expect("sessionless");
        let tasks = store.all();
        let found: Vec<&str> = orphans(&tasks, &runs, &["claude:other".to_string()])
            .into_iter()
            .map(|t| t.id.as_str())
            .collect();
        assert!(found.contains(&sessionless.id.as_str()));

        // Detected, never repaired: the state is untouched until somebody asks.
        assert_eq!(store.get(&lost.id).expect("still there").state, State::Working);
        store.mark_orphaned(&lost.id).expect("repair");
        assert_eq!(store.get(&lost.id).expect("still there").state, State::Failed);
    }

    /// Finding 2, one assertion per field.
    ///
    /// The old version of this test exercised `title` and `prompt_ref`, which
    /// were the only two fields that already went through the filter, and so
    /// passed while fourteen other paths leaked. A field added later that
    /// forgets the filter has to fail here.
    #[test]
    fn a_credential_reaches_neither_the_record_nor_the_log() {
        let (_root, store) = store_in("task-secret");
        let key = "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01";
        let with = |field: &str| format!("{field} {key}");

        let task = store
            .start(New {
                title: with("rotate"),
                project: with("project"),
                cwd: with("/tmp/cwd"),
                agent: with("agent"),
                run: with("run"),
                session: with("session"),
                message: with("message"),
                prompt_ref: with("Authorization: Bearer"),
                deps: Vec::new(),
                retry_of: String::new(),
            })
            .expect("start");
        store.assign(&task.id, &with("assignee")).expect("assign");
        store.progress(&task.id, &with("progress note")).expect("progress");
        store.wait(&task.id, &with("waiting on")).expect("wait");
        store.link(&task.id, &with("run2"), &with("session2"), &with("message2")).expect("link");
        store.done(&task.id, &format!("rotated\nnew key is {key}\nverified")).expect("done");

        let stored = std::fs::read_to_string(store.file_of(&task.id)).expect("record");
        let record: serde_json::Value = serde_json::from_str(&stored).expect("json");
        for field in [
            "title", "project", "cwd", "agent", "run", "session", "message", "prompt_ref",
            "result", "reason", "retry_of",
        ] {
            let value = record.get(field).and_then(|v| v.as_str()).unwrap_or_default();
            assert!(!value.contains(key), "task.{field} kept a credential: {value}");
        }

        let log = std::fs::read_to_string(store.log().path()).expect("log");
        for event in store.for_task(&task.id) {
            let encoded = serde_json::to_string(&event).expect("encode");
            assert!(!encoded.contains(key), "event {} kept a credential: {encoded}", event.kind);
        }
        assert!(!stored.contains(key), "the task record kept a credential: {stored}");
        assert!(!log.contains(key), "the event log kept a credential: {log}");
        assert!(stored.contains("rotated") && stored.contains("verified"), "the rest survives");

        // A dependency is an id, so it can never be a place to hide one.
        assert!(store.queue(New { deps: vec![key.into()], ..spec("deps") }).is_err());
    }

    /// A working directory is a path, and the prose filter is the wrong
    /// instrument for one.
    ///
    /// `secrets::looks_secret` is deliberately broad because it decides what
    /// is kept out of shell history: "token" anywhere in a line is enough. A
    /// project legitimately called `token-service` was therefore stored as
    /// `[redacted]`, and `items_as` then handed the launcher a `cwd` that is
    /// not a directory — every row for that project lost its project, and
    /// `cd` from one of them would have gone nowhere.
    ///
    /// `project` is asserted here too, and that is the point. `New::here`
    /// fills it from the last component of the same cwd, so it is the same
    /// string under the same rule — and while this test looked only at `cwd`,
    /// a task started in `token-service` kept its directory and still showed
    /// `[redacted]` as its project everywhere a project is shown.
    #[test]
    fn a_project_directory_is_a_path_and_not_a_sentence() {
        let (_root, store) = store_in("task-cwd");
        for real in ["/Users/x/App/token-service", "/Users/x/App/secrets", "/Users/x/App/bearer"] {
            let name = real.rsplit('/').next().expect("a last component");
            let task = store
                .start(New { cwd: real.into(), project: name.into(), ..spec("build it") })
                .expect("start");
            assert_eq!(task.cwd, real, "a real directory was rewritten into prose");
            assert_eq!(task.project, name, "a real project was rewritten into prose");
            assert!(
                !task.cwd.contains("[redacted]") && !task.project.contains("[redacted]"),
                "items_as would hand the launcher a cwd and a project that do not exist",
            );
            // And the columns every view of a task shares carry it too.
            assert_eq!(fields_of(&task)[0], name, "the launcher row lost the project");
        }
        // Refused rather than rewritten, on `capability.rs`'s path rule: a
        // task in one of these keeps its title, its state and its history and
        // simply carries no directory.
        // `capability.rs`'s path rule is stricter than it is clever —
        // `password-manager-ui` is a plausible project and is refused anyway
        // — but it errs by storing *nothing*, which the row and the `cd`
        // both survive, rather than by storing a directory that is not there.
        for refused in [
            "/Users/x/.aws/credentials.d",
            "/Users/x/project/.env",
            "/Users/x/App/password-manager-ui",
            "/tmp/p sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ01",
        ] {
            let task = store
                .start(New { cwd: refused.into(), project: refused.into(), ..spec("rotate") })
                .expect("start");
            assert!(task.cwd.is_empty(), "{refused} was stored as {}", task.cwd);
            assert!(task.project.is_empty(), "{refused} was stored as {}", task.project);
        }
        // `~/.aws` itself says nothing about credentials and is an ordinary
        // directory: the doc used to claim it was refused, and it never was.
        let task = store
            .start(New { cwd: "/Users/x/.aws".into(), project: ".aws".into(), ..spec("look") })
            .expect("start");
        assert_eq!(task.cwd, "/Users/x/.aws", "an ordinary directory keeps its path");
        assert_eq!(task.project, ".aws");
        // And never half a path, which would be a different directory, nor
        // half a name, which matches no project.
        let deep = format!("/Users/x/{}", "a".repeat(CWD_MAX + 10));
        let task = store
            .start(New { cwd: deep, project: "b".repeat(events::FIELD_MAX + 1), ..spec("deep") })
            .expect("start");
        assert!(task.cwd.is_empty(), "a truncated path is not the same directory");
        assert!(task.project.is_empty(), "a truncated name is not the same project");
    }

    /// Finding 4: a reason, a result and a progress note are prose.
    #[test]
    fn a_free_text_verb_keeps_the_dashes_in_its_prose() {
        let (_root, store) = store_in("task-prose");
        let one = store.start(spec("one")).expect("one");
        assert_eq!(store.cli(&["fail", &one.id, "could", "not", "resolve", "--needs", "more", "time"]), 0);
        assert_eq!(
            store.get(&one.id).expect("one").reason.as_deref(),
            Some("could not resolve --needs more time"),
            "a value flag was eaten out of a failure reason"
        );

        let two = store.start(spec("two")).expect("two");
        assert_eq!(store.cli(&["done", &two.id, "it", "printed", "--json", "badly"]), 0);
        assert_eq!(
            store.get(&two.id).expect("two").result.as_deref(),
            Some("it printed --json badly"),
            "--json in prose switched the output mode"
        );

        let three = store.start(spec("three")).expect("three");
        assert_eq!(store.cli(&["progress", &three.id, "reading", "--all", "the", "files"]), 0);
        let notes: Vec<String> = store
            .for_task(&three.id)
            .into_iter()
            .filter(|e| e.kind == "task.progress")
            .filter_map(|e| e.detail)
            .collect();
        assert_eq!(notes, vec!["reading --all the files".to_string()]);

        let four = store.start(spec("four")).expect("four");
        assert_eq!(store.cli(&["wait", &four.id, "--cancel", "was", "requested"]), 0);
        assert_eq!(store.get(&four.id).expect("four").state, State::Waiting);
        let five = store.start(spec("five")).expect("five");
        assert_eq!(store.cli(&["cancel", &five.id, "the", "--project", "moved"]), 0);
        assert_eq!(
            store.get(&five.id).expect("five").reason.as_deref(),
            Some("the --project moved")
        );

        // The flags that were meant still work, in each of the three places
        // one can legitimately stand.
        let six = store.start(spec("six")).expect("six");
        assert_eq!(store.cli(&["--json", "done", &six.id, "shipped"]), 0);
        let seven = store.start(spec("seven")).expect("seven");
        assert_eq!(store.cli(&["done", "--json", &seven.id, "shipped"]), 0);
        let eight = store.start(spec("eight")).expect("eight");
        assert_eq!(store.cli(&["done", &eight.id, "--json"]), 0);
        assert_eq!(store.get(&eight.id).expect("eight").result, None);

        // `start` keeps the relaxed rule: a flag after the title still reads
        // perfectly well, which is the whole reason for the split.
        assert_eq!(store.cli(&["start", "write", "the", "docs", "--agent", "codex"]), 0);
        let written = store.all().into_iter().find(|t| t.title == "write the docs").expect("start");
        assert_eq!(written.agent, "codex");
    }

    /// Finding 13: `progress` is the one verb that needs its note.
    #[test]
    fn progress_without_a_note_says_so() {
        let (_root, store) = store_in("task-note");
        let task = store.start(spec("something")).expect("start");
        assert_eq!(store.cli(&["progress", &task.id]), 2);
        // The others are allowed to be silent.
        assert_eq!(store.cli(&["wait", &task.id]), 0);
        assert_eq!(store.cli(&["done", &task.id]), 0);
    }

    #[test]
    fn the_cli_refuses_a_bad_request_and_cannot_escape_the_store() {
        let (_root, store) = store_in("task-cli");
        assert_eq!(store.cli(&["progress", "t-nope", "hello"]), 2, "no such task");
        assert_eq!(store.cli(&["show", "../../etc/passwd"]), 2, "an id is not a path");
        assert!(store.get("../../etc/passwd").is_none());

        assert_eq!(store.cli(&["start", "write", "the", "docs", "--agent", "codex"]), 0);
        let opened = store.all();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].title, "write the docs", "flags are not part of the title");
        assert_eq!(opened[0].agent, "codex");

        assert_eq!(store.cli(&["done", &opened[0].id, "shipped"]), 0);
        assert_eq!(store.cli(&["done", &opened[0].id, "again"]), 2, "already finished");
    }

    /// Finding 8: the filename was validated and the id inside the file was
    /// believed.
    #[test]
    fn a_record_that_lies_about_its_own_id_is_not_a_record() {
        let (_root, store) = store_in("task-forged");
        let honest = store.start(spec("real work")).expect("start");
        store.ensure_dir().expect("dir");

        for forged in ["../../../../tmp/pwn", "not-the-filename"] {
            let record = serde_json::json!({
                "id": forged,
                "title": "forged",
                "created_at": 1u64,
                "updated_at": 1u64,
            });
            std::fs::write(
                store.dir().join("forged.json"),
                serde_json::to_vec(&record).expect("encode"),
            )
            .expect("write");
            let ids: Vec<String> = store.all().into_iter().map(|t| t.id).collect();
            assert_eq!(ids, vec![honest.id.clone()], "a forged id came back out of all()");
        }

        // And `save` refuses to write one in the first place.
        let doomed = Task { id: "../../../../tmp/pwn".into(), ..Default::default() };
        assert!(store.save(&doomed).is_err(), "save trusted an id from a record");
    }

    /// Finding 11: an id taken and never written is burnt forever.
    #[test]
    fn an_abandoned_reservation_gives_the_id_back() {
        let (_root, store) = store_in("task-reserve");
        let taken = {
            let held = store.reserve().expect("reserve");
            let id = held.id.clone();
            assert!(store.file_of(&id).exists(), "an id is taken by creating its file");
            assert!(store.all().is_empty(), "an empty reservation is not a task");
            id
            // `held` drops here without `keep`.
        };
        assert!(!store.file_of(&taken).exists(), "an abandoned reservation stayed behind");

        // The same id is therefore free again, rather than a permanent hole.
        let held = store.reserve().expect("reserve");
        assert_eq!(held.id, taken, "the id was burnt");
        held.keep();
        assert!(store.file_of(&taken).exists());
    }

    /// Finding 9: finished tasks are swept, and the launcher reads only what
    /// is open.
    #[test]
    fn finished_tasks_are_swept_and_open_tasks_reads_only_the_open_ones() {
        let (_root, store) = store_in("task-sweep");
        let old = store.start(spec("long done")).expect("old");
        store.done(&old.id, "shipped").expect("done");
        let kept = store.start(spec("still going")).expect("kept");
        let recent = store.start(spec("just finished")).expect("recent");
        store.done(&recent.id, "shipped").expect("done");

        // A dependency of an open task survives its retention window.
        let dep = store.start(spec("prerequisite")).expect("dep");
        store.done(&dep.id, "ready").expect("done");
        let dependant = store
            .queue(New { deps: vec![dep.id.clone()], ..spec("waiting on it") })
            .expect("dependant");

        // Age the two finished tasks past the window, by hand.
        for id in [&old.id, &dep.id] {
            let mut task = store.get(id).expect("task");
            task.finished_at = Some(1);
            task.updated_at = 1;
            store.save(&task).expect("save");
        }
        store.sweep();

        assert!(store.get(&old.id).is_none(), "a long-finished task was not swept");
        assert!(store.get(&dep.id).is_some(), "an open task's dependency must never be swept");
        assert!(store.get(&recent.id).is_some(), "a recently finished task is still the record");

        let open: Vec<String> = store.open_tasks().into_iter().map(|t| t.id).collect();
        assert_eq!(open, vec![kept.id.clone(), dependant.id.clone()], "open_tasks is the open set");

        // The index is a hint: losing it must not mean "no work outstanding".
        std::fs::remove_file(store.index()).expect("remove the index");
        let open: Vec<String> = store.open_tasks().into_iter().map(|t| t.id).collect();
        assert_eq!(open, vec![kept.id, dependant.id], "a missing index must fall back, not lie");
    }

    /// The fallback used to do 54 ms of work and throw the answer away.
    ///
    /// `sweep` only rewrites the index where a task *finishes*, so on a store
    /// where nothing finishes a lost index never came back: every launch, and
    /// every explicit command, walked the whole directory again. The scan is
    /// the same scan; what changed is that it leaves the file behind.
    #[test]
    fn a_lost_index_is_rebuilt_by_the_reader_that_paid_for_it() {
        let (_root, store) = store_in("task-index-lost");
        let first = store.start(spec("still going")).expect("first");
        let second = store.queue(spec("waiting its turn")).expect("second");
        let over = store.start(spec("already finished")).expect("over");
        store.cancel(&over.id, "not needed").expect("cancel");

        std::fs::remove_file(store.index()).expect("remove the index");
        assert!(store.read_index().is_none(), "the index is gone");

        // The right answer first — a fallback that lies is worse than a slow
        // one.
        let open: Vec<String> = store.open_tasks().into_iter().map(|t| t.id).collect();
        assert_eq!(open, vec![first.id.clone(), second.id.clone()]);

        // …and the file is back, so nobody pays for that scan again. A
        // cancelled task is not in it: cancelling is itself the decision, so
        // it is neither open nor awaiting anybody's review.
        let rebuilt = store.read_index().expect("the index was not rebuilt");
        assert_eq!(rebuilt, vec![first.id.clone(), second.id.clone()], "{rebuilt:?}");

        // And the rebuilt file is what the next read uses: nothing on disk
        // changed, so a stale reader would still see both.
        assert_eq!(store.open_tasks().len(), 2);
    }

    /// The launcher's reader never walks the directory, whatever it finds.
    ///
    /// A gather has forty milliseconds for everything; the scan alone measures
    /// 54. So this one returns nothing, hands the repair to a detached
    /// process, and is complete again the moment that process has run — while
    /// `open_tasks`, which every explicit command uses, keeps answering
    /// truthfully throughout.
    #[test]
    fn the_launcher_defers_a_rebuild_instead_of_paying_for_it() {
        let (_root, store) = store_in("task-index-defer");
        let live = store.start(spec("still going")).expect("live");
        std::fs::remove_file(store.index()).expect("remove the index");

        assert!(store.home_tasks().is_empty(), "the gather path scanned the directory");
        // Nothing was spawned for a store that is not the real one — a helper
        // resolves its own root and would have rebuilt somebody else's index.
        assert!(store.read_index().is_none(), "the launcher repaired it in the foreground");
        // The explicit path is unaffected, which is the whole reason the
        // launcher is allowed to say nothing.
        assert_eq!(store.open_tasks().len(), 1, "task list must still be true");

        store.rebuild_index();
        let shown: Vec<String> = store.home_tasks().into_iter().map(|t| t.id).collect();
        assert_eq!(shown, vec![live.id], "the next launch shows everything");
    }

    /// A rebuild reads the directory whole and replaces the file. A task
    /// opened between those two moments would never be in the new one, and
    /// nothing would notice until something else happened to finish — the
    /// exact missing row the index exists to prevent.
    #[test]
    fn a_rebuild_cannot_lose_a_task_opened_while_it_runs() {
        let (_root, store) = store_in("task-index-race");
        let each = 40;
        std::thread::scope(|scope| {
            let store = &store;
            scope.spawn(move || {
                for _ in 0..each {
                    store.rebuild_index();
                }
            });
            scope.spawn(move || {
                for n in 0..each {
                    store.start(spec(&format!("job {n}"))).expect("start");
                }
            });
        });

        let indexed: std::collections::HashSet<String> =
            store.read_index().expect("index").into_iter().collect();
        let opened: Vec<String> = store.all().into_iter().map(|t| t.id).collect();
        assert_eq!(opened.len(), each, "a record was lost outright");
        for id in &opened {
            assert!(indexed.contains(id), "{id} was opened during a rebuild and dropped from it");
        }
        assert_eq!(store.open_tasks().len(), each, "the fast path cannot see them all");
    }

    /// Slots 2 and 3 of the Agent Home: a finished task keeps asking until
    /// somebody says they have seen it.
    ///
    /// Acknowledgement is an explicit act. It is never inferred from the row
    /// having been looked at, because focus and Quick Look cross rows without
    /// any decision being made, and a task is routinely opened while it is
    /// still running — so "opening it counts" would silently drop the
    /// completion notice that arrives an hour later.
    #[test]
    fn a_finished_task_keeps_asking_until_it_is_dismissed() {
        let (_root, store) = store_in("task-ack");
        let live = store.start(spec("still going")).expect("live");
        let good = store.start(spec("shipped it")).expect("good");
        let bad = store.start(spec("could not ship it")).expect("bad");
        let stopped = store.start(spec("changed my mind")).expect("stopped");
        store.done(&good.id, "released 1.4").expect("done");
        store.fail(&bad.id, "registry refused it").expect("fail");
        store.cancel(&stopped.id, "not needed").expect("cancel");

        let shown = |store: &Store| -> Vec<String> {
            store.home_tasks().into_iter().map(|t| t.id).collect()
        };
        let rows = shown(&store);
        assert!(rows.contains(&good.id), "a completed task must await review: {rows:?}");
        assert!(rows.contains(&bad.id), "a failed task must await review: {rows:?}");
        assert!(rows.contains(&live.id));
        // Cancelling *is* the decision, so it is not news to be reported back.
        assert!(!rows.contains(&stopped.id), "a cancelled task asked to be reviewed: {rows:?}");
        // …and the open set is still only what is open.
        assert_eq!(shown_open(&store), vec![live.id.clone()]);

        // A live task is not asking to be dismissed; it is asking to be
        // finished, and saying so beats writing down an acknowledgement of a
        // result that does not exist.
        assert!(store.ack(&live.id).is_err(), "a running task was dismissed");

        let acked = store.ack(&good.id).expect("ack");
        let when = acked.acked_at.expect("dismissal is a timestamp, so `when` is answerable");
        assert!(!shown(&store).contains(&good.id), "a dismissed task kept asking");
        assert!(shown(&store).contains(&bad.id), "dismissing one dismissed the other");
        // The record itself is untouched: this is not a delete.
        let kept = store.get(&good.id).expect("the record stays");
        assert_eq!(kept.state, State::Done);
        assert_eq!(kept.result.as_deref(), Some("released 1.4"));
        assert!(store.for_task(&good.id).iter().any(|e| e.kind == "task.ack"), "no event");
        // Idempotent, and the first answer is the one kept.
        assert_eq!(store.ack(&good.id).expect("again").acked_at, Some(when));
        assert_eq!(
            store.for_task(&good.id).iter().filter(|e| e.kind == "task.ack").count(),
            1,
            "a second dismissal is not new information",
        );

        // Age is the other bound: a failure nobody dismissed by the next
        // morning has stopped being news and become history.
        let mut old = store.get(&bad.id).expect("bad");
        old.finished_at = Some(crate::bus::now().saturating_sub(AWAITING_REVIEW + 60));
        store.save(&old).expect("save");
        assert!(!shown(&store).contains(&bad.id), "a day-old failure was still on the home");
        assert!(store.get(&bad.id).is_some(), "it is history, not deleted");
    }

    fn shown_open(store: &Store) -> Vec<String> {
        store.open_tasks().into_iter().map(|t| t.id).collect()
    }

    /// Age alone is not a bound. A fleet that fails a hundred jobs overnight
    /// would put a hundred rows on the home, every one of them inside the
    /// window — which is a log, not a notice.
    #[test]
    fn undismissed_results_are_bounded_by_count_as_well_as_by_age() {
        let (_root, store) = store_in("task-ack-bound");
        for n in 0..(AWAITING_SHOWN * 3) {
            let failed = store.start(spec(&format!("failure {n}"))).expect("start");
            store.fail(&failed.id, "boom").expect("fail");
            let done = store.start(spec(&format!("success {n}"))).expect("start");
            store.done(&done.id, "fine").expect("done");
        }
        let rows = store.home_tasks();
        let count = |state: State| rows.iter().filter(|t| t.state == state).count();
        assert_eq!(count(State::Failed), AWAITING_SHOWN, "failures are unbounded");
        assert_eq!(count(State::Done), AWAITING_SHOWN, "completions are unbounded");
        // Counted per state, so a burst of one cannot push the other off the
        // screen — and the newest of each is what survives.
        let newest = rows
            .iter()
            .filter(|t| t.state == State::Failed)
            .map(|t| t.finished_at.unwrap_or_default())
            .min()
            .expect("a failure");
        let dropped = store
            .all()
            .into_iter()
            .filter(|t| t.state == State::Failed)
            .filter(|t| !rows.iter().any(|shown| shown.id == t.id))
            .all(|t| t.finished_at.unwrap_or_default() <= newest);
        assert!(dropped, "an older failure was kept over a newer one");
        // The record of every one of them is still there.
        assert_eq!(store.all().len(), AWAITING_SHOWN * 6);

        // …and the bound is in the *index*, not only in the answer. One line
        // is one `get` for the reader that must not walk the directory, so a
        // count applied after that read would have bounded nothing: this is
        // the difference between the launcher reading sixteen small files and
        // reading a whole busy day's worth.
        let index = store.read_index().expect("index");
        assert!(
            index.len() <= AWAITING_SHOWN * 2,
            "the index carries {} lines for {} rows",
            index.len(),
            rows.len(),
        );
    }

    /// Sweeping used to drop every finished task from the index the moment it
    /// finished, which is why slots 2 and 3 could never be filled: the one
    /// reader that must not walk the directory had no way to find one.
    #[test]
    fn sweeping_keeps_a_result_nobody_has_seen_yet() {
        let (_root, store) = store_in("task-ack-sweep");
        let done = store.start(spec("shipped it")).expect("start");
        store.done(&done.id, "released").expect("done");
        // `done` already swept. The index must still carry it.
        assert!(
            store.read_index().expect("index").contains(&done.id),
            "a result nobody has seen was dropped from the index",
        );
        assert!(store.get(&done.id).is_some(), "and the record must survive its own sweep");

        // Dismissing it puts it back under the ordinary thirty-day rule.
        store.ack(&done.id).expect("ack");
        assert!(!store.read_index().expect("index").contains(&done.id));
        assert!(store.get(&done.id).is_some(), "dismissing is not deleting");
    }

    /// Finding 6: `·` is East Asian Ambiguous and CJK is two columns.
    #[test]
    fn columns_are_padded_by_display_width() {
        let (_root, store) = store_in("task-width");
        let ascii = store.start(spec("ascii")).expect("ascii");
        let cjk = store
            .start(New { project: "基盤システム".into(), ..spec("cjk") })
            .expect("cjk");

        let width_to_title = |task: &Task| {
            let line = line_of(task, &[]);
            let title = line.find(&task.title).expect("the title is on the line");
            crate::width::dwidth(&line[..title])
        };
        assert_eq!(
            width_to_title(&ascii),
            width_to_title(&cjk),
            "the title column moved because a project was CJK"
        );
    }
}

