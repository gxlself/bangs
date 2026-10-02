# 本地测试：Mac ↔ iPhone 同步

按顺序走。每一步写了预期结果；不一致时看最后的「出问题时」。
这一套第一次在真机上跑，前两步最可能要修编译错误——把完整报错贴回来就行。

需要：一台装了 Xcode 15+ 的 Mac、一台 iPhone（iOS 16+），两边登录同一个 Apple ID。

## 0. 一次性准备

1. Apple Developer 后台（团队 `W8L8ZJ3N2P`），步骤在 [sync.md](sync.md#环境签名与权限)：
   - 新建 iCloud 容器 `iCloud.com.gxlself.bangs`
   - App ID `com.gxlself.bangs`（Mac）和 `com.gxlself.bangs.ios`（iOS），没有就新建，都打开 **iCloud（CloudKit，勾上这个容器）** 和 **Push Notifications**
   - 生成 **Mac App Development** profile（App ID `com.gxlself.bangs`，选上这台 Mac 和你的开发证书），
     存成 `src-tauri/icloud/dev.provisionprofile`
2. 本机：

```bash
brew install xcodegen
pnpm install
scripts/sync-preflight.sh          # 列出还缺什么；全绿再往下
```

## 1. 不需要 iCloud 的检查

```bash
scripts/sync-preflight.sh --test   # swift test + cargo test + 生成 ios/Bangs.xcodeproj
pnpm tauri dev --features icloud   # 只验证 Swift 桥能编译链接
```

预期：
- `swift test` 和 `cargo test` 全过（两边跑的是同一份 `docs/sync-vectors.json`）
- `tauri dev --features icloud` 能起来；托盘菜单里「同步到 iPhone（iCloud）」是灰的，下面写着
  「这个版本没有 iCloud 权限」——未签名的二进制本来就是这样，说明链接没问题、也没去碰 CloudKit

## 2. iPhone 端跑起来

```bash
open ios/Bangs.xcodeproj
```

选你的 iPhone（不要用模拟器，静默推送不稳），Run。

预期：
- 四个标签页：待办 / 开发 / 剪贴板 / 文件架，后三个是空状态说明
- 右上角齿轮 → 设置：iCloud 一栏几秒内变成「iCloud 已连接」或「已是最新」
- [CloudKit Dashboard](https://icloud.developer.apple.com/dashboard) → `iCloud.com.gxlself.bangs` → **Development** →
  Private Database 里出现 zone `Bangs`

## 3. Mac 端跑起来

```bash
scripts/dev-icloud.sh              # 构建、签名、运行；日志留在这个终端
```

托盘菜单 → 打开「同步到 iPhone（iCloud）」。

预期：下面的状态行几秒内从「正在连接 iCloud…」变成「已连接，改动会自动同步」，终端没有 `[sync]` 报错。

## 4. 待办（双向）

| 操作 | 预期 |
| --- | --- |
| Mac 刘海里加一条待办 | iPhone 几秒内出现（靠静默推送；没到就下拉刷新） |
| iPhone 加一条 | Mac 20 秒内出现；托盘「立即同步」马上出现 |
| iPhone 上点圆圈或左滑勾掉 Mac 加的那条 | Mac 上那条消失（同样 ≤ 20 秒） |
| Mac 上勾掉 iPhone 加的那条 | iPhone 上消失 |
| 开同步之前 Mac 上已有的待办 | 第一次打开同步后出现在 iPhone 上 |

## 5. 开发会话（Mac → iPhone）

在 Mac 上开一个 Claude Code 会话（任意项目目录里 `claude`），让它跑起来，再让它停下来问你一个问题。

预期：iPhone「开发」页按 Mac 的名字分组出现这个会话；运行中是蓝色「运行中」，等你回答时变成橙色「等你」并排到最前，
带上它在等什么。退出会话后它从 iPhone 上消失。

## 6. 剪贴板（Mac → iPhone，需要 Paste 已安装）

在 Mac 上复制一段文字。

预期：iPhone「剪贴板」页最上面出现这一条；点一下提示「已拷贝」，到备忘录里粘贴出来是完整文本。
图片和文件条目只显示预览文字（点了拷贝的是那行文字，不是图片本身）。

## 7. 文件架（Mac → iPhone）

| 操作 | 预期 |
| --- | --- |
| 拖一张小图片到刘海 | iPhone「文件架」出现，先显示「等待下载…」，随后可以点开预览 |
| 拖一个 > 25 MB 的文件 | iPhone 上显示「太大，未同步」 |
| 从刘海文件架移除 | iPhone 上消失 |

## 8. 离线与冲突

| 操作 | 预期 |
| --- | --- |
| iPhone 开飞行模式，加一条待办，再关飞行模式 | 回到前台或下拉刷新后推出去，Mac 上出现 |
| Mac 断网，勾掉一条，恢复网络 | 20 秒内 iPhone 上消失 |
| 两边都断网：Mac 勾掉一条，iPhone 加一条新的；再都恢复 | Mac 勾掉的那条两边都没了，不会复活；iPhone 新加的两边都有 |

## 9. 开关与账号

| 操作 | 预期 |
| --- | --- |
| 托盘关掉同步 | 状态行「已关闭」；之后 Mac 的改动不再到 iPhone（iCloud 里已有的保留） |
| 再打开 | 关闭期间 Mac 上加的待办同步过去 |
| iPhone 退出 iCloud 再打开 App | 设置页「没有登录 iCloud」和怎么处理的说明 |

## 出问题时

- **Mac 日志**：`scripts/dev-icloud.sh` 的终端里，Rust 端以 `[sync]` 开头，Swift 桥以 `BangsCloudBridge:` 开头。
- **iPhone 日志**：Xcode 底部控制台，`[sync]` 开头。
- **CloudKit 里实际有什么**：Dashboard → Records，选 zone `Bangs`、类型 `BangsRecord`。`body` 是加密的，看不到内容；
  `kind`、`updatedAt`、`device`、`deleted` 看得到。Dashboard 的 Logs 页能看到每个请求和错误码。

常见情况：

| 现象 | 多半是 |
| --- | --- |
| Mac 一启动就被杀掉（终端里 `Killed: 9`） | 签名里的权限和 profile 对不上：`codesign -d --entitlements - src-tauri/target/debug/bundle/macos/Bangs.app` 和 profile 比一比，或者 profile 里没有这台 Mac |
| 托盘一直是「这个版本没有 iCloud 权限」 | 跑的是 `pnpm tauri dev`，不是 `scripts/dev-icloud.sh` |
| 报 `Bad Container` / `badContainer` | App ID 没勾上容器 `iCloud.com.gxlself.bangs`，或 profile 是勾选之前生成的 |
| 报 `Not Authenticated` | 那台设备没登录 iCloud |
| 两边都正常但互相看不到 | 一边在 Development，一边在 Production（比如 iPhone 装的是 TestFlight 版）——见 [sync.md](sync.md#环境签名与权限) |
| iPhone 上改的东西 Mac 很久才到 | 正常最多 20 秒；托盘「立即同步」可以马上拉 |
| `Server Rejected Request` | 生产环境还没 Deploy Schema；开发环境一般是字段类型和已建的 schema 冲突，Dashboard 里 Reset Development Environment 后重试 |
