import AppKit
import Carbon

private let ghosttyBundleID = "com.mitchellh.ghostty"
private let terminalBundleID = "com.apple.Terminal"
private let bootstrapCommand = "exec /usr/bin/env PRELUDE_AUTOSTART=1 /bin/zsh -il"
private let hotKeySignature = OSType(0x50524C44) // PRLD

private enum Backend: String {
    case auto
    case ghostty
    case terminal
}

private struct AppPaths {
    let config: String
    let status: String

    static func bundled() -> AppPaths {
        let info = Bundle.main.infoDictionary ?? [:]
        return AppPaths(
            config: info["PreludeConfigPath"] as? String ?? "",
            status: info["PreludeStatusPath"] as? String ?? ""
        )
    }
}

private final class StatusWriter {
    private let path: String

    init(path: String) {
        self.path = path
    }

    func write(event: String, backend: String, ok: Bool, detail: String = "") {
        guard !path.isEmpty else { return }
        let bounded = String(detail.prefix(500))
        let body: [String: Any] = [
            "schema": 1,
            "time": Int(Date().timeIntervalSince1970),
            "event": event,
            "backend": backend,
            "ok": ok,
            "detail": bounded,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: body, options: [.sortedKeys]) else {
            return
        }
        let file = URL(fileURLWithPath: path)
        let dir = file.deletingLastPathComponent()
        let temp = dir.appendingPathComponent(".\(file.lastPathComponent).\(getpid()).tmp")
        do {
            try FileManager.default.createDirectory(
                at: dir,
                withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700]
            )
            try data.write(to: temp, options: [])
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: temp.path)
            if FileManager.default.fileExists(atPath: file.path) {
                _ = try FileManager.default.replaceItemAt(file, withItemAt: temp)
            } else {
                try FileManager.default.moveItem(at: temp, to: file)
            }
        } catch {
            try? FileManager.default.removeItem(at: temp)
        }
    }
}

private final class TerminalLauncher {
    private let paths: AppPaths
    private let status: StatusWriter
    private var lastLaunch = 0.0

    init(paths: AppPaths) {
        self.paths = paths
        self.status = StatusWriter(path: paths.status)
    }

    func selectedBackend() -> Backend {
        guard !paths.config.isEmpty,
              let text = try? String(contentsOfFile: paths.config, encoding: .utf8) else {
            return .auto
        }
        for raw in text.split(separator: "\n") {
            let line = raw.split(separator: "#", maxSplits: 1).first?.trimmingCharacters(in: .whitespaces) ?? ""
            guard line.hasPrefix("backend"), let equal = line.firstIndex(of: "=") else { continue }
            let value = line[line.index(after: equal)...]
                .trimmingCharacters(in: .whitespaces)
                .trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
            return Backend(rawValue: value) ?? .auto
        }
        return .auto
    }

    func ghosttyURL() -> URL? {
        NSWorkspace.shared.urlForApplication(withBundleIdentifier: ghosttyBundleID)
    }

    func probeJSON() -> String {
        let body: [String: Any] = [
            "schema": 1,
            "selected_backend": selectedBackend().rawValue,
            "ghostty_available": ghosttyURL() != nil,
            "effective_backend": effectiveBackend().rawValue,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: body, options: [.sortedKeys]),
              let text = String(data: data, encoding: .utf8) else {
            return "{}"
        }
        return text
    }

    private func effectiveBackend() -> Backend {
        let selected = selectedBackend()
        if selected == .auto {
            return ghosttyURL() == nil ? .terminal : .ghostty
        }
        return selected
    }

    func openFromHotKey() {
        let now = ProcessInfo.processInfo.systemUptime
        guard now - lastLaunch > 0.35 else { return }
        lastLaunch = now
        openTerminal(showErrors: true) { _ in }
    }

