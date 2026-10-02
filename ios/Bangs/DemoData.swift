#if DEBUG
import Foundation
import UIKit
import BangsSyncCore

/// Sample content for the App Store screenshots, which are taken in the simulator, where there
/// is no iCloud account to sync with. Debug builds only:
///
///     xcrun simctl launch <udid> com.gxlself.bangs.ios -BangsDemo YES -BangsTab dev
///
/// With `-BangsDemo YES` SyncModel never starts the CloudKit engine and never reads or writes
/// the real store (Application Support/BangsSync): it starts from the records below, kept in
/// memory, with their pictures and files in a throwaway folder in tmp/. To-dos can still be
/// added, ticked and deleted; nothing is pushed anywhere. The words follow the app's language
/// and the times are relative to now, so the lists say "3 minutes ago", whenever they are shot.
enum DemoData {
    /// Read from the launch arguments, which UserDefaults puts in its argument domain.
    static let isOn = UserDefaults.standard.bool(forKey: "BangsDemo")

    /// `-BangsTab todo|dev|clipboard|shelf|settings`, with the demo: where the app opens, so each
    /// screenshot is one launch and needs no UI automation. `settings` opens the settings sheet
    /// over the first tab.
    static let launchTab = isOn ? (UserDefaults.standard.string(forKey: "BangsTab") ?? "") : ""

    static var initialTab: AppTab {
        switch launchTab {
        case "dev": return .dev
        case "clipboard": return .clipboard
        case "shelf": return .shelf
        default: return .todo
        }
    }

    /// Whether the settings sheet still has to open for `-BangsTab settings`: once, not every
    /// time a tab appears.
    @MainActor static var settingsPending = launchTab == "settings"

    /// This phone, in the `device` field of the to-dos it writes.
    static let phoneID = "5d0c2a7e-91b4-4c3f-8e6a-0b7f1d2c3e4a"

    /// The Mac everything else comes from: its full id, the short one its records start with,
    /// and its name as the owner wrote it.
    private static let macID = "7c4e19a2-5b3d-4f6e-9a81-2d0c6b4e8f13"
    private static let mac8 = "7c4e19a2"
    private static var macName: String { t("Xiaolong 的 MacBook Pro", "Xiaolong's MacBook Pro") }

    // MARK: Store

