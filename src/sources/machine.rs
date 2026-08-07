//! Sources that describe the machine: ports, processes, containers, apps,
//! and the system commands that have no memorable incantation.

use crate::exec::{run, shq};
use crate::item::{Item, Kind};
use crate::paths;
use std::time::Duration;

/// Listening TCP ports — "what's on 3000, and kill it".
///
/// The generated command re-resolves the pid at run time rather than baking
/// in the one we saw. This source is served from cache (lsof costs ~65ms and
/// cannot be made faster — restricting it to one user is *worse*, 210ms), so
/// a cached pid could belong to a different process by the time you press
/// Enter. This command kills things; it must not trust stale data.
pub fn ports() -> Vec<Item> {
    let out = run(&["lsof", "-nP", "-iTCP", "-sTCP:LISTEN"], Duration::from_secs(3));
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for line in out.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 9 {
            continue;
        }
        let (proc, pid) = (f[0], f[1]);
        let Some(port) = f[f.len() - 2].rsplit(':').next() else { continue };
        if !port.chars().all(|c| c.is_ascii_digit()) || !seen.insert(port.to_string()) {
            continue;
        }
        items.push(
            Item::new(format!("kill $(lsof -ti tcp:{port})"), Kind::Port)
                .title(format!(":{port}  {proc}"))
                .fields([proc.to_string(), format!("pid {pid}")])
                .put("port", port)
                .put("pid", pid)
                .put("proc", proc),
        );
    }
    items.sort_by_key(|i| i.get("port").parse::<u32>().unwrap_or(0));
    items
}

/// Heaviest processes by CPU. The gap `ports` leaves: sometimes you know the
/// program is misbehaving but not which port it holds.
pub fn procs() -> Vec<Item> {
    let out = run(&["ps", "-Ao", "pid=,pcpu=,rss=,comm=", "-r"], Duration::from_secs(3));
    let mut items = Vec::new();
    for line in out.lines().take(40) {
        // Not splitn(): runs of spaces produce empty fields, and the command
        // is a path that legitimately contains spaces
        // ("Google Chrome Helper (Renderer)"), so the tail must stay intact.
        let Some((pid, cpu, rss, comm)) = split3(line) else { continue };
        let (Ok(cpu_f), Ok(rss_kb)) = (cpu.parse::<f64>(), rss.parse::<f64>()) else {
            continue;
        };
        let mb = rss_kb / 1024.0;
        if cpu_f < 0.5 && mb < 200.0 {
            continue;
        }
        let name = comm.rsplit('/').next().unwrap_or(comm).to_string();
        let mem = if mb >= 1024.0 {
            format!("{:.1}GB", mb / 1024.0)
        } else {
            format!("{mb:.0}MB")
        };
        items.push(
            Item::new(format!("kill {pid}"), Kind::Proc)
                .title(name.clone())
                .fields([format!("{cpu_f:.0}% CPU"), mem.clone(), format!("pid {pid}")])
                .put("pid", pid)
                .put("name", name)
                .put("cpu", cpu)
                .put("mem", mem)
                .put("cmd", comm),
        );
    }
    items
}

/// Take the first three whitespace-separated fields, leaving the remainder
/// untouched.
fn split3(line: &str) -> Option<(&str, &str, &str, &str)> {
    let mut rest = line.trim_start();
    let mut f = [""; 3];
    for slot in f.iter_mut() {
        let end = rest.find(char::is_whitespace)?;
        *slot = &rest[..end];
        rest = rest[end..].trim_start();
    }
    Some((f[0], f[1], f[2], rest.trim_end()))
}

/// Running containers. Silently empty when the daemon is off.
pub fn containers() -> Vec<Item> {
    let out = run(
        &["docker", "ps", "--format", "{{.Names}}\t{{.Status}}\t{{.Image}}"],
        Duration::from_secs(3),
    );
    out.lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            let (name, status, image) = (p.next()?, p.next()?, p.next()?);
            Some(
                Item::new(format!("docker exec -it {} sh", shq(name)), Kind::Container)
                    .title(name)
                    .fields([status.to_string(), image.to_string()])
                    .put("name", name)
                    .put("image", image),
            )
        })
        .collect()
}

