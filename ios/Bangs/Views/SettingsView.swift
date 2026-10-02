import SwiftUI
import BangsCloud

struct SettingsView: View {
    @EnvironmentObject private var model: SyncModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section(header: Text("iCloud")) {
                    HStack(spacing: 10) {
                        Image(systemName: statusSymbol)
                            .foregroundColor(statusColor)
                        Text(statusTitle)
                    }
                    if let help = statusHelp {
                        Text(help)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    InfoRow(title: t("上次同步", "Last sync"), value: Format.lastSync(model.lastSync))
                    Button {
                        Task {
                            await model.syncNow()
                        }
                    } label: {
                        Text(t("立即同步", "Sync now"))
                    }
                    .disabled(isBusy)
                }

                Section(
                    header: Text(t("关于", "About")),
                    footer: Text(environmentNote)
                ) {
                    InfoRow(title: t("iCloud 容器", "iCloud container"), value: SyncModel.containerID)
                    InfoRow(title: t("设备 ID", "Device ID"), value: shortDeviceID)
                    InfoRow(title: t("版本", "Version"), value: appVersion)
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle(t("设置", "Settings"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(t("完成", "Done")) {
                        dismiss()
                    }
                }
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
        case .error(let message):
            return t("同步出错：", "Sync error: ") + message
        }
    }

    private var statusHelp: String? {
        switch model.status {
        case .noAccount:
            return t(
                "打开「设置」，点最上面的“登录 iPhone”，用 Apple ID 登录并确保 iCloud 已开启，然后回到这里点「立即同步」。",
                "Open Settings, tap Sign in at the top, sign in with your Apple ID and make sure iCloud is on, then come back and tap Sync now."
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
        case .error:
            return t(
                "没有丢数据：改动会留在手机上，下次同步时再发出去。",
                "Nothing is lost: your changes stay on this phone and go out on the next sync."
            )
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
        case .noAccount, .restricted, .unavailable:
            return "exclamationmark.icloud"
        case .error:
            return "xmark.icloud"
        }
    }

    private var statusColor: Color {
        switch model.status {
        case .ready, .idle:
            return .green
        case .starting, .syncing:
            return .secondary
        case .noAccount, .restricted, .unavailable:
            return .orange
        case .error:
            return .red
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
