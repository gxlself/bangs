import Foundation
import XCTest
@testable import BangsSyncCore

final class RecordStoreTests: XCTestCase {
    private let phone = "phone-0000"
    private let mac = "mac-0000"

    private func todoBody(_ text: String) -> JSONValue {
        return JSONValue.object(["text": JSONValue.string(text), "createdAt": JSONValue.int(1)])
    }

    func testLocalUpsertCreatesAndIsNoOpOnSameBody() {
        var store = RecordStore()
        let first = store.localUpsert(kind: "todo", id: "a", body: todoBody("milk"), now: 100, device: phone)
        XCTAssertNotNil(first)
        XCTAssertEqual(first?.updatedAt, 100)
        XCTAssertEqual(first?.deleted, false)
        XCTAssertEqual(store.outbox.count, 1)

        // Same body again: nothing new to push.
        let second = store.localUpsert(kind: "todo", id: "a", body: todoBody("milk"), now: 200, device: phone)
        XCTAssertNil(second)
        XCTAssertEqual(store.records["todo:a"]?.updatedAt, 100)

        // A different body goes forward, even when the clock is behind.
        let third = store.localUpsert(kind: "todo", id: "a", body: todoBody("eggs"), now: 50, device: phone)
        XCTAssertEqual(third?.updatedAt, 101)
        XCTAssertEqual(store.live(kind: "todo").count, 1)
    }

    func testLocalDeleteWritesTombstoneOnce() {
        var store = RecordStore()
        XCTAssertNil(store.localDelete(kind: "todo", id: "a", now: 100, device: phone))

        store.localUpsert(kind: "todo", id: "a", body: todoBody("milk"), now: 100, device: phone)
        let tombstone = store.localDelete(kind: "todo", id: "a", now: 100, device: phone)
        XCTAssertEqual(tombstone?.deleted, true)
        XCTAssertEqual(tombstone?.updatedAt, 101)
        XCTAssertEqual(tombstone?.body, JSONValue.object([:]))
        XCTAssertTrue(store.live(kind: "todo").isEmpty)

        // Already a tombstone: nothing to do.
        XCTAssertNil(store.localDelete(kind: "todo", id: "a", now: 500, device: phone))
    }

    func testDeletedTodoIsNotResurrectedByAnOlderLiveRecord() {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "a", body: todoBody("milk"), now: 100, device: phone)
        store.localDelete(kind: "todo", id: "a", now: 200, device: phone)

