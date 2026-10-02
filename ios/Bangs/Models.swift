import Foundation
import BangsSyncCore

// What the views show, decoded from the `body` of each synced record (docs/sync.md, "body").
// Field names are the contract's; every field is optional because the Mac may add, drop or
// send something unexpected, and a record we cannot read must never take the app down.

// MARK: - todo

private struct TodoBody: Codable {
    var text: String?
    var createdAt: Int64?
    var done: Bool?
    var doneAt: Int64?
}

struct TodoItem: Identifiable, Equatable {
    let id: String
    let text: String
    let createdAt: Int64
    /// Ticked off; ticking again brings it back. Deleting is separate.
    let done: Bool
    /// When it was ticked off.
    let doneAt: Int64?

    init(id: String, text: String, createdAt: Int64, done: Bool = false, doneAt: Int64? = nil) {
        self.id = id
        self.text = text
        self.createdAt = createdAt
        self.done = done
        self.doneAt = done ? doneAt : nil
    }

    init?(record: SyncRecord) {
        guard record.kind == "todo", !record.deleted,
              let body = record.decodeBody(TodoBody.self),
              let text = body.text, !text.isEmpty
        else {
            return nil
        }
        // A record from before there was a done state reads as not done.
        let done = body.done ?? false
        self.init(
            id: record.id,
            text: text,
            createdAt: body.createdAt ?? record.updatedAt,
            done: done,
            doneAt: done ? (body.doneAt ?? record.updatedAt) : nil
        )
    }

    /// The record body, in the contract's field names (docs/sync.md).
    var body: JSONValue {
        var members: [String: JSONValue] = [
            "text": .string(text),
            "createdAt": .int(createdAt),
            "done": .bool(done),
        ]
        members["doneAt"] = doneAt.map { JSONValue.int($0) } ?? .null
        return .object(members)
    }

    /// Open lines newest first, then done lines most recently done first — the Mac's order.
    static func isOrderedBefore(_ a: TodoItem, _ b: TodoItem) -> Bool {
        if a.done != b.done { return !a.done }
        if a.done {
            let left = a.doneAt ?? a.createdAt
            let right = b.doneAt ?? b.createdAt
            if left != right { return left > right }
        } else if a.createdAt != b.createdAt {
            return a.createdAt > b.createdAt
        }
        return a.id < b.id
    }
}

// MARK: - session

private struct SessionBody: Codable {
    var agent: String?
    var name: String?
    var project: String?
    var path: String?
    var status: String?
    var detail: String?
    var statusAt: Int64?
    var host: String?
}

enum SessionStatus: String {
    case waiting
    case busy
    case idle

    /// Smaller sorts first: the one that needs you comes first.
    var rank: Int {
        switch self {
        case .waiting: return 0
        case .busy: return 1
        case .idle: return 2
        }
    }
}

struct SessionItem: Identifiable, Equatable {
    let id: String
    /// The short id of the Mac it runs on: the record id is `<device8>-<session id>`.
    var deviceID: String {
        let recordID = id.hasPrefix("session:") ? String(id.dropFirst("session:".count)) : id
        return String(recordID.prefix(while: { $0 != "-" }))
    }
    let agent: String
    let name: String
    let project: String
    let path: String
    let status: SessionStatus
    let detail: String?
    let statusAt: Int64
    let host: String

    init(
        id: String, agent: String, name: String, project: String, path: String,
        status: SessionStatus, detail: String?, statusAt: Int64, host: String
    ) {
        self.id = id
        self.agent = agent
        self.name = name
        self.project = project
        self.path = path
        self.status = status
        self.detail = detail
        self.statusAt = statusAt
        self.host = host
    }

    init?(record: SyncRecord) {
        guard record.kind == "session", !record.deleted,
              let body = record.decodeBody(SessionBody.self)
        else {
            return nil
        }
        let project = body.project ?? ""
        let name = body.name ?? ""
        let detail = body.detail?.trimmingCharacters(in: .whitespacesAndNewlines)
        self.init(
            id: record.key,
            agent: body.agent ?? "",
            name: name.isEmpty ? project : name,
            project: project.isEmpty ? name : project,
            path: body.path ?? "",
            status: SessionStatus(rawValue: body.status ?? "") ?? .idle,
            detail: (detail?.isEmpty ?? true) ? nil : detail,
            statusAt: body.statusAt ?? record.updatedAt,
            host: (body.host ?? "").isEmpty ? t("未知设备", "Unknown device") : (body.host ?? "")
        )
    }

    /// Waiting first, then busy, then idle; inside a status, the newest change first.
    static func isOrderedBefore(_ a: SessionItem, _ b: SessionItem) -> Bool {
        if a.status.rank != b.status.rank { return a.status.rank < b.status.rank }
        if a.statusAt != b.statusAt { return a.statusAt > b.statusAt }
        return a.id < b.id
    }
}