    func openTerminal(showErrors: Bool, completion: @escaping (Bool) -> Void) {
        let selected = selectedBackend()
        switch selected {
        case .auto:
            if let ghostty = ghosttyURL() {
                openGhostty(ghostty) { [weak self] ok, detail in
                    guard let self else { completion(false); return }
                    if ok {
                        self.status.write(event: "opened", backend: "ghostty", ok: true)
                        completion(true)
                    } else {
                        self.openTerminalApp { fallbackOK, fallbackDetail in
                            let note = fallbackOK
                                ? "Ghostty failed; opened Terminal.app instead: \(detail)"
                                : "Ghostty failed: \(detail); Terminal.app failed: \(fallbackDetail)"
                            self.status.write(event: "opened", backend: "terminal", ok: fallbackOK, detail: note)
                            if !fallbackOK && showErrors { self.alert(note) }
                            completion(fallbackOK)
                        }
                    }
                }
            } else {
                openTerminalApp { [weak self] ok, detail in
                    self?.status.write(event: "opened", backend: "terminal", ok: ok, detail: detail)
                    if !ok && showErrors { self?.alert(detail) }
                    completion(ok)
                }
            }
        case .ghostty:
            guard let ghostty = ghosttyURL() else {
                let message = "Ghostty is selected but is not installed. Run `prelude global backend auto` or install Ghostty."
                status.write(event: "open-failed", backend: "ghostty", ok: false, detail: message)
                if showErrors { alert(message) }
                completion(false)
                return
            }
            openGhostty(ghostty) { [weak self] ok, detail in
                self?.status.write(event: ok ? "opened" : "open-failed", backend: "ghostty", ok: ok, detail: detail)
                if !ok && showErrors { self?.alert(detail) }
                completion(ok)
            }
        case .terminal:
            openTerminalApp { [weak self] ok, detail in
                self?.status.write(event: ok ? "opened" : "open-failed", backend: "terminal", ok: ok, detail: detail)
                if !ok && showErrors { self?.alert(detail) }
                completion(ok)
            }
        }
    }

    private func openGhostty(_ url: URL, completion: @escaping (Bool, String) -> Void) {
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        configuration.createsNewApplicationInstance = true
        configuration.arguments = [
            "--working-directory=\(FileManager.default.homeDirectoryForCurrentUser.path)",
            "--initial-command=direct:/usr/bin/env PRELUDE_AUTOSTART=1 /bin/zsh -il",
        ]
        let lock = NSLock()
        var finished = false
        func finish(_ ok: Bool, _ detail: String) {
            lock.lock()
            guard !finished else { lock.unlock(); return }
            finished = true
            lock.unlock()
            DispatchQueue.main.async { completion(ok, String(detail.prefix(500))) }
        }
        NSWorkspace.shared.openApplication(at: url, configuration: configuration) { _, error in
            if let error {
                finish(false, error.localizedDescription)
            } else {
                finish(true, "")
            }
        }
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 10) {
            finish(false, "Ghostty did not answer within 10 seconds")
        }
    }

    private func openTerminalApp(completion: @escaping (Bool, String) -> Void) {
        // This is intentionally fixed text. Selected launcher payloads never
        // cross the Apple Event boundary or enter a process command line.
        let source = """
        tell application id "\(terminalBundleID)"
            do script "\(bootstrapCommand)"
            activate
        end tell
        """
        let process = Process()
        let errors = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", source]
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        process.standardError = errors

        let lock = NSLock()
        var finished = false
        func finish(_ ok: Bool, _ detail: String) {
            lock.lock()
            guard !finished else { lock.unlock(); return }
            finished = true
            lock.unlock()
            DispatchQueue.main.async { completion(ok, String(detail.prefix(500))) }
        }
        process.terminationHandler = { task in
            let data = errors.fileHandleForReading.readDataToEndOfFile()
            let detail = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            finish(task.terminationStatus == 0, detail)
        }
        do {
            try process.run()
        } catch {
            finish(false, error.localizedDescription)
            return
        }
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 10) {
            if process.isRunning {
                process.terminate()
                finish(false, "Terminal.app did not answer within 10 seconds")
            }
        }
    }

    private func alert(_ message: String) {
        DispatchQueue.main.async {
            NSApp.activate(ignoringOtherApps: true)
            let alert = NSAlert()
            alert.messageText = "Prelude could not open a terminal"
            alert.informativeText = message
            alert.alertStyle = .warning
            alert.addButton(withTitle: "OK")
            alert.runModal()
        }
    }

    func fatalHotKeyError(_ message: String) {
        DispatchQueue.main.async {
            NSApp.activate(ignoringOtherApps: true)
            let alert = NSAlert()
            alert.messageText = "Prelude could not register Cmd+Space"
            alert.informativeText = message
            alert.alertStyle = .warning
            alert.addButton(withTitle: "OK")
            alert.runModal()
            NSApp.terminate(nil)
        }
    }
}

