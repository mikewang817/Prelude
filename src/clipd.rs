//! Clipboard history and macOS pasteboard interop.
//!
//! `pbpaste` sees only text. Finder file references and image data live in
//! other NSPasteboard types, so the macOS watcher is one persistent JXA
//! process over AppKit. It emits small JSON records; image bytes stay in
//! Prelude's private data directory and are never put on a command line.

use crate::item::Item;
use crate::paths;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

// v3 adds content identities to image records. An old watcher cannot dedupe
// the multiple changeCount bumps some screenshot tools emit for one image.
const DAEMON_VERSION: &str = "3";
const MAX_HISTORY: usize = 500;
const MAX_IMAGES: usize = 100;

fn pidfile() -> PathBuf {
    paths::cache().join("clipd.pid")
}

fn recorded_pid() -> Option<(i32, Option<&'static str>)> {
    let text = std::fs::read_to_string(pidfile()).ok()?;
    let mut parts = text.split_whitespace();
    let pid = parts.next()?.parse().ok()?;
    let version = parts.next().map(|v| if v == DAEMON_VERSION { DAEMON_VERSION } else { "old" });
    Some((pid, version))
}

use crate::exec::alive;

pub fn is_running() -> bool {
    recorded_pid().is_some_and(|(pid, version)| version == Some(DAEMON_VERSION) && alive(pid))
}

/// Start the watcher on first use. An older Prelude daemon is stopped first;
/// otherwise an upgrade would leave its text-only watcher alive indefinitely.
pub fn ensure_running() {
    if is_running() {
        return;
    }
    if let Some((pid, _)) = recorded_pid()
        .filter(|(pid, _)| alive(*pid) && process_is_clipd(*pid))
    {
        unsafe extern "C" {
            unsafe fn kill(pid: i32, sig: i32) -> i32;
        }
        let _ = unsafe { kill(pid, 15) };
    }
    // Do the one-time v3 migration before gather takes its snapshot. Leaving
    // it solely to the detached watcher makes the first c: opened after an
    // upgrade race the rewrite and show the old triplicates once more.
    let _ = std::fs::create_dir_all(paths::data());
    let _ = std::fs::create_dir_all(assets_dir());
    private_dir(&assets_dir());
    migrate_image_fingerprints();
    crate::cache::spawn_self(&["clipd"]);
}

/// A stale pidfile may name an unrelated process after pid reuse. Pay for one
/// `ps` only while replacing an old protocol; never signal a pid merely
/// because it was written here once.
fn process_is_clipd(pid: i32) -> bool {
    let pid = pid.to_string();
    let command = crate::exec::run(&["ps", "-p", &pid, "-o", "command="], Duration::from_secs(1));
    command.split_whitespace().next_back() == Some("clipd")
}

pub fn history_path() -> PathBuf {
    paths::data().join("clipboard.jsonl")
}

pub fn assets_dir() -> PathBuf {
    paths::data().join("clipboard")
}

pub fn watch() -> i32 {
    let _ = std::fs::create_dir_all(paths::cache());
    let _ = std::fs::create_dir_all(paths::data());
    let _ = std::fs::create_dir_all(assets_dir());
    private_dir(&assets_dir());
    migrate_image_fingerprints();
    let _ = std::fs::write(
        pidfile(),
        format!("{} {DAEMON_VERSION}\n", std::process::id()),
    );

    #[cfg(target_os = "macos")]
    if crate::exec::which("osascript").is_some() {
        return watch_macos();
    }

    watch_text()
}