    /// A fresh, empty folder for the demo's pictures and files, away from the real ones.
    static func makeDirectory() -> URL {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("BangsDemo", isDirectory: true)
        try? FileManager.default.removeItem(at: directory)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    /// Every demo record, as if they had just been pulled; the pictures and files the lists show
    /// are written where `AssetFiles` looks for them.
    static func store(assetsIn directory: URL) -> RecordStore {
        let now = Int64((Date().timeIntervalSince1970 * 1000).rounded())
        let records = todos(now: now)
            + [device(now: now)]
            + sessions(now: now)
            + clips(now: now, directory: directory)
            + shelf(now: now, directory: directory)
        var byKey: [String: SyncRecord] = [:]
        for record in records {
            byKey[record.key] = record
        }
        return RecordStore(records: byKey)
    }

    private static func minutesAgo(_ minutes: Double, from now: Int64) -> Int64 {
        return now - Int64(minutes * 60_000)
    }

    private static func record(_ kind: String, _ id: String, at time: Int64, device: String, body: [String: JSONValue]) -> SyncRecord {
        return SyncRecord(kind: kind, id: id, updatedAt: time, device: device, deleted: false, body: .object(body))
    }

    /// Five open, three done; some written on this phone, some on the Mac.
    private static func todos(now: Int64) -> [SyncRecord] {
        let lines: [(text: String, created: Double, done: Double?)] = [
            (t("修复登录页的崩溃", "Fix the crash on the sign-in screen"), 12, nil),
            (t("给设计稿回复意见", "Send feedback on the new designs"), 48, nil),
            (t("周五前发 v0.3", "Ship v0.3 by Friday"), 150, nil),
            (t("约周六的牙医", "Book the dentist for Saturday"), 320, nil),
            (t("下班顺路买牛奶", "Pick up milk on the way home"), 1_500, nil),
            (t("合并剪贴板压缩的 PR", "Merge the clipboard compression PR"), 400, 30),
            (t("回复 Anna 的邮件", "Reply to Anna's email"), 600, 190),
            (t("续费域名", "Renew the domain"), 2_900, 1_400),
        ]
        return lines.enumerated().compactMap { index, line -> SyncRecord? in
            let created = minutesAgo(line.created, from: now)
            let doneAt = line.done.map { minutesAgo($0, from: now) }
            let item = TodoItem(
                id: "demo-todo-\(index)",
                text: line.text,
                createdAt: created,
                done: doneAt != nil,
                doneAt: doneAt
            )
            guard case .object(let body) = item.body else { return nil }
            return record("todo", item.id, at: doneAt ?? created, device: index % 2 == 0 ? phoneID : macID, body: body)
        }
    }

    /// A heartbeat from a minute ago: the Mac is online.
    private static func device(now: Int64) -> SyncRecord {
        let seen = minutesAgo(1, from: now)
        return record("device", mac8, at: seen, device: macID, body: [
            "host": .string(macName),
            "seenAt": .int(seen),
            "platform": .string("macOS"),
        ])
    }

    /// Claude waiting for an answer, Codex at work, Claude done for now. The names are short
    /// enough to leave room for "· 2 minutes ago" on an iPhone; the detail is Claude Code's own
    /// words, which are English whatever the language.
    private static func sessions(now: Int64) -> [SyncRecord] {
        let list: [(agent: String, name: String, project: String, status: String, detail: String?, minutes: Double)] = [
            ("claude", t("修好刘海上的拖放", "Notch drop bug"), "bangs", "waiting", "Allow Bash: pnpm build?", 2),
            ("codex", t("压缩大图再同步", "Compress images"), "paste", "busy", nil, 4),
            ("claude", t("更新下载页", "Download page"), "website", "idle", nil, 26),
        ]
        return list.enumerated().map { index, session in
            let at = minutesAgo(session.minutes, from: now)
            return record("session", "\(mac8)-demo-session-\(index)", at: at, device: macID, body: [
                "agent": .string(session.agent),
                "name": .string(session.name),
                "project": .string(session.project),
                "path": .string("/Users/xiaolong/code/" + session.project),
                "status": .string(session.status),
                "detail": session.detail.map { JSONValue.string($0) } ?? .null,
                "statusAt": .int(at),
                "host": .string(macName),
            ])
        }
    }

    /// Eight entries of different kinds from different apps, one of them a picture.
    private static func clips(now: Int64, directory: URL) -> [SyncRecord] {
        let code = """
        withAnimation(.spring(response: 0.3)) {
            isExpanded.toggle()
        }
        """
        let list: [(type: String, text: String, app: String, minutes: Double, pinned: Bool)] = [
            ("text", "https://github.com/gxlself/bangs/pull/42", "Safari", 1, false),
            ("text", code, "Xcode", 4, false),
            ("image", t("图片", "Image"), "Figma", 9, false),
            ("text", t("今晚七点老地方见，记得把充电器带上。", "See you at 7 at the usual place, and bring the charger."), t("微信", "WeChat"), 18, false),
            ("text", "pnpm tauri build --target universal-apple-darwin", t("终端", "Terminal"), 35, false),
            ("files", "/Users/xiaolong/Desktop/" + t("发布清单.numbers", "Launch checklist.numbers"), t("访达", "Finder"), 52, false),
            ("text", t("上海市黄浦区南京东路 300 号 5 楼", "500 Market Street, Suite 300, San Francisco, CA 94105"), t("备忘录", "Notes"), 130, true),
            ("text", t("评审改到周四下午三点，会议链接不变。", "The review moved to Thursday at 3 pm, same link as before."), t("邮件", "Mail"), 200, false),
        ]
        return list.enumerated().map { index, clip in
            let id = "\(mac8)-demo-clip-\(index)"
            let isText = clip.type == "text"
            let isImage = clip.type == "image"
            if isImage {
                write(chartPicture().pngData(), key: "clip:" + id, directory: directory)
            }
            return record("clip", id, at: minutesAgo(clip.minutes, from: now), device: macID, body: [
                "type": .string(clip.type),
                // As the Mac sends it: whitespace folded into one line.
                "preview": .string(clip.text.split(whereSeparator: \.isWhitespace).joined(separator: " ")),
                "text": isText ? .string(clip.text) : .null,
                "app": .string(clip.app),
                "pinned": .bool(clip.pinned),
                "createdAt": .int(minutesAgo(clip.minutes, from: now)),
                "host": .string(macName),
                "hasImage": .bool(isImage),
            ])
        }
    }

    /// A photo and a picture with thumbnails, a PDF and a keynote ready to preview, and a zip
    /// too big to sync.
    private static func shelf(now: Int64, directory: URL) -> [SyncRecord] {
        let list: [(name: String, ext: String, size: Int64, file: Data?, minutes: Double)] = [
            (t("海边日落.jpg", "Sunset at the beach.jpg"), "jpg", 2_418_305, sunsetPicture().jpegData(compressionQuality: 0.85), 6),
            (t("2026 Q3 报价单.pdf", "Q3 2026 Quote.pdf"), "pdf", 183_204, quotePDF(), 9),
            (t("设计素材.zip", "Design assets.zip"), "zip", 186_400_512, nil, 50),
            (t("发布会.key", "Launch keynote.key"), "key", 8_412_877, Data(count: 4_096), 140),
            (t("App 图标草稿.png", "App icon draft.png"), "png", 412_330, iconPicture().pngData(), 1_560),
        ]
        return list.enumerated().map { index, item in
            let id = "\(mac8)-demo-shelf-\(index)"
            // Over 25 MB there is no file, as from a real Mac: the row says it was not synced.
            if item.size <= ShelfItem.syncLimitBytes {
                write(item.file, key: "shelf:" + id, directory: directory)
            }
            return record("shelf", id, at: minutesAgo(item.minutes, from: now), device: macID, body: [
                "name": .string(item.name),
                "extension": .string(item.ext),
                "size": .int(item.size),
                "isImage": .bool(item.ext == "jpg" || item.ext == "png"),
                "addedAt": .int(minutesAgo(item.minutes, from: now)),
                "host": .string(macName),
            ])
        }
    }

    private static func write(_ data: Data?, key: String, directory: URL) {
        guard let data = data else { return }
        let url = AssetFiles.url(forKey: key, stateDirectory: directory)
        try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? data.write(to: url)
    }

    // MARK: Pictures and files, drawn at launch

    private static func rgb(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
        return UIColor(
            red: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: alpha
        ).cgColor
    }

    private static func draw(_ size: CGSize, _ body: (CGContext) -> Void) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        return UIGraphicsImageRenderer(size: size, format: format).image { context in
            body(context.cgContext)
        }
    }

