import SwiftUI
import UIKit

struct ClipboardView: View {
    @EnvironmentObject private var model: SyncModel
    @State private var showToast = false
    @State private var toastCounter = 0

    var body: some View {
        Group {
            if model.clips.isEmpty {
                EmptyStateView(
                    symbol: "doc.on.clipboard",
                    title: t("剪贴板是空的", "Clipboard is empty"),
                    message: t(
                        "在 Mac 上复制的内容会同步到这里，点一下就能拷贝到手机。Mac 上的 Bangs 需要打开 iCloud 同步。",
                        "Things you copy on your Mac show up here; tap one to copy it to this phone. Bangs on your Mac needs iCloud sync turned on."
                    )
                )
            } else {
                List {
                    ForEach(model.clips) { clip in
                        Button {
                            copy(clip)
                        } label: {
                            ClipRow(clip: clip)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .listStyle(.insetGrouped)
                .refreshable {
                    await model.syncNow()
                }
            }
        }
        .navigationTitle(t("剪贴板", "Clipboard"))
        .overlay(alignment: .bottom) {
            if showToast {
                ToastView(text: t("已拷贝", "Copied"))
                    .padding(.bottom, 24)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .settingsButton()
    }

    private func copy(_ clip: ClipItem) {
        UIPasteboard.general.string = clip.copyText
        toastCounter += 1
        let mine = toastCounter
        withAnimation {
            showToast = true
        }
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 1_300_000_000)
            if toastCounter == mine {
                withAnimation {
                    showToast = false
                }
            }
        }
    }
}

private struct ClipRow: View {
    let clip: ClipItem

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(.body)
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 24)

            VStack(alignment: .leading, spacing: 4) {
                Text(clip.preview.isEmpty ? t("（空）", "(empty)") : clip.preview)
                    .font(.body)
                    .lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Text(footer)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            if clip.pinned {
                Image(systemName: "pin.fill")
                    .font(.caption)
                    .foregroundColor(.orange)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
    }

    private var symbol: String {
        switch clip.type {
        case "image": return "photo"
        case "files": return "doc"
        default: return "text.alignleft"
        }
    }

    private var footer: String {
        let when = Format.relative(clip.createdAt)
        if clip.app.isEmpty {
            return when
        }
        return clip.app + " · " + when
    }
}