private var globalDelegate: HotKeyDelegate?

private func hotKeyEventHandler(
    _ nextHandler: EventHandlerCallRef?,
    _ event: EventRef?,
    _ userData: UnsafeMutableRawPointer?
) -> OSStatus {
    guard let event,
          GetEventClass(event) == OSType(kEventClassKeyboard),
          GetEventKind(event) == UInt32(kEventHotKeyPressed) else {
        return OSStatus(eventNotHandledErr)
    }
    DispatchQueue.main.async {
        globalDelegate?.launcher.openFromHotKey()
    }
    return noErr
}

private final class HotKeyDelegate: NSObject, NSApplicationDelegate {
    let launcher = TerminalLauncher(paths: .bundled())
    private var hotKey: EventHotKeyRef?
    private var eventHandler: EventHandlerRef?

    func applicationDidFinishLaunching(_ notification: Notification) {
        globalDelegate = self
        var eventType = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )
        let install = InstallEventHandler(
            GetApplicationEventTarget(),
            hotKeyEventHandler,
            1,
            &eventType,
            nil,
            &eventHandler
        )
        guard install == noErr else {
            StatusWriter(path: AppPaths.bundled().status).write(
                event: "registration-failed", backend: "", ok: false,
                detail: "could not install the macOS hotkey event handler (\(install))"
            )
            launcher.fatalHotKeyError("macOS refused to install Prelude's hotkey event handler (\(install)).")
            return
        }
        let id = EventHotKeyID(signature: hotKeySignature, id: 1)
        let registered = RegisterEventHotKey(
            UInt32(kVK_Space), UInt32(cmdKey), id,
            GetApplicationEventTarget(), 0, &hotKey
        )
        guard registered == noErr else {
            StatusWriter(path: AppPaths.bundled().status).write(
                event: "registration-failed", backend: "", ok: false,
                detail: "Cmd+Space is already owned by macOS or another application (\(registered))"
            )
            launcher.fatalHotKeyError(
                "Cmd+Space is already assigned to Spotlight or another application. Change it in System Settings → Keyboard → Keyboard Shortcuts, then run `prelude global start`."
            )
            return
        }
        StatusWriter(path: AppPaths.bundled().status).write(
            event: "registered", backend: launcher.selectedBackend().rawValue, ok: true
        )
    }

    func applicationWillTerminate(_ notification: Notification) {
        if let hotKey { UnregisterEventHotKey(hotKey) }
        if let eventHandler { RemoveEventHandler(eventHandler) }
    }
}

private let arguments = Set(CommandLine.arguments.dropFirst())
private let launcher = TerminalLauncher(paths: .bundled())
if arguments.contains("--probe") {
    print(launcher.probeJSON())
    exit(0)
}
if arguments.contains("--open") {
    launcher.openTerminal(showErrors: true) { ok in
        exit(ok ? 0 : 1)
    }
    dispatchMain()
}

private let app = NSApplication.shared
private let delegate = HotKeyDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
