import SwiftUI
import Translation

func note(_ s: String) {
    if ProcessInfo.processInfo.environment["TRX_DEBUG"] != nil {
        FileHandle.standardError.write("[tr] \(s)\n".data(using: .utf8)!)
    }
}

struct Host: View {
    let text: String, target: String, source: Locale.Language?
    @State private var config: TranslationSession.Configuration?
    var body: some View {
        Color.clear.frame(width: 1, height: 1)
            .translationTask(config) { session in
                note("session obtained")
                do {
                    let r = try await session.translate(text)
                    print(r.targetText)
                    exit(0)
                } catch {
                    note("translate failed: \(error)")
                    exit(1)
                }
            }
            .onAppear {
                note("onAppear -> target=\(target) source=\(source?.maximalIdentifier ?? "auto")")
                config = .init(source: source, target: Locale.Language(identifier: target))
            }
    }
}

let args = CommandLine.arguments
guard args.count >= 4 else {
    FileHandle.standardError.write(
        "usage: trtranslate <target-lang> <source-lang|auto> <text>\n".data(using: .utf8)!)
    exit(2)
}
let target = args[1]
// Auto-detection stalls forever on very short input ("hello"), so the caller
// may pin the source language instead of passing "auto".
let source = args[2] == "auto" ? nil : Locale.Language(identifier: args[2])
let text = args[3...].joined(separator: " ")

// Give up rather than hang forever if the language assets aren't installed.
let limit = Double(ProcessInfo.processInfo.environment["TRX_TR_TIMEOUT"] ?? "8") ?? 8
DispatchQueue.global().asyncAfter(deadline: .now() + limit) {
    note("timed out after \(limit)s")
    exit(3)
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)   // no Dock icon, no menu bar
// The view must actually render for .onAppear/.translationTask to fire, so the
// window stays "visible" — just parked far off-screen where nobody sees it.
let w = NSWindow(contentRect: .init(x: -20000, y: -20000, width: 1, height: 1),
                 styleMask: [.borderless], backing: .buffered, defer: false)
w.contentView = NSHostingView(rootView: Host(text: text, target: target, source: source))
w.alphaValue = 0
w.orderFrontRegardless()
note("app running")
app.run()
