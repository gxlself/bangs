import SwiftUI

struct ShelfView: View {
    @EnvironmentObject private var model: SyncModel
    @State private var preview: PreviewFile?

    var body: some View {
        Group {
            if model.shelf.isEmpty {
                // Scrollable, so pull-to-refresh works before there is anything to show.
                ScrollView {
                    EmptyStateView(
                        symbol: "tray.full",
                        title: t("文件架是空的", "The shelf is empty"),
                        message: t(
                            "放进 Mac 文件架的文件会出现在这里，25 MB 以内的可以直接预览。Mac 上的 Bangs 需要打开 iCloud 同步。",
                            "Files you put on the shelf on your Mac show up here; files up to 25 MB can be previewed. Bangs on your Mac needs iCloud sync turned on."
                        )
                    )
                        .padding(.top, 96)
                }
                .refreshable {
                    await model.syncNow()
                }
            } else {
                List {
                    ForEach(model.shelf) { item in
                        Button {
                            open(item)
                        } label: {
                            ShelfRow(
                                item: item,
                                state: state(of: item),
                                picture: state(of: item) == .available
                                    ? AssetFiles.url(forKey: item.id, stateDirectory: model.stateDirectory)
                                    : nil
                            )
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
        .navigationTitle(t("文件架", "Shelf"))
        .sheet(item: $preview) { file in
            QuickLookPreview(url: file.url)
                .ignoresSafeArea()
        }
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
        guard state(of: item) == .available else { return }
        if let url = AssetFiles.previewCopy(for: item, stateDirectory: model.stateDirectory) {
            preview = PreviewFile(url: url)
        }
    }
}

private enum ShelfFileState {
    case available
    case tooLarge
    case downloading
}

private struct ShelfRow: View {
    let item: ShelfItem
    let state: ShelfFileState
    /// The downloaded file, when there is one.
    let picture: URL?

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
                    .lineLimit(2)
                Text(details)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if let caption = caption {
                    Text(caption)
                        .font(.caption)
                        .foregroundColor(.orange)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if state == .available {
                Image(systemName: "eye")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
    }

    private var details: String {
        var parts: [String] = [Format.bytes(item.size)]
        if !item.host.isEmpty {
            parts.append(item.host)
        }
        parts.append(Format.relative(item.addedAt))
        return parts.joined(separator: " · ")
    }

    private var caption: String? {
        switch state {
        case .available:
            return nil
        case .tooLarge:
            return t("太大，未同步", "Too large to sync")
        case .downloading:
            return t("等待下载…", "Downloading…")
        }
    }
}
