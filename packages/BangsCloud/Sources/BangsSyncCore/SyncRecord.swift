import Foundation

/// JSON encoder/decoder settings shared by the store file, the CloudKit `body` field and the
/// C ABI. Always make a fresh one: the coders are classes and are not Sendable.
public enum SyncJSON {
    public static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return encoder
    }

    public static func decoder() -> JSONDecoder {
        return JSONDecoder()
    }
}

/// Any JSON value. `SyncRecord.body` is one of these so that kinds and fields this build does
/// not know about survive a round trip untouched.
public enum JSONValue: Codable, Equatable, Hashable, Sendable {
    case null
    case bool(Bool)
    case int(Int64)
    case double(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        // Integers before booleans: some Foundation versions read the numbers 0 and 1 as
        // Bool, but none of them read `true` or `false` as an integer.
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Int64.self) {
            self = .int(value)
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .double(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else {
            throw DecodingError.dataCorruptedError(in: container, debugDescription: "Not a JSON value")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case .bool(let value):
            try container.encode(value)
        case .int(let value):
            try container.encode(value)
        case .double(let value):
            try container.encode(value)
        case .string(let value):
            try container.encode(value)
        case .array(let value):
            try container.encode(value)
        case .object(let value):
            try container.encode(value)
        }
    }

    /// Parses JSON text. Returns nil when the text is not JSON.
    public init?(jsonString: String) {
        guard let data = jsonString.data(using: .utf8),
              let value = try? SyncJSON.decoder().decode(JSONValue.self, from: data)
        else {
            return nil
        }
        self = value
    }

    /// Compact JSON text with sorted keys. `{}` if the value somehow cannot be encoded.
    public var jsonString: String {
        guard let data = try? SyncJSON.encoder().encode(self),
              let text = String(data: data, encoding: .utf8)
        else {
            return "{}"
        }
        return text
    }

    public var stringValue: String? {
        if case .string(let value) = self { return value }
        return nil
    }

    public var boolValue: Bool? {
        if case .bool(let value) = self { return value }
        return nil
    }

    public var int64Value: Int64? {
        switch self {
        case .int(let value):
            return value
        case .double(let value):
            return Int64(exactly: value)
        default:
            return nil
        }
    }

    /// The member of an object, or nil for anything else.
    public subscript(key: String) -> JSONValue? {
        if case .object(let members) = self { return members[key] }
        return nil
    }
}

/// One synced record, in the wire format of docs/sync.md ("传输格式").
///
///     {"kind":"todo","id":"18f2a-0","updatedAt":1760000000000,"device":"3f1c…",
///      "deleted":false,"body":{"text":"买牛奶","createdAt":1760000000000},"asset":null}
///
/// `asset` is an absolute local path: the file to upload when sending, the downloaded copy
/// when receiving.
public struct SyncRecord: Codable, Equatable, Hashable, Sendable {
    public var kind: String
    public var id: String
    public var updatedAt: Int64
    public var device: String
    public var deleted: Bool
    public var body: JSONValue
    public var asset: String?

    public init(
        kind: String,
        id: String,
        updatedAt: Int64,
        device: String,
        deleted: Bool,
        body: JSONValue,
        asset: String? = nil
    ) {
        self.kind = kind
        self.id = id
        self.updatedAt = updatedAt
        self.device = device
        self.deleted = deleted
        self.body = body
        self.asset = asset
    }

    /// `kind:id`, which is also the CloudKit recordName.
    public var key: String {
        return kind + ":" + id
    }

    public var version: Version {
        return Version(updatedAt: updatedAt, device: device)
    }

    /// The body decoded into a caller-defined type; nil when it does not fit.
    public func decodeBody<T: Decodable>(_ type: T.Type) -> T? {
        guard let data = try? SyncJSON.encoder().encode(body) else { return nil }
        return try? SyncJSON.decoder().decode(type, from: data)
    }

    private enum CodingKeys: String, CodingKey {
        case kind, id, updatedAt, device, deleted, body, asset
    }

    // Unknown extra fields are ignored; a missing `deleted`, `body` or `asset` gets its
    // neutral value.
    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decode(String.self, forKey: .kind)
        id = try container.decode(String.self, forKey: .id)
        updatedAt = try container.decode(Int64.self, forKey: .updatedAt)
        device = try container.decode(String.self, forKey: .device)
        deleted = try container.decodeIfPresent(Bool.self, forKey: .deleted) ?? false
        body = try container.decodeIfPresent(JSONValue.self, forKey: .body) ?? JSONValue.object([:])
        asset = try container.decodeIfPresent(String.self, forKey: .asset)
    }

    // `asset` is written as null rather than left out, as the Rust side expects.
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(kind, forKey: .kind)
        try container.encode(id, forKey: .id)
        try container.encode(updatedAt, forKey: .updatedAt)
        try container.encode(device, forKey: .device)
        try container.encode(deleted, forKey: .deleted)
        try container.encode(body, forKey: .body)
        if let asset = asset {
            try container.encode(asset, forKey: .asset)
        } else {
            try container.encodeNil(forKey: .asset)
        }
    }
}