        let zombie = SyncRecord(
            kind: "todo", id: "a", updatedAt: 150, device: mac,
            deleted: false, body: todoBody("milk"), asset: nil
        )
        let applied = store.applyRemote([zombie])
        XCTAssertTrue(applied.isEmpty)
        XCTAssertTrue(store.live(kind: "todo").isEmpty)
        XCTAssertEqual(store.records["todo:a"]?.deleted, true)
    }

    func testApplyRemoteDropsTheOlderPendingPush() {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "a", body: todoBody("mine"), now: 100, device: phone)
        XCTAssertEqual(store.outbox.count, 1)

        let theirs = SyncRecord(
            kind: "todo", id: "a", updatedAt: 300, device: mac,
            deleted: false, body: todoBody("theirs"), asset: nil
        )
        XCTAssertEqual(store.applyRemote([theirs]).count, 1)
        XCTAssertTrue(store.outbox.isEmpty)
        XCTAssertEqual(store.live(kind: "todo").first?.body, todoBody("theirs"))
    }

    func testRestoreAfterFailedPush() {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "a", body: todoBody("milk"), now: 100, device: phone)
        let batch = store.takeOutbox()
        XCTAssertEqual(batch.count, 1)
        XCTAssertTrue(store.outbox.isEmpty)
        XCTAssertEqual(store.inflight.count, 1)

        store.restore(batch)
        XCTAssertEqual(store.outbox.count, 1)
        XCTAssertTrue(store.inflight.isEmpty)
        XCTAssertEqual(store.takeOutbox(), batch)
    }

    func testRestoreByKeysAndSettling() {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "a", body: todoBody("a"), now: 100, device: phone)
        store.localUpsert(kind: "todo", id: "b", body: todoBody("b"), now: 100, device: phone)
        _ = store.takeOutbox()

        store.markSettled(keys: ["todo:a"])
        store.restoreInflight(keys: ["todo:a", "todo:b"])
        XCTAssertEqual(Array(store.outbox.keys), ["todo:b"])
        XCTAssertTrue(store.inflight.isEmpty)
    }

    func testRestoreSkipsKeysThatMovedOn() {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "a", body: todoBody("v1"), now: 100, device: phone)
        let batch = store.takeOutbox()

        // While the push was in flight the user edited the same todo again ...
        store.localUpsert(kind: "todo", id: "a", body: todoBody("v2"), now: 110, device: phone)
        store.restore(batch)
        // ... so the failed v1 must not replace the pending v2.
        XCTAssertEqual(store.outbox["todo:a"]?.body, todoBody("v2"))

        // And a remote record that won also makes a failed push moot.
        var other = RecordStore()
        other.localUpsert(kind: "todo", id: "b", body: todoBody("old"), now: 100, device: phone)
        let oldBatch = other.takeOutbox()
        let winner = SyncRecord(
            kind: "todo", id: "b", updatedAt: 500, device: mac,
            deleted: false, body: todoBody("new"), asset: nil
        )
        _ = other.applyRemote([winner])
        other.restore(oldBatch)
        XCTAssertTrue(other.outbox.isEmpty)
    }

    func testATombstoneQueuedWhileTheLineIsInFlightIsNotLost() {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "a", body: todoBody("milk"), now: 100, device: phone)
        XCTAssertEqual(store.takeOutbox().count, 1)          // the live line goes out
        store.localDelete(kind: "todo", id: "a", now: 110, device: phone)
        XCTAssertTrue(store.takeOutbox().isEmpty)            // held back while "a" is in flight
        store.markSettled(keys: ["todo:a"])                  // the live line landed
        let next = store.takeOutbox()
        XCTAssertEqual(next.count, 1)
        XCTAssertEqual(next.first?.deleted, true)            // now the tombstone goes
    }

    func testRequeuePutsLiveRecordsBack() {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "a", body: todoBody("a"), now: 100, device: phone)
        store.localUpsert(kind: "todo", id: "b", body: todoBody("b"), now: 100, device: phone)
        store.localDelete(kind: "todo", id: "b", now: 110, device: phone)
        _ = store.takeOutbox()
        store.markSettled(keys: ["todo:a", "todo:b"])
        store.requeue(kind: "todo")
        XCTAssertEqual(Array(store.outbox.keys), ["todo:a"])
    }

    func testPurgeTombstones() {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "old", body: todoBody("x"), now: 10, device: phone)
        store.localDelete(kind: "todo", id: "old", now: 20, device: phone)
        store.localUpsert(kind: "todo", id: "live", body: todoBody("y"), now: 10, device: phone)

        // The tombstone is still waiting to be pushed: it stays.
        XCTAssertEqual(store.purgeTombstones(now: 1_000, ttlMs: 100), 0)

        _ = store.takeOutbox()
        store.markSettled(keys: ["todo:old", "todo:live"])
        XCTAssertEqual(store.purgeTombstones(now: 1_000, ttlMs: 100), 1)
        XCTAssertNil(store.records["todo:old"])
        XCTAssertNotNil(store.records["todo:live"])

        // Young tombstones stay.
        store.localDelete(kind: "todo", id: "live", now: 990, device: phone)
        _ = store.takeOutbox()
        store.markSettled(keys: ["todo:live"])
        XCTAssertEqual(store.purgeTombstones(now: 1_000, ttlMs: 100), 0)
    }

    func testStoreFileRoundTripKeepsInflightInTheOutbox() throws {
        var store = RecordStore()
        store.localUpsert(kind: "todo", id: "a", body: todoBody("a"), now: 100, device: phone)
        store.localUpsert(kind: "todo", id: "b", body: todoBody("b"), now: 100, device: phone)
        _ = store.takeOutbox()
        store.localUpsert(kind: "todo", id: "c", body: todoBody("c"), now: 100, device: phone)

        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("BangsSyncCoreTests-" + UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("store.json")

        // Missing file: empty store.
        XCTAssertEqual(try RecordStore.load(from: file), RecordStore())

        try store.save(to: file)
        let loaded = try RecordStore.load(from: file)
        XCTAssertEqual(loaded.records, store.records)
        // Whatever was in flight when the file was written is pending again.
        XCTAssertEqual(Set(loaded.outbox.keys), Set(["todo:a", "todo:b", "todo:c"]))
        XCTAssertTrue(loaded.inflight.isEmpty)
    }

    func testSyncRecordWireFormat() throws {
        let record = SyncRecord(
            kind: "todo", id: "18f2a-0", updatedAt: 1_760_000_000_000, device: "3f1c",
            deleted: false, body: todoBody("买牛奶/牛奶"), asset: nil
        )
        let data = try SyncJSON.encoder().encode(record)
        let text = String(data: data, encoding: .utf8) ?? ""
        XCTAssertTrue(text.contains("\"asset\":null"), text)
        XCTAssertTrue(text.contains("\"deleted\":false"), text)
        XCTAssertTrue(text.contains("\"updatedAt\":1760000000000"), text)
        XCTAssertTrue(text.contains("\"body\":{"), text)
        XCTAssertTrue(text.contains("买牛奶/牛奶"), text)

        let back = try JSONDecoder().decode(SyncRecord.self, from: data)
        XCTAssertEqual(back, record)
        XCTAssertEqual(back.key, "todo:18f2a-0")
    }

    func testSyncRecordDecodingIgnoresUnknownFieldsAndReadsNullAsset() throws {
        let json = """
        {"kind":"shelf","id":"x","updatedAt":5,"device":"d","deleted":true,
         "body":{"name":"a.pdf","size":12,"isImage":false,"extra":[1,2.5,null,{"k":"v"}]},
         "asset":null,"somethingNew":{"a":1}}
        """
        let record = try JSONDecoder().decode(SyncRecord.self, from: Data(json.utf8))
        XCTAssertEqual(record.kind, "shelf")
        XCTAssertEqual(record.deleted, true)
        XCTAssertNil(record.asset)
        XCTAssertEqual(record.body["name"]?.stringValue, "a.pdf")
        XCTAssertEqual(record.body["size"]?.int64Value, 12)
        XCTAssertEqual(record.body["isImage"]?.boolValue, false)
        XCTAssertEqual(
            record.body["extra"],
            JSONValue.array([
                JSONValue.int(1), JSONValue.double(2.5), JSONValue.null,
                JSONValue.object(["k": JSONValue.string("v")]),
            ])
        )

        // And with the optional members missing altogether.
        let bare = #"{"kind":"todo","id":"1","updatedAt":1,"device":"d"}"#
        let minimal = try JSONDecoder().decode(SyncRecord.self, from: Data(bare.utf8))
        XCTAssertEqual(minimal.deleted, false)
        XCTAssertEqual(minimal.body, JSONValue.object([:]))
        XCTAssertNil(minimal.asset)
    }

    func testBodyDecodesIntoCallerTypes() {
        struct Todo: Decodable {
            var text: String?
            var createdAt: Int64?
        }
        let record = SyncRecord(
            kind: "todo", id: "1", updatedAt: 1, device: "d",
            deleted: false, body: todoBody("milk"), asset: nil
        )
        let todo = record.decodeBody(Todo.self)
        XCTAssertEqual(todo?.text, "milk")
        XCTAssertEqual(todo?.createdAt, 1)
    }

    func testDeviceOrderIsByUTF8Bytes() {
        let low = Version(updatedAt: 1, device: "Z")
        let high = Version(updatedAt: 1, device: "a")
        XCTAssertTrue(Version.wins(high, low))
        XCTAssertFalse(Version.wins(low, high))
    }
}
