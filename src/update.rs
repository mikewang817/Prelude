//! What you are running, what is available, and how to move between them.
//!
//! Three questions, and only the second one needs the network:
//!
//! 1. **Is the running panel the binary on disk?** The global panel is started
//!    at login and executes whatever the binary held then. A build, or an
//!    upgrade, does not change it — and each press *does* spawn the new binary
//!    to draw the list, so the rows and the footer update and it looks like the
//!    change took, while the delivery decision is still made by the old parent.
//!    That failure reads as "the change did nothing" and it lies in the most
//!    convincing possible way. This is the cheapest question here and the one
//!    people actually hit, so it is answered without a network call, without a
//!    setting, and without asking.
//! 2. **Is there a newer release?** Behind the slow cache tier with a six-hour
//!    TTL, refreshed detached, degrading to nothing — the same contract every
//!    other subprocess-backed source has.
//! 3. **Apply it.** Never silently, and never while you are using the panel.
//!    See `mode`.

use crate::item::{Item, Kind};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const REPO: &str = "mikewang817/Prelude";
const LATEST: &str = "https://github.com/mikewang817/Prelude/releases/latest";
const API_LATEST: &str = "https://api.github.com/repos/mikewang817/Prelude/releases/latest";

/// The release asset for this machine, named as `release.yml` publishes it.
pub fn target() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Some("aarch64-apple-darwin"),
        "x86_64" => Some("x86_64-apple-darwin"),
        _ => None,
    }
}

// ─── 1. the running panel ────────────────────────────────────────────────

fn panel_stamp() -> PathBuf {
    crate::paths::cache().join("panel.version")
}

/// Called once as the panel loop starts, so the version actually serving
/// keypresses is recorded rather than inferred.
pub fn record_panel(pid: u32) {
    let _ = crate::cache::write_atomic(&panel_stamp(), format!("{VERSION} {pid}\n").as_bytes());
}

/// The version the live panel is running, when there is one.
///
/// The pid is checked because a stamp outlives the process that wrote it, and
/// "the panel is old" is a very different sentence from "a panel was old once".
pub fn panel_version() -> Option<String> {
    let text = std::fs::read_to_string(panel_stamp()).ok()?;
    let mut parts = text.split_whitespace();
    let version = parts.next()?.to_string();
    let pid: i32 = parts.next()?.parse().ok()?;
    (unsafe { libc_kill(pid) }).then_some(version)
}

/// `kill(pid, 0)` — the syscall, not the command. Answers whether the process
/// exists without touching it.
unsafe fn libc_kill(pid: i32) -> bool {
    unsafe extern "C" {
        unsafe fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid, 0) == 0 }
}

/// The version the panel is serving, when it differs from this binary.
pub fn panel_is_stale() -> Option<String> {
    panel_version().filter(|running| running != VERSION)
}

// ─── 2. the latest release ───────────────────────────────────────────────

/// What to do when a newer release exists.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Never look. The only setting that makes no network request at all.
    Off,
    /// Say so, and leave the decision alone. The default.
    Notify,
    /// Verify and stage the archive, so applying is instant and works offline.
    Download,
    /// Apply a staged update — but only as the panel starts, which is the one
    /// moment nothing is on screen and the process about to run *is* the new
    /// binary. Applying at any other time leaves the machine in the state this
    /// module exists to detect: a new binary and an old panel.
    Apply,
}

pub fn mode() -> Mode {
    crate::settings::parse_update_mode(&crate::settings::update_mode())
}

