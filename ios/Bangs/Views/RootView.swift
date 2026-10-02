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
                Label(t("代码", "Code"), systemImage: "terminal")
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
                Label(t("暂存", "Shelf"), systemImage: "tray.full")
            }
            .tag(AppTab.shelf)
        }
        .onAppear {
            model.start()
            model.setActive(scenePhase == .active)
        }
        .onChange(of: scenePhase) { phase in
            model.setActive(phase == .active)
            if phase == .active {
                // Back from the system settings, maybe with notifications allowed now.
                model.refreshNotificationStatus()
            }
        }
    }

    /// Sessions waiting for an answer on a Mac that is there, on the Dev tab's badge.
    private var waitingCount: Int {
        model.sessions.filter { $0.status == .waiting && model.isOnline($0.deviceID) }.count
    }
}
