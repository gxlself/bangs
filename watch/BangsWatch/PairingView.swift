import SwiftUI

/// Macs found in iCloud first; below them, the address and code the Mac's
/// tray shows under Bangs → 手表, for a Mac on another Apple ID or without iCloud.
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
                switch store.cloudSearch {
                case .idle, .searching:
                    HStack {
                        ProgressView()
                            .frame(width: 20)
                        Text("正在 iCloud 里找 Mac")
                            .font(.footnote)
                    }
                case .done where store.cloudMacs.isEmpty:
                    Text("这个 Apple 账号下还没有 Mac 打开「允许手表连接」")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                case .done:
                    ForEach(store.cloudMacs) { mac in
                        Button {
                            Task { await connect(mac) }
                        } label: {
                            Label(mac.host, systemImage: "laptopcomputer")
                        }
                        .disabled(busy)
                    }
                case .failed(let reason):
                    Text("读不了 iCloud：\(reason)")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Button("再找一次") {
                    Task { await store.searchICloud() }
                }
                .disabled(store.cloudSearch == .searching)
            } header: {
                Text("同一 Apple 账号")
            }
            Section {
                TextField("地址，如 192.168.1.8", text: $address)
                TextField("6 位配对码", text: $code)
            } header: {
                Text("用配对码")
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
        .task { await store.searchICloud() }
    }

    private func connect(_ mac: MacRecord) async {
        busy = true
        error = nil
        do {
            try await store.connect(mac)
        } catch {
            self.error = error.localizedDescription
        }
        busy = false
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