/// Installed applications. Scanning is ~2ms, so no cache needed.
pub fn apps() -> Vec<Item> {
    let home = paths::home();
    let roots = [
        std::path::PathBuf::from("/Applications"),
        std::path::PathBuf::from("/System/Applications"),
        std::path::PathBuf::from("/System/Applications/Utilities"),
        std::path::PathBuf::from("/Applications/Utilities"),
        home.join("Applications"),
    ];
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else { continue };
        for e in entries.flatten() {
            let mut found = Vec::new();
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(stem) = name.strip_suffix(".app") {
                found.push((stem.to_string(), e.path()));
            } else if e.path().is_dir() {
                // one level down: /Applications/Foo/Bar.app
                if let Ok(sub) = std::fs::read_dir(e.path()) {
                    for s in sub.flatten() {
                        let n = s.file_name().to_string_lossy().into_owned();
                        if let Some(stem) = n.strip_suffix(".app") {
                            found.push((stem.to_string(), s.path()));
                        }
                    }
                }
            }
            for (stem, path) in found {
                if !seen.insert(stem.clone()) {
                    continue;
                }
                let parent = path
                    .parent()
                    .map(|p| paths::tilde(&p.to_string_lossy()))
                    .unwrap_or_default();
                items.push(
                    Item::new(format!("open -a {}", shq(&stem)), Kind::App)
                        .title(stem)
                        .sub(parent)
                        .put("path", path.to_string_lossy()),
                );
            }
        }
    }
    items
}

/// Things macOS can do that have no memorable command. Anything destructive
/// stays insert-only: you get it on your prompt, not as a surprise.
const SYSTEM: &[(&str, &str, &str)] = &[
    ("Lock screen", "pmset displaysleepnow", "turn the display off"),
    ("Sleep now", "pmset sleepnow", "suspend the machine"),
    ("Toggle dark mode",
     "osascript -e 'tell app \"System Events\" to tell appearance preferences to set dark mode to not dark mode'",
     "flip between light and dark"),
    ("Empty the trash", "osascript -e 'tell app \"Finder\" to empty trash'", "permanently delete trashed files"),
    ("Show hidden files",
     "defaults write com.apple.finder AppleShowAllFiles -bool true; killall Finder", "reveal dotfiles in Finder"),
    ("Hide hidden files",
     "defaults write com.apple.finder AppleShowAllFiles -bool false; killall Finder", "hide dotfiles in Finder"),
    ("Flush DNS cache",
     "sudo dscacheutil -flushcache; sudo killall -HUP mDNSResponder", "after editing /etc/hosts"),
    ("Wi-Fi off", "networksetup -setairportpower en0 off", "disable the wireless interface"),
    ("Wi-Fi on", "networksetup -setairportpower en0 on", "enable the wireless interface"),
    ("Restart Finder", "killall Finder", "when Finder misbehaves"),
    ("Restart Dock", "killall Dock", "when the Dock misbehaves"),
    ("Copy my IP (local)", "ipconfig getifaddr en0", "address on the local network"),
    ("Copy my IP (public)", "curl -s ifconfig.me", "address as the internet sees you"),
    ("Caffeinate (keep awake)", "caffeinate -d", "until you press Ctrl-C"),
    ("Eject all disks",
     "osascript -e 'tell app \"Finder\" to eject (every disk whose ejectable is true)'", "before unplugging"),
    ("Restart bluetooth", "sudo pkill bluetoothd", "when devices will not pair"),
];

pub fn system() -> Vec<Item> {
    SYSTEM
        .iter()
        .map(|(label, cmd, zh)| {
            Item::new(*cmd, Kind::Sys).title(*label).sub(*zh).put("label", *label)
        })
        .collect()
}
