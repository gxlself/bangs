import SwiftUI

/// Which computer this is, and the way out.
struct SettingsView: View {
    @EnvironmentObject private var store: WatchStore
    @Environment(\.dismiss) private var dismiss
    @State private var confirming = false
    @State private var newAddress = ""
    @State private var moving = false
    @State private var failure: String?

    var body: some View {
        List {
            Section {
                LabeledContent(t("电脑", "Computer"), value: store.endpoint?.host ?? "—")
                LabeledContent(t("地址", "Address"), value: address)
                LabeledContent(t("状态", "Status"), value: status)
            }
            Section {
                TextField(t("新地址", "New address"), text: $newAddress)
                    .disabled(moving)
                    .onSubmit {
                        Task { await move() }
                    }
                if moving {
                    ProgressView()
                }
                if let failure {
                    Text(failure)
                        .font(.footnote)
                        .foregroundStyle(Palette.attention)
                }
            } header: {
                Text(t("换地址", "Change address"))
            } footer: {
                Text(t(
                    "Mac 换了网络或 IP 时，填 Bangs 菜单里的新地址，不用重新配对。",
                    "If the Mac moved to another network or IP, enter the new address from the Bangs menu — no need to pair again."
                ))
            }
            Section {
                Button(t("取消配对", "Unpair"), role: .destructive) {
                    confirming = true
                }
            } footer: {
                Text(t(
                    "取消后要在 Mac 上拿新的配对码重新配对。",
                    "To connect again you'll need a new code from the Mac."
                ))
            }
        }
        .navigationTitle(t("设置", "Settings"))
        .confirmationDialog(t("取消和这台电脑的配对？", "Unpair from this computer?"), isPresented: $confirming) {
            Button(t("取消配对", "Unpair"), role: .destructive) {
                Task {
                    await store.unpair()
                    dismiss()
                }
            }
        }
    }

    private func move() async {
        guard !newAddress.isEmpty else { return }
        moving = true
        failure = nil
        defer { moving = false }
        do {
            try await store.move(to: newAddress)
            newAddress = ""
        } catch {
            failure = error.localizedDescription
        }
    }

    private var address: String {
        guard let base = store.endpoint.flatMap({ URL(string: $0.base) }), let host = base.host else { return "—" }
        return base.port.map { "\(host):\($0)" } ?? host
    }

    private var status: String {
        switch store.connection {
        case .online: return t("已连接", "Connected")
        case .offline: return t("连不上", "Unreachable")
        case .connecting: return t("连接中", "Connecting")
        }
    }
}
