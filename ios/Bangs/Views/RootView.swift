import SwiftUI

struct RootView: View {
    @EnvironmentObject private var model: SyncModel
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        TabView {
            NavigationStack {
                TodoView()
            }
            .tabItem {
                Label(t("待办", "To-do"), systemImage: "checklist")
            }

            NavigationStack {
                DevView()
            }
            .tabItem {
                Label(t("开发", "Dev"), systemImage: "terminal")
            }

            NavigationStack {
                ClipboardView()
            }
            .tabItem {
                Label(t("剪贴板", "Clipboard"), systemImage: "doc.on.clipboard")
            }

            NavigationStack {
                ShelfView()
            }
            .tabItem {
                Label(t("文件架", "Shelf"), systemImage: "tray.full")
            }
        }
        .onAppear {
            model.start()
            model.setActive(scenePhase == .active)
        }
        .onChange(of: scenePhase) { phase in
            model.setActive(phase == .active)
        }
    }
}
