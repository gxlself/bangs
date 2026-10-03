import SwiftUI

/// Rows other programs docked on the notch; see docs/plugins.md.
struct BoardView: View {
    @EnvironmentObject private var store: WatchStore

    var body: some View {
        Group {
            if let activities = store.snapshot?.activities, !activities.isEmpty {
                List(activities) { activity in
                    ActivityRow(activity: activity)
                }
            } else if store.snapshot == nil {
                ProgressView()
            } else {
                EmptyNote(symbol: "square.grid.2x2", text: t("上岛板是空的", "Nothing on the board"))
            }
        }
        .page(t("上岛", "Board"), tint: .gray)
    }
}

private struct ActivityRow: View {
    let activity: Activity

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: symbol)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .frame(width: 18)
            VStack(alignment: .leading, spacing: 3) {
                Text(activity.title)
                    .font(.headline)
                    .lineLimit(2)
                if let subtitle = activity.subtitle {
                    Text(subtitle)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                if let progress = activity.progress {
                    ProgressView(value: progress)
                        .tint(Palette.focus)
                }
            }
        }
        .padding(.vertical, 2)
    }

    /// The glyph names plugins may use, drawn with SF Symbols.
    private var symbol: String {
        switch activity.icon {
        case "board": return "square.grid.2x2"
        case "music": return "music.note"
        case "shelf": return "tray"
        case "code": return "chevron.left.forwardslash.chevron.right"
        case "clipboard": return "doc.on.clipboard"
        case "folder": return "folder"
        case "check": return "checkmark.circle"
        case "todo": return "checklist"
        case "pin": return "pin"
        default: return "circle.fill"
        }
    }
}
