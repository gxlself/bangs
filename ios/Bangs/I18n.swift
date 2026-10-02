import Foundation

/// Chinese or English, following the system language. Same convention as the desktop app
/// (src/lib/i18n.ts): call sites pass both texts, so the words stay where they are read:
/// `t("设置", "Settings")`.
func t(_ zh: String, _ en: String) -> String {
    let language = Locale.preferredLanguages.first ?? "en"
    return language.hasPrefix("zh") ? zh : en
}
