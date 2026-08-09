import AppKit
import Carbon

private let ghosttyBundleID = "com.mitchellh.ghostty"
private let terminalBundleID = "com.apple.Terminal"
private let hotKeySignature = OSType(0x50524C44) // PRLD

private enum Backend: String {
    case auto
    case ghostty
    case terminal
}

private struct HotKeySpec {
    let canonical: String
    let keyCode: UInt32
    let modifiers: UInt32

    static func parse(_ value: String) -> HotKeySpec? {
        var cmd = false
        var option = false
        var ctrl = false
        var shift = false
        var key: String?
        for raw in value.split(separator: "+", omittingEmptySubsequences: false) {
            let part = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            switch part {
            case "cmd", "command": if cmd { return nil }; cmd = true
            case "option", "alt": if option { return nil }; option = true
            case "ctrl", "control": if ctrl { return nil }; ctrl = true
            case "shift": if shift { return nil }; shift = true
            default:
                if key != nil || part.isEmpty { return nil }
                key = part
            }
        }
        guard cmd || option || ctrl || shift,
              let key,
              let code = keyCodes[key] else { return nil }
        var names: [String] = []
        var flags: UInt32 = 0
        if cmd { names.append("cmd"); flags |= UInt32(cmdKey) }
        if option { names.append("option"); flags |= UInt32(optionKey) }
        if ctrl { names.append("ctrl"); flags |= UInt32(controlKey) }
        if shift { names.append("shift"); flags |= UInt32(shiftKey) }
        names.append(key)
        return HotKeySpec(canonical: names.joined(separator: "+"), keyCode: code, modifiers: flags)
    }

    func matchesRaycast(_ value: String) -> Bool {
        var flags: UInt32 = 0
        var code: UInt32?
        for raw in value.split(separator: "-") {
            switch raw.lowercased() {
            case "command": flags |= UInt32(cmdKey)
            case "option": flags |= UInt32(optionKey)
            case "control": flags |= UInt32(controlKey)
            case "shift": flags |= UInt32(shiftKey)
            default: code = UInt32(raw)
            }
        }
        return flags == modifiers && code == keyCode
    }

    private static let keyCodes: [String: UInt32] = [
        "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5,
        "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12,
        "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18,
        "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "9": 25,
        "7": 26, "8": 28, "0": 29, "o": 31, "u": 32, "i": 34,
        "p": 35, "l": 37, "j": 38, "k": 40, "n": 45, "m": 46,
        "space": 49,
    ]
}

private struct AppPaths {
    let config: String
    let status: String
    let active: String

    static func bundled() -> AppPaths {
        let info = Bundle.main.infoDictionary ?? [:]
        return AppPaths(
            config: info["PreludeConfigPath"] as? String ?? "",
            status: info["PreludeStatusPath"] as? String ?? "",
            active: info["PreludeActivePath"] as? String ?? ""
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

private final class LauncherLease {
    private let path: String
    private let lifetime: TimeInterval = 30 * 60

    init(path: String) { self.path = path }

    func active() -> Bool {
        guard !path.isEmpty,
              let attributes = try? FileManager.default.attributesOfItem(atPath: path),
              let modified = attributes[.modificationDate] as? Date,
              Date().timeIntervalSince(modified) < lifetime,
              let contents = try? String(contentsOfFile: path, encoding: .utf8),
              !contents.split(separator: "\n").isEmpty else {
            if !path.isEmpty { try? FileManager.default.removeItem(atPath: path) }
            return false
        }
        return true
    }

    func claim() -> String? {
        guard !active(), !path.isEmpty else { return nil }
        let token = UUID().uuidString.lowercased()
        let file = URL(fileURLWithPath: path)
        let dir = file.deletingLastPathComponent()
        let temp = dir.appendingPathComponent(".global-active.\(getpid()).tmp")
        do {
            try FileManager.default.createDirectory(
                at: dir, withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700]
            )
            try Data((token + "\n").utf8).write(to: temp)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: temp.path)
            try FileManager.default.moveItem(at: temp, to: file)
            return token
        } catch {
            try? FileManager.default.removeItem(at: temp)
            return nil
        }
    }

    func setBackend(_ backend: Backend, token: String) {
        guard let current = try? String(contentsOfFile: path, encoding: .utf8),
              current.split(separator: "\n").first.map(String.init) == token else { return }
        do {
            try Data("\(token)\n\(backend.rawValue)\n".utf8).write(
                to: URL(fileURLWithPath: path), options: [.atomic]
            )
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
        } catch { return }
    }

    func backend() -> Backend? {
        guard let current = try? String(contentsOfFile: path, encoding: .utf8) else { return nil }
        let lines = current.split(separator: "\n")
        guard lines.count > 1 else { return nil }
        return Backend(rawValue: String(lines[1]))
    }

    func release(_ token: String) {
        guard let current = try? String(contentsOfFile: path, encoding: .utf8),
              current.split(separator: "\n").first.map(String.init) == token else { return }
        try? FileManager.default.removeItem(atPath: path)
    }
}

private final class TerminalLauncher {
    private let paths: AppPaths
    private let status: StatusWriter
    private let lease: LauncherLease
    private weak var activeApplication: NSRunningApplication?
    private var lastLaunch = 0.0

