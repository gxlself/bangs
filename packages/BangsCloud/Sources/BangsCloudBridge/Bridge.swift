#if os(macOS)

import Foundation
import Security
import CloudKit
import BangsSyncCore
import BangsCloud

// The C ABI of docs/sync.md ("macOS 原生桥"), called from Rust. Five functions:
//
//   int32_t bangs_cloud_supported(void);
//   void    bangs_cloud_start(const char *state_dir, void (*callback)(const char *event_json));
//   void    bangs_cloud_push(const char *records_json);
//   void    bangs_cloud_pull(void);
//   void    bangs_cloud_stop(void);
//
// Every CloudEvent is turned into one line of JSON and handed to the callback. The callback may
// be invoked on any thread, and the string is only valid during the call.

typealias BangsEventCallback = @convention(c) (UnsafePointer<CChar>) -> Void

// MARK: - Entitlement check

/// Creating a CKContainer in a process that is not signed with the iCloud entitlements
/// crashes the process (it cannot be caught). Everything here checks this first, the same
/// way Paste's ICloudCapability does.
private enum BridgeSigning {
    static let containerID = "iCloud.com.gxlself.bangs"

    static func entitlement(_ key: String) -> CFTypeRef? {
        guard let task = SecTaskCreateFromSelf(nil) else { return nil }
        return SecTaskCopyValueForEntitlement(task, key as CFString, nil)
    }

    static func isSupported() -> Bool {
        guard let services = entitlement("com.apple.developer.icloud-services") else {
            return false
        }
        if let list = services as? [String], !list.contains("CloudKit") {
            return false
        }
        // The container must be listed too, or CloudKit raises as soon as it is opened.
        guard let containers = entitlement("com.apple.developer.icloud-container-identifiers") as? [String] else {
            return false
        }
        return containers.contains(containerID)
    }
}

// MARK: - Events as JSON

private struct WireEvent: Encodable {
    var event: String
    var state: String?
    var message: String?
    var records: [SyncRecord]?
    var keys: [String]?
    var retryAfter: Int?
    var reason: String?
}

private func statusEvent(_ state: String, message: String? = nil) -> WireEvent {
    return WireEvent(event: "status", state: state, message: message)
}

private func wireEvent(for event: CloudEvent) -> WireEvent {
    switch event {
    case .status(let status):
        switch status {
        case .starting:
            return statusEvent("starting")
        case .ready:
            return statusEvent("ready")
        case .syncing:
            return statusEvent("syncing")
        case .idle:
            return statusEvent("idle")
        case .noAccount:
            return statusEvent("noAccount")
        case .restricted:
            return statusEvent("restricted")
        case .unavailable:
            return statusEvent("unavailable")
        case .error(let message):
            return statusEvent("error", message: message)
        }
    case .records(let records):
        return WireEvent(event: "records", records: records)
    case .pushed(let keys):
        return WireEvent(event: "pushed", keys: keys)
    case .rejected(let keys):
        return WireEvent(event: "rejected", keys: keys)
    case .failed(let keys, let message, let retryAfter):
        return WireEvent(event: "failed", message: message, keys: keys, retryAfter: retryAfter)
    case .reset(let reason):
        return WireEvent(event: "reset", reason: reason)
    }
}

private func deliver(_ event: WireEvent, to callback: BangsEventCallback) {
    guard let data = try? SyncJSON.encoder().encode(event),
          let json = String(data: data, encoding: .utf8)
    else {
        logError("could not encode event \(event.event)")
        return
    }
    // The pointer is only valid inside this closure, which is all the contract promises.
    json.withCString { pointer in
        callback(pointer)
    }
}

private func logError(_ message: String) {
    let line = "BangsCloudBridge: " + message + "\n"
    FileHandle.standardError.write(Data(line.utf8))
}

// MARK: - Global state

private final class BridgeState: @unchecked Sendable {
    private let lock = NSLock()
    private var engine: CloudEngine?
    private var callback: BangsEventCallback?
    private var accountObserver: NSObjectProtocol?

