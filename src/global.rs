//! Production macOS global hotkey integration.
//!
//! The latency-sensitive launcher remains a terminal program. This module is
//! reached only through an explicit `prelude global ...` command and manages
//! the hidden Ghostty instance that owns the chord and hosts the panel. No GUI
//! framework or hotkey dependency is linked into the Prelude binary itself.
//!
//! There is no terminal backend to choose. There was, while a press built a
//! window to leave a command in; the panel copies instead, so the only
//! terminal in this module is the one the panel itself is.

use serde::Serialize;
use std::ffi::OsStr;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const LABEL: &str = "app.prelude.hotkey";
const DEFAULT_HOTKEY: &str = "cmd+shift+space";
const LEGACY_DEFAULT_HOTKEY: &str = "cmd+space";
/// The launcher panel is a Ghostty quick terminal: a real macOS panel, hidden
/// from the Dock and the app switcher, hosting one long-lived `prelude _panel`
/// loop. Ghostty registers the chord itself, so nothing of Prelude's runs at
/// press time and there is no terminal to build.
const QUICK_CONFIG: &str = "quick-terminal.ghostty";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hotkey {
    cmd: bool,
    option: bool,
    ctrl: bool,
    shift: bool,
    key: String,
    key_code: u32,
}

impl Hotkey {
    fn parse(value: &str) -> Result<Self, String> {
        let mut cmd = false;
        let mut option = false;
        let mut ctrl = false;
        let mut shift = false;
        let mut key = None;
        for raw in value.split('+') {
            let part = raw.trim().to_ascii_lowercase();
            let slot = match part.as_str() {
                "cmd" | "command" => Some(&mut cmd),
                "option" | "alt" => Some(&mut option),
                "ctrl" | "control" => Some(&mut ctrl),
                "shift" => Some(&mut shift),
                _ => None,
            };
            if let Some(slot) = slot {
                if *slot {
                    return Err(format!("duplicate hotkey modifier {part:?}"));
                }
                *slot = true;
                continue;
            }
            if key.is_some() || part.is_empty() {
                return Err(format!(
                    "invalid hotkey {value:?}; use modifiers joined to one space, letter or digit"
                ));
            }
            key = Some(part);
        }
        if !(cmd || option || ctrl || shift) {
            return Err("a global hotkey needs cmd, option, ctrl or shift".into());
        }
        let key =
            key.ok_or_else(|| "a global hotkey needs a key after its modifiers".to_string())?;
        let key_code = key_code(&key).ok_or_else(|| {
            format!("unsupported hotkey key {key:?}; choose space, a letter or a digit")
        })?;
        Ok(Self {
            cmd,
            option,
            ctrl,
            shift,
            key,
            key_code,
        })
    }

    fn canonical(&self) -> String {
        let mut parts = Vec::new();
        if self.cmd {
            parts.push("cmd");
        }
        if self.option {
            parts.push("option");
        }
        if self.ctrl {
            parts.push("ctrl");
        }
        if self.shift {
            parts.push("shift");
        }
        parts.push(&self.key);
        parts.join("+")
    }

    /// The chord as macOS records it in its own shortcut table: Cocoa event
    /// flags, not the Carbon bits `RegisterEventHotKey` takes.
    fn cocoa_mask(&self) -> u32 {
        let mut mask = 0;
        if self.shift {
            mask |= COCOA_SHIFT;
        }
        if self.ctrl {
            mask |= COCOA_CONTROL;
        }
        if self.option {
            mask |= COCOA_OPTION;
        }
        if self.cmd {
            mask |= COCOA_COMMAND;
        }
        mask
    }

    fn matches_raycast(&self, value: &str) -> bool {
        let mut command = false;
        let mut option = false;
        let mut control = false;
        let mut shift = false;
        let mut code = None;
        for part in value.split('-') {
            match part.to_ascii_lowercase().as_str() {
                "command" => command = true,
                "option" => option = true,
                "control" => control = true,
                "shift" => shift = true,
                number => code = number.parse::<u32>().ok(),
            }
        }
        command == self.cmd
            && option == self.option
            && control == self.ctrl
            && shift == self.shift
            && code == Some(self.key_code)
    }
}

fn key_code(key: &str) -> Option<u32> {
    Some(match key {
        "a" => 0,
        "s" => 1,
        "d" => 2,
        "f" => 3,
        "h" => 4,
        "g" => 5,
        "z" => 6,
        "x" => 7,
        "c" => 8,
        "v" => 9,
        "b" => 11,
        "q" => 12,
        "w" => 13,
        "e" => 14,
        "r" => 15,
        "y" => 16,
        "t" => 17,
        "1" => 18,
        "2" => 19,
        "3" => 20,
        "4" => 21,
        "6" => 22,
        "5" => 23,
        "9" => 25,
        "7" => 26,
        "8" => 28,
        "0" => 29,
        "o" => 31,
        "u" => 32,
        "i" => 34,
        "p" => 35,
        "l" => 37,
        "j" => 38,
        "k" => 40,
        "n" => 45,
        "m" => 46,
        "space" => 49,
        _ => return None,
    })
}

#[derive(Clone, Debug)]
struct GlobalConfig {
    hotkey: Hotkey,
    /// Where the panel itself stands. `None` is `$HOME`, and stays unwritten
    /// so the config carries no personal path until somebody asks for one.
    directory: Option<PathBuf>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            // Cmd+Space belongs to Spotlight on a stock Mac. Requiring a
            // second configuration command made the advertised default an
            // installation that could never work, so fresh installs start on
            // the intentionally free Cmd+Shift+Space chord.
            hotkey: Hotkey::parse(DEFAULT_HOTKEY).expect("default hotkey"),
            directory: None,
        }
    }
}

/// A configured directory reaches a process argument on one path and an Apple
/// Event on the other, so it is held to one rule strict enough for both rather
/// than escaped twice. Existence is deliberately not checked here: a directory
/// that is removed later must not make every `prelude global` command fail.
fn parse_directory(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    let expanded = match value.strip_prefix("~/") {
        Some(rest) => crate::paths::home().join(rest),
        None => PathBuf::from(value),
    };
    if !expanded.is_absolute() {
        return Err(format!(
            "the launcher directory must be an absolute path; got {value:?}"
        ));
    }
    let text = expanded.to_string_lossy();
    if let Some(bad) = text.chars().find(|c| "\"'`$\\\n\r".contains(*c)) {
        return Err(format!(
            "the launcher directory may not contain {bad:?}; it has to be safe as both a process argument and an Apple Event"
        ));
    }
    Ok(expanded)
}

fn effective_directory(config: &GlobalConfig) -> PathBuf {
    config
        .directory
        .clone()
        .unwrap_or_else(crate::paths::home)
}

