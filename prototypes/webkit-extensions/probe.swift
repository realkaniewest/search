// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

import AppKit
import WebKit

enum ProbeError: Error {
    case controllerIsolation, contextStillLoaded
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
Task { @MainActor in
    guard #available(macOS 15.4, *) else { print("Requires macOS 15.4"); exit(2) }
    let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    var status: Int32 = 1
    do {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let manifest = """
        {"manifest_version":3,"name":"Search local test","version":"1.0","description":"Local test only"}
        """
        try manifest.write(to: folder.appendingPathComponent("manifest.json"), atomically: true, encoding: .utf8)
        let item = try await WKWebExtension(resourceBaseURL: folder)
        let context = WKWebExtensionContext(for: item)
        let controller = WKWebExtensionController()
        let normalConfig = WKWebViewConfiguration()
        normalConfig.websiteDataStore = .nonPersistent()
        normalConfig.webExtensionController = controller
        let isolatedConfig = WKWebViewConfiguration()
        isolatedConfig.websiteDataStore = .nonPersistent()
        let normal = WKWebView(frame: .zero, configuration: normalConfig)
        let isolated = WKWebView(frame: .zero, configuration: isolatedConfig)
        try controller.load(context)
        guard context.isLoaded, normal.configuration.webExtensionController === controller,
              isolated.configuration.webExtensionController == nil else {
            throw ProbeError.controllerIsolation
        }
        try controller.unload(context)
        guard !context.isLoaded else { throw ProbeError.contextStillLoaded }
        status = 0
        print("PASS: WebKit loaded and unloaded a local MV3 extension in an isolated test controller")
    } catch {
        print("Extension test failed:", error)
    }
    exit(status)
}
DispatchQueue.main.asyncAfter(deadline: .now() + 15) { print("Extension test timed out"); exit(1) }
app.run()
