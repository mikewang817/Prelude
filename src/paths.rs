//! Where things live. No hard-coded personal paths — everything honours the
//! XDG variables so the binary is portable to any machine.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
        .join("prelude")
}

/// Read at most `limit` bytes of a file, for anything on the launch path.
///
/// Every source here reads files it did not create and cannot bound: a
/// `package.json` in a generated project, a `Makefile` in a monorepo, a shell
/// history that has never been rotated. The normal case is kilobytes and the
/// abnormal case is not rare enough to ignore — a hundred-megabyte history
/// file is one runaway script away, and gather would read all of it, decode
/// all of it and hold all of it, at every keystroke's parent process.
///
/// Reading the first `limit` bytes rather than refusing the file keeps the
/// useful part of a large one: these formats are line-oriented and the
/// callers parse them line by line, so a truncated read is a shorter list,
/// not a broken one. The one thing never to do is let another program's file
/// size decide this program's memory.
pub fn read_bounded(path: &std::path::Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.take(limit).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// Read at most `limit` bytes from the *end* of a file.
///
/// For an append-only file the prefix is the wrong half. A shell history is
/// the case: reading the first 32MB of a 40MB history returns the commands
/// somebody ran years ago and drops the ones they ran this morning, which is
/// the exact opposite of what the source is for. Nobody would notice, either
/// — the list would be full, just full of the wrong things.
///
/// The first line is discarded because a read that starts mid-file starts
/// mid-record. At worst that costs one entry; keeping it would put half a
/// command in the launcher.
pub fn read_tail_bounded(path: &std::path::Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size <= limit {
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).ok()?;
        return Some(buf);
    }
    file.seek(SeekFrom::Start(size - limit)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    match buf.iter().position(|b| *b == b'\n') {
        Some(cut) => Some(buf.split_off(cut + 1)),
        None => Some(buf),
    }
}

/// What a small structured file is allowed to be: manifests, configs, the
/// files a project describes itself with.
pub const SMALL_FILE: u64 = 4 * 1024 * 1024;

/// What a log-shaped file is allowed to be. Shell history is read whole and
/// deduplicated down to a few thousand rows, so the cap only has to be
/// comfortably larger than any real history.
pub const LOG_FILE: u64 = 32 * 1024 * 1024;

pub fn cache() -> PathBuf {
    xdg("XDG_CACHE_HOME", ".cache")
}
pub fn data() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}
pub fn config() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// The current directory, or `None` when it has been deleted out from under
/// the shell. Deliberately not falling back to `$HOME`: that would make every
/// project-scoped source treat the whole home directory as "the project" and
/// scan it, which measured 93ms against a 40ms budget.
pub fn cwd() -> Option<PathBuf> {
    std::env::current_dir().ok()
}

/// Paths nothing in this launcher may move, whatever the row said.
///
/// Trashing is recoverable, so this is not the last line of defence — the
/// confirmation is. It is the line against the *catastrophic* rather than
/// the merely wrong: a fuzzy list, a mis-aimed Enter, and a row whose path
/// happened to be `$HOME`. Everything here is either irreplaceable or
/// system-owned, and none of it is something a person meant to select from
/// a launcher.
///
/// `project::root` asks the same question for a different reason: these are
/// the directories that *hold* projects rather than being one, and walking
/// them as if they were is how a launcher opened in `$HOME` ends up reading
/// every other application's data.
pub fn is_protected(p: &std::path::Path) -> bool {
    let Ok(real) = p.canonicalize() else { return true };
    if real.parent().is_none() {
        return true; // the root itself
    }
    if real == home() {
        return true;
    }
    let s = real.to_string_lossy();
    // Whole trees, contents included. Compared *after* canonicalize, which
    // on macOS is the only way to catch these at all: `/etc` and `/var` are
    // symlinks into `/private`, so a list of the names people type protects
    // nothing.
    const TREES: &[&str] =
        &["/System", "/usr", "/bin", "/sbin", "/Library", "/private/etc", "/private/var"];
    if TREES.iter().any(|d| s == *d || s.starts_with(&format!("{d}/"))) {
        return true;
    }
    // …and these directories themselves, but not what is inside them. A file
    // in /tmp is an ordinary file; /tmp is not.
    const THEMSELVES: &[&str] =
        &["/Applications", "/Users", "/private/tmp", "/opt", "/opt/homebrew", "/Volumes"];
    THEMSELVES.contains(&s.as_ref())
}

/// Move something to the Trash, and say where it went.
///
/// Never `remove_file`, never `remove_dir_all`. The Trash costs the same as
/// deleting and leaves the thing sitting there to be dragged back out, which
/// turns "that was the wrong row" from a loss into an inconvenience. A name
/// already in there is never overwritten — you get `notes 2`.
pub fn trash(p: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if is_protected(p) {
        return Err(format!("{} is not something to delete from here", p.display()));
    }
    if !p.exists() {
        return Err(format!("{} is not there any more", p.display()));
    }
    let name = p.file_name().ok_or_else(|| "no name".to_string())?;
    let dir = home().join(".Trash");
    std::fs::create_dir_all(&dir).map_err(|e| format!("no Trash: {e}"))?;
    let mut dest = dir.join(name);
    let mut n = 2;
    while dest.exists() {
        dest = dir.join(format!("{} {n}", name.to_string_lossy()));
        n += 1;
        if n > 999 {
            return Err("too many copies of that name in the Trash".into());
        }
    }
    std::fs::rename(p, &dest).map_err(|e| format!("could not move it to the Trash: {e}"))?;
    Ok(dest)
}

pub fn tilde(p: &str) -> String {
    let h = home();
    let h = h.to_string_lossy();
    if !h.is_empty() && p.starts_with(h.as_ref()) {
        format!("~{}", &p[h.len()..])
    } else {
        p.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An append-only file's newest records are at the end, so a bounded read
    /// of one has to come from the end. Reading the prefix of a large history
    /// returns years-old commands and silently drops this morning's.
    #[test]
    fn a_bounded_read_of_an_append_only_file_keeps_the_newest_records() {
        let dir = std::env::temp_dir().join(format!("prelude-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history");
        let mut text = String::new();
        for n in 0..2000 {
            text.push_str(&format!("command number {n}\n"));
        }
        std::fs::write(&path, &text).unwrap();

        // Under the limit: the whole file, unchanged.
        let all = read_tail_bounded(&path, 1024 * 1024).unwrap();
        assert_eq!(all, text.as_bytes());

        // Over it: the tail, and never a half record at the front.
        let tail = String::from_utf8(read_tail_bounded(&path, 500).unwrap()).unwrap();
        assert!(tail.len() <= 500);
        assert!(tail.contains("command number 1999"), "the newest record must survive");
        assert!(!tail.contains("command number 0\n"), "the oldest must be the one dropped");
        for line in tail.lines() {
            assert!(line.starts_with("command number "), "a partial record survived: {line:?}");
        }

        // The prefix reader still exists for files that are not append-only,
        // and still takes the front.
        let head = String::from_utf8(read_bounded(&path, 500).unwrap()).unwrap();
        assert!(head.starts_with("command number 0\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
