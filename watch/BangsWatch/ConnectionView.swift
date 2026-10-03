import SwiftUI

/// Which Mac this watch talks to, whether it is answering, and the way out.
struct ConnectionView: View {
    @EnvironmentObject private var store: WatchStore

    var body: some View {
        NavigationStack {
            List {
                Section {
                    LabeledContent("电脑", value: store.state?.host ?? store.pairing?.host ?? "")
                    LabeledContent("地址", value: store.pairing?.baseURL.host ?? "")
                    LabeledContent("状态", value: status)
                }
                if case .offline(let reason) = store.connection {
                    Text(reason)
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                Button("取消配对", role: .destructive) {
                    store.forget(tellMac: true)
                }
            }
            .navigationTitle("连接")
        }
    }

    private var status: String {
        switch store.connection {
        case .online: return "已连接"
        case .connecting: return "连接中"
        case .offline: return "连不上"
        case .idle: return "未连接"
        }
    }
}
