# Bangs for Apple Watch

手表上的 Bangs：一眼看 Mac 上在放什么、Claude / Codex 忙完没有，顺手切歌、记一条待办。
它通过局域网连 Mac 上的 Bangs（协议见 [docs/watch.md](../docs/watch.md)），不经过 iPhone 上的 Bangs。
它跟着 iPhone 版一起发布（App Store 里是同一个 App，手表端包在 iPhone 包里），装好以后自己就能跑
（`WKRunsIndependentlyOfCompanionApp`），iPhone 版删掉也不影响。

纵向翻页（转表冠或上下滑）有四页：

| 页 | 内容 |
| --- | --- |
| 播放 | 封面、歌名、正在唱的那句歌词（有翻译时带翻译）、进度、上一首 / 播放暂停 / 下一首 |
| 会话 | Claude Code 和 Codex 会话：运行中（绿）、等你回复（黄）、空闲（灰）。所有在跑的都列出来，空闲的只留最近几个（一共凑满 5 个），免得翻到待办要滚很久。会话从「运行中」变成别的状态时，手表轻震一下并在顶部提示 |
| 待办 | 刘海里那张待办清单里没做完的。点一条就是做完了：Mac 上它被勾掉（和在刘海里点一样），手表上它消失；第一行输入框可以听写、手写或键盘加一条 |
| 上岛 | 其他程序挂到刘海上的行（[docs/plugins.md](../docs/plugins.md)），只读 |

左上角齿轮里能看到连的是哪台电脑，以及取消配对。

## 要求

- Xcode 16 或更新、XcodeGen（`brew install xcodegen`）
- watchOS 10 或更新
- Mac 上的 Bangs 是包含本功能的版本，手表和 Mac 在同一个局域网里

## 在模拟器上跑

1. 工程在 iPhone 版那边生成：`cd ios && xcodegen generate && open Bangs.xcodeproj`（target 定义在
   `ios/project.yml` 的 `BangsWatch`，源码就是这个目录）。scheme 选 `BangsWatch`，目标选一个 Apple Watch 模拟器，⌘R。
2. Mac 上的 Bangs：菜单栏图标 → **Apple Watch → 允许手表连接**。子菜单里会出现「地址」和「配对码」。
   如果 macOS 防火墙问是否允许 Bangs 接受传入的网络连接，选「允许」。
3. 模拟器里填地址和 6 位配对码，点「配对」。模拟器和 Mac 是同一台机器，地址也可以直接填 `127.0.0.1`。

## 在真手表上跑

1. Team 默认是 `W8L8ZJ3N2P`。换成自己的团队时，手表的 Bundle ID（`ios/project.yml` 里
   `com.gxlself.bangs.ios.watchkitapp`）和 `WKCompanionAppBundleIdentifier` 要跟着 iPhone 版的一起改：
   手表的 Bundle ID 必须以 iPhone 版的开头。
2. iPhone 连着 Mac、手表和 iPhone 配对好，并打开手表的开发者模式（设置 → 隐私与安全性 → 开发者模式）。
3. 目标选你的手表，⌘R。第一次装要在手表上信任开发者证书。
4. 照上面第 2、3 步配对，地址填菜单里显示的那个（`192.168.x.x:17651`）。

手表在 iPhone 旁边时，请求经 iPhone 转发；单独连 Wi-Fi 时直接走 Wi-Fi。两种情况 Mac 都得在同一个局域网里。

## 验收清单

- [ ] Mac 托盘菜单出现 **Apple Watch** 子菜单，打开开关后显示地址和配对码；关掉后地址和配对码消失
- [ ] 手表配对成功后进入「播放」页；Mac 菜单里配对码换了一个新的，「忘掉已配对的手表（1）」可点
- [ ] 输错配对码提示「配对码不对」；连续错 5 次后 Mac 菜单里的配对码会换掉
- [ ] 播放页：歌名、封面、进度和 Mac 一致；歌词跟着走；三个按钮能控制 Mac 上的播放器
- [ ] 会话页：开一个 Claude Code 会话让它干活，手表显示「运行中」；它停下来时手表震一下并提示
- [ ] 待办页：手表加一条，Mac 刘海的待办里出现；手表点掉一条，手表上消失、Mac 刘海里显示为已完成（划掉）
- [ ] 上岛页：按 docs/plugins.md 写一个 JSON，手表上能看到
- [ ] 关掉 Mac 上的开关，手表底部提示「连不上 Mac」；重新打开后自动恢复，不用重新配对
- [ ] Mac 菜单点「忘掉已配对的手表」，手表下一次刷新就回到配对页
- [ ] 改 Mac 的地址后（或填 `127.0.0.1` 之类同一台机器的别的地址），手表设置页「换地址」能连上，不用重新配对
- [ ] 手表设置页「取消配对」后回到配对页，Mac 菜单里的已配对数量减一

## App Store 截图

Debug 构建带一套演示数据（`BangsWatch/DemoData.swift`），不连网、不碰钥匙串，一次启动一张图：

```bash
xcrun simctl launch <udid> com.gxlself.bangs.ios.watchkitapp -BangsDemo YES -BangsPage playing   # sessions / todos / board
xcrun simctl launch <udid> com.gxlself.bangs.ios.watchkitapp -BangsDemo YES -BangsPage todos -AppleLanguages "(zh-Hans)"
xcrun simctl io <udid> screenshot shot.png
```

## 已知限制

- 只在 App 打开时（包括抬腕后熄屏、App 还在前台时）每两秒拉一次；App 退到后台后不再收提醒。
  要在后台也能提醒，需要走推送，那是下一步的事。
- 局域网里是明文 HTTP，详见 docs/watch.md 的「What travels」。不信任的网络里请把开关关掉。
- Mac 地址变了（换网络、DHCP 换了 IP）时，在手表齿轮 →「换地址」里填新地址即可，配对还在。
