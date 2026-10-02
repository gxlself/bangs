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
    /// The cloud no longer has what this device put there: "zone" (the zone was deleted, e.g.
    /// Reset Development Environment) or "account" (another iCloud account is signed in).
    /// Upload everything again; for "account", local data belongs to the old account.
    case reset(String)
}

/// Every `pull()` and `push()` ends with exactly one status event: `syncing` when it starts,
/// then `idle` on success, or `noAccount` / `error` on failure. The records / pushed /
/// rejected / failed events of a call come before its closing status. `start()` emits
/// `starting`, then `ready` (or the reason it cannot be ready). `start()`, `pull()` and
/// `push()` run one at a time, in the order they reach the actor. After `stop()` the engine is
/// done for good: no events, no token writes; make a new one to start again.
public actor CloudEngine {
    /// File name of the saved change token inside the state directory.
    public static let tokenFileName = "token.bin"
    /// Directory name, inside the state directory, of the downloaded CKAssets.
    public static let assetsDirectoryName = "assets"
    /// File name, inside the state directory, of the iCloud user record the token belongs to.
    public static let accountFileName = "account.txt"
    /// File name, inside the state directory, of the cached system fields (change tags).
    public static let systemFieldsFileName = "systemfields.plist"
    /// Present once a fetch that left out `asset` turned out to leave out `body` as well.
    static let fullFetchFileName = "fullfetch"
    /// The fields a fetch asks for when it does not want the files themselves.
    static let fieldsWithoutAsset: [CKRecord.FieldKey] = ["kind", "updatedAt", "device", "deleted", "body"]
    /// More cached system fields than this and half are dropped; a dropped one only costs
    /// one extra round trip the next time that record changes.
    static let maxSystemFields = 4000
    /// A clipboard picture rides in the record's encrypted values, which share its 1 MB limit.
    static let maxImageBytes = 900_000

    static let recordType = "BangsRecord"
    static let zoneName = "Bangs"
    static let subscriptionID = "bangs-zone"
    static let knownKinds: Set<String> = ["todo", "session", "clip", "shelf", "device"]
    /// Records per request. Bodies go up to 64 KB, and a request has a size limit.
    static let maxBatch = 25
    static let maxConflictRounds = 3

    private let container: CKContainer
    private let database: CKDatabase
    private let zoneID: CKRecordZone.ID
    private let stateDirectory: URL
    private let keepAssets: Bool
    /// Awaited before the engine moves on: whatever the caller does with an event — saving the
    /// records it brought — is done before, say, the change token covering them is saved.
    private let onEvent: @Sendable (CloudEvent) async -> Void

    private var changeToken: CKServerChangeToken?
    private var tokenLoaded = false
    private var setupDone = false
    private var stopped = false
    /// Account status and user the last `start()` saw, so a CKAccountChanged that changes
    /// nothing does not restart anything.
    private var lastAccountKey: String?
    /// recordName → the record's system fields as last seen, so a change is saved on top of the
    /// server's version instead of colliding with it first.
    private var systemFields: [String: Data] = [:]
    private var systemFieldsLoaded = false
    private var systemFieldsDirty = false
    /// The zone went missing; the next successful setup reports `reset("zone")`.
    private var zoneLost = false

    // pull() and push() run one at a time, in the order they were called.
    private var busy = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    /// `keepAssets: false` skips copying downloaded files: the Mac only ever gets back the shelf
    /// files it uploaded itself.
    public init(
        containerID: String = "iCloud.com.gxlself.bangs",
        stateDirectory: URL,
        keepAssets: Bool = true,
        onEvent: @escaping @Sendable (CloudEvent) async -> Void
    ) {
        let container = CKContainer(identifier: containerID)
        self.container = container
        self.database = container.privateCloudDatabase
        self.zoneID = CKRecordZone.ID(zoneName: CloudEngine.zoneName, ownerName: CKCurrentUserDefaultName)
        self.stateDirectory = stateDirectory
        self.keepAssets = keepAssets
        self.onEvent = onEvent
    }

    // MARK: Public API

    /// Checks the iCloud account, creates the zone and its subscription, then reports `ready`.
    /// Safe to call again (for instance when the app comes back to the foreground).
    public func start() async {
        await acquire()
        defer { release() }
        if stopped { return }
        await emit(.status(.starting))
        do {
            let account = try await container.accountStatus()
            lastAccountKey = "\(account.rawValue):"
            switch account {
            case .available:
                break
            case .noAccount:
                await emit(.status(.noAccount))
                return
            case .restricted:
                await emit(.status(.restricted))
                return
            case .couldNotDetermine, .temporarilyUnavailable:
                await emit(.status(.unavailable))
                return
            @unknown default:
                await emit(.status(.unavailable))
                return
            }
            await checkAccount()
            try await ensureSetup()
            if stopped { return }
            await emit(.status(.ready))
        } catch {
            await emit(.status(statusFor(error)))
        }
    }

    /// A different iCloud account than last time: the saved token and everything this device
    /// thinks is in the cloud belong to the old one.
    private func checkAccount() async {
        guard let recordID = try? await container.userRecordID(), !stopped else { return }
        let current = recordID.recordName
        lastAccountKey = "\(CKAccountStatus.available.rawValue):\(current)"
        let url = stateDirectory.appendingPathComponent(CloudEngine.accountFileName)
        let saved = (try? String(contentsOf: url, encoding: .utf8))?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if saved == current { return }
        try? FileManager.default.createDirectory(at: stateDirectory, withIntermediateDirectories: true)
        try? current.write(to: url, atomically: true, encoding: .utf8)
        guard let saved = saved, !saved.isEmpty else { return }
        changeToken = nil
        tokenLoaded = true
        saveToken()
        forgetSystemFields()
        setupDone = false
        await emit(.reset("account"))
    }

    /// Fetches everything that changed since the saved change token.
    public func pull() async {
        await acquire()
        defer { release() }
        if stopped { return }
        await emit(.status(.syncing))
        do {
            try await ensureSetup()
            try await fetchChanges()
            if stopped { return }
            await emit(.status(.idle))
        } catch {
            if stopped { return }
            await emit(.status(statusFor(error)))
        }
    }

    /// Sends records to CloudKit with last-writer-wins conflict handling.
    public func push(_ records: [SyncRecord]) async {
        await acquire()
        defer { release() }
        if stopped { return }
        await emit(.status(.syncing))

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
            await emit(.status(.idle))
            return
        }

        do {
            try await ensureSetup()
        } catch {
            let message = error.localizedDescription
            await emit(.failed(keys: order, message: message, retryAfter: retryAfterSeconds(of: error)))
            await emit(.status(statusFor(error)))
            return
        }

        var firstFailure: CloudStatus? = nil
        var offset = 0
        while offset < unique.count {
            let end = min(offset + CloudEngine.maxBatch, unique.count)
            let failure = await pushBatch(Array(unique[offset..<end]))
            if firstFailure == nil {
                firstFailure = failure
            }
            offset = end
        }
        if stopped { return }
        await emit(.status(firstFailure ?? .idle))
    }

    /// The system says the iCloud account changed. Starts over only when it really did: the
    /// notification also comes on launch and wake with nothing changed, and a needless restart
    /// would make the caller send everything in flight again.
    public func accountChanged() async {
        if stopped { return }
        let status = (try? await container.accountStatus())?.rawValue ?? -1
        var key = "\(status):"
        if status == CKAccountStatus.available.rawValue {
            let user = try? await container.userRecordID()
            key += user?.recordName ?? ""
        }
        if key == lastAccountKey { return }
        await start()
    }

    /// Stops for good: queued and later calls do nothing, nothing is emitted or saved.
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
        if zoneLost {
            zoneLost = false
            changeToken = nil
            tokenLoaded = true
            saveToken()
            forgetSystemFields()
            await emit(.reset("zone"))
        }
    }

    private func statusFor(_ error: Error) -> CloudStatus {
        if let ckError = error as? CKError, ckError.code == .notAuthenticated {
            return .noAccount
        }
        return .error(error.localizedDescription)
    }

    /// Errors a retry will not fix. Reported as `rejected` so they do not hold up every
    /// later change.
    private func isPermanent(_ error: Error) -> Bool {
        guard let ckError = error as? CKError else { return false }
        switch ckError.code {
        case .invalidArguments, .assetFileNotFound, .permissionFailure:
            return true
        default:
            return false
        }
    }

    private func log(_ message: String) {
        FileHandle.standardError.write(Data(("BangsCloud: " + message + "\n").utf8))
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
                    // ensureSetup recreates it and reports reset("zone"), so the caller puts
                    // everything back; the fetch then starts from an empty zone.
                    zoneLost = true
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
            if stopped { return }
            let fields = fieldsToFetch()
            let result = try await database.recordZoneChanges(
                inZoneWith: zoneID,
                since: changeToken,
                desiredKeys: fields,
                resultsLimit: nil
            )
            var ckRecords: [CKRecord] = []
            for (_, modification) in result.modificationResultsByID {
                guard case .success(let change) = modification else { continue }
                ckRecords.append(change.record)
            }
            // Every record Bangs writes has a body. One without it means asking for named fields
            // leaves the encrypted one out: fetch this page again whole, and from now on.
            if fields != nil,
               ckRecords.contains(where: { $0.recordType == CloudEngine.recordType && $0.encryptedValues["body"] == nil }) {
                log("a fetch without assets came back without bodies; fetching whole records from now on")
                disablePartialFetch()
                continue
            }
            var batch: [SyncRecord] = []
            for ckRecord in ckRecords {
                remember(ckRecord)
                if let record = convert(ckRecord) {
                    batch.append(record)
                }
            }
            // Deletions from CloudKit are not used: Bangs deletes with tombstone records.
            batch.sort { lhs, rhs in
                if lhs.updatedAt != rhs.updatedAt { return lhs.updatedAt < rhs.updatedAt }
                return lhs.key < rhs.key
            }
            // Stopped meanwhile: nobody takes these, so the token must not move past them.
            if stopped { return }
            if !batch.isEmpty {
                await emit(.records(batch))
            }
            if stopped { return }
            changeToken = result.changeToken
            saveToken()
            saveSystemFields()
            more = result.moreComing
        }
    }

    /// nil (everything) where files are kept; the fields without `asset` on the Mac, which only
    /// ever gets back the files it uploaded — unless that turned out to drop `body` too.
    private func fieldsToFetch() -> [CKRecord.FieldKey]? {
        if keepAssets { return nil }
        let flag = stateDirectory.appendingPathComponent(CloudEngine.fullFetchFileName)
        if FileManager.default.fileExists(atPath: flag.path) { return nil }
        return CloudEngine.fieldsWithoutAsset
    }

    private func disablePartialFetch() {
        try? FileManager.default.createDirectory(at: stateDirectory, withIntermediateDirectories: true)
        let flag = stateDirectory.appendingPathComponent(CloudEngine.fullFetchFileName)
        try? Data().write(to: flag)
    }

    // MARK: Pushing

    /// Pushes up to `maxBatch` records. Emits pushed / rejected / records / failed, and returns
    /// the status the push should end with when something failed.
    private func pushBatch(_ batch: [SyncRecord]) async -> CloudStatus? {
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
        var failureStatus: CloudStatus? = nil
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
                    case .success(let saved):
                        remember(saved)
                        pushed.append(key)
                    case .failure(let error):
                        if let ckError = error as? CKError, ckError.code == .unknownItem {
                            // Cached system fields of a record the server no longer has: save
                            // it as new.
                            forgetSystemFields(of: key)
                            if round < CloudEngine.maxConflictRounds {
                                again.append(makeCKRecord(mine))
                            } else {
                                noteFailure(key: key, message: error.localizedDescription, retryAfter: nil)
                            }
                        } else if let ckError = error as? CKError,
                           ckError.code == .serverRecordChanged,
                           let server = ckError.serverRecord {
                            remember(server)
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
                        } else if isPermanent(error) {
                            log("dropping \(key): \(error.localizedDescription)")
                            rejected.append(key)
                        } else {
                            if isMissingZone(error) {
                                setupDone = false
                                zoneLost = true
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
                    zoneLost = true
                }
                failureStatus = statusFor(error)
                let message = error.localizedDescription
                let retry = retryAfterSeconds(of: error)
                for record in toSave {
                    noteFailure(key: record.recordID.recordName, message: message, retryAfter: retry)
                }
                toSave = []
            }
        }

        saveSystemFields()
        if !pushed.isEmpty {
            await emit(.pushed(pushed))
        }
        if !rejected.isEmpty {
            await emit(.rejected(rejected))
        }
        if !winners.isEmpty {
            await emit(.records(winners))
        }
        if !failedKeys.isEmpty {
            await emit(.failed(keys: failedKeys, message: failureMessage ?? "Push failed", retryAfter: failureRetry))
        }
        if let status = failureStatus {
            return status
        }
        if let message = failureMessage {
            return .error(message)
        }
        return nil
    }

    // MARK: CKRecord <-> SyncRecord

    /// On top of the server's last known version when there is one, so the save does not
    /// collide with it first; otherwise a new record.
    private func makeCKRecord(_ record: SyncRecord) -> CKRecord {
        loadSystemFieldsIfNeeded()
        let ckRecord: CKRecord
        if let data = systemFields[record.key],
           let known = decodeSystemFields(data),
           known.recordID.recordName == record.key,
           known.recordID.zoneID == zoneID {
            ckRecord = known
        } else {
            let recordID = CKRecord.ID(recordName: record.key, zoneID: zoneID)
            ckRecord = CKRecord(recordType: CloudEngine.recordType, recordID: recordID)
        }
        fill(ckRecord, from: record)
        return ckRecord
    }

    // MARK: System fields

    private var systemFieldsURL: URL {
        return stateDirectory.appendingPathComponent(CloudEngine.systemFieldsFileName)
    }

    private func loadSystemFieldsIfNeeded() {
        if systemFieldsLoaded { return }
        systemFieldsLoaded = true
        guard let data = try? Data(contentsOf: systemFieldsURL),
              let saved = try? PropertyListDecoder().decode([String: Data].self, from: data)
        else {
            return
        }
        systemFields = saved
    }

    private func remember(_ ckRecord: CKRecord) {
        guard ckRecord.recordType == CloudEngine.recordType else { return }
        loadSystemFieldsIfNeeded()
        systemFields[ckRecord.recordID.recordName] = encodeSystemFields(ckRecord)
        systemFieldsDirty = true
        if systemFields.count > CloudEngine.maxSystemFields {
            for key in systemFields.keys.prefix(systemFields.count / 2) {
                systemFields[key] = nil
            }
        }
    }

    private func forgetSystemFields(of key: String) {
        loadSystemFieldsIfNeeded()
        if systemFields.removeValue(forKey: key) != nil {
            systemFieldsDirty = true
        }
    }

    /// The zone is new, or the account is another one: none of the change tags mean anything.
    private func forgetSystemFields() {
        systemFieldsLoaded = true
        systemFields = [:]
        systemFieldsDirty = true
        saveSystemFields()
    }

    private func saveSystemFields() {
        if stopped || !systemFieldsDirty { return }
        systemFieldsDirty = false
        do {
            try FileManager.default.createDirectory(at: stateDirectory, withIntermediateDirectories: true)
            let encoder = PropertyListEncoder()
            encoder.outputFormat = .binary
            try encoder.encode(systemFields).write(to: systemFieldsURL, options: .atomic)
        } catch {
            // Not fatal: the next change of each record just takes one more round trip.
        }
    }

    private func encodeSystemFields(_ ckRecord: CKRecord) -> Data {
        let coder = NSKeyedArchiver(requiringSecureCoding: true)
        ckRecord.encodeSystemFields(with: coder)
        coder.finishEncoding()
        return coder.encodedData
    }

    private func decodeSystemFields(_ data: Data) -> CKRecord? {
        guard let coder = try? NSKeyedUnarchiver(forReadingFrom: data) else { return nil }
        coder.requiresSecureCoding = true
        let ckRecord = CKRecord(coder: coder)
        coder.finishDecoding()
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
        let path = record.asset.flatMap { FileManager.default.fileExists(atPath: $0) ? $0 : nil }
        if record.kind == "clip" {
            // A copied picture can be as private as a password: it goes end-to-end encrypted,
            // as bytes, which CloudKit only offers for values, not for CKAssets.
            ckRecord["asset"] = nil
            if let path = path,
               let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
               data.count <= CloudEngine.maxImageBytes {
                ckRecord.encryptedValues["image"] = data as NSData
            } else {
                ckRecord.encryptedValues["image"] = nil
            }
        } else if let path = path {
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
        if keepAssets, !deleted {
            if kind == "clip", let image = ckRecord.encryptedValues["image"] as? Data {
                assetPath = storeAsset(recordName: name) { destination in
                    try image.write(to: destination, options: .atomic)
                }
            } else if let asset = ckRecord["asset"] as? CKAsset, let source = asset.fileURL {
                assetPath = storeAsset(recordName: name) { destination in
                    try FileManager.default.copyItem(at: source, to: destination)
                }
            }
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

    /// Keeps a downloaded file — a CKAsset's temporary copy, or a clipboard picture's bytes — at
    /// <state>/assets/<recordName with ':' -> '_'>.
    private func storeAsset(recordName: String, write: (URL) throws -> Void) -> String? {
        let fileManager = FileManager.default
        let directory = stateDirectory.appendingPathComponent(CloudEngine.assetsDirectoryName, isDirectory: true)
        var fileName = recordName.replacingOccurrences(of: ":", with: "_")
        fileName = fileName.replacingOccurrences(of: "/", with: "_")
        let destination = directory.appendingPathComponent(fileName)
        do {
            try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
            // Copies of files that live in iCloud anyway: not worth a place in the backup.
            var folder = directory
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? folder.setResourceValues(values)
            if fileManager.fileExists(atPath: destination.path) {
                try fileManager.removeItem(at: destination)
            }
            try write(destination)
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
        if stopped { return }
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

    private func emit(_ event: CloudEvent) async {
        if stopped { return }
        await onEvent(event)
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