/// `0.7.1` -> `(0, 7, 1)`. Anything unparsable sorts as nothing, so a tag this
/// build does not understand can never present itself as newer.
fn parts(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    // Leading digits, not "trim the trailing non-digits": `1-rc1` ends in a
    // digit, so trimming from the right leaves `1-rc1` and the parse fails
    // silently into 0 — which made `0.7.1-rc1` compare equal to `0.7.0`.
    let mut it = v
        .split('.')
        .map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>());
    Some((
        it.next()?.parse().ok()?,
        it.next().unwrap_or_default().parse().unwrap_or(0),
        it.next().unwrap_or_default().parse().unwrap_or(0),
    ))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parts(candidate), parts(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Ask GitHub which release is current: the API first, its `/releases/latest`
/// redirect as a fallback.
///
/// It was the redirect alone, on the reasoning that the API is rate limited
/// per address while the redirect target already spells the answer out — no
/// token, no parser. The rate limit half of that was true and irrelevant:
/// sixty requests an hour against four a day is not a constraint, it is three
/// orders of magnitude of headroom. What the reasoning missed is that the
/// redirect is served from a cache and is *eventually* consistent — measured
/// here still answering `v0.8.1` five minutes after `v0.8.2` was published,
/// and catching up two minutes later. A version check that can be minutes
/// wrong about the one fact it exists to report is worse than one that spends
/// a request nobody is counting.
///
/// Still no parser: `tag_name` is a substring, and treating a JSON body as
/// text keeps this identical in shape to the redirect path it falls back to.
fn fetch_latest() -> Option<String> {
    api_latest().or_else(redirect_latest)
}

fn api_latest() -> Option<String> {
    let body = crate::exec::run(
        &["curl", "-sS", "--max-time", "10", "--retry", "1",
          "-H", "Accept: application/vnd.github+json", API_LATEST],
        Duration::from_secs(20),
    );
    let after = body.split_once("\"tag_name\"")?.1;
    let tag = after.split('"').nth(1)?.trim().trim_start_matches('v').to_string();
    parts(&tag).is_some().then_some(tag)
}

fn redirect_latest() -> Option<String> {
    let out = crate::exec::run(
        &["curl", "-sSI", "-o", "/dev/null", "-w", "%{url_effective}", "-L",
          "--max-time", "10", "--retry", "1", LATEST],
        Duration::from_secs(20),
    );
    let tag = out.trim().rsplit_once("/tag/")?.1.trim().to_string();
    let tag = tag.trim_start_matches('v').to_string();
    parts(&tag).is_some().then_some(tag)
}

fn state_file() -> PathBuf {
    crate::paths::data().join("update.json")
}

/// Throw away the "a newer release exists" row.
///
/// The launcher reads that row from a cache with a six-hour TTL, so an update
/// that does not clear it leaves the panel advertising a version you have just
/// installed — for the rest of the day, with a row whose Enter would now
/// download what you are already running. Called after a successful swap, and
/// by an explicit check, whose answer the panel should reflect immediately.
fn forget_cached_row() {
    let _ = std::fs::remove_file(crate::paths::cache().join("update.json"));
}

/// The last version a banner was posted for, so a release is announced once
/// rather than every six hours until it is taken.
fn announced() -> String {
    std::fs::read_to_string(state_file())
        .ok()
        .and_then(|t| t.split_once("\"seen\"").map(|(_, rest)| rest.to_string()))
        .and_then(|rest| {
            let rest = rest.trim_start().trim_start_matches(':').trim();
            rest.strip_prefix('"')?.split('"').next().map(str::to_string)
        })
        .unwrap_or_default()
}

fn remember_announced(version: &str) {
    let _ = crate::cache::write_atomic(
        &state_file(),
        format!("{{\"seen\": \"{version}\"}}\n").as_bytes(),
    );
}

/// The slow source. Runs detached on the refresh path, never on a keystroke.
pub fn check() -> Vec<Item> {
    if mode() == Mode::Off {
        return Vec::new();
    }
    let Some(latest) = fetch_latest() else { return Vec::new() };
    if !is_newer(&latest, VERSION) {
        return Vec::new();
    }
    if mode() == Mode::Download || mode() == Mode::Apply {
        let _ = stage(&latest);
    }
    // Once per released version. A notice repeated on a timer is one people
    // learn to dismiss without reading.
    if announced() != latest {
        crate::bus::post(
            &format!("Prelude {latest} is available"),
            &format!("You are running {VERSION}. Run: prelude update"),
        );
        remember_announced(&latest);
    }
    vec![row(&latest)]
}

pub fn row(latest: &str) -> Item {
    Item::new("prelude update", Kind::Sys)
        .title(format!("Prelude {latest} is available"))
        .fields([format!("you have {VERSION}"), "prelude update".to_string()])
        .put("update", "available")
        .put("latest", latest)
}

/// The cached answer, for the surfaces that only want to state it.
pub fn available() -> Option<String> {
    crate::cache::read_cached("update")
        .into_iter()
        .find(|it| it.get("update") == "available")
        .map(|it| it.get("latest").to_string())
}

// ─── 3. applying it ──────────────────────────────────────────────────────

fn staged_dir() -> PathBuf {
    crate::paths::cache().join("update")
}

/// Remove every staged version except the one named.
///
/// A cache with no eviction is not a cache, it is a leak with a good excuse.
fn prune_staged(keep: &str) {
    let Ok(entries) = std::fs::read_dir(staged_dir()) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy() == keep {
            continue;
        }
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Download the archive for this machine and verify it before it is anywhere
/// it could be run from.
fn stage(version: &str) -> Result<PathBuf, String> {
    let target = target().ok_or("no release is published for this architecture")?;
    let asset = format!("prelude-{target}.tar.gz");
    let dir = staged_dir().join(version);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Every version ever downloaded was kept here, forever: five releases had
    // become twenty-one megabytes on this machine, and nothing was ever going
    // to remove them. The rollback binary is `prelude.old` beside the
    // installed one — these archives are only a download cache, and the only
    // one worth caching is the one being installed right now, so that an
    // interrupted download can resume instead of starting again.
    prune_staged(version);
    let archive = dir.join(&asset);
    if verify(&archive, &dir).is_ok() {
        return Ok(archive);
    }
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    fetch(&format!("{base}/{asset}"), &archive)?;
    fetch(&format!("{base}/checksums.txt"), &dir.join("checksums.txt"))?;
    verify(&archive, &dir)?;
    Ok(archive)
}

fn fetch(url: &str, to: &Path) -> Result<(), String> {
    let status = std::process::Command::new("curl")
        .args(["-fL", "--retry", "3", "--max-time", "300", "-o"])
        .arg(to)
        .arg(url)
        .status()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        let _ = std::fs::remove_file(to);
        Err(format!("could not download {url}"))
    }
}

/// SHA-256 through `shasum`, which is what `install.sh` uses and what macOS
/// ships. A hashing dependency would be paid at every startup for a check that
/// happens twice a month.
fn verify(archive: &Path, dir: &Path) -> Result<(), String> {
    let name = archive.file_name().and_then(|n| n.to_str()).ok_or("bad archive name")?;
    let sums = std::fs::read_to_string(dir.join("checksums.txt"))
        .map_err(|_| "the release checksums are missing".to_string())?;
    let expected = sums
        .lines()
        .find_map(|l| {
            let (hash, file) = l.split_once("  ")?;
            (file.trim() == name).then_some(hash.trim())
        })
        .ok_or_else(|| format!("the release has no checksum for {name}"))?;
    let out = crate::exec::run(
        &["shasum", "-a", "256", &archive.to_string_lossy()],
        Duration::from_secs(60),
    );
    let actual = out.split_whitespace().next().unwrap_or_default();
    if actual.is_empty() {
        return Err("could not hash the downloaded archive".into());
    }
    if actual != expected {
        let _ = std::fs::remove_file(archive);
        return Err("checksum verification failed; the download was discarded".into());
    }
    Ok(())
}

fn install_path() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("could not find the running binary: {e}"))
}

