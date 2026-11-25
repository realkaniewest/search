// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import AppKit
import WebKit

enum ProbeError: Error {
    case wrongSheet, wrongDecision, missingNavigation
}

@MainActor
final class BrowserFixture: NSObject, WKNavigationDelegate {
    let view: WKWebView
    let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 640, height: 420),
        styleMask: [.titled, .closable], backing: .buffered, defer: false
    )
    private var navigation = 0
    private var pending: NSAlert?
    private var testLateClick: ((NSAlert) -> Void)?

    override init() {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        view = WKWebView(frame: .zero, configuration: configuration)
        super.init()
        window.title = "Search · Проверка диалога"
        window.contentView = view
        view.navigationDelegate = self
        window.center()
        window.orderFront(nil)
    }

    func webView(_ webView: WKWebView, didStartProvisionalNavigation _: WKNavigation!) {
        navigation += 1
        guard let alert = pending else { return }
        if let lateClick = testLateClick {
            testLateClick = nil
            lateClick(alert)
        } else {
            window.endSheet(alert.window, returnCode: .abort)
        }
    }

    func decision(for origin: String, simulate: @escaping (NSAlert) -> Void) async -> Bool {
        let requestedNavigation = navigation
        let alert = NSAlert()
        alert.messageText = "Разрешить доступ к микрофону?"
        alert.informativeText = origin
        alert.addButton(withTitle: "Запретить").keyEquivalent = "\r"
        alert.addButton(withTitle: "Разрешить").keyEquivalent = ""
        let result = await withCheckedContinuation { continuation in
            pending = alert
            alert.beginSheetModal(for: window) { response in
                self.pending = nil
                continuation.resume(returning: response)
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(250)) {
                simulate(alert)
            }
        }
        return result == .alertSecondButtonReturn && navigation == requestedNavigation
    }

    func run() async throws {
        let origin = "https://permission-fixture.example.test:443"
        var sheetVerified = false
        let allowed = await decision(for: origin) { alert in
            sheetVerified = self.window.attachedSheet === alert.window &&
                alert.informativeText == origin && alert.buttons[0].keyEquivalent == "\r"
            alert.buttons[1].performClick(nil)
        }
        guard sheetVerified else { throw ProbeError.wrongSheet }
        guard allowed else { throw ProbeError.wrongDecision }

        let denied = await decision(for: origin) { alert in
            alert.buttons[0].performClick(nil)
        }
        guard !denied else { throw ProbeError.wrongDecision }

        let before = navigation
        let stale = await decision(for: origin) { _ in
            self.view.loadHTMLString("<title>New document</title>", baseURL: nil)
        }
        guard navigation > before else { throw ProbeError.missingNavigation }
        guard !stale else { throw ProbeError.wrongDecision }
        let ignored = await decision(for: origin) { _ in
            self.testLateClick = { alert in alert.buttons[1].performClick(nil) }
            self.view.loadHTMLString("<title>Another document</title>", baseURL: nil)
        }
        guard !ignored else { throw ProbeError.wrongDecision }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
Task { @MainActor in
    let fixture = BrowserFixture()
    do {
        try await fixture.run()
        fixture.window.close()
        print("PASS: native sheet displayed the origin, denied by default, and closed on WebKit navigation")
        exit(0)
    } catch {
        fixture.window.close()
        print("FAIL: permission sheet fixture:", error)
        exit(1)
    }
}
DispatchQueue.main.asyncAfter(deadline: .now() + 15) {
    print("FAIL: permission sheet fixture timed out")
    exit(1)
}
app.run()
