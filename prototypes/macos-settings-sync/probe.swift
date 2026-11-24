// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, you can obtain one at https://mozilla.org/MPL/2.0/.

import Foundation

enum ProbeError: Error {
    case unsupportedVersion, invalidArguments, staleRevision, coordinationFailed
}

enum Palette: String, Codable {
    case graphite, midnight, paper, system
}

struct Snapshot: Codable {
    let version: Int
    var revision: UUID
    var palette: Palette
    var compactTabs: Bool
    var bookmarks: [String: String]

    static func read(_ url: URL) throws -> Snapshot {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }
        let limit = 4 * 1024 * 1024
        let bytes = try handle.read(upToCount: limit + 1) ?? Data()
        guard bytes.count <= limit else { throw ProbeError.invalidArguments }
        let snapshot = try JSONDecoder().decode(Self.self, from: bytes)
        guard snapshot.version == 1 else { throw ProbeError.unsupportedVersion }
        return snapshot
    }

    func write(_ url: URL) throws {
        try JSONEncoder().encode(self).write(to: url, options: .atomic)
    }
}

func update(_ url: URL, expected: UUID, change: @escaping (inout Snapshot) -> Void) throws {
    let coordinator = NSFileCoordinator(filePresenter: nil)
    var coordinationError: NSError?
    var result: Result<Void, Error>?
    coordinator.coordinate(writingItemAt: url, options: .forReplacing, error: &coordinationError) { coordinatedURL in
        result = Result {
            var current = try Snapshot.read(coordinatedURL)
            guard current.revision == expected else { throw ProbeError.staleRevision }
            change(&current)
            current.revision = UUID()
            try current.write(coordinatedURL)
        }
    }
    if let coordinationError { throw coordinationError }
    guard let result else { throw ProbeError.coordinationFailed }
    try result.get()
}

do {
    let arguments = Array(CommandLine.arguments.dropFirst())
    guard arguments.count >= 2 else { throw ProbeError.invalidArguments }
    let url = URL(fileURLWithPath: arguments[1])
    switch arguments[0] {
    case "seed" where arguments.count == 2:
        let snapshot = Snapshot(version: 1, revision: UUID(), palette: .graphite,
                                compactTabs: true, bookmarks: ["https://example.test/": "Fixture"])
        try snapshot.write(url)
        print(snapshot.revision.uuidString)
    case "palette" where arguments.count == 4:
        guard let revision = UUID(uuidString: arguments[2]), let palette = Palette(rawValue: arguments[3])
        else { throw ProbeError.invalidArguments }
        try update(url, expected: revision) { $0.palette = palette }
    case "bookmark" where arguments.count == 5:
        guard let revision = UUID(uuidString: arguments[2]), let bookmark = URL(string: arguments[3]),
              bookmark.scheme == "https", bookmark.host != nil, bookmark.user == nil, bookmark.password == nil
        else { throw ProbeError.invalidArguments }
        try update(url, expected: revision) { $0.bookmarks[bookmark.absoluteString] = arguments[4] }
    default:
        throw ProbeError.invalidArguments
    }
} catch ProbeError.staleRevision {
    print("conflict")
    exit(3)
} catch {
    print(error)
    exit(1)
}
