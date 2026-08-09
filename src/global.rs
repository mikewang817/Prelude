//! Production macOS global hotkey integration.
//!
//! The latency-sensitive launcher remains a terminal program. This module is
//! reached only through an explicit `prelude global ...` command and manages a
//! tiny AppKit helper which owns Cmd+Space and creates a fresh Ghostty or
//! Terminal.app window. No GUI framework or hotkey dependency is linked into
//! the Prelude binary itself.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const LABEL: &str = "app.prelude.hotkey";
const APP_NAME: &str = "Prelude Hotkey.app";
const EXECUTABLE: &str = "PreludeHotkey";
const SWIFT: &str = include_str!("../macos/PreludeHotkey/main.swift");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Auto,
    Ghostty,
    Terminal,
}

impl Backend {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim() {
            "auto" => Ok(Self::Auto),
            "ghostty" => Ok(Self::Ghostty),
            "terminal" => Ok(Self::Terminal),
            other => Err(format!(
                "unknown terminal backend {other:?}; choose auto, ghostty or terminal"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ghostty => "ghostty",
            Self::Terminal => "terminal",
        }
    }
}

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
    backend: Backend,
    hotkey: Hotkey,
    /// Where a launcher window starts. `None` is `$HOME`, and stays unwritten
    /// so the config carries no personal path until somebody asks for one.
    directory: Option<PathBuf>,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            backend: Backend::Auto,
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
    pub launcher_active: bool,
    /// The shell that owns the open launcher, when one has reported itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launcher_pid: Option<u32>,
    pub selected_hotkey: String,
    /// The macOS shortcut or application known to hold the chord.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotkey_owner: Option<String>,
    /// False when a source could not be read, so an absent owner means none
    /// was found rather than none exists.
    pub owner_checks_complete: bool,
    pub selected_backend: String,
    /// Where a launcher window starts, and whether it is still there.
    pub launch_directory: String,
    pub launch_directory_exists: bool,
    pub ghostty_available: Option<bool>,
    pub effective_backend: Option<String>,
    pub zsh_widget_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event: Option<serde_json::Value>,
}

fn app_path() -> PathBuf {
    crate::paths::home().join("Applications").join(APP_NAME)
}

fn executable_path() -> PathBuf {
    app_path().join("Contents/MacOS").join(EXECUTABLE)
}