struct SessionGroup: Identifiable {
    let deviceID: String
    let host: String
    let items: [SessionItem]
    var id: String { deviceID }

    /// Sessions grouped by Mac; a Mac with something waiting comes first.
    static func groups(from sessions: [SessionItem]) -> [SessionGroup] {
        var byDevice: [String: [SessionItem]] = [:]
        for session in sessions {
            byDevice[session.deviceID, default: []].append(session)
        }
        var groups: [SessionGroup] = []
        for (deviceID, items) in byDevice {
            let sorted = items.sorted(by: SessionItem.isOrderedBefore)
            groups.append(SessionGroup(deviceID: deviceID, host: sorted.first?.host ?? "", items: sorted))
        }
        groups.sort { lhs, rhs in
            let l = lhs.items.first?.status.rank ?? 3
            let r = rhs.items.first?.status.rank ?? 3
            if l != r { return l < r }
            return lhs.host.localizedStandardCompare(rhs.host) == .orderedAscending
        }
        return groups
    }
}

// MARK: - clip

private struct ClipBody: Codable {
    var type: String?
    var preview: String?
    var text: String?
    var app: String?
    var pinned: Bool?
    var createdAt: Int64?
    var host: String?
    var hasImage: Bool?
}

struct ClipItem: Identifiable, Equatable {
    let id: String
    let type: String
    let preview: String
    let text: String?
    let app: String
    let pinned: Bool
    let createdAt: Int64
    let host: String
    /// A picture: the Mac sent it (as a JPEG, end-to-end encrypted) and it is downloaded, or
    /// on its way. False when it was too big to send.
    let hasImage: Bool

    init(
        id: String, type: String, preview: String, text: String?, app: String,
        pinned: Bool, createdAt: Int64, host: String, hasImage: Bool = false
    ) {
        self.id = id
        self.type = type
        self.preview = preview
        self.text = text
        self.app = app
        self.pinned = pinned
        self.createdAt = createdAt
        self.host = host
        self.hasImage = hasImage
    }

    var isImage: Bool { type == "image" }
    var isFiles: Bool { type == "files" }

    init?(record: SyncRecord) {
        guard record.kind == "clip", !record.deleted,
              let body = record.decodeBody(ClipBody.self)
        else {
            return nil
        }
        self.init(
            id: record.key,
            type: body.type ?? "text",
            preview: body.preview ?? "",
            text: body.text,
            app: body.app ?? "",
            pinned: body.pinned ?? false,
            createdAt: body.createdAt ?? record.updatedAt,
            host: body.host ?? "",
            hasImage: body.hasImage ?? false
        )
    }

    /// What tapping copies: the full text when there is one, otherwise the preview line.
    var copyText: String {
        if let text = text, !text.isEmpty { return text }
        return preview
    }

    /// Pinned first, then newest first.
    static func isOrderedBefore(_ a: ClipItem, _ b: ClipItem) -> Bool {
        if a.pinned != b.pinned { return a.pinned }
        if a.createdAt != b.createdAt { return a.createdAt > b.createdAt }
        return a.id < b.id
    }
}

// MARK: - shelf

private struct ShelfBody: Codable {
    var name: String?
    var fileExtension: String?
    var size: Int64?
    var isImage: Bool?
    var addedAt: Int64?
    var host: String?

    enum CodingKeys: String, CodingKey {
        case name
        case fileExtension = "extension"
        case size
        case isImage
        case addedAt
        case host
    }
}

struct ShelfItem: Identifiable, Equatable {
    /// Files bigger than this are never uploaded (docs/sync.md), so there is no asset to wait for.
    static let syncLimitBytes: Int64 = 25_000_000

    let id: String
    let name: String
    let fileExtension: String
    let size: Int64
    let isImage: Bool
    let addedAt: Int64
    let host: String

    init(
        id: String, name: String, fileExtension: String, size: Int64,
        isImage: Bool, addedAt: Int64, host: String
    ) {
        self.id = id
        self.name = name
        self.fileExtension = fileExtension
        self.size = size
        self.isImage = isImage
        self.addedAt = addedAt
        self.host = host
    }

    init?(record: SyncRecord) {
        guard record.kind == "shelf", !record.deleted,
              let body = record.decodeBody(ShelfBody.self)
        else {
            return nil
        }
        self.init(
            id: record.key,
            name: (body.name ?? "").isEmpty ? record.id : (body.name ?? ""),
            fileExtension: body.fileExtension ?? "",
            size: body.size ?? 0,
            isImage: body.isImage ?? false,
            addedAt: body.addedAt ?? record.updatedAt,
            host: body.host ?? ""
        )
    }

    var tooLargeToSync: Bool {
        return size > ShelfItem.syncLimitBytes
    }

    /// Newest first.
    static func isOrderedBefore(_ a: ShelfItem, _ b: ShelfItem) -> Bool {
        if a.addedAt != b.addedAt { return a.addedAt > b.addedAt }
        return a.id < b.id
    }
}
