// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, you can obtain one at https://mozilla.org/MPL/2.0/.

import Darwin
import Foundation

enum ProbeError: Error {
    case unsupportedVersion, invalidArguments, staleRevision, coordinationFailed
    case divergentField(String), invalidSnapshotFile, unknownSnapshotFields, snapshotAlreadyExists
}

enum Palette: String, Codable {
    case graphite, midnight, paper, system
}

enum PageZoom: String, Codable {
    case small = "75", reduced = "90", standard = "100", enlarged = "110"
    case large = "125", extraLarge = "150", double = "200"
}

struct ProfileAppearance: Decodable {
    let palette: Palette
    let compactTabs: Bool
    let pageZoom: PageZoom?

    enum CodingKeys: String, CodingKey {
        case palette
        case compactTabs = "compact_tabs"
        case pageZoom = "page_zoom"
    }
}

struct Snapshot: Codable {
    let version: Int
    var revision: UUID
    var palette: Palette
    var compactTabs: Bool
    var pageZoom: PageZoom?
    var bookmarks: [String: String]

    static let maximumBytes = 4 * 1024 * 1024

    static func read(_ url: URL) throws -> Snapshot {
        let descriptor = open(url.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
        guard descriptor >= 0 else { throw ProbeError.invalidSnapshotFile }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        defer { try? handle.close() }
        var info = stat()
        guard fstat(descriptor, &info) == 0, info.st_mode & S_IFMT == S_IFREG,
              info.st_size <= Self.maximumBytes else {
            throw ProbeError.invalidSnapshotFile
        }
        let bytes = try readBounded(handle, maximumBytes: maximumBytes)
        let expected: Set<String> = ["version", "revision", "palette", "compactTabs", "bookmarks"]
        guard let fields = try JSONSerialization.jsonObject(with: bytes) as? [String: Any],
              Set(fields.keys).subtracting(["pageZoom"]) == expected,
              !(fields["pageZoom"] is NSNull) else { throw ProbeError.unknownSnapshotFields }
        let snapshot = try JSONDecoder().decode(Self.self, from: bytes)
        try snapshot.validate()
        return snapshot
    }

    func validate() throws {
        guard version == 1 else { throw ProbeError.unsupportedVersion }
        guard bookmarks.count <= 500, bookmarks.allSatisfy({ address, title in
            guard address.utf8.count <= 1024, title.utf8.count <= 512,
                  title == title.trimmingCharacters(in: .whitespacesAndNewlines),
                  !title.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
                  !address.contains("\\"),
                  !address.unicodeScalars.contains(where: CharacterSet.whitespacesAndNewlines.union(.controlCharacters).contains),
                  let url = URLComponents(string: address), url.string == address,
                  url.scheme == "https", url.user == nil, url.password == nil,
                  url.port == nil || url.port == 443, let host = url.host?.lowercased()
            else { return false }
            let domain = String(host.reversed().drop(while: { $0 == "." }).reversed())
            return domain.contains(".") && !domain.hasSuffix(".localhost") && !domain.hasSuffix(".local")
                && domain.split(separator: ".").last?.first?.isLetter == true
        }) else { throw ProbeError.invalidArguments }
    }

    func write(_ url: URL) throws {
        try validate()
        let data = try JSONEncoder().encode(self)
        guard data.count <= Self.maximumBytes else { throw ProbeError.invalidArguments }
        try data.write(to: url, options: .atomic)
    }
}

func readBounded(_ handle: FileHandle, maximumBytes: Int) throws -> Data {
    var bytes = Data()
    while bytes.count <= maximumBytes {
        let length = min(64 * 1024, maximumBytes + 1 - bytes.count)
        guard let chunk = try handle.read(upToCount: length), !chunk.isEmpty else { break }
        bytes.append(chunk)
    }
    guard bytes.count <= maximumBytes else { throw ProbeError.invalidSnapshotFile }
    return bytes
}

func importBookmarks() throws -> [String: String] {
    let bytes = try readBounded(.standardInput, maximumBytes: Snapshot.maximumBytes)
    let entries = try JSONDecoder().decode([[String: String]].self, from: bytes)
    guard entries.count <= 500 else { throw ProbeError.invalidArguments }

    var bookmarks: [String: String] = [:]
    for entry in entries {
        guard entry.count == 2, let address = entry["url"], let title = entry["title"],
              bookmarks[address] == nil else { throw ProbeError.invalidArguments }
        bookmarks[address] = title
    }
    return bookmarks
}

func coordinateWrite(_ url: URL, action: @escaping (URL) throws -> Void) throws {
    let coordinator = NSFileCoordinator(filePresenter: nil)
    var coordinationError: NSError?
    var result: Result<Void, Error>?
    coordinator.coordinate(writingItemAt: url, options: .forReplacing, error: &coordinationError) { coordinatedURL in
        result = Result { try action(coordinatedURL) }
    }
    if let coordinationError { throw coordinationError }
    guard let result else { throw ProbeError.coordinationFailed }
    try result.get()
}

func initialize(_ url: URL, snapshot: Snapshot) throws {
    try coordinateWrite(url) { destination in
        let staging = destination.deletingLastPathComponent()
            .appendingPathComponent(".search-sync-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: staging) }
        try snapshot.write(staging)
        // Publish the complete file without replacing an existing snapshot.
        guard link(staging.path, destination.path) == 0 else {
            let code = errno
            if code == EEXIST { throw ProbeError.snapshotAlreadyExists }
            throw NSError(domain: NSPOSIXErrorDomain, code: Int(code))
        }
    }
}

func mutate(_ url: URL, change: @escaping (inout Snapshot) throws -> Void) throws {
    try coordinateWrite(url) { coordinatedURL in
        var current = try Snapshot.read(coordinatedURL)
        let before = current
        try change(&current)
        guard before.palette != current.palette || before.compactTabs != current.compactTabs ||
                before.pageZoom != current.pageZoom || before.bookmarks != current.bookmarks else { return }
        current.revision = UUID()
        try current.write(coordinatedURL)
    }
}

func update(_ url: URL, expected: UUID, change: @escaping (inout Snapshot) -> Void) throws {
    try mutate(url) { current in
        guard current.revision == expected else { throw ProbeError.staleRevision }
        change(&current)
    }
}

func merge<Value: Equatable>(_ base: Value, _ pending: Value, _ shared: Value, field: String) throws -> Value {
    if pending == base { return shared }
    if shared == base || shared == pending { return pending }
    throw ProbeError.divergentField(field)
}

func reconcile(_ url: URL, base: Snapshot, pending: Snapshot) throws {
    try mutate(url) { shared in
        shared.palette = try merge(base.palette, pending.palette, shared.palette, field: "palette")
        shared.compactTabs = try merge(base.compactTabs, pending.compactTabs, shared.compactTabs, field: "compactTabs")
        shared.pageZoom = try merge(base.pageZoom, pending.pageZoom, shared.pageZoom, field: "pageZoom")
        for address in Set(base.bookmarks.keys).union(pending.bookmarks.keys).sorted() {
            shared.bookmarks[address] = try merge(base.bookmarks[address], pending.bookmarks[address],
                                                   shared.bookmarks[address], field: "bookmarks")
        }
    }
}

do {
    let arguments = Array(CommandLine.arguments.dropFirst())
    guard arguments.count >= 2 else { throw ProbeError.invalidArguments }
    let url = URL(fileURLWithPath: arguments[1])
    switch arguments[0] {
    case "seed" where arguments.count == 2:
        let snapshot = Snapshot(version: 1, revision: UUID(), palette: .graphite,
                                compactTabs: true, pageZoom: .standard, bookmarks: ["https://example.test/": "Fixture"])
        try initialize(url, snapshot: snapshot)
        print(snapshot.revision.uuidString)
    case "palette" where arguments.count == 4:
        guard let revision = UUID(uuidString: arguments[2]), let palette = Palette(rawValue: arguments[3])
        else { throw ProbeError.invalidArguments }
        try update(url, expected: revision) { $0.palette = palette }
    case "import-bookmarks" where arguments.count == 3:
        guard let revision = UUID(uuidString: arguments[2]) else { throw ProbeError.invalidArguments }
        let bookmarks = try importBookmarks()
        try update(url, expected: revision) { $0.bookmarks = bookmarks }
    case "import-settings" where arguments.count == 3:
        guard let revision = UUID(uuidString: arguments[2]) else { throw ProbeError.invalidArguments }
        let bytes = try readBounded(.standardInput, maximumBytes: 4096)
        let settings = try JSONDecoder().decode(ProfileAppearance.self, from: bytes)
        try update(url, expected: revision) {
            $0.palette = settings.palette
            $0.compactTabs = settings.compactTabs
            if let zoom = settings.pageZoom {
                $0.pageZoom = zoom
            }
        }
    case "apply-settings" where arguments.count == 2:
        let snapshot = try Snapshot.read(url)
        let bytes = try readBounded(.standardInput, maximumBytes: 4096)
        guard var settings = try JSONSerialization.jsonObject(with: bytes) as? [String: Any]
        else { throw ProbeError.invalidArguments }
        settings["palette"] = snapshot.palette.rawValue
        settings["compact_tabs"] = snapshot.compactTabs
        if let zoom = snapshot.pageZoom {
            settings["page_zoom"] = zoom.rawValue
        }
        FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: settings, options: .sortedKeys))
    case "export-bookmarks" where arguments.count == 2:
        let snapshot = try Snapshot.read(url)
        let bookmarks = snapshot.bookmarks.keys.sorted().map { address in
            ["url": address, "title": snapshot.bookmarks[address, default: ""]]
        }
        FileHandle.standardOutput.write(try JSONEncoder().encode(bookmarks))
    case "page-zoom" where arguments.count == 4:
        guard let revision = UUID(uuidString: arguments[2]), let zoom = PageZoom(rawValue: arguments[3])
        else { throw ProbeError.invalidArguments }
        try update(url, expected: revision) { $0.pageZoom = zoom }
    case "bookmark" where arguments.count == 5:
        guard let revision = UUID(uuidString: arguments[2]), let bookmark = URL(string: arguments[3]),
              bookmark.scheme == "https", bookmark.host != nil, bookmark.user == nil, bookmark.password == nil
        else { throw ProbeError.invalidArguments }
        try update(url, expected: revision) { $0.bookmarks[bookmark.absoluteString] = arguments[4] }
    case "reconcile" where arguments.count == 4:
        let base = try Snapshot.read(URL(fileURLWithPath: arguments[2]))
        let pending = try Snapshot.read(URL(fileURLWithPath: arguments[3]))
        try reconcile(url, base: base, pending: pending)
    default:
        throw ProbeError.invalidArguments
    }
} catch ProbeError.divergentField(let field) {
    print("conflict: \(field)")
    exit(3)
} catch ProbeError.staleRevision {
    print("conflict")
    exit(3)
} catch {
    print(error)
    exit(1)
}
