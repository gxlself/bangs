import SwiftUI

struct RootView: View {
    @EnvironmentObject private var model: SyncModel
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        TabView(selection: $model.selectedTab) {
            NavigationStack {
                TodoView()
            }
            .tabItem {
                Label(t("待办", "To-do"), systemImage: "checklist")
            }
            .tag(AppTab.todo)

            NavigationStack {
                DevView()
            }
            .tabItem {
                Label(t("开发", "Dev"), systemImage: "terminal")
            }
            .badge(waitingCount)
            .tag(AppTab.dev)

            NavigationStack {
                ClipboardView()
            }
            .tabItem {
                Label(t("剪贴板", "Clipboard"), systemImage: "doc.on.clipboard")
            }
            .tag(AppTab.clipboard)

            NavigationStack {
                ShelfView()
            }
            .tabItem {
                Label(t("文件架", "Shelf"), systemImage: "tray.full")
            }
            .tag(AppTab.shelf)
        }
        .onAppear {
            model.start()
            model.setActive(scenePhase == .active)
        }
        .onChange(of: scenePhase) { phase in
            model.setActive(phase == .active)
        }
    }

    /// Sessions waiting for an answer, on the Dev tab's badge.
    private var waitingCount: Int {
        model.sessions.filter { $0.status == .waiting }.count
    }
}
