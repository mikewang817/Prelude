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

#[derive(Debug, Serialize)]
pub struct GlobalStatus {
    pub schema: u8,
    pub app_installed: bool,
    pub launch_agent_installed: bool,
    pub helper_running: bool,
    pub hotkey_registered: bool,
    pub selected_backend: String,
    pub ghostty_available: Option<bool>,
    pub effective_backend: Option<String>,
    pub zsh_widget_available: bool,
    pub spotlight_owns_cmd_space: Option<bool>,
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

fn stdout_path() -> PathBuf {
    crate::paths::cache().join("global-hotkey.log")
}

fn stderr_path() -> PathBuf {
    crate::paths::cache().join("global-hotkey-error.log")
}

fn backend_from(path: &Path) -> Result<Backend, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Backend::Auto),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    let parsed = crate::minitoml::parse(&text);
    let value = parsed
        .get("")
        .and_then(|root| root.get("backend"))
        .map(String::as_str)
        .unwrap_or("auto");
    Backend::parse(value)
}

fn configured_backend() -> Result<Backend, String> {
    backend_from(&config_path())
}

fn write_backend(backend: Backend) -> Result<(), String> {
    let path = config_path();
    crate::cache::write_atomic(
        &path,
        format!(
            "# Terminal created by the global Cmd+Space helper.\n# auto prefers Ghostty and falls back to Terminal.app.\nbackend = \"{}\"\n",
            backend.as_str()
        )
        .as_bytes(),
    )
    .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    private_file(&path)?;
    Ok(())
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

fn info_plist(config: &Path, status: &Path) -> String {
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
</dict></plist>
"#,
        version = env!("CARGO_PKG_VERSION"),
        config = xml(&config.to_string_lossy()),
        status = xml(&status.to_string_lossy()),
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
        info_plist(&config_path(), &status_path()),
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
    let submitted = if loaded() {
        launchctl(&["kickstart", "-k", &service()])
    } else {
        launchctl(&["bootstrap", &domain(), &path.to_string_lossy()])
    };
    if !submitted {
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
    if !config_path().exists() {
        write_backend(Backend::Auto)?;
    } else {
        configured_backend()?;
    }
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

    stop_helper();
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

    let spotlight = spotlight_owns_cmd_space();
    let mut result = format!(
        "installed {}\nbackend: {}\n",
        destination.display(),
        configured_backend()?.as_str()
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
    if spotlight == Some(true) {
        result.push_str(
            "Cmd+Space is still assigned to Spotlight. Disable it in System Settings → Keyboard → Keyboard Shortcuts → Spotlight.\n",
        );
    } else if registration_failed {
        result.push_str(
            "Cmd+Space is owned by another application. Free it, then run `prelude global start`.\n",
        );
    } else {
        result.push_str("Cmd+Space is registered; each press opens a fresh terminal.\n");
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
        for path in [config_path(), status_path(), stdout_path(), stderr_path()] {
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

fn spotlight_owns_cmd_space() -> Option<bool> {
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

pub fn status() -> GlobalStatus {
    let selected = configured_backend()
        .map(|b| b.as_str().to_string())
        .unwrap_or_else(|e| format!("invalid: {e}"));
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
    let spotlight = spotlight_owns_cmd_space();
    GlobalStatus {
        schema: 1,
        app_installed: executable_path().is_file(),
        launch_agent_installed: launch_agent_path().is_file(),
        helper_running,
        hotkey_registered: helper_running && !registration_failed && spotlight != Some(true),
        selected_backend: selected,
        ghostty_available,
        effective_backend,
        zsh_widget_available: zsh_widget_available(),
        spotlight_owns_cmd_space: spotlight,
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
            "Cmd+Space registered",
            s.hotkey_registered,
            if s.hotkey_registered {
                "listener active"
            } else {
                "free the shortcut, then run: prelude global start"
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
            "  {} Spotlight  {}",
            if s.spotlight_owns_cmd_space == Some(true) {
                "✗"
            } else {
                "✓"
            },
            match s.spotlight_owns_cmd_space {
                Some(true) => "still owns Cmd+Space",
                Some(false) => "Cmd+Space is free",
                None => "shortcut state unavailable",
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
        && s.spotlight_owns_cmd_space != Some(true)
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
        ["backend"] => configured_backend().map(|b| println!("{}", b.as_str())),
        ["backend", value] => Backend::parse(value).and_then(|backend| {
            write_backend(backend)?;
            println!("global terminal backend: {}", backend.as_str());
            Ok(())
        }),
        _ => Err(
            "usage: prelude global install|uninstall|start|stop|status|open|backend [auto|ghostty|terminal]"
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
    fn backend_config_accepts_only_the_three_public_choices() {
        let path = temp("backend");
        std::fs::write(&path, "backend = \"ghostty\"\n").unwrap();
        assert_eq!(backend_from(&path).unwrap(), Backend::Ghostty);
        std::fs::write(&path, "backend = \"something-else\"\n").unwrap();
        assert!(backend_from(&path)
            .unwrap_err()
            .contains("auto, ghostty or terminal"));
        let _ = std::fs::remove_file(path);
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
        for path in [status_path(), stdout_path(), stderr_path()] {
            assert!(path.starts_with(crate::paths::cache()));
        }
    }

    #[test]
    fn generated_metadata_escapes_paths_and_contains_no_selected_payload() {
        let info = info_plist(Path::new("/tmp/a&b/config"), Path::new("/tmp/<status>"));
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
        assert!(SWIFT.contains("--initial-command=direct:/usr/bin/env PRELUDE_AUTOSTART=1"));
        assert!(SWIFT.contains("/usr/bin/osascript"));
        assert!(SWIFT.contains("did not answer within 10 seconds"));
        assert!(SWIFT.contains("RegisterEventHotKey"));
        assert!(!SWIFT.contains("SELECTED_COMMAND"));
        assert!(!SWIFT.contains("sh -c"));
    }
}
