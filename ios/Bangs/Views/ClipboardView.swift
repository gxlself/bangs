import SwiftUI
import UIKit

struct ClipboardView: View {
    @EnvironmentObject private var model: SyncModel
    @State private var toast: String?
    @State private var toastCounter = 0

    var body: some View {
        Group {
            if model.clips.isEmpty {
                // Scrollable, so pull-to-refresh works before there is anything to show.
                ScrollView {
                    EmptyStateView(
                        symbol: "doc.on.clipboard",
                        title: t("剪贴板是空的", "Clipboard is empty"),
                        message: t(
                            "在 Mac 上复制的文字和图片会同步到这里，点一下就能拷贝到手机。Mac 上的 Bangs 需要打开 iCloud 同步。",
                            "Text and pictures you copy on your Mac show up here; tap one to copy it to this phone. Bangs on your Mac needs iCloud sync turned on."
                        )
                    )
                        .padding(.top, 96)
                }
                .refreshable {
                    await model.syncNow()
                }
            } else {
                List {
                    ForEach(model.clips) { clip in
                        Button {
                            copy(clip)
                        } label: {
                            ClipRow(clip: clip, picture: picture(of: clip))
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
            if let toast = toast {
                ToastView(text: toast)
                    .padding(.bottom, 24)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .settingsButton()
    }

    /// The downloaded picture of an image entry, found by its key at display time.
    private func picture(of clip: ClipItem) -> URL? {
        guard clip.isImage else { return nil }
        let url = AssetFiles.url(forKey: clip.id, stateDirectory: model.stateDirectory)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    private func copy(_ clip: ClipItem) {
        if clip.isImage {
            guard let url = picture(of: clip), let image = UIImage(contentsOfFile: url.path) else {
                show(clip.hasImage ? t("图片还在下载…", "Still downloading the picture…") : t("这张图片太大，没有同步过来", "This picture was too big to sync"))
                return
            }
            UIPasteboard.general.image = image
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            show(t("已拷贝图片", "Picture copied"))
        } else if clip.isFiles {
            show(t("文件只能在 Mac 上粘贴", "Files can only be pasted on the Mac"))
        } else {
            UIPasteboard.general.string = clip.copyText
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            show(t("已拷贝", "Copied"))
        }
    }

    private func show(_ text: String) {
        toastCounter += 1
        let mine = toastCounter
        withAnimation {
            toast = text
        }
        Task { @MainActor in
            try? await Task.sleep(nanoseconds: 1_400_000_000)
            if toastCounter == mine {
                withAnimation {
                    toast = nil
                }
            }
        }
    }
}

private struct ClipRow: View {
    let clip: ClipItem
    let picture: URL?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            if let picture = picture {
                ThumbnailView(url: picture)
            } else {
                Image(systemName: symbol)
                    .font(.body)
                    .foregroundStyle(.secondary)
                    .frame(width: 24, height: 24)
            }

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
        if clip.isImage { return "photo" }
        if clip.isFiles { return "doc" }
        return "text.alignleft"
    }

    private var footer: String {
        var parts: [String] = []
        if !clip.app.isEmpty {
            parts.append(clip.app)
        }
        parts.append(Format.relative(clip.createdAt))
        if clip.isImage && picture == nil {
            parts.append(clip.hasImage ? t("下载中", "downloading") : t("太大，未同步", "too big to sync"))
        }
        return parts.joined(separator: " · ")
    }
}