    init(paths: AppPaths) {
        self.paths = paths
        self.status = StatusWriter(path: paths.status)
        self.lease = LauncherLease(path: paths.active)
    }

    private func configValue(_ wanted: String) -> String? {
        guard !paths.config.isEmpty,
              let text = try? String(contentsOfFile: paths.config, encoding: .utf8) else {
            return nil
        }
        for raw in text.split(separator: "\n") {
            let line = raw.split(separator: "#", maxSplits: 1).first?.trimmingCharacters(in: .whitespaces) ?? ""
            guard let equal = line.firstIndex(of: "=") else { continue }
            let key = line[..<equal].trimmingCharacters(in: .whitespaces)
            guard key == wanted else { continue }
            return line[line.index(after: equal)...]
                .trimmingCharacters(in: .whitespaces)
                .trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
        }
        return nil
    }

    func selectedBackend() -> Backend {
        Backend(rawValue: configValue("backend") ?? "auto") ?? .auto
    }

    func selectedHotKey() -> HotKeySpec {
        HotKeySpec.parse(configValue("hotkey") ?? "cmd+space")
            ?? HotKeySpec.parse("cmd+space")!
    }

    func ghosttyURL() -> URL? {
        NSWorkspace.shared.urlForApplication(withBundleIdentifier: ghosttyBundleID)
    }

    func knownHotKeyOwner() -> String? {
        let spec = selectedHotKey()
        let preferences = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Preferences")
        let raycast = preferences.appendingPathComponent("com.raycast.macos.plist")
        if let values = NSDictionary(contentsOf: raycast),
           let value = values["raycastGlobalHotkey"] as? String,
           spec.matchesRaycast(value) {
            return "Raycast"
        }
        if spec.canonical == "cmd+space" {
            let system = preferences.appendingPathComponent("com.apple.symbolichotkeys.plist")
            if let values = NSDictionary(contentsOf: system),
               let all = values["AppleSymbolicHotKeys"] as? [String: Any],
               let spotlight = all["64"] as? [String: Any],
               spotlight["enabled"] as? Bool == true {
                return "Spotlight"
            }
        }
        return nil
    }

