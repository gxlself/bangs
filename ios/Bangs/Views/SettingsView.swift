import SwiftUI
import BangsCloud

struct SettingsView: View {
    @EnvironmentObject private var model: SyncModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            // Redrawn every minute: "last sync 3 minutes ago" and a Mac going offline stay true.
            TimelineView(.everyMinute) { context in
                List {
                    Section(
                        header: Text("iCloud"),
                        footer: Text(t(
                            "Mac 上的 Bangs 也要打开「与 iPhone 同步（iCloud）」，两边登录同一个 Apple 账户。内容端到端加密，只有你的设备能看到。",
                            "Turn on Sync with iPhone (iCloud) in Bangs on your Mac too, signed in to the same Apple Account. Everything is end-to-end encrypted: only your devices can read it."
                        ))
                    ) {
                        HStack(spacing: 10) {
                            Image(systemName: statusSymbol)
                                .foregroundColor(statusColor)
                                .accessibilityHidden(true)
                            Text(statusTitle)
                        }
                        if let help = statusHelp {
                            Text(help)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
                        }
                        InfoRow(title: t("上次同步", "Last sync"), value: Format.lastSync(model.lastSync, now: context.date))
                        Button {
                            Task {
                                await model.syncNow()
                            }
                        } label: {
                            Text(t("立即同步", "Sync now"))
                        }
                        .disabled(isBusy)
                    }

                    if model.status != .noAccount {
                        Section(header: Text("Mac")) {
                            if model.devices.isEmpty {
                                Text(SyncModel.turnOnOnMac)
                                    .font(.footnote)
                                    .foregroundStyle(.secondary)
                            } else {
                                ForEach(model.devices) { device in
                                    MacRow(device: device, online: model.isOnline(device.id, at: context.date), now: context.date)
                                }
                            }
                        }
                    }

                    Section(
                        header: Text(t("通知", "Notifications")),
                        footer: Text(t(
                            "Mac 上的 Claude Code / Codex 会话停下来问你问题时提醒你。App 不在前台时靠 iCloud 的静默推送唤醒，系统可能会推迟；从多任务里划掉 App 后就收不到了。",
                            "When a Claude Code or Codex session on your Mac stops to ask you something. With the app in the background this relies on iCloud's silent push, which the system may delay, and which stops once the app is swiped away."
                        ))
                    ) {
                        Toggle(
                            t("会话等你时通知", "Notify when a session waits"),
                            isOn: Binding(get: { model.notifyWaiting }, set: { model.setNotifyWaiting($0) })
                        )
                        if model.notifyWaiting && model.notificationsDenied {
                            Button(t("在系统设置里允许 Bangs 发通知", "Allow notifications in Settings")) {
                                if let url = URL(string: UIApplication.openSettingsURLString) {
                                    UIApplication.shared.open(url)
                                }
                            }
                        }
                    }

                    #if DEBUG
                    // Not in the demo's screenshots, which stand for the App Store build.
                    if !DemoData.isOn {
                        Section(
                            header: Text(t("开发者", "Developer")),
                            footer: Text(environmentNote)
                        ) {
                            InfoRow(title: t("iCloud 容器", "iCloud container"), value: SyncModel.containerID)
                            InfoRow(title: t("环境", "Environment"), value: "Development")
                            InfoRow(title: t("设备 ID", "Device ID"), value: shortDeviceID)
                        }
                    }
                    #endif

                    Section {
                        InfoRow(title: t("版本", "Version"), value: appVersion)
                    }
                }
                .listStyle(.insetGrouped)
            }
            .navigationTitle(t("设置", "Settings"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("完成", "Done")) {
                        dismiss()
                    }
                }
            }
            .task {
                model.refreshNotificationStatus()
            }
        }
    }

    private var isBusy: Bool {
        switch model.status {
        case .starting, .syncing:
            return true
        default:
            return false
        }
    }

    private var statusTitle: String {
        switch model.status {
        case .starting:
            return t("正在连接 iCloud…", "Connecting to iCloud…")
        case .ready:
            return t("iCloud 已连接", "Connected to iCloud")
        case .syncing:
            return t("正在同步…", "Syncing…")
        case .idle:
            return t("已是最新", "Up to date")
        case .noAccount:
            return t("没有登录 iCloud", "Not signed in to iCloud")
        case .restricted:
            return t("iCloud 被限制", "iCloud is restricted")
        case .unavailable:
            return t("iCloud 暂时不可用", "iCloud is temporarily unavailable")
        case .error:
            return t("暂时没能同步", "Couldn't sync just now")
        }
    }

    private var statusHelp: String? {
        switch model.status {
        case .noAccount:
            return t(
                "打开「设置」，点最上面的「登录 iPhone」，用 Apple 账户登录并确保 iCloud 已开启，然后回到这里点「立即同步」。",
                "Open Settings, tap Sign in to your iPhone at the top, sign in with your Apple Account and make sure iCloud is on, then come back and tap Sync now."
            )
        case .restricted:
            return t(
                "屏幕使用时间或设备管理限制了 iCloud。放开限制后再试。",
                "Screen Time or device management is restricting iCloud. Lift the restriction and try again."
            )
        case .unavailable:
            return t(
                "可能是网络不好，或 iCloud 在维护。稍后会自动重试，也可以点「立即同步」。",
                "The network may be down or iCloud may be having trouble. Bangs retries on its own, or tap Sync now."
            )
        case .error(let message):
            let reassurance = t(
                "没有丢数据：改动留在这台 iPhone 上，稍后会自动重试。",
                "Nothing is lost: your changes stay on this iPhone and Bangs retries on its own."
            )
            return message.isEmpty ? reassurance : reassurance + "\n" + message
        default:
            return nil
        }
    }

    private var statusSymbol: String {
        switch model.status {
        case .ready, .idle:
            return "checkmark.icloud"
        case .starting, .syncing:
            return "arrow.triangle.2.circlepath.icloud"
        case .noAccount, .restricted, .unavailable, .error:
            return "exclamationmark.icloud"
        }
    }

    private var statusColor: Color {
        switch model.status {
        case .ready, .idle:
            return .green
        case .starting, .syncing:
            return .secondary
        case .noAccount, .restricted, .unavailable, .error:
            return .orange
        }
    }

    private var environmentNote: String {
        return t(
            "Mac 和 iPhone 必须都用 Development（开发）环境，或都用 Production（正式）环境，否则互相看不到对方的数据。用 Xcode 直接运行的是开发环境；TestFlight 和 App Store 是正式环境。",
            "The Mac and the iPhone must both be on the Development CloudKit environment, or both on Production, or they cannot see each other's data. Running from Xcode uses Development; TestFlight and the App Store use Production."
        )
    }

    private var shortDeviceID: String {
        let digits = model.deviceID.replacingOccurrences(of: "-", with: "")
        return String(digits.prefix(8))
    }

    private var appVersion: String {
        let info = Bundle.main.infoDictionary
        let version = (info?["CFBundleShortVersionString"] as? String) ?? "?"
        let build = (info?["CFBundleVersion"] as? String) ?? "?"
        return version + " (" + build + ")"
    }
}

/// A Mac that syncs: its name, and whether it is there now or when it last was.
private struct MacRow: View {
    let device: DeviceItem
    let online: Bool
    let now: Date

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "laptopcomputer")
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text(device.host.isEmpty ? "Mac" : device.host)
                .lineLimit(1)
            Spacer()
            if online {
                HStack(spacing: 5) {
                    Circle()
                        .fill(Color.green)
                        .frame(width: 7, height: 7)
                        .accessibilityHidden(true)
                    Text(t("在线", "Online"))
                }
                .font(.callout)
                .foregroundStyle(.secondary)
            } else {
                Text(t("上次在线 ", "Last seen ") + Format.relative(device.seenAt, now: now))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

private struct InfoRow: View {
    let title: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title)
            Spacer()
            Text(value)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
        }
    }
}
