import Foundation
import SwiftUI
import CloudKit
import BangsSyncCore
import BangsCloud

private let tombstoneTTLms: Int64 = 90 * 24 * 60 * 60 * 1000
private let todoMaxLength = 200

private func nowMillis() -> Int64 {
    return Int64((Date().timeIntervalSince1970 * 1000).rounded())
}

/// The id of this phone in the `device` field of the records it writes (docs/sync.md: a full
/// UUID). Kept in UserDefaults, so it survives launches and app updates.
private func loadDeviceID() -> String {
    let key = "bangs.sync.deviceID"
    if let saved = UserDefaults.standard.string(forKey: key), !saved.isEmpty {
        return saved
    }
    let created = UUID().uuidString.lowercased()
    UserDefaults.standard.set(created, forKey: key)
    return created
}

/// Everything the views read, and the only thing that talks to the sync engine.
///
/// Flow: load the store from disk, start the engine; when it says `ready`, pull, and once that
/// pull is done push whatever piled up offline. Records from CloudKit go through
/// `RecordStore.applyRemote`, then the lists are rebuilt. Local edits go into the store first
/// (and to disk), then are pushed right away.
@MainActor
final class SyncModel: ObservableObject {
    static let shared = SyncModel()

    static let containerID = "iCloud.com.gxlself.bangs"

    @Published private(set) var todos: [TodoItem] = []
    @Published private(set) var sessions: [SessionItem] = []
    @Published private(set) var clips: [ClipItem] = []
    @Published private(set) var shelf: [ShelfItem] = []
    @Published private(set) var status: CloudStatus = .starting
    @Published private(set) var lastSync: Date? = nil

    let deviceID: String
    let stateDirectory: URL

    private let storeURL: URL
    private var store: RecordStore
    private var engine: CloudEngine?
    private var started = false
    private var pushAfterPull = false
    private var failureStreak = 0
    private var retryTask: Task<Void, Never>?
    private var pollTask: Task<Void, Never>?
    private var accountObserver: NSObjectProtocol?

    private init() {
        let fileManager = FileManager.default
        let support = (try? fileManager.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )) ?? fileManager.temporaryDirectory
        let directory = support.appendingPathComponent("BangsSync", isDirectory: true)
        try? fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("store.json")
        let tokenURL = directory.appendingPathComponent(CloudEngine.tokenFileName)

        var loaded = RecordStore()
        if fileManager.fileExists(atPath: url.path) {
            do {
                loaded = try RecordStore.load(from: url)
            } catch {
                print("[sync] store.json is unreadable, starting over: \(error)")
                try? fileManager.moveItem(at: url, to: url.appendingPathExtension("corrupt"))
                try? fileManager.removeItem(at: tokenURL)
            }
        } else {
            // No records on disk, so a saved change token would skip everything it covers.
            try? fileManager.removeItem(at: tokenURL)
        }
        loaded.purgeTombstones(now: nowMillis(), ttlMs: tombstoneTTLms)

