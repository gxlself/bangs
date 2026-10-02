#if canImport(CloudKit)

import Foundation
import CloudKit
import BangsSyncCore

// The CloudKit transport of docs/sync.md: one private zone, one generic record type.
// It knows nothing about todos or clipboards; it moves SyncRecords and reports what happened.

public enum CloudStatus: Equatable, Sendable {
    case starting
    case ready
    case syncing
    case idle
    case noAccount
    case restricted
    case unavailable
    case error(String)
}

public enum CloudEvent: Sendable {
    case status(CloudStatus)
    /// Records pulled from CloudKit (tombstones included), or the winning server records of
    /// lost conflicts. Hand them to `RecordStore.applyRemote`.
    case records([SyncRecord])
    /// Keys the server accepted.
    case pushed([String])
    /// Keys that lost a conflict; the winning record arrives as a `records` event.
    case rejected([String])
    /// Keys that could not be pushed right now; put them back and try again later.
    case failed(keys: [String], message: String, retryAfter: Int?)
}

/// Every `pull()` and `push()` ends with exactly one status event: `syncing` when it starts,
/// then `idle` on success or `error` on failure. The records / pushed / rejected / failed
/// events of a call come before its closing status. `start()` emits `starting`, then `ready`
/// (or the reason it cannot be ready).
public actor CloudEngine {
    /// File name of the saved change token inside the state directory.
    public static let tokenFileName = "token.bin"
    /// Directory name, inside the state directory, of the downloaded CKAssets.
    public static let assetsDirectoryName = "assets"

    static let recordType = "BangsRecord"
    static let zoneName = "Bangs"
    static let subscriptionID = "bangs-zone"
    static let knownKinds: Set<String> = ["todo", "session", "clip", "shelf"]
    static let maxBatch = 100
    static let maxConflictRounds = 3

    private let container: CKContainer
    private let database: CKDatabase
    private let zoneID: CKRecordZone.ID
    private let stateDirectory: URL
    private let onEvent: @Sendable (CloudEvent) -> Void

    private var changeToken: CKServerChangeToken?
    private var tokenLoaded = false
    private var setupDone = false
    private var stopped = false

    // pull() and push() run one at a time, in the order they were called.
    private var busy = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    public init(
        containerID: String = "iCloud.com.gxlself.bangs",
        stateDirectory: URL,
        onEvent: @escaping @Sendable (CloudEvent) -> Void
    ) {
        let container = CKContainer(identifier: containerID)
        self.container = container
        self.database = container.privateCloudDatabase
        self.zoneID = CKRecordZone.ID(zoneName: "Bangs", ownerName: CKCurrentUserDefaultName)
        self.stateDirectory = stateDirectory
        self.onEvent = onEvent
    }

    // MARK: Public API

    /// Checks the iCloud account, creates the zone and its subscription, then reports `ready`.
    /// Safe to call again (for instance when the app comes back to the foreground).
    public func start() async {
        stopped = false
        emit(.status(.starting))
        do {
            let account = try await container.accountStatus()
            switch account {
            case .available:
                break
            case .noAccount:
                emit(.status(.noAccount))
                return
            case .restricted:
                emit(.status(.restricted))
                return
            case .couldNotDetermine, .temporarilyUnavailable:
                emit(.status(.unavailable))
                return
            @unknown default:
                emit(.status(.unavailable))
                return
            }
            try await ensureSetup()
            if stopped { return }
            emit(.status(.ready))
        } catch {
            emit(.status(statusFor(error)))
        }
    }

    /// Fetches everything that changed since the saved change token.
    public func pull() async {
        await acquire()
        defer { release() }
        if stopped { return }
        emit(.status(.syncing))
        do {
            try await ensureSetup()
            try await fetchChanges()
            emit(.status(.idle))
        } catch {
            emit(.status(.error(error.localizedDescription)))
        }
    }

    /// Sends records to CloudKit with last-writer-wins conflict handling.
    public func push(_ records: [SyncRecord]) async {
        await acquire()
        defer { release() }
        if stopped { return }
        emit(.status(.syncing))

        // One request cannot name the same record twice: keep the winner per key.
        var latest: [String: SyncRecord] = [:]
        var order: [String] = []
        for record in records {
            let key = record.key
            if let existing = latest[key] {
                if Version.wins(record.version, existing.version) {
                    latest[key] = record
                }
            } else {
                latest[key] = record
                order.append(key)
            }
        }
        var unique: [SyncRecord] = []
        for key in order {
            if let record = latest[key] {
                unique.append(record)
            }
        }
        if unique.isEmpty {
            emit(.status(.idle))
            return
        }

        do {
            try await ensureSetup()
        } catch {
            let message = error.localizedDescription
            emit(.failed(keys: order, message: message, retryAfter: retryAfterSeconds(of: error)))
            emit(.status(.error(message)))
            return
        }

        var firstFailure: String? = nil
        var offset = 0
        while offset < unique.count {
            let end = min(offset + CloudEngine.maxBatch, unique.count)
            let failure = await pushBatch(Array(unique[offset..<end]))
            if firstFailure == nil {
                firstFailure = failure
            }
            offset = end
        }
        if let message = firstFailure {
            emit(.status(.error(message)))
        } else {
            emit(.status(.idle))
        }
    }

    /// Stops reporting; queued and later calls do nothing. `start()` revives the engine.
    public func stop() {
        stopped = true
    }

    // MARK: Serialising pull and push

    private func acquire() async {
        if !busy {
            busy = true
            return
        }
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            waiters.append(continuation)
        }
        // `busy` stayed true: the previous call handed the turn straight over to us.
    }

    private func release() {
        if waiters.isEmpty {
            busy = false
        } else {
            let next = waiters.removeFirst()
            next.resume()
        }
    }

    // MARK: Setup

    private func ensureSetup() async throws {
        if setupDone { return }

        let zone = CKRecordZone(zoneID: zoneID)
        let zoneResult = try await database.modifyRecordZones(saving: [zone], deleting: [])
        if let outcome = zoneResult.saveResults[zoneID], case .failure(let error) = outcome {
            throw error
        }

        let subscription = CKRecordZoneSubscription(zoneID: zoneID, subscriptionID: CloudEngine.subscriptionID)
        let info = CKSubscription.NotificationInfo()
        info.shouldSendContentAvailable = true
        subscription.notificationInfo = info
        let subscriptionResult = try await database.modifySubscriptions(saving: [subscription], deleting: [])
        if let outcome = subscriptionResult.saveResults[CloudEngine.subscriptionID], case .failure(let error) = outcome {
            throw error
        }

        setupDone = true
    }

    private func statusFor(_ error: Error) -> CloudStatus {
        if let ckError = error as? CKError, ckError.code == .notAuthenticated {
            return .noAccount
        }
        return .error(error.localizedDescription)
    }

    // MARK: Pulling

    private func fetchChanges() async throws {
        loadTokenIfNeeded()
        var attempts = 0
        while true {
            do {
                try await fetchPages()
                return
            } catch {
                attempts += 1
                guard attempts < 3, let ckError = error as? CKError else {
                    throw error
                }
                switch ckError.code {
                case .changeTokenExpired:
                    // The server no longer knows our token: start over. Records we already
                    // have come back as echoes and are ignored by the merge rules.
                    changeToken = nil
                    saveToken()
                case .zoneNotFound, .userDeletedZone:
                    changeToken = nil
                    saveToken()
                    setupDone = false
                    try await ensureSetup()
                default:
                    throw error
                }
            }
        }
    }

    private func fetchPages() async throws {
        var more = true
        while more {
            let result = try await database.recordZoneChanges(
                inZoneWith: zoneID,
                since: changeToken,
                desiredKeys: nil,
                resultsLimit: nil
            )
            var batch: [SyncRecord] = []
            for (_, modification) in result.modificationResultsByID {
                guard case .success(let change) = modification else { continue }
                if let record = convert(change.record) {
                    batch.append(record)
                }
            }
            // Deletions from CloudKit are not used: Bangs deletes with tombstone records.
            batch.sort { lhs, rhs in
                if lhs.updatedAt != rhs.updatedAt { return lhs.updatedAt < rhs.updatedAt }
                return lhs.key < rhs.key
            }
            if !batch.isEmpty {
                emit(.records(batch))
            }
            changeToken = result.changeToken
            saveToken()
            more = result.moreComing
        }
    }

    // MARK: Pushing

    /// Pushes up to `maxBatch` records. Emits pushed / rejected / records / failed, and returns
    /// the first failure message, if any.
    private func pushBatch(_ batch: [SyncRecord]) async -> String? {
        var ours: [String: SyncRecord] = [:]
        var toSave: [CKRecord] = []
        for record in batch {
            ours[record.key] = record
            toSave.append(makeCKRecord(record))
        }

        var pushed: [String] = []
        var rejected: [String] = []
        var winners: [SyncRecord] = []
        var failedKeys: [String] = []
        var failureMessage: String? = nil
        var failureRetry: Int? = nil

        func noteFailure(key: String, message: String, retryAfter: Int?) {
            failedKeys.append(key)
            if failureMessage == nil {
                failureMessage = message
            }
            if let seconds = retryAfter {
                failureRetry = max(failureRetry ?? 0, seconds)
            }
        }

        var round = 0
        while !toSave.isEmpty {
            round += 1
            var again: [CKRecord] = []
            do {
                let result = try await database.modifyRecords(
                    saving: toSave,
                    deleting: [],
                    savePolicy: .ifServerRecordUnchanged,
                    atomically: false
                )
                for (recordID, outcome) in result.saveResults {
                    let key = recordID.recordName
                    guard let mine = ours[key] else { continue }
                    switch outcome {
                    case .success:
                        pushed.append(key)
                    case .failure(let error):
                        if let ckError = error as? CKError,
                           ckError.code == .serverRecordChanged,
                           let server = ckError.serverRecord {
                            let theirs = serverVersion(of: server)
                            if theirs == mine.version {
                                // Already there: an earlier attempt got through.
                                pushed.append(key)
                            } else if Version.wins(mine.version, theirs) {
                                if round < CloudEngine.maxConflictRounds {
                                    fill(server, from: mine)
                                    again.append(server)
                                } else {
                                    noteFailure(key: key, message: "Conflict not resolved after \(round) tries", retryAfter: nil)
                                }
                            } else {
                                rejected.append(key)
                                if let winner = convert(server) {
                                    winners.append(winner)
                                }
                            }
                        } else {
                            if isMissingZone(error) {
                                setupDone = false
                            }
                            noteFailure(key: key, message: error.localizedDescription, retryAfter: retryAfterSeconds(of: error))
                        }
                    }
                }
                toSave = again
            } catch {
                // The whole request failed (network, account, throttling, ...).
                if isMissingZone(error) {
                    setupDone = false
                }
                let message = error.localizedDescription
                let retry = retryAfterSeconds(of: error)
                for record in toSave {
                    noteFailure(key: record.recordID.recordName, message: message, retryAfter: retry)
                }
                toSave = []
            }
        }

        if !pushed.isEmpty {
            emit(.pushed(pushed))
        }
        if !rejected.isEmpty {
            emit(.rejected(rejected))
        }
        if !winners.isEmpty {
            emit(.records(winners))
        }
        if !failedKeys.isEmpty {
            emit(.failed(keys: failedKeys, message: failureMessage ?? "Push failed", retryAfter: failureRetry))
        }
        return failureMessage
    }

    // MARK: CKRecord <-> SyncRecord

    private func makeCKRecord(_ record: SyncRecord) -> CKRecord {
        let recordID = CKRecord.ID(recordName: record.key, zoneID: zoneID)
        let ckRecord = CKRecord(recordType: CloudEngine.recordType, recordID: recordID)
        fill(ckRecord, from: record)
        return ckRecord
    }

    /// Copies our fields onto a CKRecord. `body` goes into `encryptedValues` (end-to-end
    /// encrypted); everything else is a plain field.
    private func fill(_ ckRecord: CKRecord, from record: SyncRecord) {
        // NSString and NSNumber rather than String and Int64: they certainly are CloudKit
        // values, whatever the Swift overlay conforms. Reading back with `as? String` and
        // `as? Int64` bridges them again.
        ckRecord["kind"] = NSString(string: record.kind)
        ckRecord["updatedAt"] = NSNumber(value: record.updatedAt)
        ckRecord["device"] = NSString(string: record.device)
        ckRecord["deleted"] = NSNumber(value: Int64(record.deleted ? 1 : 0))
        ckRecord.encryptedValues["body"] = NSString(string: record.body.jsonString)
        if let path = record.asset, FileManager.default.fileExists(atPath: path) {
            ckRecord["asset"] = CKAsset(fileURL: URL(fileURLWithPath: path))
        } else {
            ckRecord["asset"] = nil
        }
    }

    private func serverVersion(of ckRecord: CKRecord) -> Version {
        let updatedAt = (ckRecord["updatedAt"] as? Int64) ?? 0
        let device = (ckRecord["device"] as? String) ?? ""
        return Version(updatedAt: updatedAt, device: device)
    }

    /// nil for records that are not ours: other record types, kinds we do not know.
    private func convert(_ ckRecord: CKRecord) -> SyncRecord? {
        guard ckRecord.recordType == CloudEngine.recordType else { return nil }
        guard let kind = ckRecord["kind"] as? String, CloudEngine.knownKinds.contains(kind) else { return nil }
        let name = ckRecord.recordID.recordName
        let prefix = kind + ":"
        guard name.hasPrefix(prefix), name.count > prefix.count else { return nil }
        let id = String(name.dropFirst(prefix.count))
        guard let updatedAt = ckRecord["updatedAt"] as? Int64,
              let device = ckRecord["device"] as? String
        else {
            return nil
        }
        let deleted = ((ckRecord["deleted"] as? Int64) ?? 0) != 0

        var body = JSONValue.object([:])
        if let text = ckRecord.encryptedValues["body"] as? String,
           let parsed = JSONValue(jsonString: text) {
            body = parsed
        }

        var assetPath: String? = nil
        if !deleted, let asset = ckRecord["asset"] as? CKAsset {
            assetPath = copyAsset(asset, recordName: name)
        }

        return SyncRecord(
            kind: kind,
            id: id,
            updatedAt: updatedAt,
            device: device,
            deleted: deleted,
            body: body,
            asset: assetPath
        )
    }

    /// CloudKit hands out a temporary file; keep a copy at <state>/assets/<recordName with ':' -> '_'>.
    private func copyAsset(_ asset: CKAsset, recordName: String) -> String? {
        guard let source = asset.fileURL else { return nil }
        let fileManager = FileManager.default
        let directory = stateDirectory.appendingPathComponent(CloudEngine.assetsDirectoryName, isDirectory: true)
        var fileName = recordName.replacingOccurrences(of: ":", with: "_")
        fileName = fileName.replacingOccurrences(of: "/", with: "_")
        let destination = directory.appendingPathComponent(fileName)
        do {
            try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
            if fileManager.fileExists(atPath: destination.path) {
                try fileManager.removeItem(at: destination)
            }
            try fileManager.copyItem(at: source, to: destination)
            return destination.path
        } catch {
            return nil
        }
    }

    // MARK: Change token

    private var tokenURL: URL {
        return stateDirectory.appendingPathComponent(CloudEngine.tokenFileName)
    }

    private func loadTokenIfNeeded() {
        if tokenLoaded { return }
        tokenLoaded = true
        guard let data = try? Data(contentsOf: tokenURL) else { return }
        changeToken = try? NSKeyedUnarchiver.unarchivedObject(ofClass: CKServerChangeToken.self, from: data)
    }

    private func saveToken() {
        let fileManager = FileManager.default
        guard let token = changeToken else {
            try? fileManager.removeItem(at: tokenURL)
            return
        }
        do {
            try fileManager.createDirectory(at: stateDirectory, withIntermediateDirectories: true)
            let data = try NSKeyedArchiver.archivedData(withRootObject: token, requiringSecureCoding: true)
            try data.write(to: tokenURL, options: .atomic)
        } catch {
            // Not fatal: the next pull starts from an older token (or from scratch) and the
            // merge rules make the repeat harmless.
        }
    }

    // MARK: Helpers

    private func emit(_ event: CloudEvent) {
        onEvent(event)
    }

    private func isMissingZone(_ error: Error) -> Bool {
        guard let ckError = error as? CKError else { return false }
        return ckError.code == .zoneNotFound || ckError.code == .userDeletedZone
    }

    private func retryAfterSeconds(of error: Error) -> Int? {
        guard let ckError = error as? CKError, let seconds = ckError.retryAfterSeconds else { return nil }
        return Int(seconds.rounded(.up))
    }
}

#endif
