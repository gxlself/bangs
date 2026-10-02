import Foundation
import XCTest
@testable import BangsSyncCore

// Runs every case in docs/sync-vectors.json. The Rust side (src-tauri/src/sync/ledger.rs)
// runs the same file, so the two implementations cannot drift apart unnoticed.

private struct VersionJSON: Decodable {
    var updatedAt: Int64
    var device: String
}

private struct CompareCase: Decodable {
    var name: String
    var a: VersionJSON
    var b: VersionJSON
    var aWins: Bool
}

private struct StampCase: Decodable {
    var name: String
    var prev: Int64?
    var now: Int64
    var stamp: Int64
}

private struct VersionEntry: Decodable, Equatable {
    var key: String
    var updatedAt: Int64
    var device: String
    var deleted: Bool
}

private struct ApplyCase: Decodable {
    var name: String
    var versions: [VersionEntry]
    var incoming: [SyncRecord]
    var applied: [String]
    var versionsAfter: [VersionEntry]
}

private struct Vectors: Decodable {
    var compare: [CompareCase]
    var nextStamp: [StampCase]
    var applyRemote: [ApplyCase]
}

private func loadVectors() throws -> Vectors {
    // .../packages/BangsCloud/Tests/BangsSyncCoreTests/ConformanceTests.swift -> repo root
    var url = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 {
        url = url.deletingLastPathComponent()
    }
    url = url.appendingPathComponent("docs").appendingPathComponent("sync-vectors.json")
    let data = try Data(contentsOf: url)
    return try JSONDecoder().decode(Vectors.self, from: data)
}

private func placeholder(_ entry: VersionEntry) -> SyncRecord {
    let parts = entry.key.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
    let kind = parts.count > 0 ? String(parts[0]) : ""
    let id = parts.count > 1 ? String(parts[1]) : ""
    return SyncRecord(
        kind: kind,
        id: id,
        updatedAt: entry.updatedAt,
        device: entry.device,
        deleted: entry.deleted,
        body: JSONValue.object([:]),
        asset: nil
    )
}

final class ConformanceTests: XCTestCase {
    func testCompareVectors() throws {
        let vectors = try loadVectors()
        XCTAssertFalse(vectors.compare.isEmpty)
        for item in vectors.compare {
            let a = Version(updatedAt: item.a.updatedAt, device: item.a.device)
            let b = Version(updatedAt: item.b.updatedAt, device: item.b.device)
            XCTAssertEqual(Version.wins(a, b), item.aWins, item.name)
        }
    }

    func testNextStampVectors() throws {
        let vectors = try loadVectors()
        XCTAssertFalse(vectors.nextStamp.isEmpty)
        for item in vectors.nextStamp {
            XCTAssertEqual(Version.nextStamp(prev: item.prev, now: item.now), item.stamp, item.name)
        }
    }

    func testApplyRemoteVectors() throws {
        let vectors = try loadVectors()
        XCTAssertFalse(vectors.applyRemote.isEmpty)
        for item in vectors.applyRemote {
            var seed: [String: SyncRecord] = [:]
            for entry in item.versions {
                seed[entry.key] = placeholder(entry)
            }
            var store = RecordStore(records: seed)

            let applied = store.applyRemote(item.incoming).map { $0.key }
            XCTAssertEqual(applied, item.applied, item.name)

            let after = store.records.values
                .map { VersionEntry(key: $0.key, updatedAt: $0.updatedAt, device: $0.device, deleted: $0.deleted) }
                .sorted { $0.key < $1.key }
            let expected = item.versionsAfter.sorted { $0.key < $1.key }
            XCTAssertEqual(after, expected, item.name)
        }
    }
}