    private static func gradient(_ colors: [CGColor]) -> CGGradient? {
        return CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors as CFArray, locations: nil)
    }

    /// The clipboard's picture: a chart copied out of Figma.
    private static func chartPicture() -> UIImage {
        let size = CGSize(width: 1000, height: 750)
        return draw(size) { context in
            if let background = gradient([rgb(0x5E5CE6), rgb(0xBF5AF2)]) {
                context.drawLinearGradient(background, start: .zero, end: CGPoint(x: size.width, y: size.height), options: [])
            }
            let card = CGRect(x: 90, y: 90, width: size.width - 180, height: size.height - 180)
            context.setShadow(offset: CGSize(width: 0, height: 18), blur: 40, color: rgb(0x1C1250, 0.35))
            context.setFillColor(rgb(0xFFFFFF))
            context.addPath(UIBezierPath(roundedRect: card, cornerRadius: 44).cgPath)
            context.fillPath()
            context.setShadow(offset: .zero, blur: 0, color: nil)

            // A title and a subtitle, as grey bars.
            context.setFillColor(rgb(0x1C1C1E))
            context.addPath(UIBezierPath(roundedRect: CGRect(x: card.minX + 56, y: card.minY + 54, width: 300, height: 30), cornerRadius: 15).cgPath)
            context.fillPath()
            context.setFillColor(rgb(0xC7C7CC))
            context.addPath(UIBezierPath(roundedRect: CGRect(x: card.minX + 56, y: card.minY + 100, width: 190, height: 20), cornerRadius: 10).cgPath)
            context.fillPath()

            let heights: [CGFloat] = [0.34, 0.52, 0.44, 0.68, 0.58, 0.86, 0.74]
            let baseline = card.maxY - 60
            let chartHeight = baseline - (card.minY + 160)
            let slot = (card.width - 112) / CGFloat(heights.count)
            for (index, height) in heights.enumerated() {
                let bar = CGRect(
                    x: card.minX + 56 + CGFloat(index) * slot + slot * 0.18,
                    y: baseline - chartHeight * height,
                    width: slot * 0.64,
                    height: chartHeight * height
                )
                context.setFillColor(index == 5 ? rgb(0xFF9F0A) : rgb(0x5E5CE6, 0.85))
                context.addPath(UIBezierPath(roundedRect: bar, cornerRadius: 14).cgPath)
                context.fillPath()
            }
            context.setFillColor(rgb(0xE5E5EA))
            context.fill(CGRect(x: card.minX + 56, y: baseline + 8, width: card.width - 112, height: 4))
        }
    }

    /// The shelf's photo: a sunset over the sea.
    private static func sunsetPicture() -> UIImage {
        let size = CGSize(width: 1200, height: 900)
        let horizon = size.height * 0.62
        return draw(size) { context in
            let sky = CGRect(x: 0, y: 0, width: size.width, height: horizon)
            context.saveGState()
            context.clip(to: sky)
            if let colors = gradient([rgb(0x2E1F5E), rgb(0xA8457A), rgb(0xFF8A5B), rgb(0xFFC27A)]) {
                context.drawLinearGradient(colors, start: .zero, end: CGPoint(x: 0, y: horizon), options: [])
            }
            let sun = CGPoint(x: size.width * 0.52, y: horizon - 20)
            if let glow = gradient([rgb(0xFFE7A3, 0.9), rgb(0xFFE7A3, 0)]) {
                context.drawRadialGradient(glow, startCenter: sun, startRadius: 0, endCenter: sun, endRadius: size.width * 0.38, options: [])
            }
            context.setFillColor(rgb(0xFFEBB8))
            context.fillEllipse(in: CGRect(x: sun.x - 120, y: sun.y - 120, width: 240, height: 240))
            context.restoreGState()

            // Hills on both sides, in front of the sun.
            let hills = UIBezierPath()
            hills.move(to: CGPoint(x: 0, y: horizon))
            hills.addLine(to: CGPoint(x: 0, y: horizon - 70))
            hills.addQuadCurve(to: CGPoint(x: size.width * 0.36, y: horizon), controlPoint: CGPoint(x: size.width * 0.16, y: horizon - 170))
            hills.addLine(to: CGPoint(x: size.width * 0.64, y: horizon))
            hills.addQuadCurve(to: CGPoint(x: size.width, y: horizon - 110), controlPoint: CGPoint(x: size.width * 0.84, y: horizon - 220))
            hills.addLine(to: CGPoint(x: size.width, y: horizon))
            hills.close()
            context.setFillColor(rgb(0x3B2560))
            context.addPath(hills.cgPath)
            context.fillPath()

            let sea = CGRect(x: 0, y: horizon, width: size.width, height: size.height - horizon)
            context.saveGState()
            context.clip(to: sea)
            if let colors = gradient([rgb(0x5A3A78), rgb(0x1C1640)]) {
                context.drawLinearGradient(colors, start: CGPoint(x: 0, y: horizon), end: CGPoint(x: 0, y: size.height), options: [])
            }
            context.restoreGState()

            // The sun's reflection, in broken lines.
            var y = horizon + 16
            for index in 0..<8 {
                let width = size.width * (0.26 - CGFloat(index) * 0.026)
                let line = CGRect(x: size.width * 0.52 - width / 2, y: y, width: width, height: 7)
                context.setFillColor(rgb(0xFFD08A, 0.8 - CGFloat(index) * 0.08))
                context.addPath(UIBezierPath(roundedRect: line, cornerRadius: 3.5).cgPath)
                context.fillPath()
                y += 20 + CGFloat(index) * 6
            }
        }
    }

    /// The shelf's PNG: a sketch of an app icon, with the notch.
    private static func iconPicture() -> UIImage {
        let size = CGSize(width: 1024, height: 1024)
        return draw(size) { context in
            context.setFillColor(rgb(0xF2F2F7))
            context.fill(CGRect(origin: .zero, size: size))

            let icon = CGRect(x: 192, y: 192, width: 640, height: 640)
            context.saveGState()
            context.setShadow(offset: CGSize(width: 0, height: 24), blur: 48, color: rgb(0x000000, 0.18))
            context.addPath(UIBezierPath(roundedRect: icon, cornerRadius: 144).cgPath)
            context.setFillColor(rgb(0xFF6A3D))
            context.fillPath()
            context.restoreGState()
            context.saveGState()
            context.addPath(UIBezierPath(roundedRect: icon, cornerRadius: 144).cgPath)
            context.clip()
            if let colors = gradient([rgb(0xFF9F0A), rgb(0xFF375F)]) {
                context.drawLinearGradient(colors, start: CGPoint(x: icon.minX, y: icon.minY), end: CGPoint(x: icon.maxX, y: icon.maxY), options: [])
            }
            context.restoreGState()

            let notch = CGRect(x: icon.midX - 150, y: icon.minY + 70, width: 300, height: 76)
            context.setFillColor(rgb(0x000000))
            context.addPath(UIBezierPath(roundedRect: notch, cornerRadius: 38).cgPath)
            context.fillPath()

            let check = UIBezierPath()
            check.move(to: CGPoint(x: icon.midX - 140, y: icon.midY + 60))
            check.addLine(to: CGPoint(x: icon.midX - 40, y: icon.midY + 160))
            check.addLine(to: CGPoint(x: icon.midX + 150, y: icon.midY - 40))
            context.setStrokeColor(rgb(0xFFFFFF))
            context.setLineWidth(56)
            context.setLineCap(.round)
            context.setLineJoin(.round)
            context.addPath(check.cgPath)
            context.strokePath()
        }
    }

    /// The shelf's PDF: a one-page quote, so the preview has something to show.
    private static func quotePDF() -> Data {
        let page = CGRect(x: 0, y: 0, width: 595, height: 842)
        let rows: [(String, String)] = [
            (t("设计与原型", "Design and prototyping"), t("¥ 18,000", "$ 2,500")),
            (t("iPhone 客户端开发", "iPhone app development"), t("¥ 46,000", "$ 6,400")),
            (t("Mac 端同步模块", "Mac sync module"), t("¥ 28,000", "$ 3,900")),
            (t("测试与上架", "Testing and App Store release"), t("¥ 8,000", "$ 1,200")),
        ]
        let title: [NSAttributedString.Key: Any] = [.font: UIFont.systemFont(ofSize: 28, weight: .bold)]
        let caption: [NSAttributedString.Key: Any] = [.font: UIFont.systemFont(ofSize: 12), .foregroundColor: UIColor.gray]
        let body: [NSAttributedString.Key: Any] = [.font: UIFont.systemFont(ofSize: 14)]
        let strong: [NSAttributedString.Key: Any] = [.font: UIFont.systemFont(ofSize: 14, weight: .semibold)]
        return UIGraphicsPDFRenderer(bounds: page).pdfData { context in
            context.beginPage()
            NSAttributedString(string: t("报价单", "Quote"), attributes: title).draw(at: CGPoint(x: 56, y: 64))
            NSAttributedString(string: t("2026 年第三季度 · 有效期 30 天", "Q3 2026 · valid for 30 days"), attributes: caption)
                .draw(at: CGPoint(x: 56, y: 106))
            var y: CGFloat = 170
            for (item, price) in rows {
                NSAttributedString(string: item, attributes: body).draw(at: CGPoint(x: 56, y: y))
                let amount = NSAttributedString(string: price, attributes: body)
                amount.draw(at: CGPoint(x: page.width - 56 - amount.size().width, y: y))
                UIColor(white: 0.88, alpha: 1).setFill()
                UIRectFill(CGRect(x: 56, y: y + 28, width: page.width - 112, height: 1))
                y += 44
            }
            NSAttributedString(string: t("合计", "Total"), attributes: strong).draw(at: CGPoint(x: 56, y: y + 8))
            let total = NSAttributedString(string: t("¥ 100,000", "$ 14,000"), attributes: strong)
            total.draw(at: CGPoint(x: page.width - 56 - total.size().width, y: y + 8))
        }
    }
}
#endif
