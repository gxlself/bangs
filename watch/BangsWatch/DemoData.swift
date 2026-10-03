#if DEBUG
import SwiftUI

/// Sample content for the App Store screenshots, which are taken in the simulator with no Mac
/// to pair with. Debug builds only:
///
///     xcrun simctl launch <udid> com.gxlself.bangs.watch -BangsDemo YES -BangsPage sessions
///
/// With `-BangsDemo YES` the store never talks to the network and never touches the keychain:
/// it starts paired, online, and showing the snapshot below. The transport buttons, ticking a
/// to-do off and adding one still work, on this copy only. The words follow the watch's
/// language and the times are relative to now, so the song is always partway through.
enum DemoData {
    /// Read from the launch arguments, which UserDefaults puts in its argument domain.
    static let isOn = UserDefaults.standard.bool(forKey: "BangsDemo")

    /// `-BangsPage playing|sessions|todos|board`: the page the app opens on, so each
    /// screenshot is one launch and needs no UI automation.
    static var initialPage: Int {
        switch isOn ? UserDefaults.standard.string(forKey: "BangsPage") ?? "" : "" {
        case "sessions": return 1
        case "todos": return 2
        case "board": return 3
        default: return 0
        }
    }

    static let endpoint = Endpoint(base: "http://192.168.1.20:17651", token: "demo", host: host)

    private static var host: String { t("Xiaolong 的 MacBook Pro", "Xiaolong's MacBook Pro") }

    static func snapshot() -> Snapshot {
        let now = Date().timeIntervalSince1970 * 1000
        let minutes = { (count: Double) in now - count * 60_000 }
        return Snapshot(
            host: host,
            now: now,
            media: Media(
                title: t("夏夜的风", "Night Drive"),
                artist: "The Lanterns",
                album: t("城市灯火", "City Lights"),
                appName: "Music",
                playing: true,
                duration: 214,
                elapsed: 71,
                elapsedAt: now,
                artwork: "demo",
                lyrics: "demo"
            ),
            sessions: [
                Session(id: "1", agent: "claude", name: t("修好刘海上的拖放", "Notch drop bug"), project: "bangs",
                        status: "waiting", detail: t("允许运行 pnpm build？", "Allow Bash: pnpm build?"), updatedAt: minutes(2)),
                Session(id: "2", agent: "codex", name: t("压缩大图再同步", "Compress images"), project: "paste",
                        status: "busy", detail: nil, updatedAt: minutes(4)),
                Session(id: "3", agent: "claude", name: t("更新下载页", "Download page"), project: "website",
                        status: "busy", detail: nil, updatedAt: minutes(7)),
                Session(id: "4", agent: "codex", name: t("清理旧接口", "Remove old endpoints"), project: "api",
                        status: "idle", detail: nil, updatedAt: minutes(26)),
            ],
            todos: [
                Todo(id: "1", text: t("修复登录页的崩溃", "Fix the crash on the sign-in screen"), createdAt: minutes(12)),
                Todo(id: "2", text: t("给设计稿回复意见", "Send feedback on the new designs"), createdAt: minutes(48)),
                Todo(id: "3", text: t("周五前发 v0.3", "Ship v0.3 by Friday"), createdAt: minutes(150)),
                Todo(id: "4", text: t("下班顺路买牛奶", "Pick up milk on the way home"), createdAt: minutes(320)),
            ],
            activities: [
                Activity(id: "ci", title: t("CI 正在跑", "CI is running"), subtitle: t("214 个用例里的第 131 个", "Test 131 of 214"),
                         icon: "code", progress: 0.61, updatedAt: minutes(1) / 1000),
                Activity(id: "export", title: t("视频导出完成", "Video export finished"), subtitle: "launch-teaser.mp4",
                         icon: "check", progress: nil, updatedAt: minutes(9) / 1000),
                Activity(id: "deploy", title: t("已部署到预发环境", "Deployed to staging"), subtitle: "web · 3f2a9c1",
                         icon: "board", progress: nil, updatedAt: minutes(30) / 1000),
            ]
        )
    }

    /// Made up for the demo, line by line, every few seconds of the song.
    static let lyrics: [LyricLine] = {
        let lines: [(String, String)] = [
            ("Windows down, empty road", "车窗摇下，路上没有别人"),
            ("Streetlights count us home", "路灯一盏盏数着我们回家"),
            ("Radio hums a song we know", "收音机哼着我们熟悉的歌"),
            ("Summer night, take it slow", "夏夜的风，慢一点吹"),
            ("Every mile a little glow", "每一里路都亮着一点光"),
            ("Hold it, then let it go", "抓住这一刻，再放它走"),
        ]
        return (0..<36).map { index in
            let (english, chinese) = lines[index % lines.count]
            return speaksChinese
                ? LyricLine(at: Double(index) * 6, text: chinese, translation: nil)
                : LyricLine(at: Double(index) * 6, text: english, translation: chinese)
        }
    }()

    /// A cover in the notch's music color, drawn rather than shipped.
    @MainActor static func artwork() -> UIImage? {
        let renderer = ImageRenderer(content: ZStack {
            LinearGradient(colors: [Palette.music, Color(red: 0.42, green: 0.18, blue: 0.62)],
                           startPoint: .topLeading, endPoint: .bottomTrailing)
            Image(systemName: "moon.stars.fill")
                .font(.system(size: 64))
                .foregroundStyle(.white.opacity(0.9))
        }
        .frame(width: 160, height: 160))
        renderer.scale = 2
        return renderer.uiImage
    }
}
#endif
