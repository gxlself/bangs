import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var store: WatchStore
    @Environment(\.scenePhase) private var phase

    var body: some View {
        Group {
            if store.endpoint == nil {
                PairView()
            } else {
                MainView()
            }
        }
        .onAppear { store.resume() }
        // Keep following while the wrist is down and the app is still in
        // front, so a finished session can still tap; stop once it is not.
        .onChange(of: phase) { _, phase in
            if phase == .background {
                store.pause()
            } else {
                store.resume()
            }
        }
    }
}

/// One page per panel, paged with the Digital Crown.
struct MainView: View {
    @EnvironmentObject private var store: WatchStore
    @State private var page = MainView.initialPage

    var body: some View {
        NavigationStack {
            TabView(selection: $page) {
                NowPlayingView().tag(0)
                SessionsView().tag(1)
                TodosView().tag(2)
                BoardView().tag(3)
            }
            .tabViewStyle(.verticalPage)
        }
        .overlay(alignment: .top) {
            if let banner = store.banner {
                Text(banner)
                    .font(.footnote.weight(.semibold))
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Palette.attention, in: Capsule())
                    .foregroundStyle(.black)
                    .padding(.top, 2)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.spring(duration: 0.35), value: store.banner)
    }
}

extension MainView {
    static var initialPage: Int {
        #if DEBUG
        return DemoData.initialPage
        #else
        return 0
        #endif
    }
}

/// The colors the notch uses on the computer.
enum Palette {
    static let focus = Color(red: 0x94 / 255, green: 0xFA / 255, blue: 0xAB / 255)
    static let attention = Color(red: 0xFF / 255, green: 0xCF / 255, blue: 0x6B / 255)
    static let music = Color(red: 0xFF / 255, green: 0x5A / 255, blue: 0x78 / 255)
    static let calm = Color(red: 0x6B / 255, green: 0xC7 / 255, blue: 0xFA / 255)
}

extension View {
    /// Title, settings button, page tint and the offline note every page shares.
    /// `title` nil leaves the row under the clock to the page itself.
    func page(_ title: String?, tint: Color) -> some View {
        modifier(PageChrome(title: title, tint: tint))
    }
}

private struct PageChrome: ViewModifier {
    @EnvironmentObject private var store: WatchStore
    let title: String?
    let tint: Color

    func body(content: Content) -> some View {
        content
            .navigationTitle(title ?? "")
            .containerBackground(tint.gradient.opacity(0.35), for: .tabView)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    NavigationLink {
                        SettingsView()
                    } label: {
                        Image(systemName: "gearshape")
                    }
                }
            }
            .safeAreaInset(edge: .bottom) {
                if store.connection == .offline {
                    Label(t("连不上 Mac", "Can't reach the Mac"), systemImage: "wifi.slash")
                        .font(.caption2)
                        .foregroundStyle(Palette.attention)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 3)
                        .background(.black.opacity(0.6), in: Capsule())
                }
            }
    }
}

/// What a page shows when there is nothing to show.
struct EmptyNote: View {
    let symbol: String
    let text: String

    var body: some View {
        VStack(spacing: 8) {
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
