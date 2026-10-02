import Foundation

/// Small formatters for the lists. Times in the synced records are Unix milliseconds.
enum Format {
    static func date(fromMillis ms: Int64) -> Date {
        return Date(timeIntervalSince1970: Double(ms) / 1000)
    }

    /// "3 min ago" / "3 分钟前". A time slightly in the future (clock skew) reads as now.
    static func relative(_ ms: Int64) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        formatter.locale = Locale(identifier: t("zh-Hans", "en"))
        let now = Date()
        let then = min(date(fromMillis: ms), now)
        return formatter.localizedString(for: then, relativeTo: now)
    }

    static func bytes(_ count: Int64) -> String {
        return ByteCountFormatter.string(fromByteCount: count, countStyle: .file)
    }

    static func lastSync(_ date: Date?) -> String {
        guard let date = date else { return t("还没同步过", "Not synced yet") }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: t("zh-Hans", "en"))
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }
}
