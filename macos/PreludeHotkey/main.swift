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

    static let cocoaShift: UInt32 = 1 << 17
    static let cocoaControl: UInt32 = 1 << 18
    static let cocoaOption: UInt32 = 1 << 19
    static let cocoaCommand: UInt32 = 1 << 20
    static let cocoaModifiers: UInt32 = cocoaShift | cocoaControl | cocoaOption | cocoaCommand

    /// The chord as macOS's own shortcut table records it: Cocoa event flags,
    /// not the Carbon bits `RegisterEventHotKey` takes.
    var cocoaMask: UInt32 {
        var mask: UInt32 = 0
        if modifiers & UInt32(shiftKey) != 0 { mask |= HotKeySpec.cocoaShift }
        if modifiers & UInt32(controlKey) != 0 { mask |= HotKeySpec.cocoaControl }
        if modifiers & UInt32(optionKey) != 0 { mask |= HotKeySpec.cocoaOption }
        if modifiers & UInt32(cmdKey) != 0 { mask |= HotKeySpec.cocoaCommand }
        return mask
    }

    /// Only the handful worth naming are named; the rest are reported by id
    /// rather than guessed at.
    static func symbolicName(_ id: String) -> String {
        switch id {
        case "60": return "the macOS previous-input-source shortcut"
        case "61": return "the macOS next-input-source shortcut"
        case "64": return "Spotlight"
        case "65": return "the Spotlight file search window"
        default: return "a macOS keyboard shortcut (id \(id))"
        }
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
    // A claim that never produces a shell is a launch that did not happen. The
    // grace window is long enough for a cold terminal and short enough that
    // such a launch cannot hold the hotkey for the rest of the afternoon.
    private let grace: TimeInterval = 20
    // A backstop against a recycled pid, not against an open launcher.
    private let lifetime: TimeInterval = 30 * 60

    init(path: String) { self.path = path }

    private func fields() -> [String]? {
        guard !path.isEmpty,
              let contents = try? String(contentsOfFile: path, encoding: .utf8) else { return nil }
        return contents.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
    }

    private func drop() {
        guard !path.isEmpty else { return }
        try? FileManager.default.removeItem(atPath: path)
    }

    /// The shell a launch creates writes its own pid into the lease, and that
    /// pid is the only thing here that can be asked whether it is still alive.
    /// A terminal killed outright therefore frees the launcher at once instead
    /// of waiting out a timeout — which is what a stale lease used to do.
    func active() -> Bool {
        guard !path.isEmpty else { return false }
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: path),
              let modified = attributes[.modificationDate] as? Date,
              Date().timeIntervalSince(modified) < lifetime,
              let fields = fields(),
              let token = fields.first,
              !token.isEmpty else {
            drop()
            return false
        }
        guard fields.count > 2, !fields[2].isEmpty else {
            if Date().timeIntervalSince(modified) < grace { return true }
            drop()
            return false
        }
        guard let pid = pid_t(fields[2]), pid > 0, kill(pid, 0) == 0 || errno == EPERM else {
            drop()
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
        guard let fields = fields(), fields.first == token else { return }
        // The shell may already have claimed the lease; never write its pid away.
        var body = "\(token)\n\(backend.rawValue)\n"
        if fields.count > 2, !fields[2].isEmpty { body += "\(fields[2])\n" }
        do {
            try Data(body.utf8).write(to: URL(fileURLWithPath: path), options: [.atomic])
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
        } catch { return }
    }

    func backend() -> Backend? {
        guard let fields = fields(), fields.count > 1 else { return nil }
        return Backend(rawValue: fields[1])
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

    /// Where a launcher window starts. The CLI validates this on the way in;
    /// check it again here so a hand-edited config cannot send an unexpected
    /// path into an Apple Event, and so a directory removed after it was
    /// configured degrades to `$HOME` rather than failing the launch.
    func launchDirectory() -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        guard let value = configValue("directory"), !value.isEmpty, value.hasPrefix("/"),
              !value.contains(where: { "\"'`$\\\n\r".contains($0) }) else { return home }
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: value, isDirectory: &isDirectory),
              isDirectory.boolValue else { return home }
        return value
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
        if let owner = systemOwner(of: spec, in: preferences) { return owner }
        let raycast = preferences.appendingPathComponent("com.raycast.macos.plist")
        if let values = NSDictionary(contentsOf: raycast),
           let value = values["raycastGlobalHotkey"] as? String,
           spec.matchesRaycast(value) {
            return "Raycast"
        }
        return nil
    }

    /// macOS records a symbolic hotkey only once it differs from the default,
    /// so asking whether Spotlight's entry is enabled answers nothing when the
    /// entry is absent — which is the ordinary case for an untouched Mac. Read
    /// the whole table, and fall back to the defaults for the ids a launcher
    /// chord collides with.
    private func systemOwner(of spec: HotKeySpec, in preferences: URL) -> String? {
        let defaults: [(String, UInt32, UInt32)] = [
            ("60", 49, HotKeySpec.cocoaControl),
            ("61", 49, HotKeySpec.cocoaControl | HotKeySpec.cocoaOption),
            ("64", 49, HotKeySpec.cocoaCommand),
            ("65", 49, HotKeySpec.cocoaCommand | HotKeySpec.cocoaOption),
        ]
        let table = NSDictionary(contentsOf: preferences.appendingPathComponent("com.apple.symbolichotkeys.plist"))
        let recorded = (table?["AppleSymbolicHotKeys"] as? [String: Any]) ?? [:]
        for (id, entry) in recorded {
            guard let entry = entry as? [String: Any],
                  (entry["enabled"] as? Bool) ?? true,
                  let value = entry["value"] as? [String: Any],
                  let parameters = value["parameters"] as? [NSNumber],
                  parameters.count > 2 else { continue }
            let code = parameters[1].uint32Value
            let mask = parameters[2].uint32Value & HotKeySpec.cocoaModifiers
            if code == spec.keyCode && mask == spec.cocoaMask {
                return HotKeySpec.symbolicName(id)
            }
        }
        for (id, code, mask) in defaults
        where recorded[id] == nil && code == spec.keyCode && mask == spec.cocoaMask {
            return HotKeySpec.symbolicName(id)
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

    private func ghosttyInstances() -> Set<pid_t> {
        Set(
            NSRunningApplication.runningApplications(withBundleIdentifier: ghosttyBundleID)
                .filter { !$0.isTerminated }
                .map(\.processIdentifier)
        )
    }

    private func openGhostty(_ url: URL, token: String, completion: @escaping (Bool, String) -> Void) {
        let existing = ghosttyInstances()
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = true
        configuration.createsNewApplicationInstance = true
        configuration.arguments = [
            "--working-directory=\(launchDirectory())",
            // Ghostty's macOS build has no way to add a window to the instance
            // the person is already using, so a launch is always a second
            // instance — and a second instance restores the previous session's
            // windows and saves its own on the way out. One launcher press then
            // costs a duplicate of every window you had. Declining saved state
            // gives the one window that was asked for, leaves the state of the
            // Ghostty being worked in untouched, and lets the instance retire
            // when its shell exits instead of accumulating.
            "--window-save-state=never",
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
                return
            }
            // A status event is not proof of a terminal. Launch Services can
            // answer success having handed the request to the instance that was
            // already running, which discards these arguments and opens no
            // Prelude. The pid it returns is the only way to tell the two apart.
            guard let application, !existing.contains(application.processIdentifier) else {
                finish(false, "Launch Services reused the running Ghostty instance and discarded the launcher command")
                return
            }
            self?.activeApplication = application
            finish(true, "")
        }
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 10) {
            finish(false, "Ghostty did not answer within 10 seconds")
        }
    }

    private func openTerminalApp(token: String, completion: @escaping (Bool, String) -> Void) {
        // Only a generated UUID and a directory the CLI already refused to
        // accept quotes in join this fixed bootstrap. Selected launcher
        // payloads never cross the Apple Event boundary or enter a process
        // command line.
        let bootstrap = "cd '\(launchDirectory())' && exec /usr/bin/env PRELUDE_AUTOSTART=1 PRELUDE_GLOBAL_TOKEN=\(token) /bin/zsh -il"
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

    /// Said once, and never by exiting. The helper that quits because a chord
    /// was busy at login is a helper that stays dead after the other
    /// application releases it, and nothing short of `prelude global start`
    /// brings it back.
    func hotKeyWarning(_ message: String) {
        DispatchQueue.main.async {
            NSApp.activate(ignoringOtherApps: true)
            let alert = NSAlert()
            alert.messageText = "Prelude could not register its global hotkey"
            alert.informativeText = message
            alert.alertStyle = .warning
            alert.addButton(withTitle: "OK")
            alert.runModal()
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
    private var retry: Timer?
    private var warned = false

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
            // Nothing to retry: without a handler no hotkey can ever arrive.
            StatusWriter(path: AppPaths.bundled().status).write(
                event: "registration-failed", backend: "", ok: false,
                detail: "could not install the macOS hotkey event handler (\(install))"
            )
            launcher.hotKeyWarning("macOS refused to install Prelude's hotkey event handler (\(install)).")
            return
        }
        register()
    }

    /// Registration is attempted, not asserted. Raycast, Spotlight or any other
    /// owner may hold the chord at login and release it later; keep asking.
    private func register() {
        let spec = launcher.selectedHotKey()
        if let owner = launcher.knownHotKeyOwner() {
            registrationFailed(
                "\(spec.canonical) is already configured for \(owner)",
                "\(spec.canonical) is already configured for \(owner). Change it there or choose another with `prelude global hotkey HOTKEY`. Prelude keeps trying until the chord is free."
            )
            return
        }
        let id = EventHotKeyID(signature: hotKeySignature, id: 1)
        var reference: EventHotKeyRef?
        let registered = RegisterEventHotKey(
            spec.keyCode, spec.modifiers, id,
            GetApplicationEventTarget(), 0, &reference
        )
        guard registered == noErr, let reference else {
            registrationFailed(
                "\(spec.canonical) is already owned by macOS or another application (\(registered))",
                "\(spec.canonical) is already assigned to macOS or another application. Free it or choose another with `prelude global hotkey HOTKEY`. Prelude keeps trying until the chord is free."
            )
            return
        }
        hotKey = reference
        retry?.invalidate()
        retry = nil
        StatusWriter(path: AppPaths.bundled().status).write(
            event: "registered", backend: launcher.selectedBackend().rawValue, ok: true
        )
    }

    private func registrationFailed(_ detail: String, _ advice: String) {
        StatusWriter(path: AppPaths.bundled().status).write(
            event: "registration-failed", backend: "", ok: false, detail: detail
        )
        if !warned {
            warned = true
            launcher.hotKeyWarning(advice)
        }
        guard retry == nil else { return }
        retry = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            self?.register()
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        retry?.invalidate()
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
