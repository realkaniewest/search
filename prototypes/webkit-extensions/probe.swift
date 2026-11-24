// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import AppKit
import WebKit

enum ProbeError: Error {
    case controllerIsolation, contextStillLoaded, unexpectedMarker(String)
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
    guard CommandLine.arguments.count == 4,
          let profileID = UUID(uuidString: CommandLine.arguments[3]),
          let allowed = URL(string: CommandLine.arguments[1]), allowed.host == "localhost",
          let denied = URL(string: CommandLine.arguments[2]), denied.host == "127.0.0.1",
          allowed.scheme == "http", denied.scheme == "http" else {
        print("Run this probe with run.py")
        exit(2)
    }
    let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    var status: Int32 = 1
    do {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let manifest = """
        {"manifest_version":3,"name":"Search local test","version":"1.0","description":"Local test only","content_scripts":[{"matches":["http://localhost/*","http://127.0.0.1/*"],"js":["marker.js"]}]}
        """
        try manifest.write(to: folder.appendingPathComponent("manifest.json"), atomically: true, encoding: .utf8)
        try "document.documentElement.dataset.searchExtensionProbe = 'ran';"
            .write(to: folder.appendingPathComponent("marker.js"), atomically: true, encoding: .utf8)
        let item = try await WKWebExtension(resourceBaseURL: folder)
        let context = WKWebExtensionContext(for: item)
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
        context.setPermissionStatus(.grantedExplicitly, for: allowed)
        context.setPermissionStatus(.deniedExplicitly, for: denied)
        try await check("allowed site", url: allowed, in: normal, marked: true)
        try await check("denied site", url: denied, in: normal, marked: false)
        try await check("isolated view", url: allowed, in: isolated, marked: false)
        try await check("private view", url: allowed, in: privateView, marked: false)
        context.setPermissionStatus(.deniedExplicitly, for: allowed)
        try await check("revoked site", url: allowed, in: normal, marked: false)
        try controller.unload(context)
        guard !context.isLoaded else { throw ProbeError.contextStillLoaded }
        status = 0
        print("PASS: WebKit executed the marker only for the allowed site and stopped after revocation")
    } catch {
        print("Extension test failed:", error)
    }
    exit(status)
}
DispatchQueue.main.asyncAfter(deadline: .now() + 15) { print("Extension test timed out"); exit(1) }
app.run()
