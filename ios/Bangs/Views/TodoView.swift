import SwiftUI

struct TodoView: View {
    @EnvironmentObject private var model: SyncModel
    @State private var draft = ""

    var body: some View {
        List {
            Section {
                HStack(spacing: 10) {
                    Image(systemName: "plus.circle.fill")
                        .foregroundColor(.accentColor)
                    TextField(t("加一件事，回车记下", "Add something, press return"), text: $draft)
                        .submitLabel(.done)
                        .onSubmit {
                            submit()
                        }
                }
            }

            Section {
                if model.todos.isEmpty {
                    Text(t("没有待办", "Nothing to do"))
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(model.todos) { todo in
                        TodoRow(todo: todo) {
                            complete(todo)
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle(t("待办", "To-do"))
        .refreshable {
            await model.syncNow()
        }
        .settingsButton()
    }

    private func submit() {
        if model.addTodo(draft) {
            draft = ""
        }
    }

    private func complete(_ todo: TodoItem) {
        withAnimation {
            model.completeTodo(id: todo.id)
        }
    }
}

private struct TodoRow: View {
    let todo: TodoItem
    let onComplete: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button {
                onComplete()
            } label: {
                Image(systemName: "circle")
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(t("完成", "Done"))

            Text(todo.text)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
            Button {
                onComplete()
            } label: {
                Label(t("完成", "Done"), systemImage: "checkmark")
            }
            .tint(.green)
        }
    }
}
