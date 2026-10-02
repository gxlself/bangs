import Foundation

/// Small formatters for the lists. Times in the synced records are Unix milliseconds.
enum Format {
    static func date(fromMillis ms: Int64) -> Date {
        return Date(timeIntervalSince1970: Double(ms) / 1000)
    }

    private static let relativeFormatter: RelativeDateTimeFormatter = {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        formatter.dateTimeStyle = .named
        formatter.locale = Locale(identifier: t("zh-Hans", "en"))
        return formatter
    }()

    /// "just now" / "刚刚" for the last minute (and a clock a little ahead), then "3 minutes
    /// ago" / "3 分钟前" — the Mac's words.
    static func relative(_ ms: Int64, now: Date = Date()) -> String {
        let then = date(fromMillis: ms)
        if now.timeIntervalSince(then) < 60 {
            return t("刚刚", "just now")
        }
        return relativeFormatter.localizedString(for: then, relativeTo: now)
    }

    static func relative(_ date: Date, now: Date = Date()) -> String {
        return relative(Int64(date.timeIntervalSince1970 * 1000), now: now)
    }

    static func bytes(_ count: Int64) -> String {
        return ByteCountFormatter.string(fromByteCount: count, countStyle: .file)
    }

    static func lastSync(_ date: Date?, now: Date = Date()) -> String {
        guard let date = date else { return t("还没同步过", "Not synced yet") }
        return relative(date, now: now)
    }
}
