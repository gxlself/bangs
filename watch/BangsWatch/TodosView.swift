import SwiftUI

/// The notch's to-do list. Tap a line when it is done; it is gone for good,
/// as it is on the computer.
struct TodosView: View {
    @EnvironmentObject private var store: WatchStore
    @State private var draft = ""

    var body: some View {
        List {
            TextField(t("添加一条…", "Add a to-do…"), text: $draft)
                .onSubmit {
                    store.addTodo(draft)
                    draft = ""
                }
            ForEach(store.todos) { todo in
                Button {
                    store.complete(todo)
                } label: {
                    Label {
                        Text(todo.text)
                            .lineLimit(3)
                    } icon: {
                        Image(systemName: "circle")
                            .foregroundStyle(Palette.calm)
                    }
                }
            }
            if store.snapshot != nil && store.todos.isEmpty {
                Text(t("没有要做的事", "Nothing to do"))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .listRowBackground(Color.clear)
            }
        }
        .animation(.default, value: store.todos)
        .page(t("待办", "To-do"), tint: Palette.calm)
    }
}
