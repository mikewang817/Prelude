//! Caching, background refresh, and the tiered gather.

use crate::item::Item;
use crate::paths;
use std::time::Duration;

/// Replace a file's contents in one step, with no moment at which a reader
/// can see half of them.
///
/// The temporary name has to be unique per writer, and it was not: it was
/// `path.with_extension("tmp")`, one name per destination for the whole
/// machine. Two Preludes writing the same cache — which is the normal state
/// of affairs, since every shell has one and the panel has another — wrote
/// their bytes into the *same* temporary file, interleaved, and then each
/// renamed it. The rename is atomic and so the result is never half a file;
/// it is something better hidden and worse: a whole file containing two
/// answers spliced together. `with_extension` also merges distinct
/// destinations, so `list.txt` and a future `list.json` would collide too.
///
/// `create_new` makes the name ours or fails, and the pid, a counter and the
/// clock make a collision take deliberate effort.
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    write_with(path, bytes, false)
}

/// The same, for a file the person cannot rebuild: preferences, favorites,
/// messages, session metadata.
///
/// Two differences, both about surviving. It is created 0600 rather than
/// inheriting the umask, and the bytes are flushed to the disk before the
/// rename rather than after it — without that, a crash between the two can
/// leave the directory entry pointing at a file whose contents never landed,
/// which is how an atomic write turns a good file into an empty one. A cache
/// does not need either: it is derived, and the cost is a real fsync per
/// write on a path that runs per launch.
pub fn write_state(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    write_with(path, bytes, true)
}

fn write_with(path: &std::path::Path, bytes: &[u8], durable: bool) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = unique_temp(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    if durable {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        if durable {
            file.sync_all()?;
        }
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        // Never leave our own litter behind for the next writer to trip over.
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `name.tmp.<pid>.<nonce>`, beside the destination so the rename stays
/// within one filesystem.
fn unique_temp(path: &std::path::Path) -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = format!(".{name}.tmp.{}.{:x}", std::process::id(), n ^ clock);
    match path.parent() {
        Some(dir) => dir.join(tmp),
        None => std::path::PathBuf::from(tmp),
    }
}

// ─── read, change, write ─────────────────────────────────────────────────
//
// An atomic rename stops a reader seeing half a file. It does nothing at all
// about two writers who each read the whole file, changed their own copy and
// wrote it back: both writes are individually atomic and one of them is
// simply gone. Every such cycle here is a person's own data — the command
// they just ran, the thing they just favourited, the capability they just
// archived — so the lost update is silent and the file looks fine.
//
// `flock` is the smallest thing that fixes it. It is advisory, which is all
// that is needed when every writer is this program, and it is held on a
// separate `.lock` file so the lock survives the rename that replaces the
// data file underneath it.

const LOCK_EX: i32 = 2;
const LOCK_UN: i32 = 8;
const LOCK_NB: i32 = 4;

unsafe extern "C" {
    unsafe fn flock(fd: i32, operation: i32) -> i32;
}

pub struct Lock(std::fs::File);

impl Drop for Lock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe { flock(self.0.as_raw_fd(), LOCK_UN) };
    }
}

