import SwiftUI

/// The notch's to-do list. Tap a line when it is done; it is gone for good,
/// as on the Mac.
struct TodosView: View {
    @EnvironmentObject private var store: WatchStore
    @State private var draft = ""

    var body: some View {
        NavigationStack {
            Connected { state in
                List {
                    TextField("添加待办", text: $draft)
                        .onSubmit {
                            store.addTodo(draft)
                            draft = ""
                        }
                    ForEach(state.todos) { todo in
                        Button {
                            store.removeTodo(todo.id)
                        } label: {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Image(systemName: "circle")
                                    .foregroundStyle(.secondary)
                                Text(todo.text)
                                    .lineLimit(3)
                            }
                        }
                    }
                    if state.todos.isEmpty {
                        Text("清单是空的")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle("待办")
        }
    }
}