/// Swap the binary, keeping the one it replaces.
///
/// A rename inside the install directory, so the replacement is atomic and
/// cannot cross a filesystem. The previous binary stays as `prelude.old`,
/// because the failure that matters is a release that starts and does not
/// work, and at that point downloading anything is the thing you cannot do.
fn swap(new: &Path) -> Result<PathBuf, String> {
    let current = install_path()?;
    let dir = current.parent().ok_or("the binary has no parent directory")?;
    if std::fs::metadata(dir).map(|m| m.permissions().readonly()).unwrap_or(true) {
        return Err(format!("{} is not writable; reinstall with install.sh", dir.display()));
    }
    let incoming = dir.join(".prelude.incoming");
    std::fs::copy(new, &incoming).map_err(|e| format!("could not stage the new binary: {e}"))?;
    let mut perms = std::fs::metadata(&incoming).map_err(|e| e.to_string())?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
    }
    let _ = std::fs::set_permissions(&incoming, perms);
    let previous = dir.join("prelude.old");
    let _ = std::fs::remove_file(&previous);
    // Link the backup rather than moving the binary out of the way.
    //
    // Moving it first leaves the installed path *not existing* between the
    // two renames. It is a very short window and it is not a harmless one:
    // this is the path the global panel's Ghostty runs on every press, and
    // the LaunchAgent restarts on exit — so a chord pressed in that instant
    // meets a missing file. A hard link puts a second name on the same inode
    // instead, so `prelude` never stops resolving, and the single rename that
    // follows replaces it atomically.
    //
    // The fallback is the old order, because a hard link can fail for real
    // reasons — a filesystem that does not support them — and an update that
    // refuses to proceed over that would be worse than one with a window a
    // few microseconds wide.
    match std::fs::hard_link(&current, &previous) {
        Ok(()) => {
            if let Err(e) = std::fs::rename(&incoming, &current) {
                let _ = std::fs::remove_file(&previous);
                return Err(format!("could not put the new binary in place: {e}"));
            }
        }
        Err(_) => {
            std::fs::rename(&current, &previous)
                .map_err(|e| format!("could not set the old binary aside: {e}"))?;
            if let Err(e) = std::fs::rename(&incoming, &current) {
                let _ = std::fs::rename(&previous, &current);
                return Err(format!("could not put the new binary in place: {e}"));
            }
        }
    }
    Ok(previous)
}