        self.stateDirectory = directory
        self.storeURL = url
        self.deviceID = loadDeviceID()
        self.store = loaded
        publish()
    }

    // MARK: Lifecycle

    /// Creates the engine and starts it. Safe to call more than once.
    func start() {
        if started { return }
        started = true
        let engine = CloudEngine(
            containerID: SyncModel.containerID,
            stateDirectory: stateDirectory,
            onEvent: { [weak self] event in
                Task { @MainActor in
                    self?.handle(event)
                }
            }
        )
        self.engine = engine
        // Signing out, or into another account: look again. A different account arrives as
        // `reset("account")` before `ready`.
        accountObserver = NotificationCenter.default.addObserver(
            forName: .CKAccountChanged,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                guard let engine = self?.engine else { return }
                await engine.accountChanged()
            }
        }
        Task {
            await engine.start()
        }
    }

    /// Called when the app becomes active or goes to the background. While active it syncs
    /// now and then once a minute (silent pushes do not always arrive).
    func setActive(_ active: Bool) {
        pollTask?.cancel()
        pollTask = nil
        guard active else { return }
        start()
        Task {
            await self.syncNow()
        }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 60_000_000_000)
                if Task.isCancelled { return }
                await self?.pollOnce()
            }
        }
    }

    /// Pull, then push what is waiting. Also what pull-to-refresh and the Sync now button call.
    func syncNow() async {
        start()
        guard let engine = engine else { return }
        switch status {
        case .starting:
            // The first start() is still running; its `ready` does the pull and the push.
            return
        case .ready, .syncing, .idle:
            pushAfterPull = true
            await engine.pull()
        case .noAccount, .restricted, .unavailable, .error:
            // Look at the account again; `ready` follows when it is fine, and does the rest.
            await engine.start()
        }
    }

    /// A CloudKit silent push arrived (AppDelegate). Pull, and do not return before the
    /// events have been handled, because the system suspends the app once the handler is done.
    func handleRemoteNotification() async {
        start()
        guard let engine = engine else { return }
        switch status {
        case .ready, .syncing, .idle, .error:
            break
        case .starting, .noAccount, .restricted, .unavailable:
            await engine.start()
        }
        if !store.outbox.isEmpty {
            pushAfterPull = true
        }
        await engine.pull()
        await Task.yield()
    }

    private func pollOnce() async {
        switch status {
        case .ready, .idle, .error:
            // Leftovers (a push that failed, a reset's re-queued lines) go out after this pull.
            if !store.outbox.isEmpty {
                pushAfterPull = true
            }
            await engine?.pull()
        default:
            break
        }
    }

    // MARK: Local edits

    /// Adds a to-do. Returns false when the text is empty after cleaning.
    @discardableResult
    func addTodo(_ raw: String) -> Bool {
        let text = SyncModel.cleanTodo(raw)
        if text.isEmpty { return false }
        let now = nowMillis()
        let item = TodoItem(id: UUID().uuidString.lowercased(), text: text, createdAt: now)
        let record = store.localUpsert(kind: "todo", id: item.id, body: item.body, asset: nil, now: now, device: deviceID)
        if record == nil { return false }
        changed()
        return true
    }

    /// Ticks a line off, or brings a done one back — as on the Mac, ticking does not delete.
    func toggleTodo(id: String) {
        guard let item = todos.first(where: { $0.id == id }) else { return }
        let now = nowMillis()
        let next = TodoItem(id: item.id, text: item.text, createdAt: item.createdAt, done: !item.done, doneAt: now)
        guard store.localUpsert(kind: "todo", id: id, body: next.body, asset: nil, now: now, device: deviceID) != nil else { return }
        changed()
    }

    /// Deletes a line, here and on the Mac: a tombstone goes out.
    func deleteTodo(id: String) {
        guard store.localDelete(kind: "todo", id: id, now: nowMillis(), device: deviceID) != nil else { return }
        changed()
    }

    /// Deletes every done line.
    func clearCompletedTodos() {
        let now = nowMillis()
        var any = false
        for item in todos where item.done {
            if store.localDelete(kind: "todo", id: item.id, now: now, device: deviceID) != nil {
                any = true
            }
        }
        if any { changed() }
    }

    /// After a local edit: on disk, on screen, on its way.
    private func changed() {
        saveStore()
        publish()
        pushSoon()
    }

    /// One line: newlines and tabs become spaces, trimmed, at most 200 characters.
    static func cleanTodo(_ raw: String) -> String {
        let flattened = String(raw.map { character -> Character in
            if character.isNewline || character == "\t" { return " " }
            return character
        })
        var text = flattened.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.count > todoMaxLength {
            text = String(text.prefix(todoMaxLength)).trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return text
    }

    // MARK: Engine events

    private func handle(_ event: CloudEvent) {
        switch event {
        case .status(let newStatus):
            status = newStatus
            switch newStatus {
            case .ready:
                // Pull first; the push follows when that pull is over.
                pushAfterPull = true
                runPull()
            case .idle:
                lastSync = Date()
                pushIfWaiting()
            case .error:
                pushIfWaiting()
            default:
                break
            }
        case .records(let records):
            let adopted = store.applyRemote(records)
            if !adopted.isEmpty {
                saveStore()
                publish()
                // A file taken off the Mac's shelf: its downloaded copy goes too.
                for record in adopted where record.kind == "shelf" && record.deleted {
                    try? FileManager.default.removeItem(at: AssetFiles.url(forKey: record.key, stateDirectory: stateDirectory))
                }
            }
        case .pushed(let keys):
            store.markSettled(keys: keys)
            failureStreak = 0
            saveStore()
            pushIfMore()
        case .rejected(let keys):
            // Lost a conflict (the winning record comes as a `records` event), or a change
            // CloudKit will never take.
            store.markSettled(keys: keys)
            saveStore()
            pushIfMore()
        case .reset(let reason):
            if reason == "account" {
                // Another iCloud account: what is here belongs to the old one.
                store = RecordStore()
                try? FileManager.default.removeItem(
                    at: stateDirectory.appendingPathComponent(CloudEngine.assetsDirectoryName, isDirectory: true)
                )
            } else {
                // The zone was emptied: put this phone's to-dos back. The Mac puts back its own.
                store.requeue(kind: "todo")
            }
            saveStore()
            publish()
            // A reset comes in the middle of a pull or before `ready`; the queue goes out after.
            pushAfterPull = true
        case .failed(let keys, let message, let retryAfter):
            print("[sync] push failed: \(message)")
            store.restoreInflight(keys: keys)
            saveStore()
            failureStreak += 1
            let backoff = min(30 * (1 << min(failureStreak - 1, 4)), 600)
            scheduleRetry(afterSeconds: max(retryAfter ?? backoff, 1))
        }
    }

    private func runPull() {
        guard let engine = engine else { return }
        Task {
            await engine.pull()
        }
    }

    /// A key held back while its earlier version was in flight is free now.
    private func pushIfMore() {
        if !store.outbox.isEmpty {
            pushSoon()
        }
    }

    /// The first idle (or error) after a pull that asked for it: now push the queue.
    private func pushIfWaiting() {
        if pushAfterPull {
            pushAfterPull = false
            pushSoon()
        }
    }

    // MARK: Pushing

    private var canPush: Bool {
        switch status {
        case .ready, .syncing, .idle, .error:
            return true
        case .starting, .noAccount, .restricted, .unavailable:
            return false
        }
    }

    private func pushSoon() {
        Task {
            await self.pushOutbox()
        }
    }

    private func pushOutbox() async {
        guard let engine = engine, canPush else { return }
        let batch = store.takeOutbox()
        if batch.isEmpty { return }
        saveStore()
        await engine.push(batch)
    }

    private func scheduleRetry(afterSeconds seconds: Int) {
        retryTask?.cancel()
        retryTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(seconds) * 1_000_000_000)
            if Task.isCancelled { return }
            await self?.pushOutbox()
        }
    }

    // MARK: Store and published lists

    private func saveStore() {
        do {
            try store.save(to: storeURL)
        } catch {
            print("[sync] cannot save store: \(error)")
        }
    }

    private func publish() {
        todos = store.live(kind: "todo")
            .compactMap { TodoItem(record: $0) }
            .sorted(by: TodoItem.isOrderedBefore)
        sessions = store.live(kind: "session")
            .compactMap { SessionItem(record: $0) }
            .sorted(by: SessionItem.isOrderedBefore)
        clips = store.live(kind: "clip")
            .compactMap { ClipItem(record: $0) }
            .sorted(by: ClipItem.isOrderedBefore)
        shelf = store.live(kind: "shelf")
            .compactMap { ShelfItem(record: $0) }
            .sorted(by: ShelfItem.isOrderedBefore)
    }
}