    /// Starts the engine again whenever the iCloud account changes, once per process.
    func observeAccountChanges() {
        lock.lock()
        defer { lock.unlock() }
        if accountObserver != nil { return }
        accountObserver = NotificationCenter.default.addObserver(
            forName: .CKAccountChanged,
            object: nil,
            queue: nil
        ) { [weak self] _ in
            guard let engine = self?.currentEngine() else { return }
            Task { await engine.start() }
        }
    }

    /// Installs a new engine and callback; returns the engine it replaced.
    func install(engine: CloudEngine?, callback: BangsEventCallback?) -> CloudEngine? {
        lock.lock()
        defer { lock.unlock() }
        let old = self.engine
        self.engine = engine
        self.callback = callback
        return old
    }

    /// Removes the engine; returns it so the caller can stop it.
    func removeEngine() -> CloudEngine? {
        lock.lock()
        defer { lock.unlock() }
        let old = engine
        engine = nil
        return old
    }

    func currentEngine() -> CloudEngine? {
        lock.lock()
        defer { lock.unlock() }
        return engine
    }

    func currentCallback() -> BangsEventCallback? {
        lock.lock()
        defer { lock.unlock() }
        return callback
    }
}

private let bridgeState = BridgeState()

/// A pull or push that cannot run still owes the host its closing status event.
private func reportError(_ message: String) {
    if let callback = bridgeState.currentCallback() {
        deliver(statusEvent("error", message: message), to: callback)
    } else {
        logError(message)
    }
}

// MARK: - C ABI

@_cdecl("bangs_cloud_supported")
public func bangsCloudSupported() -> Int32 {
    return BridgeSigning.isSupported() ? 1 : 0
}

@_cdecl("bangs_cloud_start")
public func bangsCloudStart(
    _ stateDir: UnsafePointer<CChar>,
    _ callbackOrNil: (@convention(c) (UnsafePointer<CChar>) -> Void)?
) {
    // Optional only so that the pointer is plainly allowed to outlive this call; Rust always
    // passes a real function.
    guard let callback = callbackOrNil else {
        logError("start: callback is null")
        return
    }
    let directory = URL(fileURLWithPath: String(cString: stateDir), isDirectory: true)

    // Never touch CloudKit before this check passes.
    guard BridgeSigning.isSupported() else {
        let old = bridgeState.install(engine: nil, callback: callback)
        if let old = old {
            Task { await old.stop() }
        }
        deliver(statusEvent("unavailable", message: "not signed with iCloud entitlements"), to: callback)
        return
    }

    let engine = CloudEngine(
        containerID: BridgeSigning.containerID,
        stateDirectory: directory,
        // The Mac only gets back the shelf files it uploaded; no point keeping copies.
        keepAssets: false,
        onEvent: { event in
            deliver(wireEvent(for: event), to: callback)
        }
    )
    let old = bridgeState.install(engine: engine, callback: callback)
    if let old = old {
        Task { await old.stop() }
    }
    bridgeState.observeAccountChanges()
    Task { await engine.start() }
}

@_cdecl("bangs_cloud_push")
public func bangsCloudPush(_ recordsJSON: UnsafePointer<CChar>) {
    let data = Data(String(cString: recordsJSON).utf8)
    let records: [SyncRecord]
    do {
        records = try SyncJSON.decoder().decode([SyncRecord].self, from: data)
    } catch {
        // No keys are known for a batch that does not parse, so there is nothing to mark
        // as failed. The host still gets its closing status event.
        logError("push: cannot decode records: \(error)")
        reportError("push: invalid records JSON")
        return
    }
    guard let engine = bridgeState.currentEngine() else {
        reportError("push: iCloud sync is not running")
        return
    }
    Task { await engine.push(records) }
}

@_cdecl("bangs_cloud_pull")
public func bangsCloudPull() {
    guard let engine = bridgeState.currentEngine() else {
        reportError("pull: iCloud sync is not running")
        return
    }
    Task { await engine.pull() }
}

@_cdecl("bangs_cloud_stop")
public func bangsCloudStop() {
    if let old = bridgeState.removeEngine() {
        Task { await old.stop() }
    }
}

#endif