fn launch_agent_path() -> PathBuf {
    crate::paths::home()
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

fn config_path() -> PathBuf {
    crate::paths::config().join("global.toml")
}

fn status_path() -> PathBuf {
    crate::paths::cache().join("global-status.json")
}

fn active_path() -> PathBuf {
    crate::paths::cache().join("global-active")
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
    let backend = Backend::parse(
        root.and_then(|table| table.get("backend"))
            .map(String::as_str)
            .unwrap_or("auto"),
    )?;
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
    Ok(GlobalConfig {
        backend,
        hotkey,
        directory,
    })
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
        "# Terminal, chord and starting directory used by the global launcher\n# helper. auto prefers Ghostty and falls back to Terminal.app; an unset\n# directory means $HOME.\nbackend = \"{}\"\nhotkey = \"{}\"\n{directory}",
        config.backend.as_str(),
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

fn write_backend(backend: Backend) -> Result<(), String> {
    if launcher_active() {
        return Err("a global Prelude launcher is open; close it before changing the backend".into());
    }
    let mut config = configured()?;
    config.backend = backend;
    write_config(&config)
}

/// Changing where a window opens needs no helper restart: the helper reads the
/// directory when it launches one, so the next press uses it.
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

fn helper_supports_configurable_hotkey() -> bool {
    helper_probe()
        .and_then(|probe| probe.get("selected_hotkey").cloned())
        .and_then(|value| value.as_str().map(str::to_string))
        .is_some()
}

fn write_hotkey(hotkey: Hotkey) -> Result<String, String> {
    let mut config = configured()?;
    if let Some(owner) = known_conflict(&hotkey) {
        if config.hotkey == hotkey {
            stop_helper();
        }
        return Err(format!(
            "{} is already configured for {owner}; change it there or choose another chord",
            hotkey.canonical()
        ));
    }
    if config.hotkey == hotkey {
        return Ok(format!("global hotkey already uses {}", hotkey.canonical()));
    }
    if launcher_active() {
        return Err(
            "a global Prelude launcher is open; close it before changing the hotkey".into(),
        );
    }

    let old = std::fs::read(config_path()).ok();
    let was_running = running();
    let installed = executable_path().is_file();
    stop_helper();
    config.hotkey = hotkey.clone();
    if let Err(e) = write_config(&config) {
        if was_running {
            let _ = start_helper();
        }
        return Err(e);
    }

    if !installed {
        return Ok(format!(
            "global hotkey: {} (install with `prelude global install`)",
            hotkey.canonical()
        ));
    }
    if !helper_supports_configurable_hotkey() {
        return Ok(format!(
            "global hotkey: {}. The installed helper predates configurable keys; run `prelude global install` to upgrade and start it.",
            hotkey.canonical()
        ));
    }

    let restore = |bytes: Option<&[u8]>| {
        match bytes {
            Some(bytes) => {
                let _ = crate::cache::write_atomic(&config_path(), bytes);
                let _ = private_file(&config_path());
            }
            None => {
                let _ = std::fs::remove_file(config_path());
            }
        }
        if was_running {
            let _ = start_helper();
        }
    };
    if let Err(e) = check_hotkey_with(&executable_path()) {
        restore(old.as_deref());
        return Err(format!(
            "{} is unavailable: {e}; the previous hotkey was restored",
            hotkey.canonical()
        ));
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

fn info_plist(config: &Path, status: &Path, active: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>{EXECUTABLE}</string>
  <key>CFBundleIdentifier</key><string>{LABEL}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Prelude Hotkey</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSAppleEventsUsageDescription</key><string>Prelude opens a new Terminal window when Ghostty is unavailable.</string>
  <key>PreludeConfigPath</key><string>{config}</string>
  <key>PreludeStatusPath</key><string>{status}</string>
  <key>PreludeActivePath</key><string>{active}</string>
</dict></plist>
"#,
        version = env!("CARGO_PKG_VERSION"),
        config = xml(&config.to_string_lossy()),
        status = xml(&status.to_string_lossy()),
        active = xml(&active.to_string_lossy()),
    )
}

fn launch_agent(executable: &Path, stdout: &Path, stderr: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{executable}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>{stdout}</string>
  <key>StandardErrorPath</key><string>{stderr}</string>
</dict></plist>
"#,
        executable = xml(&executable.to_string_lossy()),
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

fn build_app(destination: &Path) -> Result<(), String> {
    let contents = destination.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos)
        .and_then(|_| std::fs::create_dir_all(&resources))
        .map_err(|e| format!("could not create app bundle: {e}"))?;

    let source = resources.join("main.swift");
    std::fs::write(&source, SWIFT).map_err(|e| format!("could not stage helper source: {e}"))?;
    std::fs::write(
        contents.join("Info.plist"),
        info_plist(&config_path(), &status_path(), &active_path()),
    )
    .map_err(|e| format!("could not write helper metadata: {e}"))?;

    let output = macos.join(EXECUTABLE);
    let source_s = source.to_string_lossy().into_owned();
    let output_s = output.to_string_lossy().into_owned();
    run_visible(
        "/usr/bin/xcrun",
        &[
            "swiftc",
            &source_s,
            "-O",
            "-warnings-as-errors",
            "-o",
            &output_s,
            "-framework",
            "AppKit",
            "-framework",
            "Carbon",
        ],
    )?;
    let info = contents.join("Info.plist").to_string_lossy().into_owned();
    run_visible("/usr/bin/plutil", &["-lint", &info])?;
    let destination_s = destination.to_string_lossy().into_owned();
    run_visible(
        "/usr/bin/codesign",
        &["--force", "--sign", "-", "--timestamp=none", &destination_s],
    )?;
    Ok(())
}

fn rollback_install(destination: &Path, backup: &Path, agent: &Path, old_agent: Option<&[u8]>) {
    stop_helper();
    if destination.exists() {
        let _ = std::fs::remove_dir_all(destination);
    }
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

fn service() -> String {
    format!("{}/{LABEL}", domain())
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

fn service_info() -> String {
    crate::exec::run(
        &["/bin/launchctl", "print", &service()],
        Duration::from_secs(2),
    )
}

fn loaded() -> bool {
    !service_info().is_empty()
}

fn running() -> bool {
    let info = service_info();
    info.lines().any(|line| line.trim() == "state = running")
        && info
            .lines()
            .any(|line| line.trim_start().starts_with("pid = "))
}

fn stop_helper() {
    let path = launch_agent_path();
    if path.exists() {
        let _ = launchctl(&["bootout", &domain(), &path.to_string_lossy()]);
    }
}

fn wait_until_running() -> bool {
    for _ in 0..30 {
        if running() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn start_helper() -> Result<(), String> {
    let path = launch_agent_path();
    if !path.exists() {
        return Err("global hotkey is not installed; run `prelude global install`".into());
    }
    let config = configured()?;
    if let Some(owner) = known_conflict(&config.hotkey) {
        stop_helper();
        return Err(format!(
            "{} is already configured for {owner}; change it there or choose another Prelude chord",
            config.hotkey.canonical()
        ));
    }
    if loaded() {
        stop_helper();
    }
    if helper_supports_configurable_hotkey() {
        check_hotkey_with(&executable_path())
            .map_err(|e| format!("{} is unavailable: {e}", config.hotkey.canonical()))?;
    }
    if !launchctl(&["bootstrap", &domain(), &path.to_string_lossy()]) {
        return Err(
            "launchd could not start the Prelude hotkey helper; run `prelude global status`".into(),
        );
    }
    if wait_until_running() {
        Ok(())
    } else {
        Err("the Prelude hotkey helper exited during startup; run `prelude global status`".into())
    }
}

fn install() -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("the global hotkey is available on macOS only".into());
    }
    let config = configured()?;
    if let Some(owner) = known_conflict(&config.hotkey) {
        stop_helper();
        return Err(format!(
            "{} is already configured for {}; change it there or choose another Prelude chord with `prelude global hotkey HOTKEY`",
            config.hotkey.canonical(), owner
        ));
    }
    if launcher_active() {
        return Err(
            "a global Prelude launcher is open; close it before upgrading the helper".into(),
        );
    }
    write_config(&config)?;
    std::fs::create_dir_all(crate::paths::home().join("Applications"))
        .map_err(|e| format!("could not create ~/Applications: {e}"))?;
    std::fs::create_dir_all(crate::paths::cache())
        .map_err(|e| format!("could not create the Prelude cache: {e}"))?;
    for log in [stdout_path(), stderr_path()] {
        if !log.exists() {
            std::fs::write(&log, [])
                .map_err(|e| format!("could not create {}: {e}", log.display()))?;
        }
        private_file(&log)?;
    }

    let destination = app_path();
    let staged = destination.with_file_name(format!(".{APP_NAME}.new-{}", std::process::id()));
    if staged.exists() {
        std::fs::remove_dir_all(&staged).map_err(|e| format!("could not clear staging: {e}"))?;
    }
    if let Err(e) = build_app(&staged) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }

    let agent = launch_agent_path();
    let old_agent = std::fs::read(&agent).ok();
    let backup = destination.with_file_name(format!(".{APP_NAME}.backup-{}", std::process::id()));
    if backup.exists() {
        std::fs::remove_dir_all(&backup)
            .map_err(|e| format!("could not clear old installer backup: {e}"))?;
    }

    let was_running = running();
    stop_helper();
    if let Err(e) = check_hotkey_with(&staged.join("Contents/MacOS").join(EXECUTABLE)) {
        let _ = std::fs::remove_dir_all(&staged);
        if was_running {
            let _ = start_helper();
        }
        return Err(format!(
            "{} is unavailable: {e}. Change the other application or run `prelude global hotkey HOTKEY`",
            config.hotkey.canonical()
        ));
    }
    if destination.exists() {
        std::fs::rename(&destination, &backup)
            .map_err(|e| format!("could not preserve the installed helper: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&staged, &destination) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &destination);
        }
        if old_agent.is_some() {
            let _ = start_helper();
        }
        return Err(format!(
            "could not install the new helper: {e}; the previous helper was restored"
        ));
    }

    let new_agent = launch_agent(&executable_path(), &stdout_path(), &stderr_path());
    if let Err(e) = crate::cache::write_atomic(&agent, new_agent.as_bytes()) {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(format!(
            "could not write {}: {e}; the previous helper was restored",
            agent.display()
        ));
    }
    if let Err(e) = private_file(&agent) {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(format!("{e}; the previous helper was restored"));
    }
    let agent_s = agent.to_string_lossy().into_owned();
    if let Err(e) = run_visible("/usr/bin/plutil", &["-lint", &agent_s]) {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(format!("{e}; the previous helper was restored"));
    }
    let _ = std::fs::remove_file(status_path());
    if let Err(e) = start_helper() {
        rollback_install(&destination, &backup, &agent, old_agent.as_deref());
        return Err(format!("{e}; the previous helper was restored"));
    }
    for _ in 0..20 {
        if status_path().is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if backup.exists() {
        let _ = std::fs::remove_dir_all(&backup);
    }

    let mut result = format!(
        "installed {}\nbackend: {}\nhotkey: {}\n",
        destination.display(),
        config.backend.as_str(),
        config.hotkey.canonical()
    );
    let registration_failed = std::fs::read(status_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|event| {
            event
                .get("event")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .as_deref()
        == Some("registration-failed");
    if registration_failed {
        result.push_str(
            "The chord was claimed after preflight. Free it or choose another; the helper stays up and keeps retrying every few seconds.\n",
        );
    } else {
        result.push_str("The global chord is registered; one launcher may be open at a time.\n");
    }
    result.push_str("Terminal.app may ask for Automation permission the first time it is used.\n");
    if !zsh_widget_available() {
        result.push_str(
            "The login zsh does not load _prelude_widget yet. Add `eval \"$(prelude init zsh)\"` to ~/.zshrc before using Cmd+Space.",
        );
    }
    Ok(result)
}

fn uninstall(reset: bool) -> Result<String, String> {
    stop_helper();
    let _ = std::fs::remove_file(active_path());
    let agent = launch_agent_path();
    if agent.exists() {
        std::fs::remove_file(&agent)
            .map_err(|e| format!("could not remove {}: {e}", agent.display()))?;
    }
    let app = app_path();
    if app.exists() {
        std::fs::remove_dir_all(&app)
            .map_err(|e| format!("could not remove {}: {e}", app.display()))?;
    }
    if reset {
        for path in [
            config_path(),
            status_path(),
            active_path(),
            stdout_path(),
            stderr_path(),
        ] {
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("could not remove {}: {e}", path.display()))?;
            }
        }
    }
    Ok(if reset {
        "removed the global hotkey helper and its Prelude-owned preferences".into()
    } else {
        "removed the global hotkey helper; backend preference retained".into()
    })
}

fn helper_probe() -> Option<serde_json::Value> {
    let exe = executable_path();
    if !exe.is_file() {
        return None;
    }
    let output = Command::new(exe)
        .arg("--probe")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 16 * 1024 {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
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

fn check_hotkey_with(executable: &Path) -> Result<(), String> {
    let status = Command::new(executable)
        .arg("--check-hotkey")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("could not check the requested global hotkey: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(
            "the hotkey is already registered by macOS or another application such as Raycast"
                .into(),
        )
    }
}

/// A launch that never produced a shell is not a launcher, and a shell that has
/// gone is not one either. The grace window covers only the seconds between the
/// helper claiming the lease and the terminal's zsh writing its pid into it.
const LEASE_GRACE: Duration = Duration::from_secs(20);
/// A backstop against a recycled pid, not against a launcher left open.
const LEASE_LIFETIME: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lease {
    /// Nothing holds the launcher; the next hotkey creates one.
    Free,
    /// Claimed, with no shell reporting itself yet.
    Starting,
    /// A live shell owns the launcher.
    Held(u32),
}

fn pid_alive(pid: u32) -> bool {
    // A pid larger than the platform's own is not a process we can ask about.
    i32::try_from(pid).is_ok_and(crate::exec::alive)
}

fn lease_at(path: &Path) -> Lease {
    let Some(age) = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.elapsed().ok())
    else {
        return Lease::Free;
    };
    if age >= LEASE_LIFETIME {
        return Lease::Free;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Lease::Free;
    };
    let mut lines = text.lines();
    if lines.next().is_none_or(|token| token.trim().is_empty()) {
        return Lease::Free;
    }
    let _backend = lines.next();
    match lines.next().and_then(|pid| pid.trim().parse::<u32>().ok()) {
        Some(pid) if pid_alive(pid) => Lease::Held(pid),
        Some(_) => Lease::Free,
        None if age < LEASE_GRACE => Lease::Starting,
        None => Lease::Free,
    }
}

fn launcher_lease() -> Lease {
    let path = active_path();
    let lease = lease_at(&path);
    if lease == Lease::Free && path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    lease
}

fn launcher_active() -> bool {
    launcher_lease() != Lease::Free
}

pub fn status() -> GlobalStatus {
    let parsed = configured();
    let selected_backend = parsed
        .as_ref()
        .map(|config| config.backend.as_str().to_string())
        .unwrap_or_else(|e| format!("invalid: {e}"));
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
    let probe = helper_probe();
    let ghostty_available = probe
        .as_ref()
        .and_then(|v| v.get("ghostty_available"))
        .and_then(serde_json::Value::as_bool);
    let effective_backend = probe
        .as_ref()
        .and_then(|v| v.get("effective_backend"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let last_event: Option<serde_json::Value> = std::fs::read(status_path())
        .ok()
        .filter(|v| v.len() <= 16 * 1024)
        .and_then(|v| serde_json::from_slice(&v).ok());
    let helper_running = running();
    let registration_failed = last_event
        .as_ref()
        .and_then(|event| event.get("event"))
        .and_then(serde_json::Value::as_str)
        == Some("registration-failed");
    let owner = hotkey_owner(&hotkey);
    let lease = launcher_lease();
    GlobalStatus {
        schema: 4,
        app_installed: executable_path().is_file(),
        launch_agent_installed: launch_agent_path().is_file(),
        helper_running,
        hotkey_registered: helper_running && !registration_failed && owner.owner.is_none(),
        launcher_active: lease != Lease::Free,
        launcher_pid: match lease {
            Lease::Held(pid) => Some(pid),
            _ => None,
        },
        selected_hotkey,
        hotkey_owner: owner.owner,
        owner_checks_complete: owner.complete,
        selected_backend,
        launch_directory: directory.to_string_lossy().into_owned(),
        launch_directory_exists: directory.is_dir(),
        ghostty_available,
        effective_backend,
        zsh_widget_available: zsh_widget_available(),
        last_event,
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
        println!("Prelude global hotkey\n");
        line("helper app", s.app_installed, &app_path().to_string_lossy());
        line(
            "LaunchAgent",
            s.launch_agent_installed,
            &launch_agent_path().to_string_lossy(),
        );
        line(
            "helper running",
            s.helper_running,
            if s.helper_running {
                "native helper process active"
            } else {
                "run: prelude global start"
            },
        );
        line(
            &format!("{} registered", s.selected_hotkey),
            s.hotkey_registered,
            if s.hotkey_registered {
                "listener active"
            } else {
                "free the shortcut or choose another, then run: prelude global start"
            },
        );
        line(
            "launcher singleton",
            true,
            &match (s.launcher_active, s.launcher_pid) {
                (true, Some(pid)) => format!("one Prelude launcher is open (shell {pid})"),
                (true, None) => "a launcher is starting".into(),
                (false, _) => "ready".to_string(),
            },
        );
        line(
            "zsh widget",
            s.zsh_widget_available,
            if s.zsh_widget_available {
                "_prelude_widget is loaded"
            } else {
                "add eval \"$(prelude init zsh)\" to ~/.zshrc"
            },
        );
        println!(
            "  {} backend  {}{}",
            if s.effective_backend.is_some() {
                "✓"
            } else {
                "✗"
            },
            s.selected_backend,
            s.effective_backend
                .as_ref()
                .map(|v| format!(" → {v}"))
                .unwrap_or_default()
        );
        println!(
            "  {} directory  {}{}",
            if s.launch_directory_exists { "✓" } else { "✗" },
            s.launch_directory,
            if s.launch_directory_exists {
                String::new()
            } else {
                format!(
                    " (missing; launches fall back to {})",
                    crate::paths::home().display()
                )
            }
        );
        println!(
            "  {} Ghostty  {}",
            if s.ghostty_available == Some(true) {
                "✓"
            } else {
                "·"
            },
            match s.ghostty_available {
                Some(true) => "installed",
                Some(false) => "not installed; Terminal.app will be used",
                None => "unknown until the helper is installed",
            }
        );
        println!(
            "  {} conflicts  {}",
            if s.hotkey_owner.is_some() { "✗" } else { "✓" },
            match (&s.hotkey_owner, s.owner_checks_complete) {
                (Some(owner), _) => format!("{owner} owns this chord"),
                // macOS names no registry of the applications that merely watch
                // a key, so the honest claim is about what was looked at.
                (None, true) => "no known owner".into(),
                (None, false) => "no known owner, but some owner records could not be read".to_string(),
            }
        );
        if let Some(event) = &s.last_event {
            println!(
                "\nlast event: {}",
                serde_json::to_string(event).unwrap_or_default()
            );
        }
    }
    if s.app_installed
        && s.launch_agent_installed
        && s.helper_running
        && s.hotkey_registered
        && s.zsh_widget_available
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

fn open_once() -> Result<(), String> {
    let config = configured()?;
    if let Some(owner) = known_conflict(&config.hotkey) {
        return Err(format!(
            "{} is already configured for {owner}; choose another with `prelude global hotkey HOTKEY`",
            config.hotkey.canonical()
        ));
    }
    let exe = executable_path();
    if !exe.is_file() {
        return Err("global hotkey helper is not installed; run `prelude global install`".into());
    }
    let status = Command::new(exe)
        .arg("--open")
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("could not start the hotkey helper: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(
            "the selected terminal backend could not open a window; run `prelude global status`"
                .into(),
        )
    }
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
        ["clear"] => {
            let _ = std::fs::remove_file(active_path());
            println!("cleared the global launcher lease");
            Ok(())
        }
        ["hotkey"] => configured().map(|config| println!("{}", config.hotkey.canonical())),
        ["hotkey", value] => Hotkey::parse(value).and_then(write_hotkey).map(|message| println!("{message}")),
        ["directory"] => {
            configured().map(|config| println!("{}", effective_directory(&config).display()))
        }
        ["directory", "--default"] => clear_directory().map(|message| println!("{message}")),
        ["directory", value] => write_directory(value).map(|message| println!("{message}")),
        ["backend"] => configured().map(|config| println!("{}", config.backend.as_str())),
        ["backend", value] => Backend::parse(value).and_then(|backend| {
            write_backend(backend)?;
            println!("global terminal backend: {}", backend.as_str());
            Ok(())
        }),
        _ => Err(
            "usage: prelude global install|uninstall|start|stop|status|open|clear|hotkey [CHORD]|backend [auto|ghostty|terminal]|directory [PATH|--default]"
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
    fn config_accepts_only_public_backends_and_canonical_hotkeys() {
        let path = temp("backend");
        std::fs::write(
            &path,
            "backend = \"ghostty\"\nhotkey = \"Shift+Command+K\"\n",
        )
        .unwrap();
        let config = config_from(&path).unwrap();
        assert_eq!(config.backend, Backend::Ghostty);
        assert_eq!(config.hotkey.canonical(), "cmd+shift+k");
        std::fs::write(&path, "backend = \"something-else\"\n").unwrap();
        assert!(config_from(&path)
            .unwrap_err()
            .contains("auto, ghostty or terminal"));
        for bad in ["space", "cmd", "cmd+f1", "cmd+space+q", "cmd+cmd+q"] {
            std::fs::write(&path, format!("hotkey = \"{bad}\"\n")).unwrap();
            assert!(config_from(&path).is_err(), "{bad}");
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_launcher_directory_is_optional_absolute_and_safe_in_both_backends() {
        let home = crate::paths::home();
        assert_eq!(parse_directory("~/").unwrap(), home);
        assert_eq!(parse_directory("/tmp").unwrap(), PathBuf::from("/tmp"));
        for bad in ["relative/path", "", "~notauser/x"] {
            assert!(parse_directory(bad).is_err(), "{bad}");
        }
        // One rule covers a process argument and an Apple Event, rather than
        // two escapes that can disagree.
        for bad in ["/tmp/a\"b", "/tmp/a'b", "/tmp/a`b", "/tmp/a$b", "/tmp/a\nb"] {
            assert!(parse_directory(bad).is_err(), "{bad}");
        }

        // Unset means $HOME, and stays out of the file so no personal path is
        // written until somebody asks for one.
        let path = temp("directory");
        std::fs::write(&path, "backend = \"auto\"\nhotkey = \"cmd+space\"\n").unwrap();
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
    fn a_dismissed_launcher_window_closes_and_leaves_no_history() {
        let zsh = crate::init::ZSH;
        // `status` is a read-only alias for $? in zsh; declaring it local
        // fails the whole widget before Prelude is ever run.
        assert!(!zsh.contains("local out verb payload status"));
        assert!(zsh.contains("_prelude_result"));
        assert!(zsh.contains("(( code == 130 )) || _prelude_result=FAILED"));
        assert!(zsh.contains("setopt hist_ignore_space"));
        assert!(zsh.contains("BUFFER=' exit'"));
        // Only the one-shot window closes itself; Ctrl+R must not.
        let dispatch = zsh.find("_prelude_autostart_dispatch() {").unwrap();
        assert!(zsh.find("BUFFER=' exit'").unwrap() > dispatch);
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
        assert_eq!(
            app_path(),
            crate::paths::home().join("Applications").join(APP_NAME)
        );
        assert_eq!(
            launch_agent_path(),
            crate::paths::home().join("Library/LaunchAgents/app.prelude.hotkey.plist")
        );
        assert!(config_path().starts_with(crate::paths::config()));
        for path in [status_path(), active_path(), stdout_path(), stderr_path()] {
            assert!(path.starts_with(crate::paths::cache()));
        }
    }

    #[test]
    fn generated_metadata_escapes_paths_and_contains_no_selected_payload() {
        let info = info_plist(
            Path::new("/tmp/a&b/config"),
            Path::new("/tmp/<status>"),
            Path::new("/tmp/active"),
        );
        assert!(info.contains("/tmp/a&amp;b/config"));
        assert!(info.contains("/tmp/&lt;status&gt;"));
        assert!(info.contains("<key>LSUIElement</key><true/>"));
        assert!(!info.contains("INSERT"));
        let agent = launch_agent(
            Path::new("/tmp/a&b/helper"),
            Path::new("/tmp/out"),
            Path::new("/tmp/err"),
        );
        assert!(agent.contains("/tmp/a&amp;b/helper"));
        assert!(agent.contains("<key>RunAtLoad</key><true/>"));
        // A helper that dies must come back. `Crashed` is not enough: launchd
        // counts only the classic fault signals as a crash, so a helper killed
        // for memory pressure would stay dead until somebody noticed the
        // hotkey had stopped working. `prelude global stop` boots the job out
        // rather than exiting the process, so it is unaffected.
        assert!(agent.contains("<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>"));
        assert!(!agent.contains("<key>KeepAlive</key><true/>"));
        assert!(!agent.contains("<key>KeepAlive</key><false/>"));
    }

    #[test]
    fn a_lease_is_only_held_while_the_shell_that_claimed_it_is_alive() {
        let path = temp("lease");
        let mine = std::process::id();

        std::fs::write(&path, format!("token\nghostty\n{mine}\n")).unwrap();
        assert_eq!(lease_at(&path), Lease::Held(mine));

        // A launcher whose terminal was killed outright frees the hotkey at
        // once; it does not wait out a timeout nobody can see.
        std::fs::write(&path, "token\nghostty\n999999999\n").unwrap();
        assert_eq!(lease_at(&path), Lease::Free);

        // Claimed but not yet reported: real for a few seconds, then not.
        std::fs::write(&path, "token\n").unwrap();
        assert_eq!(lease_at(&path), Lease::Starting);

        std::fs::write(&path, "\nghostty\n").unwrap();
        assert_eq!(lease_at(&path), Lease::Free);
        std::fs::write(&path, format!("token\n\n{mine}\n")).unwrap();
        assert_eq!(lease_at(&path), Lease::Held(mine));

        let _ = std::fs::remove_file(&path);
        assert_eq!(lease_at(&path), Lease::Free);
        assert!(LEASE_GRACE < LEASE_LIFETIME);
    }

    #[test]
    fn a_stale_grace_window_expires_without_a_reported_shell() {
        // The grace window is measured from the lease's own mtime, so an old
        // unclaimed lease is free however it was written.
        let path = temp("lease-grace");
        std::fs::write(&path, "token\n").unwrap();
        let old = std::time::SystemTime::now() - (LEASE_GRACE + Duration::from_secs(5));
        std::fs::File::open(&path)
            .and_then(|file| file.set_modified(old))
            .unwrap();
        assert_eq!(lease_at(&path), Lease::Free);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pid_liveness_answers_for_this_process_and_refuses_impossible_ones() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
        assert!(!pid_alive(u32::MAX));
    }

    #[test]
    fn swift_helper_uses_fixed_bootstrap_and_never_shells_a_selection() {
        assert!(SWIFT.contains("PRELUDE_AUTOSTART=1"));
        assert!(SWIFT.contains("com.mitchellh.ghostty"));
        assert!(SWIFT.contains("createsNewApplicationInstance = true"));
        // A launcher window is not a session to restore, and must not become
        // the saved state of the Ghostty the person actually works in.
        assert!(SWIFT.contains("\"--window-save-state=never\""));
        // Launch Services answering success is not proof that the arguments
        // above were delivered rather than dropped into the running instance.
        assert!(SWIFT.contains("existing.contains(application.processIdentifier)"));
        assert!(SWIFT.contains("\"-e\""));
        assert!(SWIFT.contains("\"/usr/bin/env\""));
        assert!(SWIFT.contains("\"PRELUDE_AUTOSTART=1\""));
        assert!(SWIFT.contains("PRELUDE_GLOBAL_TOKEN="));
        assert!(SWIFT.contains("/usr/bin/osascript"));
        assert!(SWIFT.contains("did not answer within 10 seconds"));
        assert!(SWIFT.contains("RegisterEventHotKey"));
        assert!(SWIFT.contains("--check-hotkey"));
        assert!(SWIFT.contains("PreludeActivePath"));
        assert!(!SWIFT.contains("SELECTED_COMMAND"));
        assert!(!SWIFT.contains("sh -c"));
    }

    #[test]
    fn a_busy_chord_leaves_the_helper_running_and_retrying() {
        // Exiting on a chord conflict strands the helper: the other owner can
        // release the key and nothing will ever ask for it again.
        assert!(!SWIFT.contains("NSApp.terminate"));
        assert!(SWIFT.contains("Timer.scheduledTimer"));
        assert!(SWIFT.contains("private func register()"));
        assert!(SWIFT.contains("registration-failed"));
        // And it says so once, not every five seconds.
        assert!(SWIFT.contains("if !warned {"));
    }

    #[test]
    fn the_shell_reports_its_pid_into_the_lease_and_removes_only_its_own() {
        let zsh = crate::init::ZSH;
        assert!(zsh.contains("_prelude_global_claim"));
        assert!(zsh.contains("print -rl -- \"$owner\" \"$backend\" \"$$\""));
        assert!(zsh.contains("[[ \"$owner\" == \"$PRELUDE_GLOBAL_TOKEN\" ]] || return 0"));
        // The claim happens once the autostart shell exists, before Prelude opens.
        let claim = zsh.find("  _prelude_global_claim").unwrap();
        let widget = zsh.find("zle _prelude_widget").unwrap();
        assert!(claim < widget);
    }
}
