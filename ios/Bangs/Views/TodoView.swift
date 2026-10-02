import SwiftUI
import UIKit

/// Open lines first; tap a line to tick it off (it moves to Done, struck through) and tap again
/// to bring it back — as on the Mac, ticking never deletes. Deleting is a swipe (or the context
/// menu): the line comes apart before it goes, like the notch's dust.
struct TodoView: View {
    @EnvironmentObject private var model: SyncModel
    @State private var draft = ""
    /// Lines coming apart right now.
    @State private var dissolving: Set<String> = []
    @State private var confirmClear = false
    @FocusState private var adding: Bool

    private var open: [TodoItem] { model.todos.filter { !$0.done } }
    private var done: [TodoItem] { model.todos.filter { $0.done } }

    var body: some View {
        List {
            Section {
                HStack(spacing: 10) {
                    Image(systemName: "plus.circle.fill")
                        .foregroundColor(.accentColor)
                        .accessibilityHidden(true)
                    TextField(
                        model.todoListFull
                            ? t("列表满了，先完成几件吧", "The list is full — finish a few first")
                            : t("加一件事", "Add something"),
                        text: $draft
                    )
                    .focused($adding)
                    .submitLabel(.done)
                    .disabled(model.todoListFull)
                    .onSubmit {
                        submit()
                    }
                }
            } footer: {
                if case .noAccount = model.status {
                    Text(t(
                        "这台 iPhone 没有登录 iCloud，待办只存在这台手机上，登录后会同步到 Mac。",
                        "This iPhone isn't signed in to iCloud; to-dos stay on this phone until it is, then sync to your Mac."
                    ))
                }
            }

            Section {
                if open.isEmpty {
                    Text(model.todos.isEmpty ? t("这里还空着", "Nothing on the list") : t("都做完了", "All done"))
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
                        .confirmationDialog(
                            done.count == 1
                                ? t("清除 1 件已完成的待办？", "Clear 1 done to-do?")
                                : t("清除 \(done.count) 件已完成的待办？", "Clear \(done.count) done to-dos?"),
                            isPresented: $confirmClear,
                            titleVisibility: .visible
                        ) {
                            Button(t("清除", "Clear"), role: .destructive) {
                                // Exactly the lines that were done when asked: one ticked or
                                // unticked meanwhile is not part of the answer.
                                let ids = done.map(\.id)
                                dissolve(ids) {
                                    model.deleteTodos(ids: ids, onlyIfDone: true)
                                }
                            }
                        } message: {
                            Text(t("Mac 上也会一起删掉。", "They'll be deleted on your Mac too."))
                        }
                    }
                    .textCase(nil)
                }
            }
        }
        .listStyle(.insetGrouped)
        .animation(.default, value: model.todos)
        .navigationTitle(t("待办", "To-do"))
        .refreshable {
            await model.syncNow()
        }
        .settingsButton()
    }

    private func row(_ todo: TodoItem) -> some View {
        TodoRow(todo: todo, dissolving: dissolving.contains(todo.id)) {
            toggle(todo)
        }
        // No `role: .destructive`: with it the List takes the row away at once and the dust
        // would play on nothing.
        .swipeActions(edge: .trailing, allowsFullSwipe: true) {
            Button {
                delete(todo)
            } label: {
                Label(t("删除", "Delete"), systemImage: "trash")
            }
            .tint(.red)
        }
        .swipeActions(edge: .leading, allowsFullSwipe: true) {
            Button {
                toggle(todo)
            } label: {
                if todo.done {
                    Label(t("恢复", "Reopen"), systemImage: "arrow.uturn.backward")
                } else {
                    Label(t("完成", "Done"), systemImage: "checkmark")
                }
            }
            .tint(todo.done ? .gray : .accentColor)
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
                Label(t("复制", "Copy"), systemImage: "doc.on.doc")
            }
            Button(role: .destructive) {
                delete(todo)
            } label: {
                Label(t("删除", "Delete"), systemImage: "trash")
            }
        }
    }

    private func submit() {
        guard model.addTodo(draft) else { return }
        draft = ""
        // Another line is likely: keep the keyboard. An empty return still closes it.
        DispatchQueue.main.async {
            adding = true
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

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Image(systemName: todo.done ? "checkmark.circle.fill" : "circle")
                .font(.title3)
                .foregroundStyle(todo.done ? Color.accentColor : Color.secondary)

            Text(todo.text)
                .strikethrough(todo.done, color: .secondary)
                .foregroundStyle(todo.done ? Color.secondary : Color.primary)
                .frame(maxWidth: .infinity, alignment: .leading)
                // Coming apart: blurs, drifts up and away, fades — the notch's dust, simplified.
                // With Reduce Motion it only fades.
                .blur(radius: dissolving && !reduceMotion ? 6 : 0)
                .offset(x: dissolving && !reduceMotion ? 18 : 0, y: dissolving && !reduceMotion ? -8 : 0)
                .scaleEffect(dissolving && !reduceMotion ? 0.9 : 1, anchor: .leading)
                .opacity(dissolving ? 0 : 1)
        }
        // The whole line is the switch, as on the Mac.
        .contentShape(Rectangle())
        .onTapGesture(perform: onToggle)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(todo.text)
        .accessibilityValue(todo.done ? t("已完成", "Done") : "")
        .accessibilityAddTraits(.isButton)
        .accessibilityHint(todo.done ? t("点两下恢复", "Double-tap to reopen") : t("点两下标记为完成", "Double-tap to mark as done"))
    }
}
