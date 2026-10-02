import SwiftUI
import UIKit

/// Open lines first; tap the circle to tick one off (it moves to Done, struck through) and tap
/// again to bring it back — as on the Mac, ticking never deletes. Deleting is a swipe (or the
/// context menu): the line comes apart before it goes, like the notch's dust.
struct TodoView: View {
    @EnvironmentObject private var model: SyncModel
    @State private var draft = ""
    /// Lines coming apart right now.
    @State private var dissolving: Set<String> = []
    @State private var confirmClear = false

    private var open: [TodoItem] { model.todos.filter { !$0.done } }
    private var done: [TodoItem] { model.todos.filter { $0.done } }

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
                if open.isEmpty {
                    Text(model.todos.isEmpty ? t("没有待办", "Nothing to do") : t("都做完了", "All done"))
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(open) { todo in
                        row(todo)
                    }
                }
            }

            if !done.isEmpty {
                Section {
                    ForEach(done) { todo in
                        row(todo)
                    }
                } header: {
                    HStack {
                        Text(t("已完成", "Done"))
                        Spacer()
                        Button(t("清除", "Clear")) {
                            confirmClear = true
                        }
                        .font(.footnote.weight(.semibold))
                        .textCase(nil)
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .animation(.default, value: model.todos)
        .navigationTitle(t("待办", "To-do"))
        .refreshable {
            await model.syncNow()
        }
        .confirmationDialog(
            t("删除全部已完成的待办？", "Delete every done item?"),
            isPresented: $confirmClear,
            titleVisibility: .visible
        ) {
            Button(t("删除 \(done.count) 项", "Delete \(done.count)"), role: .destructive) {
                dissolve(done.map(\.id)) {
                    model.clearCompletedTodos()
                }
            }
        }
        .settingsButton()
    }

    private func row(_ todo: TodoItem) -> some View {
        TodoRow(todo: todo, dissolving: dissolving.contains(todo.id)) {
            toggle(todo)
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
            Button(role: .destructive) {
                delete(todo)
            } label: {
                Label(t("删除", "Delete"), systemImage: "trash")
            }
        }
        .swipeActions(edge: .leading, allowsFullSwipe: true) {
            Button {
                toggle(todo)
            } label: {
                if todo.done {
                    Label(t("恢复", "Undo"), systemImage: "arrow.uturn.backward")
                } else {
                    Label(t("完成", "Done"), systemImage: "checkmark")
                }
            }
            .tint(todo.done ? .gray : .green)
        }
        .contextMenu {
            Button {
                toggle(todo)
            } label: {
                if todo.done {
                    Label(t("标记为未完成", "Mark as not done"), systemImage: "arrow.uturn.backward")
                } else {
                    Label(t("标记为完成", "Mark as done"), systemImage: "checkmark.circle")
                }
            }
            Button {
                UIPasteboard.general.string = todo.text
            } label: {
                Label(t("拷贝", "Copy"), systemImage: "doc.on.doc")
            }
            Button(role: .destructive) {
                delete(todo)
            } label: {
                Label(t("删除", "Delete"), systemImage: "trash")
            }
        }
    }

    private func submit() {
        if model.addTodo(draft) {
            draft = ""
        }
    }

    private func toggle(_ todo: TodoItem) {
        guard !dissolving.contains(todo.id) else { return }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        model.toggleTodo(id: todo.id)
    }

    private func delete(_ todo: TodoItem) {
        dissolve([todo.id]) {
            model.deleteTodo(id: todo.id)
        }
    }

    /// Lets the lines come apart, then takes them off the list.
    private func dissolve(_ ids: [String], then remove: @escaping () -> Void) {
        let fresh = ids.filter { !dissolving.contains($0) }
        guard !fresh.isEmpty else { return }
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        withAnimation(.easeOut(duration: TodoRow.dissolveSeconds)) {
            dissolving.formUnion(fresh)
        }
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: UInt64(TodoRow.dissolveSeconds * 1_000_000_000))
            remove()
            dissolving.subtract(fresh)
        }
    }
}

private struct TodoRow: View {
    static let dissolveSeconds = 0.45

    let todo: TodoItem
    let dissolving: Bool
    let onToggle: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button {
                onToggle()
            } label: {
                Image(systemName: todo.done ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(todo.done ? Color.accentColor : Color.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(todo.done ? t("标记为未完成", "Mark as not done") : t("标记为完成", "Mark as done"))

            Text(todo.text)
                .strikethrough(todo.done, color: .secondary)
                .foregroundStyle(todo.done ? Color.secondary : Color.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                // Coming apart: blurs, drifts up and away, fades — the notch's dust, simplified.
                .blur(radius: dissolving ? 6 : 0)
                .offset(x: dissolving ? 18 : 0, y: dissolving ? -8 : 0)
                .scaleEffect(dissolving ? 0.9 : 1, anchor: .leading)
                .opacity(dissolving ? 0 : 1)
        }
        .contentShape(Rectangle())
    }
}