#[derive(Debug, Serialize)]
pub struct GlobalStatus {
    pub schema: u8,
    pub app_installed: bool,
    pub launch_agent_installed: bool,
    pub helper_supervised: bool,
    pub helper_running: bool,
    /// `Some(true)` means Ghostty's global event tap reported ready. `None`
    /// means there is no running panel or its startup result is not available.
    pub accessibility_granted: Option<bool>,
    pub hotkey_registered: bool,
    pub selected_hotkey: String,
    /// The macOS shortcut or application known to hold the chord.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotkey_owner: Option<String>,
    /// False when a source could not be read, so an absent owner means none
    /// was found rather than none exists.
    pub owner_checks_complete: bool,
    /// Where the panel itself stands, and whether it is still there.
    pub launch_directory: String,
    pub launch_directory_exists: bool,
    pub ghostty_available: Option<bool>,
    pub zsh_widget_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event: Option<serde_json::Value>,
}

pub(crate) fn ghostty_app() -> Option<PathBuf> {
    [
        PathBuf::from("/Applications/Ghostty.app"),
        crate::paths::home().join("Applications/Ghostty.app"),
    ]
    .into_iter()
    .find(|path| path.is_dir())
}

fn ghostty_executable() -> Option<PathBuf> {
    ghostty_app().map(|app| app.join("Contents/MacOS/ghostty"))
}

fn quick_config_path() -> PathBuf {
    crate::paths::config().join(QUICK_CONFIG)
}

fn launch_agent_path() -> PathBuf {
    crate::paths::home()
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

fn config_path() -> PathBuf {
    crate::paths::config().join("global.toml")
}



fn stdout_path() -> PathBuf {
    crate::paths::cache().join("global-hotkey.log")
}

fn stderr_path() -> PathBuf {
    crate::paths::cache().join("global-hotkey-error.log")
}

fn event_tap_path() -> PathBuf {
    crate::paths::cache().join("global-event-tap")
}

fn config_from(path: &Path) -> Result<GlobalConfig, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(GlobalConfig::default()),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let parsed = crate::minitoml::parse(&text);
    let root = parsed.get("");
    // A `backend` key written by an older build is simply not read. It chose
    // where a handed-over command opened, and nothing opens now.
    let hotkey = Hotkey::parse(
        root.and_then(|table| table.get("hotkey"))
            .map(String::as_str)
            .unwrap_or(DEFAULT_HOTKEY),
    )?;
    let directory = root
        .and_then(|table| table.get("directory"))
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(parse_directory)
        .transpose()?;
    Ok(GlobalConfig { hotkey, directory })
}

fn configured() -> Result<GlobalConfig, String> {
    config_from(&config_path())
}

fn config_body(config: &GlobalConfig) -> String {
    let directory = match &config.directory {
        Some(directory) => format!("directory = \"{}\"\n", directory.display()),
        None => String::new(),
    };
    format!(
        "# Chord and starting directory for the global launcher panel.\n# An unset directory means $HOME.\nhotkey = \"{}\"\n{directory}",
        config.hotkey.canonical()
    )
}

fn write_config(config: &GlobalConfig) -> Result<(), String> {
    let path = config_path();
    crate::cache::write_atomic(&path, config_body(config).as_bytes())
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    private_file(&path)?;
    Ok(())
}

fn write_directory(value: &str) -> Result<String, String> {
    let directory = parse_directory(value)?;
    if !directory.is_dir() {
        return Err(format!("{} is not a directory", directory.display()));
    }
    let mut config = configured()?;
    config.directory = Some(directory.clone());
    write_config(&config)?;
    Ok(format!("global launcher directory: {}", directory.display()))
}

fn clear_directory() -> Result<String, String> {
    let mut config = configured()?;
    config.directory = None;
    write_config(&config)?;
    Ok(format!(
        "global launcher directory: {} (default)",
        crate::paths::home().display()
    ))
}


/// What the settings panel shows without parsing `global.toml` itself.
///
/// One reader, so a row and `prelude global status` cannot disagree about
/// which chord is configured.
pub struct GlobalSummary {
    pub hotkey: String,
    pub directory: String,
    pub hotkey_source: &'static str,
    pub directory_source: &'static str,
    pub installed: bool,
}

pub fn configured_summary() -> GlobalSummary {
    let saved = config_path().is_file();
    let config = configured().unwrap_or_default();
    let directory_saved = config.directory.is_some();
    GlobalSummary {
        hotkey: config.hotkey.canonical(),
        directory: effective_directory(&config).to_string_lossy().into_owned(),
        hotkey_source: if saved { "saved" } else { "default" },
        directory_source: if directory_saved { "saved" } else { "default" },
        installed: quick_config_path().is_file(),
    }
}

/// The settings surface may reveal or copy the authoritative file, but never
/// parses or writes it itself.
pub fn config_file() -> PathBuf {
    config_path()
}

/// Set the chord from the settings panel, with the same validation, the same
/// conflict check and the same panel restart the CLI performs.
pub fn set_hotkey(value: &str) -> Result<String, String> {
    Hotkey::parse(value).and_then(write_hotkey)
}

pub fn set_directory(value: &str) -> Result<String, String> {
    write_directory(value)
}

pub fn set_directory_default() -> Result<String, String> {
    clear_directory()
}

/// Start the panel instance if it is not up. There is nothing to reveal
/// without one, and the chord belongs to Ghostty rather than to us — so this
/// is "make sure it exists", not "show it".
pub fn open_panel() -> Result<(), String> {
    if running() {
        return Err("the panel is already running — press the chord to reveal it".into());
    }
    start_current()
}

/// Stop and restart the instance, which is what a rebuilt binary needs.
///
/// The loop was started with whatever the binary held then and executes it
/// until it exits. Each press does spawn the new binary as its child, so the
/// rows and the footer update and it looks like the build took — while the
/// half that decides what to do with an answer is still the old one.
pub fn restart_panel() -> Result<String, String> {
    if !quick_config_path().is_file() {
        return Err("the launcher panel is not installed; run `prelude global install`".into());
    }
    // A rebuild can change both the parent loop and Prelude-owned Ghostty
    // bindings. Refresh while the old panel is still healthy, then replace it.
    write_quick_config(&quick_config_path())?;
    refresh_launch_agent()?;
    stop_helper();
    start_helper()?;
    ensure_global_key_ready()?;
    Ok("panel restarted; it now runs the binary and panel configuration on disk".into())
}

