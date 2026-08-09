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
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const GHOSTTY_BUNDLE: &str = "com.mitchellh.ghostty";
const GHOSTTY_APP: &str = "/Applications/Ghostty.app";
const LABEL: &str = "app.prelude.hotkey";
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
            hotkey: Hotkey::parse("cmd+space").expect("default hotkey"),
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
    pub helper_running: bool,
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
            .unwrap_or("cmd+space"),
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
}

pub fn configured_summary() -> GlobalSummary {
    let config = configured().unwrap_or_default();
    GlobalSummary {
        hotkey: config.hotkey.canonical(),
        directory: effective_directory(&config).to_string_lossy().into_owned(),
    }
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
    start_helper()
}

/// Stop and restart the instance, which is what a rebuilt binary needs.
///
/// The loop was started with whatever the binary held then and executes it
/// until it exits. Each press does spawn the new binary as its child, so the
/// rows and the footer update and it looks like the build took — while the
/// half that decides what to do with an answer is still the old one.
pub fn restart_panel() -> Result<String, String> {
    stop_helper();
    start_helper()?;
    Ok("panel restarted; it now runs the binary on disk".into())
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
    if let Err(e) = start_helper() {
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


/// Start the panel's instance at login. `open` returns as soon as Launch
/// Services has the request, so this job is expected to exit cleanly; there is
/// nothing here to keep alive.
fn launch_agent(config: &Path, stdout: &Path, stderr: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>/usr/bin/open</string><string>-nb</string><string>{GHOSTTY_BUNDLE}</string><string>--args</string><string>--config-file={config}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
  <key>ProcessType</key><string>Interactive</string>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>{stdout}</string>
  <key>StandardErrorPath</key><string>{stderr}</string>
</dict></plist>
"#,
        config = xml(&config.to_string_lossy()),
        stdout = xml(&stdout.to_string_lossy()),
        stderr = xml(&stderr.to_string_lossy()),
    )
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
         working-directory = {directory}\n\
         keybind = global:{chord}=toggle_quick_terminal\n\
         keybind = unconsumed:escape=toggle_quick_terminal\n\
         command = {exe} _panel\n",
        chord = hotkey.canonical(),
        exe = exe.display(),
        directory = directory.display(),
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
    run_visible(
        "/Applications/Ghostty.app/Contents/MacOS/ghostty",
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



/// How many launcher instances are up. launchd only starts one at login —
/// `open` exits immediately, so the job is never the thing to ask — but two
/// can appear if something else starts one at the same moment, and two
/// instances mean two panels fighting over one chord.
fn instances() -> usize {
    let marker = format!("config-file={}", quick_config_path().display());
    crate::exec::run(&["/usr/bin/pgrep", "-f", &marker], Duration::from_secs(2))
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

fn running() -> bool {
    instances() > 0
}

fn stop_helper() {
    let marker = format!("config-file={}", quick_config_path().display());
    let _ = crate::exec::run(&["/usr/bin/pkill", "-f", &marker], Duration::from_secs(2));
    for _ in 0..20 {
        if !running() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// One panel, or none. Anything else is a chord with two owners.
fn enforce_single_instance() -> Result<(), String> {
    if instances() <= 1 {
        return Ok(());
    }
    stop_helper();
    start_helper()
}

fn wait_until_running() -> bool {
    for _ in 0..40 {
        if running() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn start_helper() -> Result<(), String> {
    if !quick_config_path().is_file() {
        return Err("the launcher panel is not installed; run `prelude global install`".into());
    }
    let config = configured()?;
    if let Some(owner) = known_conflict(&config.hotkey) {
        return Err(format!(
            "{} is already configured for {owner}; change it there or choose another Prelude chord",
            config.hotkey.canonical()
        ));
    }
    match instances() {
        1 => return Ok(()),
        // Two panels both claim the chord, and the one that loses registration
        // still answers a toggle, so the panel appears to open every other
        // press. Reduce to one rather than adding a third.
        n if n > 1 => stop_helper(),
        _ => {}
    }
    let path = format!("--config-file={}", quick_config_path().display());
    let started = Command::new("/usr/bin/open")
        .args(["-nb", GHOSTTY_BUNDLE, "--args", &path])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !started {
        return Err("could not start the launcher panel's Ghostty instance".into());
    }
    if wait_until_running() {
        Ok(())
    } else {
        Err("the launcher panel's Ghostty instance did not stay up; run `prelude global status`".into())
    }
}

fn install() -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("the launcher panel is available on macOS only".into());
    }
    if !Path::new(GHOSTTY_APP).is_dir() {
        return Err(
            "the launcher panel is a Ghostty quick terminal, and Ghostty is not installed"
                .into(),
        );
    }
    let config = configured()?;
    if let Some(owner) = known_conflict(&config.hotkey) {
        return Err(format!(
            "{} is already configured for {}; change it there or choose another chord with `prelude global hotkey HOTKEY`",
            config.hotkey.canonical(), owner
        ));
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

    let new_agent = launch_agent(&destination, &stdout_path(), &stderr_path());
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
    let _ = launchctl(&["bootout", &domain(), &agent_s]);
    // RunAtLoad starts the instance, so bootstrapping *is* the start. Racing
    // it with an explicit launch is how two panels end up sharing one chord.
    let _ = launchctl(&["bootstrap", &domain(), &agent_s]);
    if !wait_until_running() {
        if let Err(e) = start_helper() {
            rollback_install(&destination, &backup, &agent, old_agent.as_deref());
            return Err(format!("{e}; the previous launcher was restored"));
        }
    }
    if let Err(e) = enforce_single_instance() {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(format!("{e}; the previous launcher was restored"));
    }
    let _ = std::fs::remove_file(&backup);

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
            "The login zsh does not load _prelude_widget yet. Add `eval \"$(prelude init zsh)\"` to ~/.zshrc so handed-over commands land on a prompt.",
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
        for path in [config_path(), stdout_path(), stderr_path()] {
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
    GlobalStatus {
        schema: 5,
        app_installed: installed,
        launch_agent_installed: launch_agent_path().is_file(),
        helper_running: panel_running,
        // Ghostty registers the chord from the panel's configuration, so the
        // panel being up with no known owner is the whole of the claim.
        hotkey_registered: panel_running && owner.owner.is_none(),
        selected_hotkey,
        hotkey_owner: owner.owner,
        owner_checks_complete: owner.complete,
        launch_directory: directory.to_string_lossy().into_owned(),
        launch_directory_exists: directory.is_dir(),
        ghostty_available: Some(Path::new(GHOSTTY_APP).is_dir()),
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
            s.launch_agent_installed,
            &launch_agent_path().to_string_lossy(),
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
            &format!("{} registered", s.selected_hotkey),
            s.hotkey_registered,
            if s.hotkey_registered {
                "Ghostty owns the chord"
            } else {
                "free the shortcut or choose another, then run: prelude global install"
            },
        );
        line(
            "zsh widget",
            s.zsh_widget_available,
            if s.zsh_widget_available {
                "_prelude_widget is loaded; handed-over commands land on a prompt"
            } else {
                "add eval \"$(prelude init zsh)\" to ~/.zshrc"
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
        && s.helper_running
        && s.hotkey_registered
        && s.zsh_widget_available
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
        println!("the launcher panel is already running; press the chord to reveal it");
        return Ok(());
    }
    start_helper()?;
    println!("launcher panel started; press the chord to reveal it");
    Ok(())
}

pub fn dispatch(args: &[&str]) -> i32 {
    let result = match args {
        ["install"] => install().map(|s| println!("{s}")),
        ["uninstall"] => uninstall(false).map(|s| println!("{s}")),
        ["uninstall", "--reset"] => uninstall(true).map(|s| println!("{s}")),
        ["start"] => start_helper().map(|_| println!("Prelude global hotkey started")),
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
        for path in [stdout_path(), stderr_path()] {
            assert!(path.starts_with(crate::paths::cache()));
        }
    }

    #[test]
    fn generated_metadata_escapes_paths_and_contains_no_selected_payload() {
        let agent = launch_agent(
            Path::new("/tmp/a&b/panel.ghostty"),
            Path::new("/tmp/out"),
            Path::new("/tmp/err"),
        );
        assert!(agent.contains("/tmp/a&amp;b/panel.ghostty"));
        assert!(agent.contains("<key>RunAtLoad</key><true/>"));
        // `open` hands the request to Launch Services and exits, so there is
        // nothing here for launchd to keep alive. The panel's own Ghostty
        // instance is the long-lived thing, and it is not a launchd job.
        assert!(agent.contains("/usr/bin/open"));
        assert!(agent.contains("<key>KeepAlive</key><false/>"));
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
        assert!(config.contains("command = /opt/homebrew/bin/prelude _panel"));
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
