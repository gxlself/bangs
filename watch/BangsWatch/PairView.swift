import SwiftUI

/// First run: the address and the one-time code from the Bangs menu on the
/// computer (Apple Watch → Allow the watch to connect).
struct PairView: View {
    @EnvironmentObject private var store: WatchStore
    @State private var address = UserDefaults.standard.string(forKey: Defaults.lastAddress) ?? ""
    @State private var code = ""
    @State private var pairing = false
    @State private var failure: String?

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(t("地址，如 192.168.1.20", "Address, e.g. 192.168.1.20"), text: $address)
                    TextField(t("6 位配对码", "6-digit code"), text: $code)
                } footer: {
                    Text(t(
                        "在 Mac 菜单栏点 Bangs → Apple Watch → 允许手表连接，照着填下面显示的地址和配对码。手表和 Mac 要在同一个网络里。",
                        "On the Mac, open the Bangs menu → Apple Watch → Allow the watch to connect, then enter the address and code it shows. The watch and the Mac need to be on the same network."
                    ))
                }

                Section {
                    Button {
                        Task { await pair() }
                    } label: {
                        HStack {
                            Spacer()
                            if pairing {
                                ProgressView()
                            } else {
                                Text(t("配对", "Pair"))
                                    .fontWeight(.semibold)
                            }
                            Spacer()
                        }
                    }
                    .disabled(pairing || address.isEmpty || code.filter(\.isNumber).count != 6)
                    if let failure {
                        Text(failure)
                            .font(.footnote)
                            .foregroundStyle(Palette.attention)
                    }
                }
            }
            .navigationTitle("Bangs")
        }
    }

    private func pair() async {
        pairing = true
        failure = nil
        defer { pairing = false }
        do {
            try await store.pair(address: address, code: code)
        } catch {
            failure = error.localizedDescription
            code = ""
        }
    }
}
