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
///
/// Asking `ps` for `comm=` doubles the cost of the whole call — 21ms against
/// 10ms here, for ~850 processes — because the kernel has to be asked for
/// each process's argument block one at a time to recover argv[0]. That was
/// the largest single cost in `gather`, and it was being paid for eight
/// hundred rows to display two dozen.
///
/// So the list is fetched with the fields that come free out of the process
/// table, and the name is upgraded afterwards for the handful of rows that
/// survive the filter — the same sysctl, forty times instead of eight
/// hundred and fifty.
pub fn procs() -> Vec<Item> {
    let out = run(&["ps", "-Ao", "pid=,pcpu=,rss=,ucomm=", "-r"], Duration::from_secs(3));
    let mut items = Vec::new();
    // One buffer for the whole pass. It is virtual until written to, and the
    // kernel copies only the arguments that exist — a few kilobytes.
    let mut buf = vec![0u8; arg_max()];
    for line in out.lines().take(40) {
        // Not splitn(): runs of spaces produce empty fields, and the name is
        // a path that legitimately contains spaces ("Input Source Pro"), so
        // the tail must stay intact.
        let Some((pid, cpu, rss, ucomm)) = split3(line) else { continue };
        let (Ok(cpu_f), Ok(rss_kb)) = (cpu.parse::<f64>(), rss.parse::<f64>()) else {
            continue;
        };
        let mb = rss_kb / 1024.0;
        if cpu_f < 0.5 && mb < 200.0 {
            continue;
        }
        let full = argv0(pid, &mut buf).or_else(|| exe_path(pid));
        let comm = full.as_deref().unwrap_or(ucomm);
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

unsafe extern "C" {
    unsafe fn sysctl(
        name: *const i32,
        namelen: u32,
        oldp: *mut u8,
        oldlenp: *mut usize,
        newp: *const u8,
        newlen: usize,
    ) -> i32;
}

const CTL_KERN: i32 = 1;
const KERN_ARGMAX: i32 = 8;
const KERN_PROCARGS2: i32 = 49;

/// The largest argument block the kernel will ever hand back, which is how
/// big the scratch buffer has to be.
///
/// It cannot be guessed at. The block is `argc`, the executable path, then
/// *padding* before argv begins — and the padding is whatever the stack
/// layout made it, four kilobytes for a Chrome-style helper here. A buffer
/// too small to reach past it comes back looking like a process with no
/// arguments, and the row silently falls back to the short name.
fn arg_max() -> usize {
    let mib = [CTL_KERN, KERN_ARGMAX];
    let mut out = 0i32;
    let mut len = std::mem::size_of::<i32>();
    let ok = unsafe {
        sysctl(mib.as_ptr(), 2, (&raw mut out).cast(), &mut len, std::ptr::null(), 0) == 0
    };
    if ok && out > 0 { out as usize } else { 256 * 1024 }
}

/// How a process was invoked, which is what `ps -o comm=` prints.
///
/// Not the executable path: an agent CLI is routinely a script, so
/// `proc_pidpath` would report `pi` and `claude` as `node` — precisely the
/// two rows a launcher for agents must not mislabel. argv[0] is what the
/// person typed, and it is what `ps` shows.
///
/// `/bin/ps` is setuid root and we are not, so another user's process — most
/// of the system daemons — answers EINVAL rather than its arguments. Those
/// fall back to `proc_pidpath`, which is readable for anything and gives the
/// executable in full; the order matters and cannot be swapped, because
/// `proc_pidpath` is exactly the answer that turns `pi` into `node`.
fn argv0(pid: &str, buf: &mut [u8]) -> Option<String> {
    let pid: i32 = pid.parse().ok()?;
    let mib = [CTL_KERN, KERN_PROCARGS2, pid];
    let mut len = buf.len();
    let ok =
        unsafe { sysctl(mib.as_ptr(), 3, buf.as_mut_ptr(), &mut len, std::ptr::null(), 0) == 0 };
    if !ok || len < 8 {
        return None;
    }
    let block = &buf[..len];
    if i32::from_ne_bytes(block[..4].try_into().ok()?) < 1 {
        return None; // no arguments recorded
    }
    let rest = &block[4..];
    let exec_end = rest.iter().position(|b| *b == 0)?;
    let start = exec_end + rest[exec_end..].iter().take_while(|b| **b == 0).count();
    let arg = rest.get(start..)?;
    let end = arg.iter().position(|b| *b == 0).unwrap_or(arg.len());
    let s = String::from_utf8_lossy(&arg[..end]).into_owned();
    (!s.is_empty()).then_some(s)
}

/// The executable behind a pid, readable even for another user's process.
///
/// Only ever a fallback — see `argv0`. It reports what was *executed*, so a
/// script's interpreter, which is the wrong answer for every agent CLI and
/// the right one for the system daemons that will not show their arguments.
fn exe_path(pid: &str) -> Option<String> {
    unsafe extern "C" {
        unsafe fn proc_pidpath(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;
    }
    const PROC_PIDPATHINFO_MAXSIZE: usize = 4 * 1024;
    let pid: i32 = pid.parse().ok()?;
    let mut buf = [0u8; PROC_PIDPATHINFO_MAXSIZE];
    let n = unsafe { proc_pidpath(pid, buf.as_mut_ptr(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
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

/// Where the docker daemon listens, when that is a unix socket we can name.
///
/// Used only to decide *not* to ask. `docker ps` costs the same 14ms whether
/// or not a daemon answers it — `docker --version` alone costs 13 — because
/// the cost is the CLI binary starting rather than the query. Docker Desktop
/// and OrbStack are both routinely installed and not running, and that is
/// the case worth not paying for: there is no socket, and no subprocess is
/// going to find one.
///
/// Anything this cannot resolve — a TCP `DOCKER_HOST`, a context layout that
/// has moved on — returns `None` and the subprocess runs as it always did.
/// Being unsure must cost a launch 14ms, never a container.
fn docker_socket() -> Option<std::path::PathBuf> {
    let unix = |s: &str| s.strip_prefix("unix://").map(std::path::PathBuf::from);
    if let Some(host) = std::env::var_os("DOCKER_HOST") {
        // A TCP host is not ours to second-guess; fall through and ask.
        return unix(&host.to_string_lossy());
    }
    let dir = paths::home().join(".docker");
    let json = |p: std::path::PathBuf| -> Option<serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
    };
    let ctx = std::env::var("DOCKER_CONTEXT").ok().or_else(|| {
        json(dir.join("config.json"))?
            .get("currentContext")?
            .as_str()
            .map(str::to_string)
    });
    let Some(ctx) = ctx.filter(|c| c != "default") else {
        return Some(std::path::PathBuf::from("/var/run/docker.sock"));
    };
    // Contexts are stored under a hash of their name, so the name has to be
    // read back out of each one rather than computed.
    for e in std::fs::read_dir(dir.join("contexts/meta")).ok()?.flatten() {
        let Some(meta) = json(e.path().join("meta.json")) else { continue };
        if meta.get("Name").and_then(|n| n.as_str()) != Some(ctx.as_str()) {
            continue;
        }
        return unix(meta.pointer("/Endpoints/docker/Host")?.as_str()?);
    }
    None
}

/// Running containers. Silently empty when the daemon is off.
pub fn containers() -> Vec<Item> {
    // Nothing is listening, so nothing can be running.
    if docker_socket().is_some_and(|s| !s.exists()) {
        return Vec::new();
    }
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
