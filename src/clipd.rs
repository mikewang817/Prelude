//! Clipboard watcher. macOS has no clipboard-change event, so this polls.

use crate::paths;
use std::time::Duration;

fn pidfile() -> std::path::PathBuf {
    paths::cache().join("clipd.pid")
}

pub fn is_running() -> bool {
    let Ok(t) = std::fs::read_to_string(pidfile()) else { return false };
    let Ok(pid) = t.trim().parse::<i32>() else { return false };
    unsafe extern "C" {
        unsafe fn kill(pid: i32, sig: i32) -> i32;
    }
    // signal 0 just tests existence
    unsafe { kill(pid, 0) == 0 }
}

/// Start the watcher on first use, then leave it running.
pub fn ensure_running() {
    if is_running() {
        return;
    }
    crate::cache::spawn_self(&["clipd"]);
}

pub fn history_path() -> std::path::PathBuf {
    paths::data().join("clipboard.jsonl")
}

pub fn watch() -> i32 {
    let _ = std::fs::create_dir_all(paths::cache());
    let _ = std::fs::create_dir_all(paths::data());
    let _ = std::fs::write(pidfile(), std::process::id().to_string());

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
            // never record credentials
            if !crate::secrets::looks_secret(&text) {
                let rec = serde_json::json!({ "ts": crate::frecency::now(), "t": text });
                append_line(&rec.to_string());
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn append_line(line: &str) {
    use std::io::Write;
    let p = history_path();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{line}");
    }
    trim(&p, 500);
}

fn trim(p: &std::path::Path, max_lines: usize) {
    let Ok(m) = std::fs::metadata(p) else { return };
    if m.len() < 400_000 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(p) else { return };
    let lines: Vec<&str> = text.lines().collect();
    let keep = lines[lines.len().saturating_sub(max_lines)..].join("\n");
    let _ = crate::cache::write_atomic(p, format!("{keep}\n").as_bytes());
}
