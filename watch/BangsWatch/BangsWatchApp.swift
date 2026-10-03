import SwiftUI

@main
struct BangsWatchApp: App {
    @StateObject private var store = WatchStore()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(store)
        }
    }
}

/// Pairing until there is a Mac to talk to, then the pages.
struct RootView: View {
    @EnvironmentObject private var store: WatchStore
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        Group {
            if store.pairing == nil {
                NavigationStack {
                    PairingView()
                }
            } else {
                TabView {
                    NowPlayingView()
                    SessionsView()
                    TodosView()
                    ConnectionView()
                }
                .tabViewStyle(.page)
            }
        }
        .onAppear { store.start() }
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .active: store.start()
            case .background: store.stop()
            default: break
            }
        }
    }
}

/// Shows `content` once the Mac has answered, and why not until then.
struct Connected<Content: View>: View {
    @EnvironmentObject var store: WatchStore
    @ViewBuilder var content: (WatchState) -> Content

    var body: some View {
        if let state = store.state {
            content(state)
        } else {
            switch store.connection {
            case .offline(let reason):
                EmptyState(symbol: "wifi.exclamationmark", text: "连不上 Mac\n\(reason)")
            default:
                ProgressView()
            }
        }
    }
}

struct EmptyState: View {
    let symbol: String
    let text: String

    var body: some View {
        VStack(spacing: 6) {
            Image(systemName: symbol)
                .font(.title2)
                .foregroundStyle(.secondary)
            Text(text)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
