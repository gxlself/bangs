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

/// A short confirmation that floats at the bottom ("Copied").
struct ToastView: View {
    let text: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundColor(.accentColor)
            Text(text)
                .font(.subheadline.weight(.medium))
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 11)
        .background(.ultraThinMaterial, in: Capsule())
        .shadow(color: Color.black.opacity(0.15), radius: 12, x: 0, y: 4)
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