fn write_hotkey(hotkey: Hotkey) -> Result<String, String> {
    let mut config = configured()?;
    if let Some(owner) = known_conflict(&hotkey) {
        return Err(format!(
            "{} is already configured for {owner}; change it there or choose another chord",
            hotkey.canonical()
        ));
    }
    if config.hotkey == hotkey {
        return Ok(format!("global hotkey already uses {}", hotkey.canonical()));
    }
    let old = std::fs::read(config_path()).ok();
    config.hotkey = hotkey.clone();
    write_config(&config)?;
    if !quick_config_path().is_file() {
        return Ok(format!(
            "global hotkey: {} (install with `prelude global install`)",
            hotkey.canonical()
        ));
    }
    // Ghostty reads the chord from the panel's configuration at startup, so
    // the instance is replaced rather than signalled.
    let restore = |bytes: Option<&[u8]>| {
        if let Some(bytes) = bytes {
            let _ = crate::cache::write_atomic(&config_path(), bytes);
            let _ = private_file(&config_path());
            let _ = write_quick_config(&quick_config_path());
        }
        let _ = start_helper();
    };
    stop_helper();
    if let Err(e) = write_quick_config(&quick_config_path()) {
        restore(old.as_deref());
        return Err(format!("{e}; the previous hotkey was restored"));
    }
    if let Err(e) = refresh_launch_agent()
        .and_then(|_| start_helper())
        .and_then(|_| ensure_global_key_ready())
    {
        restore(old.as_deref());
        return Err(format!("{e}; the previous hotkey was restored"));
    }
    Ok(format!("global hotkey: {}", hotkey.canonical()))
}

#[cfg(unix)]
fn private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("could not protect {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn private_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}


/// Start and supervise the hidden panel instance at login.
///
/// Ghostty explicitly requires macOS applications to be launched through
/// Launch Services, not by executing `Contents/MacOS/ghostty` directly.
/// `open -W` remains alive until that exact new application instance exits, so
/// launchd still has a real process to supervise and can replace the hidden
/// quick-terminal owner after a crash or Quit.
fn launch_agent(app: &Path, config: &Path, stdout: &Path, stderr: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>/usr/bin/open</string><string>-W</string><string>-n</string><string>-a</string><string>{app}</string><string>--args</string><string>--config-file={config}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>ThrottleInterval</key><integer>3</integer>
  <key>StandardOutPath</key><string>{stdout}</string>
  <key>StandardErrorPath</key><string>{stderr}</string>
</dict></plist>
"#,
        app = xml(&app.to_string_lossy()),
        config = xml(&config.to_string_lossy()),
        stdout = xml(&stdout.to_string_lossy()),
        stderr = xml(&stderr.to_string_lossy()),
    )
}

/// Upgrade an installed login agent in place. This lets `prelude global start`
/// repair integrations from both the old non-waiting `open` job and the direct
/// executable design instead of making an upgrader know to repeat `install`.
fn refresh_launch_agent() -> Result<(), String> {
    let path = launch_agent_path();
    if !path.is_file() {
        return Err("the launcher login agent is missing; run `prelude global install`".into());
    }
    let app = ghostty_app().ok_or_else(|| {
        "the launcher panel needs Ghostty in /Applications or ~/Applications".to_string()
    })?;
    let body = launch_agent(&app, &quick_config_path(), &stdout_path(), &stderr_path());
    if std::fs::read(&path).is_ok_and(|old| old == body.as_bytes()) {
        return Ok(());
    }
    stop_helper();
    crate::cache::write_atomic(&path, body.as_bytes())
        .map_err(|e| format!("could not update {}: {e}", path.display()))?;
    private_file(&path)?;
    let path_s = path.to_string_lossy().into_owned();
    run_visible("/usr/bin/plutil", &["-lint", &path_s])
}

fn run_visible(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

/// A surface made by the quick-terminal action carries Ghostty's explicit
/// marker. An ordinary `open Ghostty` request can be routed to this hidden app
/// after the panel was most recently active, so process identity alone cannot
/// decide which command belongs in the surface.
fn is_quick_terminal(marker: Option<&OsStr>) -> bool {
    marker == Some(OsStr::new("1"))
}

/// `prelude _surface` — route a surface created inside the dedicated Ghostty.
///
/// The quick terminal owns Prelude. Any ordinary window was delivered to the
/// wrong Ghostty instance by Launch Services; replace it with a fresh,
/// default-configured Ghostty application and let this transient surface close.
pub fn run_surface() -> i32 {
    if is_quick_terminal(std::env::var_os("GHOSTTY_QUICK_TERMINAL").as_deref()) {
        return crate::panel::run();
    }
    let Some(app) = ghostty_app() else {
        eprintln!("prelude: Ghostty is no longer installed");
        return 2;
    };
    match crate::openwith::launch_now(&app.to_string_lossy()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("prelude: {error}");
            2
        }
    }
}

