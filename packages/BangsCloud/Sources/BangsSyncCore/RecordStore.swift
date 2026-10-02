import Foundation

/// The local copy of the synced records plus the queue of changes still to push
/// (docs/sync.md, "合并规则").
///
/// A plain value type: keep one inside your model, mutate it on one thread at a time, and
/// call `save(to:)` after changes.
///
/// - `records`: the latest known record per key, tombstones included. The version of a key is
///   that record's `(updatedAt, device)`, so a late, older live record cannot bring a deleted
///   key back.
/// - `outbox`: local changes that have not been handed to the engine yet.
/// - `inflight`: changes handed over by `takeOutbox()` that have not been confirmed yet. They
///   are saved with the store and go back to the outbox on `load`, so a crash in the middle of
///   a push loses nothing (pushing the same version twice is harmless).
public struct RecordStore: Codable, Equatable, Sendable {
    public private(set) var records: [String: SyncRecord]
    public private(set) var outbox: [String: SyncRecord]
    public private(set) var inflight: [String: SyncRecord]

    public init(
        records: [String: SyncRecord] = [:],
        outbox: [String: SyncRecord] = [:],
        inflight: [String: SyncRecord] = [:]
    ) {
        self.records = records
        self.outbox = outbox
        self.inflight = inflight
    }

    // MARK: Remote changes

    /// Merges records that came from CloudKit, in order. A record is adopted when its key has
    /// no version yet or when it beats the local version; adopting drops any older pending
    /// push of the same key. Returns the adopted records, in the order they were adopted.
    public mutating func applyRemote(_ incoming: [SyncRecord]) -> [SyncRecord] {
        var adopted: [SyncRecord] = []
        for record in incoming {
            let key = record.key
            if let current = records[key], !Version.wins(record.version, current.version) {
                continue
            }
            records[key] = record
            outbox[key] = nil
            inflight[key] = nil
            adopted.append(record)
        }
        return adopted
    }

    // MARK: Local changes

    /// Creates or changes a record. Returns the new record to push, or nil when the live
    /// record already has this body and asset.
    @discardableResult
    public mutating func localUpsert(
        kind: String,
        id: String,
        body: JSONValue,
        asset: String? = nil,
        now: Int64,
        device: String
    ) -> SyncRecord? {
        let key = kind + ":" + id
        let previous = records[key]
        if let previous = previous, !previous.deleted, previous.body == body, previous.asset == asset {
            return nil
        }
        let record = SyncRecord(
            kind: kind,
            id: id,
            updatedAt: Version.nextStamp(prev: previous?.updatedAt, now: now),
            device: device,
            deleted: false,
            body: body,
            asset: asset
        )
        records[key] = record
        outbox[key] = record
        return record
    }

    /// Deletes a record by writing a tombstone. Returns the tombstone to push, or nil when
    /// there is no such live record.
    @discardableResult
    public mutating func localDelete(
        kind: String,
        id: String,
        now: Int64,
        device: String
    ) -> SyncRecord? {
        let key = kind + ":" + id
        guard let previous = records[key], !previous.deleted else {
            return nil
        }
        let tombstone = SyncRecord(
            kind: kind,
            id: id,
            updatedAt: Version.nextStamp(prev: previous.updatedAt, now: now),
            device: device,
            deleted: true,
            body: JSONValue.object([:]),
            asset: nil
        )
        records[key] = tombstone
        outbox[key] = tombstone
        return tombstone
    }

    // MARK: Pushing

    /// Hands out the pending changes (oldest first) and remembers them as in flight. A key that
    /// is already in flight stays in the outbox until that push is settled: otherwise its newer
    /// version would replace the in-flight one, and the answer about the older push would settle
    /// the newer change without it ever being sent.
    public mutating func takeOutbox() -> [SyncRecord] {
        let batch = outbox.values
            .filter { inflight[$0.key] == nil }
            .sorted { lhs, rhs in
                if lhs.updatedAt != rhs.updatedAt { return lhs.updatedAt < rhs.updatedAt }
                return lhs.key < rhs.key
            }
        for record in batch {
            inflight[record.key] = record
            outbox[record.key] = nil
        }
        return batch
    }

    /// Queues every live record of `kind` again, unchanged: the cloud lost them (a reset zone).
    public mutating func requeue(kind: String) {
        for record in records.values where record.kind == kind && !record.deleted {
            if inflight[record.key] == nil {
                outbox[record.key] = record
            }
        }
    }

    /// Puts records from a failed push back into the outbox, unless their key has moved on
    /// since (a newer local edit, or a remote record that won).
    public mutating func restore(_ failed: [SyncRecord]) {
        for record in failed {
            let key = record.key
            inflight[key] = nil
            guard let current = records[key], current.version == record.version else {
                continue
            }
            outbox[key] = record
        }
    }

    /// `restore` for the keys named by an engine `failed` event.
    public mutating func restoreInflight(keys: [String]) {
        var failed: [SyncRecord] = []
        for key in keys {
            if let record = inflight[key] {
                failed.append(record)
            }
        }
        restore(failed)
    }

    /// Forgets in-flight records the engine has settled: pushed, or lost a conflict.
    public mutating func markSettled(keys: [String]) {
        for key in keys {
            inflight[key] = nil
        }
    }

    // MARK: Reading

    /// The live (not deleted) records of one kind, ordered by key.
    public func live(kind: String) -> [SyncRecord] {
        let matching = records.values.filter { $0.kind == kind && !$0.deleted }
        return matching.sorted { $0.key < $1.key }
    }

    // MARK: Housekeeping

    /// Drops tombstones older than `ttlMs` that are not waiting to be pushed. Returns how many.
    @discardableResult
    public mutating func purgeTombstones(now: Int64, ttlMs: Int64) -> Int {
        var removed = 0
        for (key, record) in records {
            guard record.deleted, outbox[key] == nil, inflight[key] == nil else { continue }
            if now - record.updatedAt > ttlMs {
                records[key] = nil
                removed += 1
            }
        }
        return removed
    }

    // MARK: Persistence

    private enum CodingKeys: String, CodingKey {
        case records, outbox, inflight
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        records = try container.decodeIfPresent([String: SyncRecord].self, forKey: .records) ?? [:]
        outbox = try container.decodeIfPresent([String: SyncRecord].self, forKey: .outbox) ?? [:]
        inflight = try container.decodeIfPresent([String: SyncRecord].self, forKey: .inflight) ?? [:]
    }

    /// Reads a store saved by `save(to:)`. A missing file is an empty store; a file that
    /// cannot be parsed throws. Records that were in flight when the file was written go back
    /// to the outbox.
    public static func load(from url: URL) throws -> RecordStore {
        guard FileManager.default.fileExists(atPath: url.path) else {
            return RecordStore()
        }
        let data = try Data(contentsOf: url)
        var store = try SyncJSON.decoder().decode(RecordStore.self, from: data)
        store.recoverInflight()
        return store
    }

    /// Writes the store atomically (temp file, then rename), creating the directory if needed.
    public func save(to url: URL) throws {
        let directory = url.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let data = try SyncJSON.encoder().encode(self)
        try data.write(to: url, options: .atomic)
    }

    private mutating func recoverInflight() {
        let pending = Array(inflight.values)
        inflight = [:]
        restore(pending)
    }
}
