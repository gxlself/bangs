import SwiftUI
import UIKit

struct ClipboardView: View {
    @EnvironmentObject private var model: SyncModel
    @State private var toast: Toast?

    var body: some View {
        Group {
            if model.clips.isEmpty {
                // Scrollable, so pull-to-refresh works before there is anything to show.
                ScrollView {
                    SyncEmptyState(
                        symbol: "doc.on.clipboard",
                        title: t("剪贴板是空的", "Clipboard is empty"),
                        connectedButEmpty: t(
                            "Mac 上还没有剪贴板历史。在 Mac 上复制的文字和图片会出现在这里，点一下就复制到这台 iPhone。",
                            "No clipboard history on your Mac yet. Text and pictures you copy there show up here; tap one to copy it to this iPhone."
                        )
                    )
                    .padding(.top, 96)
                }
                .refreshable {
                    await model.syncNow()
                }
            } else {
                // Redrawn every minute, so "3 minutes ago" stays true.
                TimelineView(.everyMinute) { context in
                    List {
                        ForEach(model.clips) { clip in
                            Button {
                                copy(clip)
                            } label: {
                                ClipRow(clip: clip, picture: picture(of: clip), now: context.date)
                            }
                        }
                    }
                }
                .listStyle(.insetGrouped)
                .refreshable {
                    await model.syncNow()
                }
            }
        }
        .navigationTitle(t("剪贴板", "Clipboard"))
        .toast($toast)
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
                show(
                    clip.hasImage
                        ? t("图片还在下载，稍等一下", "The picture is still downloading")
                        : t("这张图片太大，没有同步过来", "This picture was too big to sync"),
                    success: false
                )
                return
            }
            UIPasteboard.general.image = image
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            show(t("已复制图片", "Picture copied"), success: true)
        } else if clip.isFiles {
            show(t("文件只能在 Mac 上粘贴", "Files can only be pasted on the Mac"), success: false)
        } else {
            UIPasteboard.general.string = clip.copyText
            UINotificationFeedbackGenerator().notificationOccurred(.success)
            show(t("已复制", "Copied"), success: true)
        }
    }

    private func show(_ text: String, success: Bool) {
        toast = Toast(text: text, success: success)
    }
}

private struct ClipRow: View {
    let clip: ClipItem
    let picture: URL?
    let now: Date

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
                    .foregroundStyle(.primary)
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
                    .accessibilityLabel(t("已置顶", "Pinned"))
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityHint(clip.isFiles ? "" : t("复制到这台 iPhone", "Copies it to this iPhone"))
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
        parts.append(Format.relative(clip.createdAt, now: now))
        if clip.isImage && picture == nil {
            parts.append(clip.hasImage ? t("下载中…", "downloading…") : t("太大，未同步", "too big to sync"))
        }
        return parts.joined(separator: " · ")
    }
}
