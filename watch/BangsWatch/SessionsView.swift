import SwiftUI

/// Claude Code and Codex sessions on the Mac, as the notch's dev panel lists them.
struct SessionsView: View {
    var body: some View {
        NavigationStack {
            Connected { state in
                if state.sessions.isEmpty {
                    EmptyState(symbol: "terminal", text: "没有 Claude 或 Codex 会话")
                } else {
                    List(state.sessions) { session in
                        SessionRow(session: session)
                    }
                }
            }
            .navigationTitle("会话")
        }
    }
}

struct SessionRow: View {
    let session: Session

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(session.status.color)
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 2) {
                Text(session.project.isEmpty ? session.name : session.project)
                    .font(.headline)
                    .lineLimit(1)
                Text(subtitle)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
    }

    private var subtitle: String {
        let agent = session.agent == "codex" ? "Codex" : "Claude"
        if session.status == .waiting, let detail = session.detail, !detail.isEmpty {
            return "\(agent) · \(detail)"
        }
        return "\(agent) · \(session.status.label)"
    }
}

extension Session.Status {
    /// The same words the notch uses.
    var label: String {
        switch self {
        case .busy: return "运行中"
        case .waiting: return "等你回复"
        case .idle: return "空闲"
        }
    }

    /// The notch's dot colors (src/styles.css).
    var color: Color {
        switch self {
        case .busy: return Color(red: 0.580, green: 0.980, blue: 0.671)
        case .waiting: return Color(red: 1.000, green: 0.812, blue: 0.420)
        case .idle: return Color.gray.opacity(0.6)
        }
    }
}