/// The panel's Ghostty configuration.
///
/// Every line here is load-bearing, and three were found the hard way.
/// `macos-hidden` is what keeps a launcher out of the Dock and the app
/// switcher — Ghostty documents it for exactly this use. `initial-window`
/// keeps the instance at rest with no window and no shell until the chord is
/// first pressed. And `window-save-state` has to be declined or the instance
/// restores the previous session's windows, so one press arrives with a crowd.
///
/// `unconsumed:escape` is the dismissal. It hides the panel *and* passes the
/// key through, so fzf aborts, the loop starts a fresh launcher behind the
/// hidden panel, and the next press is a reveal rather than a rebuild.
fn quick_config(exe: &Path, hotkey: &Hotkey, directory: &Path) -> String {
    // Ghostty starts its command through a non-interactive login shell. That
    // shell does not read ~/.zshrc, so it otherwise loses Homebrew's fzf path.
    // Preserve the installer's PATH for the long-lived panel and retain a
    // useful macOS fallback if Prelude is launched from a minimal environment.
    let path = std::env::var("PATH")
        .unwrap_or_else(|_| "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into())
        .replace(['\n', '\r'], "");
    format!(
        "# Prelude launcher panel. Written by `prelude global install`.\n\
         # This configures a dedicated, hidden Ghostty instance; it does not\n\
         # affect the Ghostty you work in.\n\
         initial-window = false\n\
         macos-hidden = always\n\
         window-save-state = never\n\
         quick-terminal-position = center\n\
         quick-terminal-size = 62%,58%\n\
         quick-terminal-autohide = true\n\
         quick-terminal-space-behavior = move\n\
         quick-terminal-animation-duration = 0.08\n\
         confirm-close-surface = false\n\
         # An ordinary open request can briefly create a routing surface here.\n\
         # Its successful, immediate exit is intentional and must close cleanly.\n\
         abnormal-command-exit-runtime = 0\n\
         env = PATH={path}\n\
         working-directory = {directory}\n\
         keybind = global:{chord}=toggle_quick_terminal\n\
         keybind = unconsumed:escape=toggle_quick_terminal\n\
         # fzf cannot receive Command. Translate it to Prelude's private Ctrl+G\n\
         # inside this dedicated panel; the person's normal Ghostty is untouched.\n\
         keybind = cmd+enter=text:\\x07\n\
         command = {exe} _surface\n",
        chord = hotkey.canonical(),
        exe = exe.display(),
        directory = directory.display(),
        path = path,
    )
}

fn write_quick_config(destination: &Path) -> Result<(), String> {
    let config = configured()?;
    let exe = std::env::current_exe()
        .map_err(|e| format!("could not find the Prelude binary: {e}"))?;
    let body = quick_config(&exe, &config.hotkey, &effective_directory(&config));
    crate::cache::write_atomic(destination, body.as_bytes())
        .map_err(|e| format!("could not write {}: {e}", destination.display()))?;
    let path = destination.to_string_lossy().into_owned();
    let ghostty = ghostty_executable()
        .ok_or_else(|| "Ghostty is not installed in /Applications or ~/Applications".to_string())?;
    run_visible(
        &ghostty.to_string_lossy(),
        &["+validate-config", &format!("--config-file={path}")],
    )
    .map_err(|e| format!("Ghostty rejected the generated launcher configuration: {e}"))?;
    Ok(())
}

fn rollback_install(destination: &Path, backup: &Path, agent: &Path, old_agent: Option<&[u8]>) {
    stop_helper();
    let _ = std::fs::remove_file(destination);
    if backup.exists() {
        let _ = std::fs::rename(backup, destination);
    }
    match old_agent {
        Some(bytes) => {
            let _ = crate::cache::write_atomic(agent, bytes);
            let _ = private_file(agent);
            let _ = start_helper();
        }
        None => {
            let _ = std::fs::remove_file(agent);
        }
    }
}

fn domain() -> String {
    format!("gui/{}", unsafe { libc_getuid() })
}

#[cfg(unix)]
unsafe fn libc_getuid() -> u32 {
    unsafe extern "C" {
        safe fn getuid() -> u32;
    }
    getuid()
}

#[cfg(not(unix))]
unsafe fn libc_getuid() -> u32 {
    0
}


fn launchctl(args: &[&str]) -> bool {
    Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}



/// The dedicated Ghostty pids. One is healthy; two means two event taps
/// fighting over the same chord.
fn instance_pids() -> Vec<u32> {
    let marker = format!("config-file={}", quick_config_path().display());
    crate::exec::run(&["/usr/bin/pgrep", "-f", &marker], Duration::from_secs(2))
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect()
}

fn instances() -> usize {
    instance_pids().len()
}

fn running() -> bool {
    instances() > 0
}

fn service_target() -> String {
    format!("{}/{}", domain(), LABEL)
}

fn service_loaded() -> bool {
    launchctl(&["print", &service_target()])
}

/// Stop both launchd supervision and any process left from an older installer.
/// Booting the job out first is load-bearing now that KeepAlive is true: a
/// plain kill would immediately create the process we were trying to remove.
fn stop_helper() {
    let target = service_target();
    let agent = launch_agent_path().to_string_lossy().into_owned();
    let domain = domain();
    let _ = launchctl(&["bootout", &target]);
    let _ = launchctl(&["bootout", &domain, &agent]);

    let marker = format!("config-file={}", quick_config_path().display());
    let _ = crate::exec::run(&["/usr/bin/pkill", "-f", &marker], Duration::from_secs(2));
    for _ in 0..20 {
        if !running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// One supervised panel, or none. Anything else is a chord with two owners.
fn enforce_single_instance() -> Result<(), String> {
    if instances() <= 1 && service_loaded() {
        return Ok(());
    }
    stop_helper();
    start_helper()
}

fn wait_until_running() -> bool {
    for _ in 0..40 {
        if instances() == 1 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventTapState {
    Ready,
    PermissionDenied,
    Pending,
}

fn event_tap_state_from_log(log: &str) -> EventTapState {
    let ready = log.rfind("global event tap enabled for global keybinds");
    let denied = log
        .rfind("creating global event tap failed, missing permissions?")
        .into_iter()
        .chain(log.rfind("invalidating event tap mach port"))
        .max();
    match (ready, denied) {
        (Some(ready), Some(denied)) if ready > denied => EventTapState::Ready,
        (Some(_), Some(_)) | (None, Some(_)) => EventTapState::PermissionDenied,
        (Some(_), None) => EventTapState::Ready,
        (None, None) => EventTapState::Pending,
    }
}

/// Ghostty's process existing is not proof that a global key works. Global
/// keybinds use an accessibility event tap, and Ghostty reports the actual
/// registration result to the unified log. This is explicit setup/status work,
/// never part of gather.
fn event_tap_state() -> EventTapState {
    let Some(pid) = instance_pids().into_iter().next() else {
        return EventTapState::Pending;
    };
    let cached = std::fs::read_to_string(event_tap_path())
        .ok()
        .and_then(|line| {
            let (saved_pid, state) = line.trim().split_once(' ')?;
            (saved_pid.parse::<u32>().ok()? == pid).then_some(match state {
                "ready" => EventTapState::Ready,
                "denied" => EventTapState::PermissionDenied,
                _ => EventTapState::Pending,
            })
        });
    let predicate = format!(
        "processID == {pid} AND subsystem == \"com.mitchellh.ghostty\" AND category == \"GlobalEventTap\""
    );
    let log = crate::exec::run(
        &[
            "/usr/bin/log",
            "show",
            "--info",
            "--last",
            "7d",
            "--style",
            "compact",
            "--predicate",
            &predicate,
        ],
        Duration::from_secs(4),
    );
    let state = event_tap_state_from_log(&log);
    if state == EventTapState::Pending {
        // Unified logs eventually age out. A record is valid only for this
        // exact live pid; a launchd restart forces a fresh observation.
        return cached.unwrap_or(EventTapState::Pending);
    }
    let word = if state == EventTapState::Ready {
        "ready"
    } else {
        "denied"
    };
    let path = event_tap_path();
    if crate::cache::write_atomic(&path, format!("{pid} {word}\n").as_bytes()).is_ok() {
        let _ = private_file(&path);
    }
    state
}

fn wait_for_event_tap() -> EventTapState {
    let started = Instant::now();
    loop {
        let state = event_tap_state();
        if state != EventTapState::Pending || started.elapsed() >= Duration::from_secs(8) {
            return state;
        }
        std::thread::sleep(Duration::from_millis(750));
    }
}

/// macOS does not let an installer grant Accessibility on a person's behalf.
/// Make the one unavoidable click part of setup, then restart and verify the
/// *actual* event tap instead of printing success because a process exists.
fn ensure_global_key_ready() -> Result<(), String> {
    if wait_for_event_tap() == EventTapState::Ready {
        return Ok(());
    }
    let instruction = "enable Ghostty in System Settings → Privacy & Security → Accessibility";
    if !std::io::stdin().is_terminal() {
        return Err(format!(
            "the panel is installed, but macOS has not enabled its global shortcut; {instruction}, then run `prelude global start`"
        ));
    }

    let _ = run_visible(
        "/usr/bin/open",
        &["x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"],
    );
    eprintln!("\nOne macOS permission is needed for the global shortcut.");
    eprintln!("  1. Turn on Ghostty in Accessibility.");
    eprintln!("  2. Return here and press Enter.\n");
    eprint!("Press Enter after Ghostty is enabled… ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| format!("could not read the permission confirmation: {e}"))?;

    stop_helper();
    start_helper()?;
    if wait_for_event_tap() == EventTapState::Ready {
        Ok(())
    } else {
        Err(format!(
            "Ghostty still cannot register the global shortcut; {instruction}, then run `prelude global start`"
        ))
    }
}

fn start_current() -> Result<(), String> {
    if quick_config_path().is_file() {
        write_quick_config(&quick_config_path())?;
    }
    refresh_launch_agent()?;
    start_helper()?;
    ensure_global_key_ready()
}

fn start_helper() -> Result<(), String> {
    if !quick_config_path().is_file() {
        return Err("the launcher panel is not installed; run `prelude global install`".into());
    }
    if !launch_agent_path().is_file() {
        return Err("the launcher login agent is missing; run `prelude global install`".into());
    }
    let config = configured()?;
    if let Some(owner) = known_conflict(&config.hotkey) {
        return Err(format!(
            "{} is already configured for {owner}; change it there or choose another Prelude chord",
            config.hotkey.canonical()
        ));
    }

    match (instances(), service_loaded()) {
        (1, true) => return Ok(()),
        // A manually launched or duplicated panel is not supervised. Replace
        // it once rather than leaving the shortcut dead after its next exit.
        (0, true) => {
            let target = service_target();
            let _ = launchctl(&["kickstart", "-k", &target]);
        }
        _ => {
            stop_helper();
            let domain = domain();
            let agent = launch_agent_path().to_string_lossy().into_owned();
            if !launchctl(&["bootstrap", &domain, &agent]) && !service_loaded() {
                return Err(format!(
                    "could not load the launcher login agent; inspect {}",
                    stderr_path().display()
                ));
            }
        }
    }

    if wait_until_running() {
        Ok(())
    } else {
        Err(format!(
            "the launcher panel did not stay up; inspect {}",
            stderr_path().display()
        ))
    }
}

fn install() -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("the launcher panel is available on macOS only".into());
    }
    let app = ghostty_app().ok_or_else(|| {
        "the launcher panel needs Ghostty in /Applications or ~/Applications".to_string()
    })?;

    let mut config = configured()?;
    if let Some(owner) = known_conflict(&config.hotkey) {
        // Builds before 0.6 wrote Cmd+Space as their default even though a
        // stock Mac gives it to Spotlight. Migrate only that known-broken
        // default; an explicitly chosen chord is the user's to resolve.
        let fallback = Hotkey::parse(DEFAULT_HOTKEY).expect("default hotkey");
        if config.hotkey.canonical() == LEGACY_DEFAULT_HOTKEY
            && known_conflict(&fallback).is_none()
        {
            config.hotkey = fallback;
        } else {
            return Err(format!(
                "{} is already configured for {}; change it there or choose another chord with `prelude global hotkey HOTKEY`",
                config.hotkey.canonical(), owner
            ));
        }
    }
    write_config(&config)?;
    std::fs::create_dir_all(crate::paths::cache())
        .map_err(|e| format!("could not create the Prelude cache: {e}"))?;
    for log in [stdout_path(), stderr_path()] {
        if !log.exists() {
            std::fs::write(&log, [])
                .map_err(|e| format!("could not create {}: {e}", log.display()))?;
        }
        private_file(&log)?;
    }

    let destination = quick_config_path();
    let staged = destination.with_file_name(format!(".{QUICK_CONFIG}.new-{}", std::process::id()));
    let _ = std::fs::remove_file(&staged);
    if let Err(e) = write_quick_config(&staged) {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }

    let agent = launch_agent_path();
    let old_agent = std::fs::read(&agent).ok();
    let backup = destination.with_file_name(format!(".{QUICK_CONFIG}.backup-{}", std::process::id()));
    let _ = std::fs::remove_file(&backup);

    stop_helper();
    if destination.exists() {
        std::fs::rename(&destination, &backup)
            .map_err(|e| format!("could not preserve the installed configuration: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&staged, &destination) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &destination);
        }
        let _ = start_helper();
        return Err(format!(
            "could not install the launcher configuration: {e}; the previous one was restored"
        ));
    }

    let new_agent = launch_agent(&app, &destination, &stdout_path(), &stderr_path());
    if let Err(e) = crate::cache::write_atomic(&agent, new_agent.as_bytes()) {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(format!("could not write {}: {e}", agent.display()));
    }
    if let Err(e) = private_file(&agent) {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(e);
    }
    let agent_s = agent.to_string_lossy().into_owned();
    if let Err(e) = run_visible("/usr/bin/plutil", &["-lint", &agent_s]) {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(e);
    }
    // A waiting `open` process is the LaunchAgent. It exits with the exact
    // Ghostty application instance, so KeepAlive repairs a later crash.
    if let Err(e) = start_helper() {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(format!("{e}; the previous launcher was restored"));
    }
    if let Err(e) = enforce_single_instance() {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(format!("{e}; the previous launcher was restored"));
    }
    let _ = std::fs::remove_file(&backup);

    // A running process without its accessibility event tap is the main cause
    // of an installation that looks successful while Cmd+Shift+Space does
    // nothing. Resolve the permission now and verify Ghostty's own result.
    ensure_global_key_ready()?;

    let mut result = format!(
        "installed {}\nhotkey: {}\npanel stands in: {}\n",
        destination.display(),
        config.hotkey.canonical(),
        effective_directory(&config).display(),
    );
    result.push_str(
        "The launcher is a hidden Ghostty panel. Press the chord to reveal it, Escape to dismiss.\n",
    );
    if !zsh_widget_available() {
        result.push_str(
            "Optional Ctrl+R integration: add `eval \"$(prelude init zsh)\"` to ~/.zshrc. The global panel is already ready.",
        );
    }
    Ok(result)
}

fn uninstall(reset: bool) -> Result<String, String> {
    stop_helper();
    let agent = launch_agent_path();
    if agent.exists() {
        let _ = launchctl(&["bootout", &domain(), &agent.to_string_lossy()]);
        std::fs::remove_file(&agent)
            .map_err(|e| format!("could not remove {}: {e}", agent.display()))?;
    }
    let quick = quick_config_path();
    if quick.exists() {
        std::fs::remove_file(&quick)
            .map_err(|e| format!("could not remove {}: {e}", quick.display()))?;
    }
    if reset {
        for path in [config_path(), stdout_path(), stderr_path(), event_tap_path()] {
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("could not remove {}: {e}", path.display()))?;
            }
        }
    }
    Ok(if reset {
        "removed the launcher panel and its Prelude-owned preferences".into()
    } else {
        "removed the launcher panel; preferences retained".into()
    })
}


fn zsh_widget_available() -> bool {
    let out = crate::exec::run(
        &["/bin/zsh", "-ilc", "whence -w _prelude_widget 2>/dev/null"],
        Duration::from_secs(3),
    );
    out.lines()
        .any(|line| line.trim() == "_prelude_widget: function")
}

const COCOA_SHIFT: u32 = 1 << 17;
const COCOA_CONTROL: u32 = 1 << 18;
const COCOA_OPTION: u32 = 1 << 19;
const COCOA_COMMAND: u32 = 1 << 20;
const COCOA_MODIFIERS: u32 = COCOA_SHIFT | COCOA_CONTROL | COCOA_OPTION | COCOA_COMMAND;

/// macOS writes a symbolic hotkey into its table only once it stops matching
/// the default, so an id that is absent is at its default and still live —
/// reading the table alone under-reports, and reading only Spotlight's id
/// under-reported further. These are the defaults a launcher chord actually
/// collides with. A recorded entry always wins over the default for its own id,
/// including when what it records is that the person turned it off.
const SYSTEM_DEFAULTS: &[(&str, u32, u32)] = &[
    ("60", 49, COCOA_CONTROL),
    ("61", 49, COCOA_CONTROL | COCOA_OPTION),
    ("64", 49, COCOA_COMMAND),
    ("65", 49, COCOA_COMMAND | COCOA_OPTION),
];

/// What macOS's Keyboard settings call the shortcut. Only the handful worth
/// naming are named; the rest are reported by id rather than guessed at.
fn symbolic_name(id: &str) -> String {
    match id {
        "60" => "the macOS previous-input-source shortcut".into(),
        "61" => "the macOS next-input-source shortcut".into(),
        "64" => "Spotlight".into(),
        "65" => "the Spotlight file search window".into(),
        other => format!("a macOS keyboard shortcut (id {other})"),
    }
}

fn system_owns(hotkey: &Hotkey, recorded: Option<&serde_json::Value>) -> Option<String> {
    let mut seen: Vec<&str> = Vec::new();
    let entries = recorded
        .and_then(|v| v.get("AppleSymbolicHotKeys"))
        .and_then(serde_json::Value::as_object);
    for (id, entry) in entries.into_iter().flatten() {
        seen.push(id);
        if !entry
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
        {
            continue;
        }
        let Some(parameters) = entry
            .get("value")
            .and_then(|value| value.get("parameters"))
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        let field = |i: usize| {
            parameters
                .get(i)
                .and_then(serde_json::Value::as_i64)
                .and_then(|v| u32::try_from(v).ok())
        };
        if field(1) == Some(hotkey.key_code)
            && field(2).is_some_and(|mask| mask & COCOA_MODIFIERS == hotkey.cocoa_mask())
        {
            return Some(symbolic_name(id));
        }
    }
    SYSTEM_DEFAULTS
        .iter()
        .find(|(id, code, mask)| {
            !seen.contains(id) && *code == hotkey.key_code && *mask == hotkey.cocoa_mask()
        })
        .map(|(id, ..)| symbolic_name(id))
}

fn symbolic_hotkeys() -> Option<serde_json::Value> {
    let plist = crate::paths::home().join("Library/Preferences/com.apple.symbolichotkeys.plist");
    let plist = plist.to_string_lossy();
    let out = crate::exec::run(
        &["/usr/bin/plutil", "-convert", "json", "-o", "-", &plist],
        Duration::from_secs(2),
    );
    serde_json::from_str(&out).ok()
}

fn raycast_owns(hotkey: &Hotkey) -> Option<bool> {
    let plist = crate::paths::home().join("Library/Preferences/com.raycast.macos.plist");
    if !plist.exists() {
        return Some(false);
    }
    let plist = plist.to_string_lossy();
    let out = crate::exec::run(
        &[
            "/usr/bin/plutil",
            "-extract",
            "raycastGlobalHotkey",
            "raw",
            &plist,
        ],
        Duration::from_secs(2),
    );
    if out.trim().is_empty() {
        None
    } else {
        Some(hotkey.matches_raycast(out.trim()))
    }
}

/// Who holds the chord, and whether every source could be consulted. macOS
/// exposes no registry of the applications that merely watch a key, so a
/// complete check still only means no *known* owner — the Carbon reservation
/// in the helper is the generic backstop.
struct OwnerCheck {
    owner: Option<String>,
    complete: bool,
}

fn hotkey_owner(hotkey: &Hotkey) -> OwnerCheck {
    let recorded = symbolic_hotkeys();
    let raycast = raycast_owns(hotkey);
    OwnerCheck {
        owner: system_owns(hotkey, recorded.as_ref())
            .or_else(|| (raycast == Some(true)).then(|| "Raycast".to_string())),
        complete: recorded.is_some() && raycast.is_some(),
    }
}

fn known_conflict(hotkey: &Hotkey) -> Option<String> {
    hotkey_owner(hotkey).owner
}








pub fn status() -> GlobalStatus {
    let parsed = configured();
    let selected_hotkey = parsed
        .as_ref()
        .map(|config| config.hotkey.canonical())
        .unwrap_or_else(|e| format!("invalid: {e}"));
    let directory = parsed
        .as_ref()
        .map(effective_directory)
        .unwrap_or_else(|_| crate::paths::home());
    let hotkey = parsed
        .map(|config| config.hotkey)
        .unwrap_or_else(|_| GlobalConfig::default().hotkey);
    let owner = hotkey_owner(&hotkey);
    let panel_running = running();
    let installed = quick_config_path().is_file();
    let tap = if panel_running {
        event_tap_state()
    } else {
        EventTapState::Pending
    };
    let accessibility_granted = match tap {
        EventTapState::Ready => Some(true),
        EventTapState::PermissionDenied => Some(false),
        EventTapState::Pending => None,
    };
    GlobalStatus {
        schema: 6,
        app_installed: installed,
        launch_agent_installed: launch_agent_path().is_file(),
        helper_supervised: service_loaded(),
        helper_running: panel_running,
        accessibility_granted,
        // A process is not enough: Ghostty must report that the event tap
        // which receives global keys is active.
        hotkey_registered: panel_running && tap == EventTapState::Ready && owner.owner.is_none(),
        selected_hotkey,
        hotkey_owner: owner.owner,
        owner_checks_complete: owner.complete,
        launch_directory: directory.to_string_lossy().into_owned(),
        launch_directory_exists: directory.is_dir(),
        ghostty_available: Some(ghostty_app().is_some()),
        zsh_widget_available: zsh_widget_available(),
        last_event: None,
    }
}

fn print_status(json: bool) -> i32 {
    let s = status();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&s).unwrap_or_else(|_| "{}".into())
        );
    } else {
        println!("Prelude launcher panel\n");
        line(
            "panel configuration",
            s.app_installed,
            &quick_config_path().to_string_lossy(),
        );
        line(
            "login agent",
            s.launch_agent_installed && s.helper_supervised,
            if s.helper_supervised {
                "loaded; launchd keeps the panel alive"
            } else {
                "not loaded; run: prelude global start"
            },
        );
        line(
            "panel running",
            s.helper_running,
            if s.helper_running {
                "hidden Ghostty instance up; press the chord to reveal it"
            } else {
                "run: prelude global open"
            },
        );
        line(
            "Ghostty Accessibility",
            s.accessibility_granted == Some(true),
            match s.accessibility_granted {
                Some(true) => "global event tap active",
                Some(false) => "enable Ghostty in System Settings → Privacy & Security → Accessibility",
                None if s.helper_running => "registration result not available yet; run: prelude global start",
                None => "starts with the panel",
            },
        );
        line(
            &format!("{} registered", s.selected_hotkey),
            s.hotkey_registered,
            if s.hotkey_registered {
                "verified ready"
            } else if s.hotkey_owner.is_some() {
                "another shortcut owns this chord"
            } else {
                "run: prelude global start"
            },
        );
        line(
            "zsh widget",
            s.zsh_widget_available,
            if s.zsh_widget_available {
                "Ctrl+R integration loaded"
            } else {
                "optional for Ctrl+R; the global panel does not need it"
            },
        );
        println!(
            "  {} panel stands in  {}{}",
            if s.launch_directory_exists { "✓" } else { "✗" },
            s.launch_directory,
            if s.launch_directory_exists {
                String::new()
            } else {
                format!(" (missing; falls back to {})", crate::paths::home().display())
            }
        );
        println!(
            "  {} Ghostty  {}",
            if s.ghostty_available == Some(true) { "✓" } else { "✗" },
            if s.ghostty_available == Some(true) {
                "installed"
            } else {
                "required: the panel is a Ghostty quick terminal"
            }
        );
        println!(
            "  {} conflicts  {}",
            if s.hotkey_owner.is_some() { "✗" } else { "✓" },
            match (&s.hotkey_owner, s.owner_checks_complete) {
                (Some(owner), _) => format!("{owner} owns this chord"),
                (None, true) => "no known owner".into(),
                (None, false) =>
                    "no known owner, but some owner records could not be read".to_string(),
            }
        );
    }
    if s.app_installed
        && s.launch_agent_installed
        && s.helper_supervised
        && s.helper_running
        && s.accessibility_granted == Some(true)
        && s.hotkey_registered
        && s.ghostty_available == Some(true)
        && s.hotkey_owner.is_none()
    {
        0
    } else {
        1
    }
}

