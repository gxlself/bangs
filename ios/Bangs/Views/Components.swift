import SwiftUI

/// A centred symbol, headline and explanation, for a tab with nothing to show yet.
struct EmptyStateView: View {
    let symbol: String
    let title: String
    let message: String

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 40, weight: .light))
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text(title)
                .font(.headline)
            Text(message)
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// A short message that floats at the bottom: "Copied", or why not.
struct ToastView: View {
    let text: String
    var success = true

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: success ? "checkmark.circle.fill" : "info.circle.fill")
                .foregroundStyle(success ? Color.accentColor : Color.secondary)
                .accessibilityHidden(true)
            Text(text)
                .font(.subheadline.weight(.medium))
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 11)
        .background(.ultraThinMaterial, in: Capsule())
        .shadow(color: Color.black.opacity(0.15), radius: 12, x: 0, y: 4)
    }
}

/// A toast to show: set it and it shows for a moment, is read out by VoiceOver, then goes.
struct Toast: Equatable {
    let text: String
    var success = true
    /// A second "Copied" in a row is a new toast, with its own time on screen.
    fileprivate let id = UUID()
}

private struct ToastModifier: ViewModifier {
    @Binding var toast: Toast?

    func body(content: Content) -> some View {
        content
            .overlay(alignment: .bottom) {
                if let toast = toast {
                    ToastView(text: toast.text, success: toast.success)
                        .padding(.bottom, 24)
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .animation(.spring(response: 0.3, dampingFraction: 0.85), value: toast)
            .task(id: toast?.id) {
                guard let shown = toast else { return }
                UIAccessibility.post(notification: .announcement, argument: shown.text)
                // Why something did not work takes longer to read than "Copied".
                try? await Task.sleep(nanoseconds: shown.success ? 1_400_000_000 : 2_400_000_000)
                if Task.isCancelled { return }
                if toast?.id == shown.id {
                    toast = nil
                }
            }
    }
}

extension View {
    func toast(_ toast: Binding<Toast?>) -> some View {
        modifier(ToastModifier(toast: toast))
    }
}

/// The message an empty tab shows, chosen from what is actually going on: still loading, this
/// iPhone not signed in, a Mac connected but with nothing to show, or no Mac syncing yet.
struct SyncEmptyState: View {
    @EnvironmentObject private var model: SyncModel
    let symbol: String
    let title: String
    /// What a connected Mac with nothing to show says, in the Mac's own words.
    let connectedButEmpty: String

    var body: some View {
        switch model.emptyReason {
        case .loading:
            VStack(spacing: 12) {
                ProgressView()
                Text(t("正在从 iCloud 载入…", "Loading from iCloud…"))
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity)
        case .notSignedIn:
            EmptyStateView(
                symbol: "icloud.slash",
                title: title,
                message: t(
                    "这台 iPhone 没有登录 iCloud，登录后才能和 Mac 同步。",
                    "This iPhone isn't signed in to iCloud, so nothing syncs with your Mac yet."
                )
            )
        case .macConnected:
            EmptyStateView(symbol: symbol, title: title, message: connectedButEmpty)
        case .noMac:
            EmptyStateView(symbol: symbol, title: title, message: SyncModel.turnOnOnMac)
        }
    }
}

/// The gear in the navigation bar of every tab, which opens the settings sheet.
struct SettingsButtonModifier: ViewModifier {
    @State private var showSettings = false

    func body(content: Content) -> some View {
        content
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button {
                        showSettings = true
                    } label: {
                        Image(systemName: "gearshape")
                    }
                    .accessibilityLabel(t("设置", "Settings"))
                }
            }
            .sheet(isPresented: $showSettings) {
                SettingsView()
            }
    }
}

extension View {
    func settingsButton() -> some View {
        modifier(SettingsButtonModifier())
    }
}