    func probeJSON() -> String {
        let body: [String: Any] = [
            "schema": 1,
            "selected_backend": selectedBackend().rawValue,
            "selected_hotkey": selectedHotKey().canonical,
            "ghostty_available": ghosttyURL() != nil,
            "effective_backend": effectiveBackend().rawValue,
            "launcher_active": lease.active(),
            "known_hotkey_owner": knownHotKeyOwner() ?? NSNull(),
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
        if lease.active() {
            focusEffectiveTerminal()
            status.write(event: "already-open", backend: effectiveBackend().rawValue, ok: true)
            completion(true)
            return
        }
        guard let token = lease.claim() else {
            let message = "Prelude could not create its private launcher lease"
            status.write(event: "open-failed", backend: effectiveBackend().rawValue, ok: false, detail: message)
            if showErrors { alert(message) }
            completion(false)
            return
        }
        openClaimedTerminal(token: token, showErrors: showErrors) { [weak self] ok in
            if !ok { self?.lease.release(token) }
            completion(ok)
        }
    }

    private func openClaimedTerminal(token: String, showErrors: Bool, completion: @escaping (Bool) -> Void) {
        let selected = selectedBackend()
        switch selected {
        case .auto:
            if let ghostty = ghosttyURL() {
                openGhostty(ghostty, token: token) { [weak self] ok, detail in
                    guard let self else { completion(false); return }
                    if ok {
                        self.lease.setBackend(.ghostty, token: token)
                        self.status.write(event: "opened", backend: "ghostty", ok: true)
                        completion(true)
                    } else {
                        self.openTerminalApp(token: token) { fallbackOK, fallbackDetail in
                            if fallbackOK { self.lease.setBackend(.terminal, token: token) }
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
                openTerminalApp(token: token) { [weak self] ok, detail in
                    if ok { self?.lease.setBackend(.terminal, token: token) }
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
            openGhostty(ghostty, token: token) { [weak self] ok, detail in
                if ok { self?.lease.setBackend(.ghostty, token: token) }
                self?.status.write(event: ok ? "opened" : "open-failed", backend: "ghostty", ok: ok, detail: detail)
                if !ok && showErrors { self?.alert(detail) }
                completion(ok)
            }
        case .terminal:
            openTerminalApp(token: token) { [weak self] ok, detail in
                if ok { self?.lease.setBackend(.terminal, token: token) }
                self?.status.write(event: ok ? "opened" : "open-failed", backend: "terminal", ok: ok, detail: detail)
                if !ok && showErrors { self?.alert(detail) }
                completion(ok)
            }
        }
    }

    private func focusEffectiveTerminal() {
        if let activeApplication, !activeApplication.isTerminated {
            activeApplication.activate(options: [.activateAllWindows])
            return
        }
        let active = lease.backend() ?? effectiveBackend()
        let bundle = active == .ghostty ? ghosttyBundleID : terminalBundleID
        NSRunningApplication.runningApplications(withBundleIdentifier: bundle)
            .last(where: { !$0.isTerminated })?
            .activate(options: [.activateAllWindows])
    }

    private func openGhostty(_ url: URL, token: String, completion: @escaping (Bool, String) -> Void) {
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        configuration.createsNewApplicationInstance = true
        configuration.arguments = [
            "--working-directory=\(FileManager.default.homeDirectoryForCurrentUser.path)",
            "-e",
            "/usr/bin/env",
            "PRELUDE_AUTOSTART=1",
            "PRELUDE_GLOBAL_TOKEN=\(token)",
            "/bin/zsh",
            "-il",
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
        NSWorkspace.shared.openApplication(at: url, configuration: configuration) { [weak self] application, error in
            if let error {
                finish(false, error.localizedDescription)
            } else {
                self?.activeApplication = application
                finish(true, "")
            }
        }
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 10) {
            finish(false, "Ghostty did not answer within 10 seconds")
        }
    }

    private func openTerminalApp(token: String, completion: @escaping (Bool, String) -> Void) {
        // Only a generated UUID joins this fixed bootstrap. Selected launcher
        // payloads never cross the Apple Event boundary or enter a process
        // command line.
        let bootstrap = "exec /usr/bin/env PRELUDE_AUTOSTART=1 PRELUDE_GLOBAL_TOKEN=\(token) /bin/zsh -il"
        let source = """
        tell application id "\(terminalBundleID)"
            do script "\(bootstrap)"
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
        process.terminationHandler = { [weak self] task in
            let data = errors.fileHandleForReading.readDataToEndOfFile()
            let detail = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if task.terminationStatus == 0 {
                self?.activeApplication = NSRunningApplication
                    .runningApplications(withBundleIdentifier: terminalBundleID)
                    .last(where: { !$0.isTerminated })
            }
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
            alert.messageText = "Prelude could not register its global hotkey"
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
        let spec = launcher.selectedHotKey()
        if let owner = launcher.knownHotKeyOwner() {
            StatusWriter(path: AppPaths.bundled().status).write(
                event: "registration-failed", backend: "", ok: false,
                detail: "\(spec.canonical) is already configured for \(owner)"
            )
            launcher.fatalHotKeyError(
                "\(spec.canonical) is already configured for \(owner). Change it there or choose another with `prelude global hotkey HOTKEY`."
            )
            return
        }
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
            spec.keyCode, spec.modifiers, id,
            GetApplicationEventTarget(), 0, &hotKey
        )
        guard registered == noErr else {
            StatusWriter(path: AppPaths.bundled().status).write(
                event: "registration-failed", backend: "", ok: false,
                detail: "\(spec.canonical) is already owned by macOS or another application (\(registered))"
            )
            launcher.fatalHotKeyError(
                "\(spec.canonical) is already assigned to macOS or another application. Free it or choose another with `prelude global hotkey HOTKEY`, then run `prelude global start`."
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
if arguments.contains("--check-hotkey") {
    if launcher.knownHotKeyOwner() != nil { exit(3) }
    let spec = launcher.selectedHotKey()
    let id = EventHotKeyID(signature: hotKeySignature, id: 99)
    var reference: EventHotKeyRef?
    let result = RegisterEventHotKey(
        spec.keyCode, spec.modifiers, id,
        GetApplicationEventTarget(), 0, &reference
    )
    if let reference { UnregisterEventHotKey(reference) }
    exit(result == noErr ? 0 : 3)
}
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
