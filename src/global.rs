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

    fn is_cmd_space(&self) -> bool {
        self.cmd && !self.option && !self.ctrl && !self.shift && self.key == "space"
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
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            backend: Backend::Auto,
            hotkey: Hotkey::parse("cmd+space").expect("default hotkey"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct GlobalStatus {
    pub schema: u8,
    pub app_installed: bool,
    pub launch_agent_installed: bool,
    pub helper_running: bool,
    pub hotkey_registered: bool,
    pub launcher_active: bool,
    pub selected_hotkey: String,
    pub spotlight_owns_hotkey: Option<bool>,
    pub raycast_owns_hotkey: Option<bool>,
    pub selected_backend: String,
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
    Ok(GlobalConfig { backend, hotkey })
}

fn configured() -> Result<GlobalConfig, String> {
    config_from(&config_path())
}

fn write_config(config: &GlobalConfig) -> Result<(), String> {
    let path = config_path();
    crate::cache::write_atomic(
        &path,
        format!(
            "# Terminal and chord used by the global launcher helper.\n# auto prefers Ghostty and falls back to Terminal.app.\nbackend = \"{}\"\nhotkey = \"{}\"\n",
            config.backend.as_str(), config.hotkey.canonical()
        )
        .as_bytes(),
    )
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
  <key>KeepAlive</key><false/>
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
            "The chord was claimed after preflight. Free it or choose another, then run `prelude global start`.\n",
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

fn spotlight_owns(hotkey: &Hotkey) -> Option<bool> {
    if !hotkey.is_cmd_space() {
        return Some(false);
    }
    let plist = crate::paths::home().join("Library/Preferences/com.apple.symbolichotkeys.plist");
    let plist = plist.to_string_lossy();
    let out = crate::exec::run(
        &[
            "/usr/bin/plutil",
            "-extract",
            "AppleSymbolicHotKeys.64.enabled",
            "raw",
            &plist,
        ],
        Duration::from_secs(2),
    );
    match out.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
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

fn known_conflict(hotkey: &Hotkey) -> Option<&'static str> {
    if spotlight_owns(hotkey) == Some(true) {
        Some("Spotlight")
    } else if raycast_owns(hotkey) == Some(true) {
        Some("Raycast")
    } else {
        None
    }
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

fn launcher_active() -> bool {
    let Ok(meta) = std::fs::metadata(active_path()) else {
        return false;
    };
    let fresh = meta
        .modified()
        .ok()
        .and_then(|time| time.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(30 * 60));
    if !fresh {
        let _ = std::fs::remove_file(active_path());
        return false;
    }
    std::fs::read_to_string(active_path()).is_ok_and(|token| !token.trim().is_empty())
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
    let spotlight = spotlight_owns(&hotkey);
    let raycast = raycast_owns(&hotkey);
    GlobalStatus {
        schema: 2,
        app_installed: executable_path().is_file(),
        launch_agent_installed: launch_agent_path().is_file(),
        helper_running,
        hotkey_registered: helper_running
            && !registration_failed
            && spotlight != Some(true)
            && raycast != Some(true),
        launcher_active: launcher_active(),
        selected_hotkey,
        spotlight_owns_hotkey: spotlight,
        raycast_owns_hotkey: raycast,
        selected_backend,
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
            if s.launcher_active {
                "one Prelude launcher is open"
            } else {
                "ready"
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
            if s.spotlight_owns_hotkey == Some(true) || s.raycast_owns_hotkey == Some(true) {
                "✗"
            } else {
                "✓"
            },
            match (s.spotlight_owns_hotkey, s.raycast_owns_hotkey) {
                (Some(true), _) => "Spotlight owns this chord",
                (_, Some(true)) => "Raycast owns this chord",
                (None, _) | (_, None) => "known-owner state partly unavailable",
                _ => "no Spotlight or Raycast conflict",
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
        && s.spotlight_owns_hotkey != Some(true)
        && s.raycast_owns_hotkey != Some(true)
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
        ["backend"] => configured().map(|config| println!("{}", config.backend.as_str())),
        ["backend", value] => Backend::parse(value).and_then(|backend| {
            write_backend(backend)?;
            println!("global terminal backend: {}", backend.as_str());
            Ok(())
        }),
        _ => Err(
            "usage: prelude global install|uninstall|start|stop|status|open|clear|hotkey [CHORD]|backend [auto|ghostty|terminal]"
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
    }

    #[test]
    fn swift_helper_uses_fixed_bootstrap_and_never_shells_a_selection() {
        assert!(SWIFT.contains("PRELUDE_AUTOSTART=1"));
        assert!(SWIFT.contains("com.mitchellh.ghostty"));
        assert!(SWIFT.contains("createsNewApplicationInstance = true"));
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
}