fn line(label: &str, good: bool, detail: &str) {
    println!("  {} {label}  {detail}", if good { "✓" } else { "✗" });
}

/// `prelude global open` starts the panel's instance. It cannot *show* the
/// panel: that is Ghostty's global chord, which is the whole point — nothing
/// of Prelude's runs when the key is pressed.
fn open_once() -> Result<(), String> {
    if running() {
        ensure_global_key_ready()?;
        println!("the launcher panel is ready; press the chord to reveal it");
        return Ok(());
    }
    start_current()?;
    println!("launcher panel started and verified; press the chord to reveal it");
    Ok(())
}

pub fn dispatch(args: &[&str]) -> i32 {
    let result = match args {
        ["install"] => install().map(|s| println!("{s}")),
        ["uninstall"] => uninstall(false).map(|s| println!("{s}")),
        ["uninstall", "--reset"] => uninstall(true).map(|s| println!("{s}")),
        ["start"] => start_current().map(|_| println!("Prelude global hotkey started")),
        ["stop"] => {
            stop_helper();
            println!("Prelude global hotkey stopped");
            Ok(())
        }
        ["status"] => return print_status(false),
        ["status", "--json"] => return print_status(true),
        ["open"] => open_once(),
        ["hotkey"] => configured().map(|config| println!("{}", config.hotkey.canonical())),
        ["hotkey", value] => Hotkey::parse(value).and_then(write_hotkey).map(|message| println!("{message}")),
        ["directory"] => {
            configured().map(|config| println!("{}", effective_directory(&config).display()))
        }
        ["directory", "--default"] => clear_directory().map(|message| println!("{message}")),
        ["directory", value] => write_directory(value).map(|message| println!("{message}")),
        // `backend` chose which terminal a handed-over command opened in.
        // Nothing opens now, so it is gone rather than accepted and ignored.
        ["backend", ..] => Err(
            "there is no terminal backend any more: the panel copies what it hands over, \
             and opens nothing"
                .into(),
        ),
        _ => Err(
            "usage: prelude global install|uninstall|start|stop|status|open|hotkey [CHORD]|directory [PATH|--default]"
                .into(),
        ),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("prelude: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "prelude-global-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
    }

    #[test]
    fn a_fresh_install_uses_a_chord_a_stock_mac_does_not_own() {
        assert_eq!(GlobalConfig::default().hotkey.canonical(), "cmd+shift+space");
        let path = temp("fresh-default");
        let config = config_from(&path).unwrap();
        assert_eq!(config.hotkey.canonical(), "cmd+shift+space");
    }

    #[test]
    fn config_canonicalizes_hotkeys_and_ignores_a_retired_backend() {
        let path = temp("hotkey");
        std::fs::write(
            &path,
            "backend = \"ghostty\"\nhotkey = \"Shift+Command+K\"\n",
        )
        .unwrap();
        // A `backend` left by an older build must not make the config
        // unreadable, and must not be honoured either: nothing opens a
        // terminal any more, so there is nothing for it to select.
        let config = config_from(&path).unwrap();
        assert_eq!(config.hotkey.canonical(), "cmd+shift+k");
        assert!(!config_body(&config).contains("backend"));
        for bad in ["space", "cmd", "cmd+f1", "cmd+space+q", "cmd+cmd+q"] {
            std::fs::write(&path, format!("hotkey = \"{bad}\"\n")).unwrap();
            assert!(config_from(&path).is_err(), "{bad}");
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_panel_directory_is_optional_absolute_and_safe() {
        let home = crate::paths::home();
        assert_eq!(parse_directory("~/").unwrap(), home);
        assert_eq!(parse_directory("/tmp").unwrap(), PathBuf::from("/tmp"));
        for bad in ["relative/path", "", "~notauser/x"] {
            assert!(parse_directory(bad).is_err(), "{bad}");
        }
        // The directory is interpolated into a generated Ghostty config, so
        // one rule refuses every character that would need escaping there.
        for bad in ["/tmp/a\"b", "/tmp/a'b", "/tmp/a`b", "/tmp/a$b", "/tmp/a\nb"] {
            assert!(parse_directory(bad).is_err(), "{bad}");
        }

        // Unset means $HOME, and stays out of the file so no personal path is
        // written until somebody asks for one.
        let path = temp("directory");
        std::fs::write(&path, "hotkey = \"cmd+space\"\n").unwrap();
        let config = config_from(&path).unwrap();
        assert_eq!(config.directory, None);
        assert_eq!(effective_directory(&config), home);

        std::fs::write(&path, "hotkey = \"cmd+space\"\ndirectory = \"/tmp\"\n").unwrap();
        let config = config_from(&path).unwrap();
        assert_eq!(effective_directory(&config), PathBuf::from("/tmp"));
        assert!(!config_body(&config).contains("directory = \"\""));
        assert!(config_body(&config).contains("directory = \"/tmp\""));

        std::fs::write(&path, "directory = \"nope\"\n").unwrap();
        assert!(config_from(&path).unwrap_err().contains("absolute"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn macos_owns_a_chord_it_never_recorded() {
        let recorded: serde_json::Value = serde_json::from_str(
            r#"{"AppleSymbolicHotKeys":{
                 "60":{"enabled":true,"value":{"parameters":[32,49,262144]}},
                 "65":{"enabled":false,"value":{"parameters":[32,49,1572864]}},
                 "222":{"enabled":true,"value":{"parameters":[32,40,1179648]}}}}"#,
        )
        .unwrap();
        let owner = |chord: &str| system_owns(&Hotkey::parse(chord).unwrap(), Some(&recorded));

        // Recorded and enabled.
        assert_eq!(
            owner("ctrl+space").as_deref(),
            Some("the macOS previous-input-source shortcut")
        );
        // Recorded as switched off, so it owns nothing — the record wins over
        // the default for its own id.
        assert_eq!(owner("cmd+option+space"), None);
        // Absent from the table is the ordinary case for an untouched Mac, and
        // means the default is still live. Asking only whether Spotlight's
        // entry was enabled answered "no" here, which is how a chord Spotlight
        // owns was reported as free.
        assert_eq!(owner("cmd+space").as_deref(), Some("Spotlight"));
        // Something the person bound themselves: reported, not named.
        assert_eq!(
            owner("cmd+shift+k").as_deref(),
            Some("a macOS keyboard shortcut (id 222)")
        );
        // The chord this launcher actually wants.
        assert_eq!(owner("cmd+shift+space"), None);
        // No table at all still applies the known defaults.
        assert_eq!(
            system_owns(&Hotkey::parse("cmd+space").unwrap(), None).as_deref(),
            Some("Spotlight")
        );
    }


    #[test]
    fn raycast_hotkey_encoding_is_compared_as_a_chord_not_as_prose() {
        let cmd_space = Hotkey::parse("cmd+space").unwrap();
        assert!(cmd_space.matches_raycast("Command-49"));
        assert!(!cmd_space.matches_raycast("Command-Shift-49"));
        assert!(!Hotkey::parse("option+space")
            .unwrap()
            .matches_raycast("Command-49"));
        assert!(Hotkey::parse("ctrl+shift+k")
            .unwrap()
            .matches_raycast("Control-Shift-40"));
    }

    #[test]
    fn managed_paths_stay_inside_their_declared_user_roots() {
        assert!(quick_config_path().starts_with(crate::paths::config()));
        assert_eq!(
            launch_agent_path(),
            crate::paths::home().join("Library/LaunchAgents/app.prelude.hotkey.plist")
        );
        assert!(config_path().starts_with(crate::paths::config()));
        for path in [stdout_path(), stderr_path(), event_tap_path()] {
            assert!(path.starts_with(crate::paths::cache()));
        }
    }

    #[test]
    fn generated_metadata_escapes_paths_and_supervises_the_supported_launch() {
        let agent = launch_agent(
            Path::new("/Users/me/Applications/Ghostty.app"),
            Path::new("/tmp/a&b/panel.ghostty"),
            Path::new("/tmp/out"),
            Path::new("/tmp/err"),
        );
        assert!(agent.contains("/tmp/a&amp;b/panel.ghostty"));
        assert!(agent.contains("/Users/me/Applications/Ghostty.app"));
        assert!(agent.contains("<key>RunAtLoad</key><true/>"));
        // Ghostty requires Launch Services on macOS. `-W` makes `open` wait
        // for this new (`-n`) application instance instead of leaving launchd
        // with a successful, already-exited launcher to supervise.
        assert!(agent
            .contains("<string>/usr/bin/open</string><string>-W</string><string>-n</string>"));
        assert!(agent.contains("<string>--args</string>"));
        assert!(agent.contains("<key>KeepAlive</key><true/>"));
    }

    #[test]
    fn event_tap_logs_are_the_registration_authority() {
        assert_eq!(
            event_tap_state_from_log("global event tap enabled for global keybinds"),
            EventTapState::Ready
        );
        assert_eq!(
            event_tap_state_from_log("creating global event tap failed, missing permissions?"),
            EventTapState::PermissionDenied
        );
        assert_eq!(
            event_tap_state_from_log(
                "global event tap enabled for global keybinds\ninvalidating event tap mach port"
            ),
            EventTapState::PermissionDenied
        );
        assert_eq!(
            event_tap_state_from_log(
                "invalidating event tap mach port\nglobal event tap enabled for global keybinds"
            ),
            EventTapState::Ready
        );
        assert_eq!(event_tap_state_from_log("Ghostty started"), EventTapState::Pending);
    }


    #[test]
    fn only_ghosttys_quick_terminal_marker_routes_to_prelude() {
        assert!(is_quick_terminal(Some(OsStr::new("1"))));
        assert!(!is_quick_terminal(None));
        assert!(!is_quick_terminal(Some(OsStr::new("0"))));
        assert!(!is_quick_terminal(Some(OsStr::new("true"))));
    }

    #[test]
    fn the_panel_is_hidden_warm_and_dismissible() {
        let config = quick_config(
            Path::new("/opt/homebrew/bin/prelude"),
            &Hotkey::parse("cmd+shift+space").unwrap(),
            Path::new("/Users/someone"),
        );
        // Out of the Dock and the app switcher: a launcher is not an app you
        // alt-tab to.
        assert!(config.contains("macos-hidden = always"));
        // At rest it owns no window and runs no shell.
        assert!(config.contains("initial-window = false"));
        // Saved state would restore the previous session's windows, so one
        // press would arrive with a crowd. This is the third time that trap
        // has been paid for.
        assert!(config.contains("window-save-state = never"));
        // Ghostty registers the chord itself; nothing of Prelude's runs when
        // the key is pressed.
        assert!(config.contains("keybind = global:cmd+shift+space=toggle_quick_terminal"));
        // Escape hides the panel *and* reaches fzf, so the launcher resets
        // behind a hidden panel and the next press is a reveal, not a rebuild.
        assert!(config.contains("keybind = unconsumed:escape=toggle_quick_terminal"));
        assert!(config.contains(r"keybind = cmd+enter=text:\x07"));
        // Every surface enters through the marker gate. Quick terminals run
        // Prelude; ordinary windows are redirected to a normal Ghostty app.
        assert!(config.contains("command = /opt/homebrew/bin/prelude _surface"));
        assert!(!config.contains("command = /opt/homebrew/bin/prelude _panel"));
        assert!(config.contains("abnormal-command-exit-runtime = 0"));
        assert!(config.contains("env = PATH="));
        assert!(config.contains("quick-terminal-position = center"));
        assert!(config.contains("quick-terminal-autohide = true"));
    }

    #[test]
    fn the_panel_configuration_carries_no_selected_payload() {
        let config = quick_config(
            Path::new("/opt/homebrew/bin/prelude"),
            &Hotkey::parse("cmd+space").unwrap(),
            Path::new("/Users/someone"),
        );
        for verb in ["INSERT", "RUN", "MSG"] {
            assert!(!config.contains(verb), "{verb}");
        }
    }
}
