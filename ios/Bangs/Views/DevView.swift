import SwiftUI

struct DevView: View {
    @EnvironmentObject private var model: SyncModel

    var body: some View {
        Group {
            if model.sessions.isEmpty {
                // Scrollable, so pull-to-refresh works before there is anything to show.
                ScrollView {
                    SyncEmptyState(
                        symbol: "terminal",
                        title: t("还没有会话", "No sessions yet"),
                        connectedButEmpty: t(
                            "Mac 上没有在跑的 Claude Code / Codex 会话。开一个，它就会出现在这里。",
                            "No Claude Code or Codex session is running on your Mac. Start one and it shows up here."
                        )
                    )
                    .padding(.top, 96)
                }
                .refreshable {
                    await model.syncNow()
                }
            } else {
                // Redrawn every minute, so a Mac that stopped sending heartbeats turns offline
                // without anything else happening.
                TimelineView(.everyMinute) { context in
                    List {
                        ForEach(SessionGroup.groups(from: model.sessions)) { group in
                            let online = model.isOnline(group.deviceID, at: context.date)
                            Section(header: GroupHeader(host: group.host, online: online)) {
                                ForEach(group.items) { session in
                                    SessionRow(session: session, online: online, now: context.date)
                                        .opacity(online ? 1 : 0.45)
                                }
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
        .navigationTitle(t("代码", "Code"))
        .onAppear {
            askForNotifications()
        }
        .onChange(of: model.sessions.isEmpty) { _ in
            askForNotifications()
        }
        .settingsButton()
    }

    /// "Notify me when it waits" is worth asking about once there is a session to wait on —
    /// not on an empty page, where the question makes no sense yet.
    private func askForNotifications() {
        if !model.sessions.isEmpty && model.selectedTab == .dev {
            model.requestNotificationPermission()
        }
    }
}

/// The Mac's name, and "offline" when it has not been heard from: asleep, or Bangs quit, so
/// what its sessions say may be out of date.
private struct GroupHeader: View {
    let host: String
    let online: Bool

    var body: some View {
        HStack(spacing: 6) {
            Text(host.isEmpty ? t("未知设备", "Unknown device") : host)
            if !online {
                Text(t("· 离线，状态可能已过期", "· offline, may be out of date"))
                    .foregroundStyle(.secondary)
            }
        }
        // A computer's name is written the way its owner wrote it.
        .textCase(nil)
        .lineLimit(1)
    }
}

private struct SessionRow: View {
    let session: SessionItem
    let online: Bool
    let now: Date

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: agentSymbol)
                .font(.title3)
                .frame(width: 30, height: 30)
                .foregroundColor(.accentColor)
                .accessibilityHidden(true)

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
                        .foregroundStyle(.primary)
                        .lineLimit(3)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            StatusBadge(status: session.status)
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityText)
    }

    private var agentName: String {
        switch session.agent {
        case "claude": return "Claude Code"
        case "codex": return "Codex"
        default: return t("会话", "Session")
        }
    }

    private var agentSymbol: String {
        switch session.agent {
        case "claude": return "sparkles"
        case "codex": return "chevron.left.forwardslash.chevron.right"
        default: return "terminal"
        }
    }

    private var subtitle: String {
        let when = Format.relative(session.statusAt, now: now)
        if session.name.isEmpty || session.name == session.project {
            return when
        }
        return session.name + " · " + when
    }

    /// "Claude Code, my-app, waiting for you: Allow edit?, 3 minutes ago".
    private var accessibilityText: String {
        var parts = [agentName, session.project, session.status.label]
        if session.status == .waiting, let detail = session.detail {
            parts.append(detail)
        }
        parts.append(Format.relative(session.statusAt, now: now))
        if !online {
            parts.append(t("Mac 离线，状态可能已过期", "Mac offline, may be out of date"))
        }
        return parts.joined(separator: ", ")
    }
}

private extension SessionStatus {
    var label: String {
        switch self {
        case .waiting: return t("等你回复", "Waiting for you")
        case .busy: return t("运行中", "Running")
        case .idle: return t("空闲", "Idle")
        }
    }

    /// The pill: short enough for the end of a row.
    var badge: String {
        switch self {
        case .waiting: return t("等你回复", "Waiting")
        case .busy: return t("运行中", "Running")
        case .idle: return t("空闲", "Idle")
        }
    }
}

/// "Waiting" is the loud one: a solid orange pill. Running and idle stay quiet.
private struct StatusBadge: View {
    let status: SessionStatus

    var body: some View {
        switch status {
        case .waiting:
            Text(status.badge)
                .font(.caption.weight(.bold))
                .foregroundColor(.white)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(Color.orange, in: Capsule())
                .fixedSize()
        case .busy:
            Text(status.badge)
                .font(.caption.weight(.semibold))
                .foregroundColor(.blue)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(Color.blue.opacity(0.15), in: Capsule())
                .fixedSize()
        case .idle:
            Text(status.badge)
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 4)
                .background(Color(UIColor.tertiarySystemFill), in: Capsule())
                .fixedSize()
        }
    }
}
