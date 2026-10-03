import SwiftUI

/// Claude Code and Codex sessions on the computer, busiest first.
struct SessionsView: View {
    @EnvironmentObject private var store: WatchStore

    var body: some View {
        Group {
            if let sessions = store.snapshot?.sessions, !sessions.isEmpty {
                List(sessions) { session in
                    SessionRow(session: session)
                }
            } else if store.snapshot == nil {
                ProgressView()
            } else {
                EmptyNote(symbol: "terminal", text: t("没有在跑的会话", "No sessions running"))
            }
        }
        .page(t("会话", "Sessions"), tint: Palette.focus)
    }
}

private struct SessionRow: View {
    let session: Session

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                StatusDot(status: session.status)
                Text(session.project)
                    .font(.headline)
                    .lineLimit(1)
            }
            HStack(spacing: 4) {
                Text(session.agentName)
                    .font(.caption2.weight(.semibold))
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .background(.white.opacity(0.14), in: Capsule())
                Text(label)
                    .font(.caption2)
                    .foregroundStyle(color)
            }
            if let detail = session.detail, !detail.isEmpty {
                Text(detail)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .padding(.vertical, 2)
    }

    private var label: String {
        switch session.status {
        case "busy": return t("运行中", "Running")
        case "waiting": return t("等你回复", "Waiting for you")
        default: return t("空闲", "Idle")
        }
    }

    private var color: Color {
        StatusDot.color(session.status)
    }
}

/// Green while working, amber while waiting, grey when idle — pulsing
/// for the two that mean something is happening.
private struct StatusDot: View {
    let status: String
    @State private var dim = false

    static func color(_ status: String) -> Color {
        switch status {
        case "busy": return Palette.focus
        case "waiting": return Palette.attention
        default: return .secondary
        }
    }

    var body: some View {
        Circle()
            .fill(Self.color(status))
            .frame(width: 8, height: 8)
            .opacity(dim && status != "idle" ? 0.35 : 1)
            .onAppear {
                guard status != "idle" else { return }
                withAnimation(.easeInOut(duration: 0.7).repeatForever(autoreverses: true)) {
                    dim = true
                }
            }
    }
}