/// Take the write lock for a file, or give up and carry on without it.
///
/// Giving up is deliberate and follows the rule every source here follows: a
/// launcher may lose an update, but it may never hang. The wait is bounded at
/// a fraction of a second, which is orders of magnitude more than any of
/// these cycles takes and far less than a person can perceive.
pub fn lock_for_write(path: &std::path::Path) -> Option<Lock> {
    use std::os::fd::AsRawFd;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned())?;
    let lock_path = path.with_file_name(format!(".{name}.lock"));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .ok()?;
    let deadline = std::time::Instant::now() + Duration::from_millis(250);
    loop {
        if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
            return Some(Lock(file));
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Re-invoke ourselves as a detached background helper.
pub fn spawn_self(args: &[&str]) {
    let Ok(exe) = std::env::current_exe() else { return };
    let _ = std::process::Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn cache_file(name: &str) -> std::path::PathBuf {
    paths::cache().join(format!("{name}.json"))
}

/// One-time removal of derived rows written by builds that retained complete
/// MCP definitions. The private `borrow/` staging area is intentional and is
/// not touched; list/search caches must never contain those values.
pub fn privacy_migrations() {
    if std::env::var_os("PRELUDE_PRIVACY_MIGRATED").is_some() {
        return;
    }
    let marker = paths::cache().join("capability-privacy-v1");
    if !marker.exists() {
        let _ = read_cached("mcp"); // rewrites the MCP cache without `def`
        for name in ["search-items.json", "list.txt", "home.txt"] {
            let _ = std::fs::remove_file(paths::cache().join(name));
        }
        let _ = write_atomic(&marker, b"1\n");
    }
    std::env::set_var("PRELUDE_PRIVACY_MIGRATED", "1");
}

/// `score` is `#[serde(skip)]`, so anything read back from disk arrives at
/// zero and sorts below all two thousand live rows — which is why an MCP
/// server with priority 985 was never once seen near the top. The band is
/// restored from the kind, and a source that ranked its own items says so
/// in `rank`.
pub fn read_cached(name: &str) -> Vec<Item> {
    let mut items: Vec<Item> = std::fs::read_to_string(cache_file(name))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut scrubbed = false;
    for it in &mut items {
        // Older MCP caches retained complete definitions for fast lending.
        // Purge every one while reading so an upgrade stops exposing private
        // arguments before the background refresh lands.
        if name == "mcp" && !it.get("def").is_empty() {
            it.data.remove("def");
            // Old caches did not distinguish why a definition was private.
            // Conservatively mark it until the authoritative CLI refreshes.
            it.data.insert("sensitive".into(), "true".into());
            scrubbed = true;
        }
        it.score = it.band() as f64 + it.get("rank").parse::<f64>().unwrap_or(0.0);
    }
    if scrubbed {
        if let Ok(json) = serde_json::to_vec(&items) {
            let _ = write_atomic(&cache_file(name), &json);
        }
        for derived in ["search-items.json", "list.txt", "home.txt"] {
            let _ = std::fs::remove_file(paths::cache().join(derived));
        }
    }
    items
}

pub fn write_cached(name: &str, items: &[Item]) {
    if let Ok(json) = serde_json::to_vec(items) {
        let _ = write_atomic(&cache_file(name), &json);
    }
}

/// Write the result of a refresh, unless the refresh did not have one.
///
/// A source returns `Vec<Item>` and has nowhere in that type to say whether
/// it *found* nothing or *learned* nothing. Both arrived here as an empty
/// vector and both were written, so a `claude mcp list` that timed out — or
/// an agent CLI mid-upgrade, or a laptop that woke with no network — replaced
/// a perfectly good inventory with an empty one, and the launcher showed a
/// person with three MCP servers that they had none. The cache then looked
/// fresh, so nothing retried until its TTL expired.
///
/// `exec::lost_commands` is where the missing bit of information lives: the
/// difference across the source's own run counts the commands that could not
/// be started or had to be killed. An empty result with none of those is a
/// real empty result and is written. An empty result *with* one is a refresh
/// that failed, and the last good answer stays where it is.
///
/// Returns whether the refresh is considered to have succeeded, which is what
/// the caller records for backoff.
pub fn write_refreshed(name: &str, items: &[Item], lost_before: usize) -> bool {
    let lost = crate::exec::lost_commands() > lost_before;
    if keeps_last_good(items.is_empty(), lost, || !read_cached(name).is_empty()) {
        return false;
    }
    write_cached_if_changed(name, items);
    true
}

/// The rule itself, with no disk under it.
///
/// All three conditions are needed, and each one carries its own case. An
/// empty result with no lost command is a source honestly reporting an empty
/// world — the last container stopped, the last MCP server was removed — and
/// must be written, or the launcher would go on showing things the person has
/// just deleted. A non-empty result is always the newest truth, whatever else
/// failed while producing it. And with nothing in the cache to protect there
/// is nothing to weigh against a fresh empty answer.
///
/// The cache read is a closure because it is the expensive term and it is the
/// one that is almost never reached.
fn keeps_last_good(result_empty: bool, lost_command: bool, cache_has_rows: impl Fn() -> bool) -> bool {
    result_empty && lost_command && cache_has_rows()
}

/// How long to leave a failing source alone, after this many consecutive
/// failures. Doubling, and capped — a source that has been broken all day is
/// checked hourly rather than every five seconds, and one broken for five
/// seconds is not treated as though it were.
fn backoff_delay(name: &str, failures: u32) -> u64 {
    // Relative to this source's own cadence, not an absolute constant. A flat
    // sixty seconds looked like a backoff and was not one for `mcp`, whose
    // TTL is also sixty: a failing agent CLI would have been retried at
    // exactly the rate a working one is refreshed. Whatever else changes, the
    // first rest is at least twice the healthy interval.
    let floor = BACKOFF_START.max(refresh_ttl(name).saturating_mul(2));
    // The shift is clamped only to keep it defined; `BACKOFF_MAX` bounds the
    // result. Clamping it at 6 made the shift the real cap and left
    // `BACKOFF_MAX` unreachable — the constant documenting the ceiling was
    // decoration.
    let doublings = failures.clamp(1, 32).saturating_sub(1);
    floor
        .saturating_mul(1u64.checked_shl(doublings).unwrap_or(u64::MAX))
        .min(BACKOFF_MAX)
}

/// A derived cache often has identical bytes from one launch to the next.
/// Avoid an atomic rewrite in that common case while still paying only one
/// linear serialization over data the caller already holds.
pub fn write_cached_if_changed(name: &str, items: &[Item]) {
    let Ok(json) = serde_json::to_vec(items) else { return };
    write_if_changed(&cache_file(name), &json);
}

/// Replace a file only when its contents would actually differ.
///
/// Writing is not free and it is not cheap relative to what it is protecting:
/// measured on this machine, the two FAST cache writes cost 3.7ms and 2.7ms
/// against a 0.9ms file scan, so a launch spent more time recording what it
/// found than finding it. The overwhelmingly common case is that nothing has
/// changed since the last launch, and a read-and-compare is a fraction of a
/// write — one sequential read against an allocation, a create, a write, an
/// fsync-less flush and a rename.
///
/// It also stops the mtime moving, which matters beyond the cost: the panel's
/// refresh thread watches exactly these mtimes to decide whether anything has
/// happened. Rewriting identical bytes told it, every single launch, that
/// everything had changed.
pub fn write_if_changed(path: &std::path::Path, bytes: &[u8]) {
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return;
    }
    let _ = write_atomic(path, bytes);
}

/// Replace several files as close to together as a filesystem allows: every
/// one is staged first, and only then are they renamed one after another.
///
/// The three search snapshots — the root list, the home list and the complete
/// catalogue — are one gather's answer written into three files, and writing
/// them one at a time left a reader able to see a new one beside an old one.
/// Staging first shrinks that window from *serialize four hundred kilobytes
/// and render two lists* to *two renames*, which is microseconds.
///
/// It is deliberately not a generation directory with a `current` symlink.
/// That would close the window completely, and there is nothing left in it to
/// close: no answer is ever composed from two of these files. `dynamic` reads
/// the catalogue exactly when `needs_static_items` is true, and every query
/// that makes it true also makes `is_special` true, which returns before the
/// root list is opened. A mixed pair is therefore not an incoherent answer,
/// only an older one — and the cost of the symlink scheme is real: a
/// directory per gather, a cleanup policy for the ones nobody is reading, and
/// every path in the program resolved through one more indirection.
pub fn write_group(files: &[(std::path::PathBuf, Vec<u8>)]) {
    let mut staged = Vec::new();
    for (path, bytes) in files {
        // Unchanged files are not restaged; the common case is all three.
        if std::fs::read(path).is_ok_and(|old| &old == bytes) {
            continue;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = unique_temp(path);
        if std::fs::write(&tmp, bytes).is_ok() {
            staged.push((tmp, path));
        }
    }
    for (tmp, path) in staged {
        if std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

// ─── one refresh per source, and a rest after a failure ──────────────────
//
// `gather` used to spawn `_refresh <name>` for every stale source, every
// time, with nothing coordinating them. Open three shells at once and three
// processes ran `claude mcp list` — which performs a network health check per
// server — against the same cache, to write the same answer, three times.
// Worse, a refresh that failed left the cache untouched and therefore still
// stale, so the *next* gather spawned it again immediately: the cheapest way
// to turn one broken agent CLI into a permanent background load.
//
// Two files per source answer both. A lease is claimed with `create_new`, so
// the claim is the filesystem's to arbitrate rather than a check followed by
// a race; a backoff records when it is worth trying again.

fn refresh_dir() -> std::path::PathBuf {
    paths::cache().join("refresh")
}

fn lease_file(name: &str) -> std::path::PathBuf {
    refresh_dir().join(format!("{name}.lease"))
}

fn backoff_file(name: &str) -> std::path::PathBuf {
    refresh_dir().join(format!("{name}.backoff"))
}

/// A lease older than this is assumed to belong to a process that died
/// without cleaning up. It has to exceed the slowest legitimate refresh —
/// `mcp-tools` starts servers and waits for them — or a slow refresh would be
/// treated as dead while it was still working.
const LEASE_MAX: Duration = Duration::from_secs(180);

/// How long the slow tier is left alone after a refresh that learned nothing,
/// doubling per consecutive failure up to `BACKOFF_MAX`. The first step is
/// deliberately longer than any TTL here: the point is to stop asking a
/// broken thing at the rate we would ask a working one.
const BACKOFF_START: u64 = 60;
const BACKOFF_MAX: u64 = 3600;

/// At most this many slow sources refresh at once. Each is a subprocess, and
/// several of them are subprocesses that start subprocesses; a laptop waking
/// with every cache stale should not answer that with nine of them.
const MAX_CONCURRENT_REFRESH: usize = 2;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Is somebody already refreshing this source?
fn lease_is_live(name: &str) -> bool {
    let path = lease_file(name);
    let Ok(text) = std::fs::read_to_string(&path) else { return false };
    let fresh = path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < LEASE_MAX);
    if !fresh {
        // Nobody is coming back for it.
        let _ = std::fs::remove_file(&path);
        return false;
    }
    match text.split_whitespace().next().and_then(|p| p.parse::<i32>().ok()) {
        Some(pid) if crate::exec::alive(pid) => true,
        _ => {
            let _ = std::fs::remove_file(&path);
            false
        }
    }
}

/// Claim the right to refresh this source, or report that somebody else has.
///
/// `create_new` is the whole mechanism: the check and the claim are one
/// syscall, so two processes that decide to refresh at the same instant
/// cannot both win. The loser exits rather than duplicating the work.
fn claim_lease(name: &str) -> bool {
    let _ = std::fs::create_dir_all(refresh_dir());
    let path = lease_file(name);
    // A lease left by a dead process is cleared first, otherwise the source
    // would be locked out until something happened to remove it.
    lease_is_live(name);
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            use std::io::Write;
            let _ = writeln!(file, "{} {}", std::process::id(), now_secs());
            true
        }
        Err(_) => false,
    }
}

fn release_lease(name: &str) {
    let _ = std::fs::remove_file(lease_file(name));
}

/// When this source may be tried again, and how many times it has failed.
fn backoff_of(name: &str) -> (u64, u32) {
    std::fs::read_to_string(backoff_file(name))
        .ok()
        .and_then(|t| {
            let mut parts = t.split_whitespace();
            let until = parts.next()?.parse().ok()?;
            let failures = parts.next().and_then(|f| f.parse().ok()).unwrap_or(1);
            Some((until, failures))
        })
        .unwrap_or((0, 0))
}

fn record_failure(name: &str) {
    let (_, failures) = backoff_of(name);
    let failures = failures.saturating_add(1);
    let _ = write_atomic(
        &backoff_file(name),
        format!("{} {}\n", now_secs() + backoff_delay(name, failures), failures).as_bytes(),
    );
}

fn record_success(name: &str) {
    let _ = std::fs::remove_file(backoff_file(name));
}

/// Is this source resting after a failure?
fn resting(name: &str) -> bool {
    backoff_of(name).0 > now_secs()
}

fn refresh_ttl(name: &str) -> u64 {
    match name {
        "mcp-tools" => 300,
        // Four times a day is often enough to hear about a release and rare
        // enough that nobody notices the request.
        "update" => 21_600,
        "mcp" => 60,
        "skill-hashes" => 30,
        _ => 5,
    }
}

pub fn stale(name: &str) -> bool {
    cache_file(name)
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .map(|age| age.as_secs() >= refresh_ttl(name))
        .unwrap_or(true)
}

/// A named source: what to call it in the cache, and how to gather it.
type Source = (&'static str, fn() -> Vec<Item>);

/// Sources that shell out but are fast enough to wait for.
///
/// The floor of a launch is the slowest entry here, so membership is the
/// performance decision. `files` stays because a file you just created must
/// be in `f:` on the very next press — the failure to avoid is a missing
/// row. `procs` and `dirs` used to be here too, and between them they *were*
/// the floor: `ps` costs 12–14ms merely enumerating a thousand processes,
/// whatever fields are asked for, and zoxide another 7ms — for rows that
/// only ever appear inside their own scopes, filtered from the snapshot, on
/// exactly the reasoning that already put 65ms `lsof` behind the cache.
const FAST: &[Source] = &[
    ("containers", crate::sources::machine::containers),
    ("files", crate::sources::project::files),
];

/// Too slow to ever block on: lsof costs ~65ms and cannot be made faster.
/// Served from cache and refreshed detached. Safe because the kill command
/// re-resolves the pid at run time rather than trusting the cached one.
const SLOW: &[Source] = &[
    ("ports", crate::sources::machine::ports),
    // A process list five seconds old answers `proc:` exactly as well; what
    // each *agent* is doing stays live through `running::live`'s syscalls.
    ("procs", crate::sources::machine::procs),
    // zoxide's ranking changes at the pace you change projects, not per press.
    ("dirs", crate::sources::user::dirs),
    // Hundreds of session files, each needing its head parsed.
    ("sessions", crate::sources::sessions::all),
    // `claude mcp list` runs a network health check on every server.
    ("mcp", crate::sources::agents::mcp),
    // Full Skill trees can contain scripts and references. Hash them away
    // from the launch path; gather reads only the small fingerprint cache.
    ("skill-hashes", crate::sources::agents::skill_hashes),
    // MCP initialize + tools/list can start local server processes. It is a
    // background inventory, never part of health gather or a keypress.
    ("mcp-tools", crate::mcp_tools::inventory),
    // One redirect to GitHub, at most four times a day, and only when the
    // `update` setting is anything but `off`. Degrades to nothing like every
    // other source here.
    ("update", crate::update::check),
    // ps with full command lines plus a bulk lsof for their working
    // directories: ~95ms, and worth having only for completeness.
    ("fleet", crate::sources::running::fleet),
];

/// The detached `_refresh <name>` process, which is one source and nothing
/// else — which is why the `lost_commands` difference below is unambiguous.
pub fn refresh_named(name: &str) -> bool {
    for (n, f) in FAST.iter().chain(SLOW.iter()) {
        if *n == name {
            // Somebody else got here first. Two processes refreshing one
            // source produce one answer at twice the cost, and for `mcp` that
            // cost is a network health check per server.
            if !claim_lease(n) {
                return true;
            }
            let before = crate::exec::lost_commands();
            let items = f();
            if write_refreshed(n, &items, before) {
                record_success(n);
            } else {
                record_failure(n);
            }
            release_lease(n);
            return true;
        }
    }
    false
}

/// The whole of `gather`, not the part of it left after the local sources
/// have run.
///
/// It used to be measured from the moment the local work finished, which
/// meant the real bound was that work *plus* this — around sixty
/// milliseconds against a budget of forty. Anchoring it at the start makes
/// the number mean what `bench` asserts: a launch takes this long at worst,
/// whatever any subprocess is doing.
const EXTERNAL_DEADLINE: Duration = Duration::from_millis(40);

/// Where a gather spends its time, phase by phase — the instrument behind
/// `bench --sources`. Sampling is off unless asked for, and the probe is one
/// atomic load when it is off.
static PROFILING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SAMPLES: std::sync::Mutex<Vec<(&'static str, f64)>> = std::sync::Mutex::new(Vec::new());

fn timed<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    if !PROFILING.load(std::sync::atomic::Ordering::Relaxed) {
        return f();
    }
    let t = std::time::Instant::now();
    let out = f();
    if let Ok(mut samples) = SAMPLES.lock() {
        samples.push((name, t.elapsed().as_secs_f64() * 1000.0));
    }
    out
}

/// One profiled gather, as (phase, milliseconds) in descending cost.
pub fn profile_gather() -> (usize, Vec<(&'static str, f64)>) {
    PROFILING.store(true, std::sync::atomic::Ordering::Relaxed);
    SAMPLES.lock().map(|mut s| s.clear()).ok();
    let n = gather().len();
    PROFILING.store(false, std::sync::atomic::Ordering::Relaxed);
    let mut out = SAMPLES.lock().map(|s| s.clone()).unwrap_or_default();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    (n, out)
}

pub fn gather() -> Vec<Item> {
    let deadline = std::time::Instant::now() + EXTERNAL_DEADLINE;
    // Kick the subprocess-backed sources off first so they overlap with the
    // cheap local ones instead of adding to them. Each reports through the
    // channel as it finishes, so nothing is polled and no one waits on a
    // source that is already done.
    let (tx, rx) = std::sync::mpsc::channel();
    for (i, (name, f)) in FAST.iter().enumerate() {
        let (name, f, tx) = (*name, *f, tx.clone());
        std::thread::spawn(move || {
            let before = crate::exec::lost_commands();
            let items = timed(name, f);
            // Only when the bytes differ. Measured on this machine, writing
            // these two caches cost more than gathering them — 3.7ms and
            // 2.7ms against a 0.9ms file scan — and the overwhelmingly common
            // case is that a launch produces exactly the bytes the last one
            // did. The `lost_commands` guard is shared with the detached
            // path; here it is slightly over-broad, since both FAST threads
            // share the counter, and over-broad in the only safe direction:
            // it can keep a good cache, never corrupt one.
            timed("write_cached(fast)", || write_refreshed(name, &items, before));
            // The receiver is gone once the deadline passes; the cache write
            // above is what that straggler was still worth.
            let _ = tx.send((i, items));
        });
    }
    drop(tx);

    let mut items = Vec::with_capacity(2600);
    // Search providers, fixed Quicklinks and scope commands belong in global
    // search, but the empty-query home filters them back out.
    items.extend(timed("quicklinks", crate::compute::quicklink_items));
    items.extend(timed("scopes", crate::compute::scope_commands));

    // Read once, then hand down. Sessions is six hundred rows of JSON that
    // the recent list, the skill ranking and the agent summary all want, and
    // the MCP list is wanted twice — between them they were most of the
    // local half of a gather, spent parsing the same two files over.
    let sessions = timed("read sessions", || read_cached("sessions"));
    let mcp = timed("read mcp", || read_cached("mcp"));
    let runs = timed("running::live", || crate::sources::running::live_with_sessions(&sessions));
    let sessions = timed("annotate", || crate::sources::running::annotate_sessions(sessions, &runs));
    timed("write sessions-linked", || write_cached_if_changed("sessions-linked", &sessions));
    timed("spawn refreshes", || {
        // The lease is claimed by the refresh process itself, so this check is
        // only an optimisation — but it is the one that matters, because the
        // cost being avoided is a process spawn per stale source per launch.
        let mut running = 0;
        for (name, _) in SLOW {
            if lease_is_live(name) {
                running += 1;
            }
        }
        for (name, _) in SLOW {
            if running >= MAX_CONCURRENT_REFRESH {
                break;
            }
            if stale(name) && !resting(name) && !lease_is_live(name) {
                spawn_self(&["_refresh", name]);
                running += 1;
            }
        }
    });
    items.extend(timed("read ports", || read_cached("ports")));
    items.extend(timed("read procs", || read_cached("procs")));
    items.extend(timed("read dirs", || read_cached("dirs")));
    items.extend(timed("read update", || read_cached("update")));
    // Sessions are numerous enough to swamp the list; only the newest
    // few go in, and `s:` searches the rest.
    items.extend(
        sessions.iter()
            .filter(|session| crate::sources::sessions::visible(session))
            .take(crate::sources::sessions::IN_MAIN_LIST)
            .cloned(),
    );
    items.extend(mcp.iter().cloned());
    // The `fleet` cache is deliberately *not* extended here, unlike its three
    // neighbours. It records who is running, not what they are doing, and
    // `running::live` below is what turns it into rows. Both went in, the
    // cached one first — and `finish` keeps the first of a duplicate pair, so
    // every run in the launcher showed a blank row while the live state it
    // had just computed was thrown away.

    // Pure file/CPU work — microseconds each, just run them.
    items.extend(timed("scripts", crate::sources::project::scripts));
    items.extend(timed("git", crate::sources::project::git));
    items.extend(timed("history", crate::sources::history::source));
    items.extend(timed("path", crate::sources::user::path_commands));
    items.extend(timed("ssh", crate::sources::user::ssh));
    items.extend(timed("snippets", crate::sources::user::snippets));
    items.extend(timed("clips", crate::sources::user::clips));
    let skills = timed("skills", || crate::sources::agents::skills_with(&sessions));
    items.extend(skills.iter().cloned());

    // Questions agents are blocked on. A directory read of a handful of small
    // files, and the most urgent thing the launcher can show.
    items.extend(timed("bus", crate::bus::items));

    // Identities come from the cache; what each one is *doing* is decided
    // here and now, out of syscalls. A fleet view that is a minute stale is
    // worse than none — it tells you an agent is stuck that has since moved
    // on, and vice versa.
    items.extend(runs.iter().cloned());
    items.extend(timed("apps", crate::sources::machine::apps));
    items.extend(timed("system", crate::sources::machine::system));
    items.extend(timed("configs", crate::sources::agents::configs));
    items.extend(timed("summary", || crate::sources::agents::summary(&skills, &mcp, &sessions, &runs)));
    // Prelude's own preferences. A handful of files under two kilobytes and
    // two `stat`s; `root_items` does not admit the kind, so they are reachable
    // through `set:` and nowhere else.
    items.extend(timed("settings", crate::settings::items));

    // Collected by index rather than in arrival order: `finish` keeps the
    // first of any duplicate pair, so which source got there first must not
    // decide what the list contains.
    let wait_started = std::time::Instant::now();
    let mut fast: Vec<Option<Vec<Item>>> = (0..FAST.len()).map(|_| None).collect();
    while fast.iter().any(Option::is_none) {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        // Missed the deadline — show the last known result rather than
        // stalling. The straggler keeps running and refreshes the cache
        // while you read the list, so the next launch is current.
        let Ok((i, v)) = rx.recv_timeout(left) else { break };
        fast[i] = Some(v);
    }
    if PROFILING.load(std::sync::atomic::Ordering::Relaxed) {
        if let Ok(mut samples) = SAMPLES.lock() {
            samples.push(("wait for FAST", wait_started.elapsed().as_secs_f64() * 1000.0));
        }
    }
    for (i, (name, _)) in FAST.iter().enumerate() {
        match fast[i].take() {
            Some(v) => items.extend(v),
            None => items.extend(read_cached(name)),
        }
    }
    timed("archive", || crate::archive::decorate(&mut items));
    timed("favorites", || crate::favorites::decorate(&mut items));
    timed("finish", || finish(items))
}

/// The agent control centre as data. Sessions deliberately have their own
/// `sessions` command and `s:` scope; including hundreds of them here made
/// both the human overview and `prelude agents --json` session listings in
/// disguise.
pub fn gather_agents() -> Vec<Item> {
    let sessions = read_cached("sessions");
    let mcp = read_cached("mcp");
    let runs = crate::sources::running::live_with_sessions(&sessions);
    let sessions = crate::sources::running::annotate_sessions(sessions, &runs);
    write_cached_if_changed("sessions-linked", &sessions);
    let skills = crate::sources::agents::skills_with(&sessions);
    let mut items = crate::bus::items();
    items.extend(crate::sources::agents::summary(&skills, &mcp, &sessions, &runs));
    items.extend(runs);
    items.extend(skills);
    items.extend(mcp);
    items.extend(crate::sources::agents::configs());
    crate::archive::decorate(&mut items);
    crate::favorites::decorate(&mut items);
    finish(items)
}

/// The one rule for what comes first: **kind decides the band, and learned
/// ranking only orders things inside it.**
///
/// This used to be a single number — kind priority plus a frecency bonus
/// capped at 60 — and the cap was doing a job it could not do. The whole
/// agent cluster spans 25 points (Agent 1000 down to Config 975) while the
/// bonus reached 60, so the bonus was two and a half times wider than the
/// thing it was supposed to nudge within. The bands stopped being bands: a
/// skill used twice this morning outranked `claude` itself, and a config
/// file outranked a skill. The comment on the cap even said what it was for
/// — rise within your kind, do not vault over a whole category — while the
/// arithmetic guaranteed the opposite.
///
/// Comparing the band first makes that structural instead of arithmetical.
/// No cap can be miscalibrated, no future change to a priority can silently
/// re-open the hole, and the two questions stay separate: *what kind of
/// thing is this* and *how much do you use this one*.
///
/// `score` still carries the band as a constant, which is harmless: it is
/// identical for every item being compared at the second level, so it
/// cancels. What is left there is the source's own ordering (a run's state,
/// a session's recency) plus frecency.
pub fn by_rank(a: &Item, b: &Item) -> std::cmp::Ordering {
    // `band`, not `kind.priority`: a saved Quicklink is banded by the person
    // having named it, not by whatever it points at. See `Kind::QUICKLINK`.
    b.band()
        .cmp(&a.band())
        .then_with(|| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
}

/// Dedupe, apply learned ranking, sort.
pub fn finish(items: Vec<Item>) -> Vec<Item> {
    let freq = crate::frecency::load();
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    let mut out = Vec::with_capacity(items.len());
    for mut it in items {
        // Keyed on kind too: the same text can legitimately appear as a file
        // and as a history entry, and they have different actions.
        if !seen.insert((it.kind, it.cmd.clone())) {
            continue;
        }
        if let Some((n, last)) = freq.get(&it.cmd) {
            // Adds to whatever the source already said about where this
            // belongs inside its kind, on the same scale — see `Item::rank`.
            it.score += crate::frecency::bonus(*n, *last);
        }
        out.push(it);
    }
    // Stable, so items the frecency cap has tied keep the order their source
    // produced them in — newest session first, and so on.
    out.sort_by(by_rank);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("prelude-cache-test-{}-{label}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The temporary name has to belong to one writer. It was
    /// `with_extension("tmp")` — one name per destination for the whole
    /// machine — so two Preludes writing the same cache wrote into the same
    /// file and then each renamed it. The rename is atomic, so the result was
    /// never half a file; it was a whole file holding two answers spliced
    /// together, which is worse for being harder to see.
    #[test]
    fn concurrent_writers_never_splice_their_bytes_together() {
        let dir = scratch("splice");
        let target = dir.join("list.txt");
        let a = vec![b'a'; 300_000];
        let b = vec![b'b'; 300_000];
        std::thread::scope(|s| {
            for bytes in [&a, &b] {
                s.spawn(|| {
                    for _ in 0..12 {
                        let _ = write_atomic(&target, bytes);
                    }
                });
            }
        });
        let got = std::fs::read(&target).unwrap();
        assert!(got == a || got == b, "the file holds a splice of two writers");
        // And nobody's litter is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
    }

    /// Two destinations must not share a temporary name either.
    /// `with_extension` merged them: `list.txt` and `list.json` both staged
    /// through `list.tmp`.
    #[test]
    fn each_destination_stages_through_its_own_name() {
        let dir = scratch("names");
        let one = unique_temp(&dir.join("list.txt"));
        let two = unique_temp(&dir.join("list.json"));
        let again = unique_temp(&dir.join("list.txt"));
        assert_ne!(one, two);
        assert_ne!(one, again, "two writes to one path must not share a name");
    }

    /// State is written 0600 and flushed before the rename. A cache is
    /// neither: it is derived, and an fsync per launch is a real cost for
    /// something a launch rebuilds anyway.
    #[test]
    fn user_state_is_private_and_durable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("state");
        let path = dir.join("favorites.txt");
        write_state(&path, b"one\n").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "user state must not be world-readable");
        assert_eq!(std::fs::read(&path).unwrap(), b"one\n");
        // Replacing it keeps the contents whole and the mode private.
        write_state(&path, b"two\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two\n");
    }

    /// An empty result and a failed refresh arrived here as the same value,
    /// and both were written. A `claude mcp list` that timed out therefore
    /// replaced a real inventory with an empty one, and the cache then looked
    /// fresh, so nothing retried until the TTL expired.
    #[test]
    fn a_refresh_that_learned_nothing_does_not_erase_what_it_knew() {
        // The only case that keeps the old answer: nothing found, a command
        // lost, and something already there worth keeping.
        assert!(keeps_last_good(true, true, || true));
        // A source honestly reporting an empty world must be written, or the
        // launcher shows containers that stopped and servers that were removed.
        assert!(!keeps_last_good(true, false, || true));
        // A result is always the newest truth, whatever else failed.
        assert!(!keeps_last_good(false, true, || true));
        // With nothing to protect, there is nothing to weigh.
        assert!(!keeps_last_good(true, true, || false));
    }

    /// A refresh that fails leaves the cache untouched and therefore still
    /// stale, so without a rest the next gather spawns it again at once —
    /// one broken agent CLI becoming a permanent background load.
    #[test]
    fn a_failing_source_is_asked_less_often_rather_than_more() {
        assert!(backoff_delay("mcp", 2) > backoff_delay("mcp", 1), "it has to grow");
        assert!(backoff_delay("mcp", 3) > backoff_delay("mcp", 2));
        assert_eq!(backoff_delay("mcp", 50), BACKOFF_MAX, "and it has to stop");
        // The point of the rest: a broken source must never be asked at the
        // rate a working one is refreshed. A flat constant failed this for
        // `mcp`, whose TTL happened to equal it.
        for (name, _) in SLOW.iter().chain(FAST.iter()) {
            let ttl = refresh_ttl(name);
            assert!(
                backoff_delay(name, 1) >= ttl.saturating_mul(2).min(BACKOFF_MAX),
                "{name} rests no longer than it normally waits"
            );
        }
    }

    /// Read, change, write is not made safe by an atomic rename: both writes
    /// land whole and one of them is simply gone.
    #[test]
    fn the_write_lock_serialises_a_read_change_write_cycle() {
        let dir = scratch("lock");
        let path = dir.join("frecency.txt");
        std::fs::write(&path, "0").unwrap();
        // Three writers, which is more Preludes than a person has open. Each
        // cycle holds the lock across the read and the write, which is the
        // whole of the guarantee being asserted.
        std::thread::scope(|s| {
            for _ in 0..3 {
                s.spawn(|| {
                    for _ in 0..15 {
                        let _lock = lock_for_write(&path);
                        let n: u64 = std::fs::read_to_string(&path)
                            .unwrap_or_default()
                            .trim()
                            .parse()
                            .unwrap_or(0);
                        let _ = write_state(&path, (n + 1).to_string().as_bytes());
                    }
                });
            }
        });
        let total: u64 = std::fs::read_to_string(&path).unwrap().trim().parse().unwrap();
        assert_eq!(total, 45, "updates were lost under the lock");
    }

    /// And the bound is real, because the alternative is worse than a lost
    /// count: this runs on the Enter path, and a launcher that waits on
    /// another process's lock is a launcher that hangs when that process
    /// hangs. A writer that cannot have the lock proceeds without it.
    #[test]
    fn the_lock_gives_up_rather_than_waiting_for_a_holder_that_never_leaves() {
        let dir = scratch("held");
        let path = dir.join("frecency.txt");
        let held = lock_for_write(&path).expect("first claim");
        let started = std::time::Instant::now();
        assert!(lock_for_write(&path).is_none(), "a held lock must be refused");
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(200), "it gave up without waiting at all");
        assert!(waited < Duration::from_secs(2), "it waited far past its own bound");
        drop(held);
        assert!(lock_for_write(&path).is_some(), "a released lock must be available");
    }

    /// Only one process refreshes a source. The claim is `create_new`, so it
    /// is the filesystem that arbitrates rather than a check followed by a
    /// race between the checkers.
    #[test]
    fn one_refresh_per_source_and_a_dead_holder_never_blocks_it() {
        let dir = scratch("lease");
        // Point the cache at scratch for the duration; these helpers address
        // `paths::cache()`, which reads this variable on every call.
        let previous = std::env::var_os("XDG_CACHE_HOME");
        unsafe { std::env::set_var("XDG_CACHE_HOME", &dir) };

        assert!(claim_lease("mcp"), "an unheld source must be claimable");
        assert!(!claim_lease("mcp"), "a held source must not be claimed twice");
        assert!(claim_lease("fleet"), "a lease is per source, not global");
        release_lease("mcp");
        assert!(claim_lease("mcp"), "a released source is claimable again");

        // A lease naming a process that no longer exists is not a lease.
        // Pid 0 can never be a live child of ours.
        release_lease("mcp");
        std::fs::write(lease_file("mcp"), "0 1\n").unwrap();
        assert!(!lease_is_live("mcp"));
        assert!(claim_lease("mcp"), "a dead holder must not lock a source out");

        match previous {
            Some(v) => unsafe { std::env::set_var("XDG_CACHE_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CACHE_HOME") },
        }
    }
}
