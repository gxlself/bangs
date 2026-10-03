import Foundation

/// Bangs speaks Chinese or English, like the app on the computer. Call sites
/// pass both texts rather than a key, so the words stay where they are read.
let speaksChinese = Locale.preferredLanguages.first?.lowercased().hasPrefix("zh") ?? false

func t(_ zh: String, _ en: String) -> String {
    speaksChinese ? zh : en
}

/// `3:07`, or `1:02:03` past the hour.
func clock(_ seconds: Double) -> String {
    let total = max(0, Int(seconds.rounded(.down)))
    let hours = total / 3600
    let minutes = total / 60 % 60
    let rest = total % 60
    return hours > 0
        ? String(format: "%d:%02d:%02d", hours, minutes, rest)
        : String(format: "%d:%02d", minutes, rest)
}
