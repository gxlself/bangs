import SwiftUI

struct DevView: View {
    @EnvironmentObject private var model: SyncModel

    var body: some View {
        Group {
            if model.sessions.isEmpty {
                // Scrollable, so pull-to-refresh works before there is anything to show.
                ScrollView {
                    EmptyStateView(
                        symbol: "terminal",
                        title: t("还没有会话", "No sessions yet"),
                        message: t(
                            "Mac 上的 Bangs 需要打开 iCloud 同步，Claude Code / Codex 的会话才会出现在这里。",
                            "Bangs on your Mac needs iCloud sync turned on before Claude Code and Codex sessions show up here."
                        )
                    )
                        .padding(.top, 96)
                }
                .refreshable {
                    await model.syncNow()
                }
            } else {
                List {
                    ForEach(SessionGroup.groups(from: model.sessions)) { group in
                        Section(header: Text(group.host)) {
                            ForEach(group.items) { session in
                                SessionRow(session: session)
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
        .navigationTitle(t("开发", "Dev"))
        .settingsButton()
    }
}

private struct SessionRow: View {
    let session: SessionItem

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: agentSymbol)
                .font(.title3)
                .frame(width: 30, height: 30)
                .foregroundColor(.accentColor)

            VStack(alignment: .leading, spacing: 3) {
                Text(session.project)
                    .font(.headline)
                    .lineLimit(1)
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if session.status == .waiting, let detail = session.detail {
                    Text(detail)
                        .font(.subheadline)
                        .foregroundColor(.orange)
                        .lineLimit(3)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            StatusBadge(status: session.status)
        }
        .padding(.vertical, 2)
    }

    private var agentSymbol: String {
        switch session.agent {
        case "claude": return "sparkles"
        case "codex": return "chevron.left.forwardslash.chevron.right"
        default: return "terminal"
        }
    }

    private var subtitle: String {
        let when = Format.relative(session.statusAt)
        if session.name.isEmpty || session.name == session.project {
            return when
        }
        return session.name + " · " + when
    }
}

/// "Waiting" is the loud one: a solid orange pill. Busy and idle stay quiet.
private struct StatusBadge: View {
    let status: SessionStatus

    var body: some View {
        switch status {
        case .waiting:
            Text(t("等你", "Waiting"))
                .font(.caption.weight(.bold))
                .foregroundColor(.white)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(Color.orange, in: Capsule())
        case .busy:
            Text(t("运行中", "Busy"))
                .font(.caption.weight(.semibold))
                .foregroundColor(.blue)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(Color.blue.opacity(0.15), in: Capsule())
        case .idle:
            Text(t("空闲", "Idle"))
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(Color(UIColor.tertiarySystemFill), in: Capsule())
        }
    }
}