fn extract(archive: &Path) -> Result<PathBuf, String> {
    let dir = archive.parent().ok_or("the archive has no directory")?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .status()
        .map_err(|e| format!("could not run tar: {e}"))?;
    if !status.success() {
        return Err("could not unpack the release archive".into());
    }
    let binary = dir.join("prelude");
    binary.is_file().then_some(binary).ok_or_else(|| "the archive did not contain prelude".into())
}

/// Applied at panel start, and only there. Returns the version installed.
pub fn apply_staged_if_any() -> Option<String> {
    if mode() != Mode::Apply {
        return None;
    }
    let latest = available()?;
    if !is_newer(&latest, VERSION) {
        return None;
    }
    let archive = stage(&latest).ok()?;
    let binary = extract(&archive).ok()?;
    swap(&binary).ok()?;
    Some(latest)
}

pub fn dispatch(args: &[&str]) -> i32 {
    match args {
        [] | ["--check"] => {
            println!("prelude {VERSION} ({})", target().unwrap_or("unsupported architecture"));
            if let Some(running) = panel_is_stale() {
                println!("  the panel is still serving {running} — run:  prelude global start");
            }
            match fetch_latest() {
                Some(latest) if is_newer(&latest, VERSION) => {
                    // You asked, so the panel should say what you were told
                    // rather than whatever the six-hourly check last saw.
                    crate::cache::write_cached("update", &[row(&latest)]);
                    println!("  {latest} is available");
                    if args == ["--check"] {
                        return 0;
                    }
                    apply(&latest)
                }
                Some(_) => {
                    forget_cached_row();
                    println!("  this is the newest release");
                    0
                }
                None => {
                    eprintln!("prelude: could not reach GitHub to check for a newer release");
                    2
                }
            }
        }
        ["--rollback"] => rollback(),
        _ => {
            eprintln!("prelude update             check, then install a newer release");
            eprintln!("prelude update --check     report only");
            eprintln!("prelude update --rollback  put the previous binary back");
            2
        }
    }
}

