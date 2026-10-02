import Foundation

/// The version of one record: who wrote it and when (docs/sync.md, "合并规则").
public struct Version: Equatable, Hashable, Sendable, Codable {
    public var updatedAt: Int64
    public var device: String

    public init(updatedAt: Int64, device: String) {
        self.updatedAt = updatedAt
        self.device = device
    }

    /// Last writer wins: the later `updatedAt` wins; on a tie the device id that sorts
    /// higher by UTF-8 bytes wins. Identical versions: neither wins.
    public static func wins(_ a: Version, _ b: Version) -> Bool {
        if a.updatedAt != b.updatedAt {
            return a.updatedAt > b.updatedAt
        }
        // Byte order, like Rust's `str::cmp`. Swift's own `<` on String compares
        // canonical-equivalent text, which is not the same thing.
        return b.device.utf8.lexicographicallyPrecedes(a.device.utf8)
    }

    /// The `updatedAt` for a new local version: the clock, but always strictly after the
    /// version it replaces, so one device's edits to one record only ever go forward.
    public static func nextStamp(prev: Int64?, now: Int64) -> Int64 {
        guard let prev = prev else { return now }
        let after = prev < Int64.max ? prev + 1 : prev
        return now > after ? now : after
    }
}