fn watch_text() -> i32 {
    let reader = if crate::exec::which("pbpaste").is_some() {
        "pbpaste"
    } else if crate::exec::which("wl-paste").is_some() {
        "wl-paste"
    } else {
        return 1;
    };

    let mut last = String::new();
    loop {
        let text = crate::exec::run(&[reader], Duration::from_secs(5)).trim().to_string();
        if !text.is_empty() && text != last && text.len() < 8000 {
            last.clone_from(&text);
            record(serde_json::json!({
                "ts": crate::frecency::now(),
                "kind": "text",
                "t": text,
            }));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(target_os = "macos")]
fn watch_macos() -> i32 {
    // Keep AppKit loaded and poll NSPasteboard.changeCount in one process.
    // Starting osascript every second costs ~50ms of CPU each time; one
    // sleeping process is effectively free.
    loop {
        let mut child = match Command::new("osascript")
            .args(["-l", "JavaScript"])
            .env("PRELUDE_CLIPBOARD_DIR", assets_dir())
            .env("PRELUDE_CLIPBOARD_PARENT", std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(_) => return watch_text(),
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(WATCH_JXA.as_bytes());
        }
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str(&line) {
                    record(value);
                }
            }
        }
        let _ = child.wait();
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn record(mut value: serde_json::Value) {
    let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("text");
    match kind {
        "text" => {
            let Some(text) = value.get("t").and_then(|v| v.as_str()) else { return };
            if text.is_empty() || text.len() >= 8000 || crate::secrets::looks_secret(text) {
                return;
            }
        }
        "files" => {
            let Some(files) = value.get("paths").and_then(|v| v.as_array()) else { return };
            if files.is_empty()
                || files.iter().filter_map(|v| v.as_str()).any(crate::secrets::looks_secret)
            {
                return;
            }
        }
        "image" => {
            let Some(path) = value.get("path").and_then(|v| v.as_str()) else { return };
            let Some(real) = private_asset(path) else { return };
            // Clipboard screenshots can be large, but an accidental 200 MB
            // bitmap must not silently turn into permanent launcher state.
            let Ok(meta) = real.metadata() else { return };
            if meta.len() > 25 * 1024 * 1024 {
                let _ = std::fs::remove_file(real);
                return;
            }
            private_file(&real);
            let Some(fingerprint) = image_fingerprint(&real) else {
                let _ = std::fs::remove_file(real);
                return;
            };
            value["fingerprint"] = serde_json::Value::String(fingerprint.clone());
            // Keep the newest occurrence, just as the read side does for text
            // and Finder objects. Some screenshot tools bump changeCount two
            // or three times while publishing one pasteboard image.
            remove_older_image(&fingerprint);
        }
        _ => return,
    }
    append_line(&value.to_string());
}

fn image_fingerprint(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(crate::capability::fingerprint(&bytes))
}

fn remove_older_image(fingerprint: &str) {
    let path = history_path();
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let mut kept = Vec::new();
    let mut removed = Vec::new();
    for line in text.lines() {
        let value = serde_json::from_str::<serde_json::Value>(line).ok();
        let duplicate = value.as_ref().is_some_and(|value| {
            value.get("kind").and_then(|v| v.as_str()) == Some("image")
                && value.get("fingerprint").and_then(|v| v.as_str()) == Some(fingerprint)
        });
        if duplicate {
            if let Some(path) = value
                .as_ref()
                .and_then(|value| value.get("path"))
                .and_then(|path| path.as_str())
            {
                removed.push(path.to_string());
            }
        } else {
            kept.push(line);
        }
    }
    if removed.is_empty() {
        return;
    }
    let bytes = if kept.is_empty() {
        Vec::new()
    } else {
        format!("{}\n", kept.join("\n")).into_bytes()
    };
    if crate::cache::write_atomic(&path, &bytes).is_err() {
        return;
    }
    private_file(&path);
    for old in removed {
        if let Some(path) = private_asset(&old) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Upgrade old image rows once, behind the daemon boundary rather than on
/// gather. The newest byte-identical image survives and receives the rank its
/// latest pasteboard change earned; old private payloads are removed.
fn migrate_image_fingerprints() {
    migrate_image_fingerprints_at(&history_path(), &assets_dir());
}

fn migrate_image_fingerprints_at(path: &Path, assets: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::new();
    let mut removed = Vec::new();
    let mut changed = false;
    for line in text.lines().rev() {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) else {
            kept.push(line.to_string());
            continue;
        };
        if value.get("kind").and_then(|v| v.as_str()) != Some("image") {
            kept.push(line.to_string());
            continue;
        }
        let Some(raw_path) = value
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            kept.push(line.to_string());
            continue;
        };
        let fingerprint = value
            .get("fingerprint")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| private_asset_in(&raw_path, assets).and_then(|path| image_fingerprint(&path)));
        let Some(fingerprint) = fingerprint else {
            kept.push(line.to_string());
            continue;
        };
        if !seen.insert(fingerprint.clone()) {
            removed.push(raw_path);
            changed = true;
            continue;
        }
        if value.get("fingerprint").and_then(|v| v.as_str()) != Some(&fingerprint) {
            value["fingerprint"] = serde_json::Value::String(fingerprint);
            changed = true;
        }
        kept.push(value.to_string());
    }
    if !changed {
        return;
    }
    kept.reverse();
    let bytes = if kept.is_empty() {
        Vec::new()
    } else {
        format!("{}\n", kept.join("\n")).into_bytes()
    };
    if crate::cache::write_atomic(path, &bytes).is_err() {
        return;
    }
    private_file(path);
    for old in removed {
        if let Some(path) = private_asset_in(&old, assets) {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn append_line(line: &str) {
    let p = history_path();
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(file, "{line}");
        private_file(&p);
    }
    trim(&p);
}

/// Keep a bounded history and remove image payloads no surviving row names.
fn trim(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut images = 0;
    for line in lines.iter().rev() {
        let is_image = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|v| v.get("kind").and_then(|k| k.as_str()).map(|k| k == "image"))
            .unwrap_or(false);
        if is_image {
            images += 1;
            if images > MAX_IMAGES {
                continue;
            }
        }
        kept.push(*line);
        if kept.len() >= MAX_HISTORY {
            break;
        }
    }
    kept.reverse();
    if kept.len() != lines.len() {
        let _ = crate::cache::write_atomic(path, format!("{}\n", kept.join("\n")).as_bytes());
    }

    let referenced: std::collections::HashSet<PathBuf> = kept
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|v| v.get("path").and_then(|p| p.as_str()).map(PathBuf::from))
        .collect();
    if let Ok(entries) = std::fs::read_dir(assets_dir()) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_file()) && !referenced.contains(&entry.path()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn private_asset_in(path: &str, root: &Path) -> Option<PathBuf> {
    let asset = Path::new(path).canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    (asset.parent() == Some(root.as_path())).then_some(asset)
}

pub(crate) fn private_asset(path: &str) -> Option<PathBuf> {
    private_asset_in(path, &assets_dir())
}

#[cfg(unix)]
fn private_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn private_dir(_: &Path) {}

#[cfg(unix)]
fn private_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn private_file(_: &Path) {}

/// Put Finder file objects on the pasteboard. This is deliberately distinct
/// from copying their path as text: Finder's Paste command needs file URLs.
pub fn copy_files(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("there are no files to copy".into());
    }
    let paths: Vec<String> = paths
        .iter()
        .filter(|p| Path::new(p).exists())
        .cloned()
        .collect();
    if paths.is_empty() {
        return Err("those files no longer exist".into());
    }
    run_jxa(COPY_FILES_JXA, &[('P', serde_json::to_string(&paths).unwrap_or_default())])
}

/// Restore a clipboard-history row using its original pasteboard type.
pub fn restore(item: &Item) -> Result<(), String> {
    match item.get("clip_kind") {
        "files" => {
            let paths = serde_json::from_str::<Vec<String>>(item.get("paths"))
                .map_err(|_| "that file list is no longer readable".to_string())?;
            copy_files(&paths)
        }
        "image" => {
            let path = item.get("path");
            if private_asset(path).is_none() {
                return Err("that clipboard image is no longer available".into());
            }
            run_jxa(COPY_IMAGE_JXA, &[('I', path.to_string())])
        }
        _ => Err("not an object clipboard row".into()),
    }
}

#[cfg(target_os = "macos")]
fn run_jxa(script: &str, env: &[(char, String)]) -> Result<(), String> {
    let mut cmd = Command::new("osascript");
    cmd.args(["-l", "JavaScript", "-e", script]);
    for (key, value) in env {
        cmd.env(format!("PRELUDE_CLIP_{key}"), value);
    }
    let out = cmd.output().map_err(|e| format!("could not reach the clipboard: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&out.stderr);
        Err(detail.trim().lines().last().unwrap_or("could not update the clipboard").to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn run_jxa(_: &str, _: &[(char, String)]) -> Result<(), String> {
    Err("file clipboard objects require macOS".into())
}

#[cfg(target_os = "macos")]
const WATCH_JXA: &str = r#"
ObjC.import('AppKit')
ObjC.import('Foundation')
ObjC.import('unistd')
const pb = $.NSPasteboard.generalPasteboard
const output = $.NSFileHandle.fileHandleWithStandardOutput
const env = $.NSProcessInfo.processInfo.environment
const directory = env.objectForKey('PRELUDE_CLIPBOARD_DIR').js
const parent = Number(env.objectForKey('PRELUDE_CLIPBOARD_PARENT').js)
let last = -1
function emit(value) {
  const line = $(JSON.stringify(value) + '\n').dataUsingEncoding($.NSUTF8StringEncoding)
  output.writeData(line)
}
while (true) {
  try {
    if (Number($.getppid()) !== parent) break
    const change = Number(pb.changeCount)
    if (change !== last) {
      last = change
      const ts = Date.now() / 1000
      const types = pb.types.js.map(t => t.js)
      let paths = []
      if (types.includes('NSFilenamesPboardType')) {
        const list = ObjC.deepUnwrap(pb.propertyListForType('NSFilenamesPboardType'))
        if (Array.isArray(list)) paths = list.filter(p => typeof p === 'string')
      }
      if (paths.length === 0) {
        for (const item of pb.pasteboardItems.js) {
          const itemTypes = item.types.js.map(t => t.js)
          if (!itemTypes.includes('public.file-url')) continue
          const raw = item.stringForType('public.file-url')
          const url = $.NSURL.URLWithString(raw)
          if (url.isFileURL) paths.push(url.path.js)
        }
      }
      if (paths.length > 0) {
        emit({ts: ts, kind: 'files', paths: paths})
      } else if (types.includes('public.png') || types.includes('public.tiff')) {
        let data
        let rep
        if (types.includes('public.png')) {
          data = pb.dataForType('public.png')
          rep = $.NSBitmapImageRep.imageRepWithData(data)
        } else {
          const tiff = pb.dataForType('public.tiff')
          rep = $.NSBitmapImageRep.imageRepWithData(tiff)
          data = rep.representationUsingTypeProperties(
            $.NSBitmapImageFileTypePNG, $.NSDictionary.dictionary)
        }
        const path = directory + '/' + change + '-' + Date.now() + '.png'
        if (data.writeToFileAtomically(path, true)) {
          emit({ts: ts, kind: 'image', path: path,
                width: Number(rep.pixelsWide), height: Number(rep.pixelsHigh)})
        }
      } else {
        let text = null
        for (const item of pb.pasteboardItems.js) {
          const itemTypes = item.types.js.map(t => t.js)
          if (!itemTypes.includes('public.utf8-plain-text')) continue
          const value = item.stringForType('public.utf8-plain-text')
          if (value) { text = value.js; break }
        }
        if (text) emit({ts: ts, kind: 'text', t: text})
      }
    }
  } catch (_) {}
  $.NSThread.sleepForTimeInterval(1)
}
"#;

#[cfg(target_os = "macos")]
const COPY_FILES_JXA: &str = r#"
ObjC.import('AppKit'); ObjC.import('Foundation')
const env = $.NSProcessInfo.processInfo.environment
const paths = JSON.parse(env.objectForKey('PRELUDE_CLIP_P').js)
const values = $.NSMutableArray.array
for (const path of paths) values.addObject($(path))
const pb = $.NSPasteboard.generalPasteboard
pb.declareTypesOwner($(['NSFilenamesPboardType']), null)
if (!pb.setPropertyListForType(values, 'NSFilenamesPboardType')) throw Error('copy failed')
"#;

#[cfg(not(target_os = "macos"))]
const COPY_FILES_JXA: &str = "";

#[cfg(target_os = "macos")]
const COPY_IMAGE_JXA: &str = r#"
ObjC.import('AppKit'); ObjC.import('Foundation')
const env = $.NSProcessInfo.processInfo.environment
const path = env.objectForKey('PRELUDE_CLIP_I').js
const image = $.NSImage.alloc.initWithContentsOfFile(path)
const values = $.NSMutableArray.array; values.addObject(image)
const pb = $.NSPasteboard.generalPasteboard; pb.clearContents
if (!pb.writeObjects(values)) throw Error('copy failed')
"#;

#[cfg(not(target_os = "macos"))]
const COPY_IMAGE_JXA: &str = "";

#[cfg(test)]
mod tests {
    use super::migrate_image_fingerprints_at;

    #[test]
    fn image_migration_keeps_only_the_newest_byte_identical_copy() {
        let root = std::env::temp_dir().join(format!(
            "prelude-clipboard-dedupe-{}-{}",
            std::process::id(),
            crate::frecency::now() as u64
        ));
        let assets = root.join("clipboard");
        let history = root.join("clipboard.jsonl");
        std::fs::create_dir_all(&assets).unwrap();
        let old = assets.join("old.png");
        let middle = assets.join("middle.png");
        let newest = assets.join("newest.png");
        let different = assets.join("different.png");
        for path in [&old, &middle, &newest] {
            std::fs::write(path, b"same pixels").unwrap();
        }
        std::fs::write(&different, b"different pixels").unwrap();
        let image = |ts: u64, path: &std::path::Path| {
            serde_json::json!({"ts":ts,"kind":"image","path":path}).to_string()
        };
        std::fs::write(
            &history,
            [
                image(1, &old),
                image(2, &middle),
                serde_json::json!({"ts":3,"kind":"text","t":"between"}).to_string(),
                image(4, &newest),
                image(5, &different),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        migrate_image_fingerprints_at(&history, &assets);

        let records: Vec<serde_json::Value> = std::fs::read_to_string(&history)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let images: Vec<&serde_json::Value> = records
            .iter()
            .filter(|record| record.get("kind").and_then(|v| v.as_str()) == Some("image"))
            .collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].get("path").and_then(|v| v.as_str()), newest.to_str());
        assert_eq!(images[1].get("path").and_then(|v| v.as_str()), different.to_str());
        assert!(images.iter().all(|image| image.get("fingerprint").is_some()));
        assert!(!old.exists() && !middle.exists());
        assert!(newest.exists() && different.exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
