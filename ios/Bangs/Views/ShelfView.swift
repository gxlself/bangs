import SwiftUI

struct ShelfView: View {
    @EnvironmentObject private var model: SyncModel
    @State private var preview: PreviewFile?
    @State private var toast: Toast?

    var body: some View {
        Group {
            if model.shelf.isEmpty {
                // Scrollable, so pull-to-refresh works before there is anything to show.
                ScrollView {
                    SyncEmptyState(
                        symbol: "tray.full",
                        title: t("暂存架是空的", "The shelf is empty"),
                        connectedButEmpty: t(
                            "Mac 的暂存架上没有文件。拖到刘海上暂存的文件会出现在这里，25 MB 以内的可以直接预览。",
                            "Nothing is on your Mac's shelf. Files you drop on the notch show up here; files up to 25 MB can be previewed."
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
                        ForEach(model.shelf) { item in
                            let fileState = state(of: item)
                            Button {
                                open(item)
                            } label: {
                                ShelfRow(
                                    item: item,
                                    state: fileState,
                                    picture: fileState == .available
                                        ? AssetFiles.url(forKey: item.id, stateDirectory: model.stateDirectory)
                                        : nil,
                                    now: context.date
                                )
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
        .navigationTitle(t("暂存架", "Shelf"))
        .sheet(item: $preview) { file in
            QuickLookPreview(url: file.url)
                .ignoresSafeArea()
        }
        .toast($toast)
        .settingsButton()
    }

    /// Looked up on every redraw: the file is found by its key, not by a stored path.
    private func state(of item: ShelfItem) -> ShelfFileState {
        if AssetFiles.exists(forKey: item.id, stateDirectory: model.stateDirectory) {
            return .available
        }
        return item.tooLargeToSync ? .tooLarge : .downloading
    }

    private func open(_ item: ShelfItem) {
        switch state(of: item) {
        case .available:
            if let url = AssetFiles.previewCopy(for: item, stateDirectory: model.stateDirectory) {
                preview = PreviewFile(url: url)
            } else {
                toast = Toast(text: t("打不开这个文件", "Couldn't open this file"), success: false)
            }
        case .tooLarge:
            toast = Toast(text: t("超过 25 MB 的文件只能在 Mac 上打开", "Files over 25 MB stay on the Mac"), success: false)
        case .downloading:
            toast = Toast(text: t("还在下载，稍等一下", "Still downloading"), success: false)
        }
    }
}

private enum ShelfFileState {
    case available
    case tooLarge
    case downloading
}

/// A button's label, so its colours are `Color.primary` / `Color.secondary`: the bare `.primary`
/// and `.secondary` styles would follow the button's tint (see ClipRow).
private struct ShelfRow: View {
    let item: ShelfItem
    let state: ShelfFileState
    /// The downloaded file, when there is one.
    let picture: URL?
    let now: Date

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            if item.isImage, let picture = picture {
                ThumbnailView(url: picture, side: 36)
            } else {
                Image(systemName: item.isImage ? "photo" : "doc")
                    .font(.title3)
                    .foregroundColor(state == .available ? .accentColor : Color.secondary)
                    .frame(width: 36, height: 36)
            }

            VStack(alignment: .leading, spacing: 3) {
                Text(item.name)
                    .font(.body)
                    .foregroundStyle(Color.primary)
                    .lineLimit(2)
                Text(details)
                    .font(.caption)
                    .foregroundStyle(Color.secondary)
                    .lineLimit(1)
                if let caption = caption {
                    Text(caption)
                        .font(.caption)
                        .foregroundStyle(Color.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if state == .available {
                Image(systemName: "eye")
                    .font(.footnote)
                    .foregroundStyle(Color.secondary)
                    .accessibilityHidden(true)
            } else if state == .downloading {
                ProgressView()
                    .controlSize(.small)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityHint(state == .available ? t("预览", "Previews it") : "")
    }

    private var details: String {
        var parts: [String] = [Format.bytes(item.size)]
        if !item.host.isEmpty {
            parts.append(item.host)
        }
        parts.append(Format.relative(item.addedAt, now: now))
        return parts.joined(separator: " · ")
    }

    private var caption: String? {
        switch state {
        case .available:
            return nil
        case .tooLarge:
            return t("太大，未同步", "Too large to sync")
        case .downloading:
            return t("下载中…", "Downloading…")
        }
    }
}
