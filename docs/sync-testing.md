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
- `tauri dev --features icloud` 能起来；托盘菜单里**没有**「与 iPhone 同步（iCloud）」这一组——未签名的二进制
  没有 iCloud 权限，菜单就不提供这个开关。能起来说明链接没问题，也没去碰 CloudKit（碰了会直接崩）

## 2. iPhone 端跑起来

```bash
open ios/Bangs.xcodeproj
```

选你的 iPhone（不要用模拟器，静默推送不稳），Run。

预期：
- 四个标签页：待办 / 代码 / 剪贴板 / 暂存。后三个先显示「正在从 iCloud 载入…」，然后是空状态，
  提示在 Mac 上打开「与 iPhone 同步（iCloud）」
- 右上角齿轮 → 设置：iCloud 一栏几秒内变成「iCloud 已连接」或「已是最新」；Mac 一栏同样提示去 Mac 上打开同步
- [CloudKit Dashboard](https://icloud.developer.apple.com/dashboard) → `iCloud.com.gxlself.bangs` → **Development** →
  Private Database 里出现 zone `Bangs`

## 3. Mac 端跑起来

```bash
scripts/dev-icloud.sh              # 构建、签名、运行；日志留在这个终端
```

托盘菜单 → 打开「与 iPhone 同步（iCloud）」。

预期：
- 下面出现状态行，几秒内从「正在连接 iCloud…」变成「已连接，改动会自动同步」，还有「立即同步」；终端没有 `[sync]` 报错
- iPhone 下拉刷新后，设置页的 Mac 一栏出现这台 Mac，显示「在线」；三个空页面的说明变成「Mac 上没有…」

调试版的同步状态在 `~/Library/Application Support/com.gxlself.bangs/sync-dev/`，和发布版的 `sync/` 分开（两个 CloudKit 环境）。

## 4. 待办（双向）

勾选 = 完成（可以再勾回来），删除是另一个操作，字会散开飘走。

| 操作 | 预期 |
| --- | --- |
| Mac 刘海里加一条待办 | iPhone 几秒内出现（靠静默推送；没到就下拉刷新） |
| iPhone 加一条 | Mac 20 秒内出现；托盘「立即同步」马上出现 |
| iPhone 上点一下 Mac 加的那条（整行都能点，键盘不会收起） | Mac 上那条变成划线、排到下面（≤ 20 秒）；再点一次两边都恢复 |
| Mac 上点一下 iPhone 加的那条 | iPhone 上它移到「已完成」 |
| Mac 上把鼠标移到一条上，点右边的 × | 字散开飘走；iPhone 上那条消失 |
| iPhone 左滑一条点「删除」（或长按 → 删除） | 字散开后消失；Mac 上那条消失 |
| Mac 上点「清除已完成（N）」 | 变成「再点一下清除」，3 秒内再点才清除；已完成的依次散开，iPhone 上也没了 |
| iPhone 上点「已完成」旁边的「清除」 | 先问「清除 N 件已完成的待办？」，确认后两边已完成的都没了 |
| 开同步之前 Mac 上已有的待办（包括已勾选的） | 第一次打开同步后出现在 iPhone 上，勾选状态一致 |
| 一边加到 60 条未完成 | 输入框变成「列表满了，先完成几件吧」，加不进去；已完成的超出时最旧的先掉 |

## 5. 代码页：开发会话（Mac → iPhone）

在 Mac 上开一个 Claude Code 会话（任意项目目录里 `claude`），让它跑起来，再让它停下来问你一个问题。

预期：iPhone「代码」页按 Mac 的名字分组出现这个会话；运行中是蓝色「运行中」，等你回答时变成橙色「等你回复」并排到最前，
下面写着它在等什么，「代码」标签上出现数字角标。退出会话后它从 iPhone 上消失。Codex 会话一直在跑时，
时间显示的是它进入「运行中」的时刻，不会每次写日志都变（也不会每次都触发一次同步）。

通知：有会话时第一次打开「代码」页，允许通知。然后把 iPhone 锁屏或切到别的 App，再让 Claude 停下来问一个问题：
几秒到几十秒内收到「Claude 在等你」（后台靠静默推送，系统可能推迟）；点通知直接打开「代码」页；
在 Mac 上回答之后那条通知自动消失。App 开着并且正停在「代码」页时不弹横幅（通知中心里有）。设置页可以关掉。

离线：把 Mac 合上盖子 20 分钟以上。iPhone 上那台 Mac 的会话变暗，分组标题写「离线，状态可能已过期」，
角标不再算它们，设置页那台 Mac 显示「上次在线 …」。打开盖子后 Mac 马上发心跳，iPhone 下一次同步时恢复。

## 6. 剪贴板（Mac → iPhone，需要 Paste 已安装）

在 Mac 上复制一段文字。

预期：iPhone「剪贴板」页最上面出现这一条；点一下提示「已复制」，到备忘录里粘贴出来是完整文本（不是截断的预览）。

再在 Mac 上截一小块文字并复制（⌃⇧⌘4）：iPhone 上这一条显示缩略图，点一下「已复制图片」，在备忘录里粘贴出来是这张图，
字和 Mac 上一样清楚（200 KB 以内的 PNG 原样同步，一个字节都不变）。截一整屏复制（Retina 全屏 PNG 一般 0.5 MB 以上）：
会被转成面积不超过 2048 × 2048 的 HEIC（全屏 3420 × 2146 变成 2585 × 1622），通常只有一两百 KB；粘贴出来放大看，字仍然清楚。
截一张长截图（几千像素高）：宽度基本保留，不会被压成一条窄缝。截一个带阴影的窗口（⌃⇧⌘4 后按空格）：
阴影外面透明的地方仍然透明。万一 HEIC 反而不比原图小（拿 Paste 里两百多张图实测没遇到过），而原图不超过 900 KB，发的就是原图。
复制一张不超过 900 KB 的 HEIC（比如 iPhone 拍的照片经「隔空投送」过来）：原样同步。写不了 HEIC 的老 Intel Mac 上转的是面积不超过 1600 × 1600 的 JPEG。

「太大，未同步」几乎见不到了：只有转到 640 × 640 的面积、质量 50 还超过 900 KB，或者 `sips` 打不开这张图、
原图又超过 900 KB 或是手机打不开的格式时才会这样。文件条目点了会提示只能在 Mac 上粘贴。

## 7. 暂存架（Mac → iPhone）

| 操作 | 预期 |
| --- | --- |
| 拖一张小图片到刘海 | iPhone「暂存」页出现，先显示「下载中…」，随后有缩略图，点开预览 |
| 拖一个 > 25 MB 的文件 | iPhone 上显示「太大，未同步」；点一下提示超过 25 MB 的文件只能在 Mac 上打开 |
| 从刘海暂存架移除 | iPhone 上消失 |

## 8. 离线与冲突

| 操作 | 预期 |
| --- | --- |
| iPhone 开飞行模式，加一条待办，再关飞行模式 | 回到前台或下拉刷新后推出去，Mac 上出现 |
| Mac 断网，删掉一条，恢复网络 | 20 秒内 iPhone 上消失 |
| 两边都断网：Mac 删掉一条，iPhone 加一条新的；再都恢复 | Mac 删掉的那条两边都没了，不会复活；iPhone 新加的两边都有 |
| 两边都断网，同一条一边勾选、一边不动；恢复 | 勾选生效 |

## 9. 开关与账号

| 操作 | 预期 |
| --- | --- |
| 托盘关掉同步 | 状态行和「立即同步」收起来，只剩开关；之后 Mac 的改动不再到 iPhone（iCloud 里已有的保留） |
| 再打开 | 关闭期间 Mac 上加的、勾选的待办同步过去 |
| iPhone 退出 iCloud 再打开 App | 各页写着这台 iPhone 没有登录 iCloud；设置页「没有登录 iCloud」和怎么处理的说明 |
| Mac 断网后重开同步（或开机时没网） | 状态行「暂时没能同步，稍后会自动重试」或「iCloud 暂时不可用」；联网后 30 秒内自己连上 |
| Dashboard → Reset Development Environment（会删掉 zone 和所有记录） | 下一次拉取时两边各自把数据重新传上去（Mac 终端里有一行 `the cloud lost this Mac's records (zone)`），iPhone 上的待办和 Mac 镜像过来的内容都回来 |
| iPhone 换一个 Apple ID 登录 | App 里旧账号的内容清空，从新账号拉；Mac 换账号则把本机的待办和镜像传到新账号 |

## 出问题时

- **Mac 日志**：`scripts/dev-icloud.sh` 的终端里，Rust 端以 `[sync]` 开头，Swift 桥以 `BangsCloudBridge:` 开头。
- **iPhone 日志**：Xcode 底部控制台，`[sync]` 开头。
- **CloudKit 里实际有什么**：Dashboard → Records，选 zone `Bangs`、类型 `BangsRecord`。`body` 是加密的，看不到内容；
  `kind`、`updatedAt`、`device`、`deleted` 看得到。Dashboard 的 Logs 页能看到每个请求和错误码。

常见情况：

| 现象 | 多半是 |
| --- | --- |
| Mac 一启动就被杀掉（终端里 `Killed: 9`） | 签名里的权限和 profile 对不上：`codesign -d --entitlements - src-tauri/target/debug/bundle/macos/Bangs.app` 和 profile 比一比，或者 profile 里没有这台 Mac |
| 托盘菜单里没有「与 iPhone 同步（iCloud）」 | 跑的是 `pnpm tauri dev`，不是 `scripts/dev-icloud.sh`：未签名的版本没有 iCloud 权限，菜单不提供开关 |
| iPhone 设置页「暂时没能同步」 | 下面一行是 CloudKit 的原话，对照这张表；改动都留在手机上，会自己重试 |
| 报 `Bad Container` / `badContainer` | App ID 没勾上容器 `iCloud.com.gxlself.bangs`，或 profile 是勾选之前生成的 |
| 报 `Not Authenticated` | 那台设备没登录 iCloud |
| 两边都正常但互相看不到 | 一边在 Development，一边在 Production（比如 iPhone 装的是 TestFlight 版）——见 [sync.md](sync.md#环境签名与权限) |
| iPhone 上改的东西 Mac 很久才到 | 正常最多 20 秒；托盘「立即同步」可以马上拉 |
| `Server Rejected Request` | 生产环境还没 Deploy Schema；开发环境一般是字段类型和已建的 schema 冲突，Dashboard 里 Reset Development Environment 后重试（两边会自己把数据传回去） |
| 发布版 Mac 和 TestFlight 版 iPhone 互相看不到 | Developer ID profile 里要有 Production 环境；`codesign -d --entitlements -` 看 `icloud-container-environment` 是不是 `Production` |
