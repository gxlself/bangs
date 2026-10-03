import SwiftUI

/// The address and code the Mac's tray shows under Bangs → 手表.
struct PairingView: View {
    @EnvironmentObject private var store: WatchStore
    /// Kept so pairing again after "取消配对" only needs a new code.
    @AppStorage("lastAddress") private var address = ""
    @State private var code = ""
    @State private var busy = false
    @State private var error: String?

    private var digits: String {
        code.filter(\.isNumber)
    }

    var body: some View {
        Form {
            Section {
                TextField("地址，如 192.168.1.8", text: $address)
                TextField("6 位配对码", text: $code)
            } footer: {
                Text("在 Mac 上点菜单栏的 Bangs 图标 → 手表 → 允许手表连接，就能看到地址和配对码。")
            }
            Button {
                Task { await pair() }
            } label: {
                if busy {
                    ProgressView()
                } else {
                    Text("配对")
                }
            }
            .disabled(busy || address.isEmpty || digits.count != 6)
            if let error {
                Text(error)
                    .font(.footnote)
                    .foregroundStyle(.red)
            }
        }
        .navigationTitle("连接 Bangs")
    }

    private func pair() async {
        busy = true
        error = nil
        do {
            try await store.pair(address: address, code: digits)
            code = ""
        } catch {
            self.error = error.localizedDescription
        }
        busy = false
    }
}
