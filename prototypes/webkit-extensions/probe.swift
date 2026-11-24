// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import AppKit
import WebKit

enum ProbeError: Error {
    case controllerIsolation, contextStillLoaded, unexpectedMarker(String), unexpectedPermissionState
}

struct FixturePermissions: Codable {
    let allowed: URL
    let denied: URL
    let grantAllowed: Bool
}

@MainActor
final class Navigation: NSObject, WKNavigationDelegate {
    var result: CheckedContinuation<Void, Error>?

    func load(_ url: URL, in view: WKWebView) async throws {
        view.navigationDelegate = self
        try await withCheckedThrowingContinuation {
            result = $0
            view.load(URLRequest(url: url))
        }
    }

    func webView(_ view: WKWebView, didFinish navigation: WKNavigation!) {
        result?.resume()
        result = nil
    }

    func webView(_ view: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        result?.resume(throwing: error)
        result = nil
    }
}

@MainActor
func check(_ label: String, url: URL, in view: WKWebView, marked: Bool) async throws {
    let navigation = Navigation()
    try await navigation.load(url, in: view)
    let actual = try await view.callAsyncJavaScript("""
        await new Promise(resolve => setTimeout(resolve, 400));
        return document.documentElement.dataset.searchExtensionProbe === 'ran';
        """, arguments: [:], in: nil, contentWorld: .page)
    guard actual as? Bool == marked else { throw ProbeError.unexpectedMarker(label) }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
Task { @MainActor in
    guard #available(macOS 15.4, *) else { print("Requires macOS 15.4"); exit(2) }
    if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--cleanup",
       let profileID = UUID(uuidString: CommandLine.arguments[2]) {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        let webKit = WKWebView(frame: .zero, configuration: config)
        do {
            try await WKWebsiteDataStore.remove(forIdentifier: profileID)
            withExtendedLifetime(webKit) {}
            exit(0)
        } catch {
            print("Fixture profile cleanup failed:", error)
            exit(1)
        }
    }
    guard CommandLine.arguments.count == 6,
          let profileID = UUID(uuidString: CommandLine.arguments[3]),
          let allowed = URL(string: CommandLine.arguments[1]), allowed.host == "localhost",
          let denied = URL(string: CommandLine.arguments[2]), denied.host == "127.0.0.1",
          allowed.scheme == "http", denied.scheme == "http" else {
        print("Run this probe with run.py")
        exit(2)
    }
    let phase = CommandLine.arguments[4]
    guard ["initial", "restart", "revoked"].contains(phase) else { exit(2) }
    let restart = phase != "initial"
    let state = URL(fileURLWithPath: CommandLine.arguments[5])
    let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    var status: Int32 = 1
    do {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let manifest = """
        {"manifest_version":3,"name":"Search local test","version":"1.0","description":"Local test only","content_scripts":[{"matches":["http://localhost/*","http://127.0.0.1/*"],"js":["marker.js"],"all_frames":true}]}
        """
        try manifest.write(to: folder.appendingPathComponent("manifest.json"), atomically: true, encoding: .utf8)
        try "document.documentElement.dataset.searchExtensionProbe = 'ran';"
            .write(to: folder.appendingPathComponent("marker.js"), atomically: true, encoding: .utf8)
        let item = try await WKWebExtension(resourceBaseURL: folder)
        let context = WKWebExtensionContext(for: item)
        context.uniqueIdentifier = "org.search.local-extension-probe"
        let controller = WKWebExtensionController(configuration: .nonPersistent())
        let normalConfig = WKWebViewConfiguration()
        normalConfig.websiteDataStore = WKWebsiteDataStore(forIdentifier: profileID)
        normalConfig.webExtensionController = controller
        let isolatedConfig = WKWebViewConfiguration()
        isolatedConfig.websiteDataStore = .nonPersistent()
        let normal = WKWebView(frame: .zero, configuration: normalConfig)
        let isolated = WKWebView(frame: .zero, configuration: isolatedConfig)
        let privateConfig = WKWebViewConfiguration()
        privateConfig.websiteDataStore = .nonPersistent()
        privateConfig.webExtensionController = controller
        let privateView = WKWebView(frame: .zero, configuration: privateConfig)
        try controller.load(context)
        guard context.isLoaded, normal.configuration.webExtensionController === controller,
              isolated.configuration.webExtensionController == nil else {
            throw ProbeError.controllerIsolation
        }
        let decisions: FixturePermissions
        if restart {
            try await check("fresh controller before restoring permissions", url: allowed, in: normal, marked: false)
            decisions = try JSONDecoder().decode(FixturePermissions.self, from: Data(contentsOf: state))
            guard decisions.allowed == allowed, decisions.denied == denied,
                  decisions.grantAllowed == (phase == "restart") else {
                throw ProbeError.unexpectedPermissionState
            }
        } else {
            decisions = FixturePermissions(allowed: allowed, denied: denied, grantAllowed: true)
        }
        context.setPermissionStatus(decisions.grantAllowed ? .grantedExplicitly : .deniedExplicitly, for: decisions.allowed)
        context.setPermissionStatus(.deniedExplicitly, for: decisions.denied)
        if phase == "revoked" {
            try await check("revocation after second restart", url: allowed, in: normal, marked: false)
        } else {
            try await check("allowed site", url: allowed, in: normal, marked: true)
            try await check("denied site", url: denied, in: normal, marked: false)
            let navigation = Navigation()
            try await navigation.load(allowed.appendingPathComponent("frames"), in: normal)
            let frameResult = try await normal.callAsyncJavaScript("""
                for (let attempt = 0; attempt < 30; attempt++) {
                    const {allowed, denied} = window.frameReports;
                    if (typeof allowed === 'boolean' && typeof denied === 'boolean') {
                        return allowed === true && denied === false;
                    }
                    await new Promise(resolve => setTimeout(resolve, 100));
                }
                return false;
                """, arguments: [:], in: nil, contentWorld: .page)
            guard frameResult as? Bool == true else {
                throw ProbeError.unexpectedMarker("same-origin and cross-origin frames")
            }
            try await check("isolated Search chrome", url: allowed.appendingPathComponent("chrome"), in: isolated, marked: false)
            try await check("private view", url: allowed, in: privateView, marked: false)
            if !restart {
                try JSONEncoder().encode(decisions).write(to: state, options: .atomic)
            }
            context.setPermissionStatus(.deniedExplicitly, for: allowed)
            try await check("revoked site", url: allowed, in: normal, marked: false)
            if restart {
                let revoked = FixturePermissions(allowed: allowed, denied: denied, grantAllowed: false)
                try JSONEncoder().encode(revoked).write(to: state, options: .atomic)
            }
        }
        try controller.unload(context)
        guard !context.isLoaded else { throw ProbeError.contextStillLoaded }
        status = 0
        print("PASS: WebKit extension permissions, phase \(phase)")
    } catch {
        print("Extension test failed:", error)
    }
    exit(status)
}
DispatchQueue.main.asyncAfter(deadline: .now() + 15) { print("Extension test timed out"); exit(1) }
app.run()
