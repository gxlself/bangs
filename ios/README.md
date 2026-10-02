# Bangs for iOS

Bangs 的 iPhone / iPad 端：用 CloudKit 和 Mac 上的 Bangs 同步，看 Mac 的开发会话、剪贴板、暂存架，并且可以直接加待办、勾待办。
同步的格式和规则在 [`docs/sync.md`](../docs/sync.md)，这里不重复。

不用 Core Data：一种记录类型（`BangsRecord`），引擎在 [`packages/BangsCloud`](../packages/BangsCloud)，和 Mac 端共用同一套合并规则。

## 快速开始

第一次的话，先跑 `scripts/sync-preflight.sh` 看还缺什么，再照 [`docs/sync-testing.md`](../docs/sync-testing.md) 一步步测。

需要：Xcode 15+、一个付费的 Apple Developer 账号（iCloud / 推送需要带 profile 的签名）、一台登录了 iCloud 的真机。

```bash
brew install xcodegen
cd ios
xcodegen generate
open Bangs.xcodeproj
```

然后在 Xcode 里：

1. 选中 Bangs target → Signing & Capabilities，确认 Team 是 `W8L8ZJ3N2P`（想换成自己的，见下面「换团队 / Bundle ID」）。
2. 运行目标选**真机**。CloudKit 的静默推送要真机；模拟器可以登录 iCloud，但推送不稳定，拉取只会在打开 App、回到前台和每分钟一次时发生。
3. Run。第一次启动后，到「设置」（右上角齿轮）里看 iCloud 状态。

一次性的 Apple Developer 配置（iCloud 容器 `iCloud.com.gxlself.bangs`、App ID `com.gxlself.bangs.ios` 打开 iCloud → CloudKit 和 Push Notifications）见 [`docs/sync.md`](../docs/sync.md) 的「环境、签名与权限」。

### 换团队 / Bundle ID

```bash
cp Local.xcconfig.sample Local.xcconfig   # Local.xcconfig 不进 git
# 编辑 Local.xcconfig，取消注释 DEVELOPMENT_TEAM / PRODUCT_BUNDLE_IDENTIFIER
xcodegen generate
```

## 在哪里看 CloudKit 里的记录

[CloudKit Dashboard](https://icloud.developer.apple.com/dashboard) → 容器 `iCloud.com.gxlself.bangs` → **Development** → Data → Private Database → zone `Bangs` → 记录类型 `BangsRecord`。

`body` 是端到端加密的（`encryptedValues`），Dashboard 里看不到内容，只看得到 `kind`、`updatedAt`、`device`、`deleted`。

## Development 和 Production 是两个数据库

用 Xcode 直接运行的 iPhone 包读写 **Development**；TestFlight / App Store 的包读写 **Production**。Mac 和 iPhone 必须在同一个环境，否则互相看不到对方的数据：

| | Mac | iPhone |
| --- | --- | --- |
| 日常联调（Development） | `scripts/dev-icloud.sh`（Apple Development 证书） | Xcode Run 到真机 |
| 正式使用（Production） | Developer ID 发布包 | TestFlight / App Store |

上生产之前，要在 CloudKit Dashboard 里把 Development 的 schema **Deploy to Production**。细节见 [`docs/sync.md`](../docs/sync.md)。

## 测试

合并规则（谁赢、墓碑、版本表）有一份 Mac 和 iOS 共用的测试向量 [`docs/sync-vectors.json`](../docs/sync-vectors.json)，Swift 这边的测试不需要 Xcode，也不需要 iCloud：

```bash
cd packages/BangsCloud && swift test
```

CloudKit 那一层（`CloudEngine`）没有自动化测试，要真机联调。

## 怎么改

| 想改什么 | 改哪个文件 |
| --- | --- |
| 待办的加 / 勾选 / 删除 / 清洗规则（长度、换行） | `ios/Bangs/SyncModel.swift`（`addTodo`、`toggleTodo`、`deleteTodo`、`deleteTodos`、`cleanTodo`） |
| 同步流程（什么时候拉、什么时候推、失败重试） | `ios/Bangs/SyncModel.swift`（`handle`、`syncNow`、`scheduleRetry`） |
| 某个 kind 的字段（body 里多一个字段） | `ios/Bangs/Models.swift`，同时更新 `docs/sync.md` |
| 待办页 | `ios/Bangs/Views/TodoView.swift` |
| 空状态（还在载入、没登录 iCloud、Mac 没打开同步） | `ios/Bangs/Views/Components.swift` 的 `SyncEmptyState`，判断在 `SyncModel.emptyReason` |
| 代码页：开发会话（分组、排序、状态标签） | `ios/Bangs/Views/DevView.swift`，排序在 `Models.swift` 的 `SessionItem.isOrderedBefore` |
| 「Claude 在等你」通知（何时发、文案、点了去哪） | `ios/Bangs/SyncModel.swift`（`announceWaiting`、`requestNotificationPermission`），前台展示和点击在 `AppDelegate.swift` |
| 剪贴板页（点一下复制、提示） | `ios/Bangs/Views/ClipboardView.swift`，提示条在 `Components.swift` 的 `Toast` |
| 暂存架页（预览、「太大未同步」） | `ios/Bangs/Views/ShelfView.swift`、`QuickLookPreview.swift`，文件路径在 `AssetFiles.swift` |
| 设置页（iCloud 状态文案、Mac 在线状态） | `ios/Bangs/Views/SettingsView.swift` |
| 中英文文案 | 就近改调用处的 `t("中文", "English")`（和桌面端一样，没有 .strings 文件） |
| 静默推送、后台唤醒 | `ios/Bangs/AppDelegate.swift`，Info.plist 的 `UIBackgroundModes` 在 `ios/project.yml` |
| iCloud 容器、推送权限 | `ios/Bangs/Bangs.entitlements` |
| 版本号、签名、最低系统 | `ios/project.yml` |
| CloudKit 传输（记录怎么存、冲突怎么重试、令牌） | `packages/BangsCloud/Sources/BangsCloud/CloudEngine.swift` |
| 合并规则（谁赢、墓碑、本地队列） | `packages/BangsCloud/Sources/BangsSyncCore/`，改完同步改 `docs/sync-vectors.json` 和 Rust 端 |

`Bangs.xcodeproj` 和 `Bangs/Info.plist` 是 `xcodegen generate` 生成的，不进 git；加文件后重新生成就行。