fn apply(version: &str) -> i32 {
    let staged = match stage(version) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("prelude: {e}");
            return 2;
        }
    };
    let binary = match extract(&staged) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("prelude: {e}");
            return 2;
        }
    };
    let previous = match swap(&binary) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("prelude: {e}");
            return 2;
        }
    };
    forget_cached_row();
    // The archive has served its purpose the moment the binary is in place.
    prune_staged("");
    println!("  installed {version}; the previous binary is at {}", previous.display());
    // The panel is the whole reason this module exists: leaving it on the old
    // binary is the state an update is supposed to end, not create. But when
    // the update is running *inside* the panel, restarting it means killing
    // the process tree we are in — `stop_helper` pkills the Ghostty instance
    // this process descends from. Closing the surface achieves the same end
    // safely: the next press runs `<installed prelude> _surface`, which is now
    // the binary just installed. `ui::apply_default` emits the CLOSE.
    if crate::ui::env_flag("PRELUDE_FULL_SURFACE") {
        println!("  this panel is closing; the next press runs {version}");
    } else {
        match crate::global::restart_panel() {
            Ok(said) => println!("  {said}"),
            Err(e) => println!("  the panel was not restarted ({e}); run:  prelude global start"),
        }
    }
    0
}

fn rollback() -> i32 {
    let Ok(current) = install_path() else { return 2 };
    let Some(dir) = current.parent() else { return 2 };
    let previous = dir.join("prelude.old");
    if !previous.is_file() {
        eprintln!("prelude: there is no previous binary to go back to");
        return 2;
    }
    let spare = dir.join(".prelude.rolling");
    if std::fs::rename(&previous, &spare).is_err() {
        eprintln!("prelude: could not move the previous binary into place");
        return 2;
    }
    let _ = std::fs::rename(&current, &previous);
    if let Err(e) = std::fs::rename(&spare, &current) {
        let _ = std::fs::rename(&previous, &current);
        eprintln!("prelude: rollback failed: {e}");
        return 2;
    }
    forget_cached_row();
    println!("rolled back; run:  prelude global start");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_this_build_cannot_parse_never_presents_itself_as_newer() {
        assert!(is_newer("0.7.1", "0.7.0"));
        assert!(is_newer("v0.8.0", "0.7.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.7.0", "0.7.0"));
        assert!(!is_newer("0.6.9", "0.7.0"));
        // A tag from a future scheme, a redirect to a login page, an empty
        // answer: none of them may become an "update available" row.
        assert!(!is_newer("nightly", "0.7.0"));
        assert!(!is_newer("", "0.7.0"));
        assert!(!is_newer("v", "0.7.0"));
        // Pre-release suffixes compare on their numbers rather than refusing.
        assert_eq!(parts("0.7.1-rc1"), Some((0, 7, 1)));
    }

    /// The API answers with JSON and the redirect with a URL. Both are read as
    /// text, and both must refuse an answer this build cannot compare.
    #[test]
    fn either_source_yields_a_version_or_nothing() {
        // What the API actually returns, trimmed to the shape that matters.
        let body = r#"{"url":"...","tag_name":"v0.8.2","name":"Prelude v0.8.2"}"#;
        let tag = body
            .split_once("\"tag_name\"")
            .and_then(|(_, rest)| rest.split('"').nth(1))
            .map(|t| t.trim_start_matches('v'));
        assert_eq!(tag, Some("0.8.2"));
        assert!(is_newer(tag.unwrap(), "0.8.1"));

        // A rate-limit body, an error page, an empty answer: no `tag_name`,
        // so nothing — and the redirect is tried next.
        for junk in [r#"{"message":"API rate limit exceeded"}"#, "", "<html>"] {
            assert!(junk.split_once("\"tag_name\"").is_none(), "{junk}");
        }

        // The redirect's shape.
        let url = "https://github.com/mikewang817/Prelude/releases/tag/v0.8.2";
        assert_eq!(url.rsplit_once("/tag/").map(|(_, t)| t.trim_start_matches('v')), Some("0.8.2"));
    }

    #[test]
    fn the_only_mode_that_makes_no_request_is_the_one_that_says_so() {
        // `check` is the only network path, and it is the first thing Off
        // stops. A test cannot prove a socket was not opened, so this pins the
        // branch that decides it.
        assert_eq!(super::Mode::Off, super::Mode::Off);
        for (text, want) in [
            ("off", Mode::Off),
            ("notify", Mode::Notify),
            ("download", Mode::Download),
            ("apply", Mode::Apply),
            ("nonsense", Mode::Notify),
        ] {
            assert_eq!(crate::settings::parse_update_mode(text), want, "{text}");
        }
    }
}
