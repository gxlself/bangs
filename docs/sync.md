# 同步：Mac ↔ iPhone / iPad（iCloud · CloudKit）

Bangs 像 Paste 一样，用 **CloudKit 私有数据库**同步，走用户自己的 Apple ID，不需要服务器。
这份文档是桌面端（Rust）和 iOS 端（Swift）共同遵守的契约：改动格式时两边一起改，
并更新 [`sync-vectors.json`](sync-vectors.json)——两边的测试都读这份向量。

```
┌────────────── macOS · Bangs（Tauri） ──────────────┐        ┌──────── iOS · Bangs（SwiftUI） ────────┐
│ todos / clipboard / dev / shelf                    │        │ To-do · 开发 · 剪贴板 · 文件架          │
│        │ 本地变更             ▲ 远端变更           │        │      ▲ 展示            │ 勾选/新增待办  │
│   sync::Ledger（版本表 + 待推送队列，Rust）        │        │  RecordStore（版本表 + 队列，Swift）    │
│        │ JSON                 │ JSON               │        │      │ 同一个 BangsCloud 引擎           │
│   C ABI（BangsCloudBridge，Swift 静态库）          │        │      ▼                                  │
└────────┼──────────────────────┼────────────────────┘        └──────┼──────────────────────────────────┘
         ▼                      ▲                                    ▼
                  CloudKit 私有库 · iCloud.com.gxlself.bangs · 自定义 zone「Bangs」
```

Windows 版没有 iCloud，同步保持关闭，行为和以前完全一样。

## 同步什么、谁说了算

| kind | 方向 | 记录 id | 说明 |
| --- | --- | --- | --- |
| `todo` | 双向 | Mac 生成的 id，或 iOS 生成的 UUID | 勾选 = 完成（`done`，可以再勾回来），是对记录的修改；删除才写墓碑 |
| `session` | Mac → iOS | `<device8>-<会话 id>` | Claude Code / Codex 会话的忙碌、等待、空闲；Mac 上消失的会话写墓碑 |
| `clip` | Mac → iOS | `<device8>-<内容哈希>` | 最近 24 条剪贴板（面板的第一页）；文本带全文（≤ 8000 字符），图片/文件只有预览文字。读不到 Paste 的数据库时这一轮不动，免得 id 全变 |
| `shelf` | Mac → iOS | `<device8>-<路径哈希>` | 文件架；≤ 25 MB（25,000,000 字节，十进制）的文件作为 CKAsset 上传，更大的只同步元数据。文件夹不同步 |

- `device8` 是设备 id（随机 UUID，去掉连字符取前 8 位小写十六进制）。镜像类 kind 的 id 带设备前缀，
  所以两台 Mac 不会互相写墓碑；镜像只会清理**本设备**写过的记录。
- iOS 端把 `session` / `clip` / `shelf` 当只读缓存；只有 `todo` 会从 iOS 写回。
- 记录 id 必须是 ASCII：`[A-Za-z0-9._-]{1,128}`（CloudKit 的 recordName 限制）。
  镜像类 kind 不满足时改用原值的 FNV-1a 64 位十六进制；待办的 id 由 Bangs 和 iOS 端生成，总是满足，
  手改 `todos.json` 造出来的不满足的 id 不同步。

## CloudKit 约定

| 项 | 值 |
| --- | --- |
| 容器 | `iCloud.com.gxlself.bangs`（私有数据库） |
| zone | `Bangs`（自定义 zone，才有变更令牌和 zone 订阅） |
| 记录类型 | `BangsRecord`（只有这一种；开发环境首次保存时自动建 schema） |
| recordName | `<kind>:<id>`，如 `todo:18f2a-0` |
| 订阅 | `CKRecordZoneSubscription`，id `bangs-zone`，`shouldSendContentAvailable = true`（静默推送） |

`BangsRecord` 的字段：

| 字段 | 类型 | 含义 |
| --- | --- | --- |
| `kind` | String | `todo` / `session` / `clip` / `shelf`；不认识的 kind 直接忽略（向前兼容） |
| `updatedAt` | Int64 | 版本时间，Unix 毫秒 |
| `device` | String | 写入这个版本的设备 id（完整 UUID） |
| `deleted` | Int64 | 1 = 墓碑。删除用墓碑而不是 `CKDatabase.deleteRecord`，这样落后的设备也能知道它没了 |
| `body` | String | 下面的 JSON 文本，≤ 64 KB；墓碑写 `{}`。**写入 `record.encryptedValues["body"]`，不是普通字段**，读取同理 |
| `asset` | Asset（可选） | 只有 `shelf` 用；普通字段（不加密） |

所有设备共用一种记录类型，是为了不让两端各自维护字段映射：加字段只改 `body`，不用动生产 schema。

`body` 走 CloudKit 的 `encryptedValues`（iOS 15 / macOS 12 起可用，端到端加密，密钥只在用户自己的设备上）：
剪贴板里可能有密码，不能让它以明文躺在 CloudKit Dashboard 里。代价是 `body` 不能被查询或建索引——我们只用变更流，不需要。
`kind`、`updatedAt`、`device`、`deleted` 仍是普通字段（只暴露种类和时间）。文件架的文件本身没有加密，和 iCloud Drive 一样只有 Apple 的静态加密。

### body

字段名全部 camelCase，不认识的字段忽略，缺失的可选字段当 `null`。时间都是 Unix 毫秒。

```jsonc
// todo
{ "text": "买牛奶", "createdAt": 1760000000000,
  "done": false,                // 勾选了没有；没有这个字段（旧版本写的）当作 false
  "doneAt": null }              // 勾选的时间；没勾选是 null

// session
{ "agent": "claude",            // "claude" | "codex"
  "name": "bangs", "project": "bangs", "path": "/Users/me/bangs",
  "status": "waiting",          // "busy" | "waiting" | "idle"
  "detail": "Allow Bash?",      // 在等什么，可为 null
  "statusAt": 1760000000000,
  "host": "gxl 的 MacBook Pro" }

// clip
{ "type": "text",               // "text" | "image" | "files"
  "preview": "https://…",       // 折叠空白、截断后的一行
  "text": "完整文本…",           // 只有 type = text 才有，≤ 8000 字符
  "app": "Safari", "pinned": false,
  "createdAt": 1760000000000, "host": "gxl 的 MacBook Pro" }

// shelf
{ "name": "报价单.pdf", "extension": "pdf", "size": 183204,
  "isImage": false, "addedAt": 1760000000000, "host": "gxl 的 MacBook Pro" }
// 文件本身在记录的 asset 字段里；size > 25 MB 时没有 asset
```

## 传输格式（Rust ↔ Swift，也是测试向量的格式）

一条记录：

```json
{"kind":"todo","id":"18f2a-0","updatedAt":1760000000000,"device":"3f1c…","deleted":false,
 "body":{"text":"买牛奶","createdAt":1760000000000},"asset":null}
```

`asset` 是一个本地文件的绝对路径：发送时是要上传的文件；接收时是引擎已经下载好的副本。
记录的 **key** 是 `kind:id`，也就是 CloudKit 的 recordName。

## 合并规则（最后写入者胜）

比较两个版本 `(updatedAt, device)`：先比 `updatedAt`，相等再按字节序比 `device`，大者胜。
完全相同（自己写的记录从 CloudKit 变更流里回来）不算胜，直接忽略。

每端维护一张**版本表**：`key → (updatedAt, device, deleted)`，墓碑也在里面——
这样晚到的旧版本不会让已经删掉的待办复活。

- **收到远端记录** `applyRemote`：没有版本，或胜过本地版本 → **采用**（写入本地数据，更新版本表，
  并把待推送队列里同一个 key 的旧记录丢掉）；否则忽略。同一批里按顺序逐条处理。
- **本地新增/修改** `localUpsert`：版本时间取 `max(now, 上一个版本的 updatedAt + 1)`，
  所以同一条记录在本机上单调递增，时钟偏慢也压得过已有版本。`body` 和上次一样就什么都不做。
  新记录放进待推送队列。
- **本地删除** `localDelete`：本地没有这条或已经是墓碑就什么都不做；否则写墓碑（同样单调递增）。
- **镜像** `reconcile(kind, 期望的列表)`（仅 Mac 的 `session` / `clip` / `shelf`）：
  期望列表里的每一项走 `localUpsert`；本设备写过、现在不在列表里的记录走 `localDelete`。
- **墓碑清理**：墓碑版本超过 90 天、且不在待推送队列里，从版本表移除。
- **首次开启同步**：本地已有但版本表里没有的待办，以它的 `createdAt` 作为版本时间收编，
  不能用「现在」，否则会压过别处更晚的修改。

引擎推送时用 `savePolicy = .ifServerRecordUnchanged`；遇到 `serverRecordChanged`，
拿服务器上那条记录按上面的规则比较：我们胜就把字段写到服务器记录上重试（最多 3 次），
服务器胜就放弃本次推送，并把服务器的记录当作远端记录交给上层。
拉取总是先于推送：启动时先拉一次再把离线期间的队列推出去。

## macOS 原生桥（C ABI）

Rust 通过 `packages/BangsCloud` 里的 `BangsCloudBridge`（Swift 静态库，`@_cdecl` 导出）调用 CloudKit，
一共五个函数，字符串都是 UTF-8、以 `\0` 结尾：

```c
int32_t bangs_cloud_supported(void);   // 当前二进制的签名里有 iCloud 权限时返回 1
void    bangs_cloud_start(const char *state_dir, void (*callback)(const char *event_json));
void    bangs_cloud_push(const char *records_json);   // JSON 数组，元素是上面的记录
void    bangs_cloud_pull(void);                       // 立刻拉一次变更
void    bangs_cloud_stop(void);
```

- **必须先调 `bangs_cloud_supported`**。进程的签名里没有 `com.apple.developer.icloud-services` 时，
  创建 `CKContainer` 会直接崩溃、捕获不了——`pnpm tauri dev` 起的未签名二进制就是这种情况。
  检查方法是 `SecTaskCopyValueForEntitlement`（Paste 的 `ICloudCapability.swift` 同一个做法）。
  比 Paste 更严一点：`icloud-services` 里要有 `CloudKit`，`icloud-container-identifiers` 里要有
  `iCloud.com.gxlself.bangs`，因为打开一个签名里没列出的容器同样会崩。
- `state_dir` 用来放变更令牌（`token.bin`）、当前 iCloud 账号（`account.txt`）和下载的资源
  （`assets/<recordName 里的 : 换成 _>`，不进备份）。Mac 端不保留资源文件：文件架只从 Mac 往外发，
  Mac 拉回来的只会是自己上传的那些。
- `callback` 可能在任意线程被调用；`event_json` 只在调用期间有效，需要自行拷贝。事件：

```jsonc
{"event":"status","state":"starting"}      // starting | ready | syncing | idle | noAccount | restricted | unavailable | error
{"event":"status","state":"error","message":"…"}
{"event":"records","records":[ /* 拉到的记录，含墓碑 */ ]}
{"event":"pushed","keys":["todo:18f2a-0"]}
{"event":"rejected","keys":["todo:18f2a-0"]}        // 冲突里输了；赢的那条会随后以 records 事件到达
{"event":"failed","keys":["todo:18f2a-0"],"message":"…","retryAfter":30}   // 临时失败，调用方放回队列
{"event":"reset","reason":"zone"}                   // 云端丢了这台设备放上去的东西，见下
```

- `ready` 表示账号可用、zone 和订阅都建好了；收到后调用方应先 `bangs_cloud_pull()`，再推送队列。
- **每次 `pull` / `push` 都以一个 `status` 事件收尾**：开始时发 `syncing`，成功结束发 `idle`，失败发 `error`
  （带 `message`）。`records` / `pushed` / `rejected` / `failed` 事件排在这个收尾事件之前。
  调用方靠「`ready` 之后的第一个 `idle`」知道启动时的那次拉取已经完成，之后才推送离线期间攒下的队列。
- 令牌过期（`changeTokenExpired`）时引擎自己丢掉令牌从头拉。
- **`reset`**：zone 没了（`zoneNotFound` / `userDeletedZone`，比如在 Dashboard 里 Reset Development Environment）
  时引擎重建 zone 并发 `{"event":"reset","reason":"zone"}`；`start()` 发现登录的 iCloud 账号和上次不同时
  （比较 `CKContainer.userRecordID()`，存在 `account.txt`）丢掉令牌并发 `{"event":"reset","reason":"account"}`。
  收到后调用方要把自己的数据**全部重新上传**：Mac 端清空版本表、重新收编待办、重新镜像其它三类；
  iOS 端遇到 `zone` 把本地的待办重新放进队列，遇到 `account` 清空本地存储（那是另一个账号的数据）。
  账号变化时（`CKAccountChanged` 通知）引擎会自己重新 `start()`；没登录（`notAuthenticated`）一律报 `noAccount`。
- 引擎被 `stop()` 之后不再发任何事件，也不再保存令牌——停掉时正在进行的拉取不会把没人收的记录算作已拉取。
- 推送时遇到不会因为重试而好转的错误（`invalidArguments`、`assetFileNotFound`、`permissionFailure`）
  按 `rejected` 报告并在 stderr 留一行日志，不会每 30 秒重试一次、挡住后面的改动。
- 托盘手动打开同步时，Mac 端会删掉令牌从头拉一次：关着的时候到达的记录没人收，令牌不能信。

- Mac 端不接收静默推送（Tauri 占着 AppDelegate），所以每 20 秒拉一次；托盘菜单的「立即同步」马上拉一次，
  并且不等失败后的重试间隔就把队列推出去。

## 环境、签名与权限

**CloudKit 的 Development 和 Production 是两个完全独立的数据库。** 用开发证书签名的包读写 Development，
TestFlight / App Store / Developer ID 的包读写 Production，两边互相看不到对方的记录
（Paste 的 `ICloudCapability.Environment` 记着同一个坑）。联调时，Mac 和 iPhone 要在同一个环境：

| 场景 | Mac | iPhone |
| --- | --- | --- |
| 日常联调（Development） | `scripts/dev-icloud.sh`，用 Apple Development 证书 + 开发 profile | Xcode 直接 Run 到真机 |
| 正式使用（Production） | Developer ID 签名的发布包，带 Developer ID profile | TestFlight / App Store |

上生产之前，要在 CloudKit Dashboard 里把 Development 的 schema **Deploy to Production**，
否则生产环境里没有 `BangsRecord` 这个记录类型，所有保存都会失败。部署之前，先在 Development 里
至少同步过一个文件架里的文件：`asset` 字段是第一次有人存它时才建出来的，没建的字段不会被部署，
生产环境里所有带文件的推送就都会失败。

发布用的 `entitlements.icloud.plist` 写明了 `com.apple.developer.icloud-container-environment = Production`，
联调用的写 `Development`：Developer ID 签名的 app 必须自己声明用哪个环境（Xcode 导出 Developer ID 时会自动加上）。

一次性的 Apple Developer 配置（团队 `W8L8ZJ3N2P`）：

1. 在 Identifiers 里新建 iCloud 容器 `iCloud.com.gxlself.bangs`。
2. App ID `com.gxlself.bangs`（Mac）打开 iCloud → CloudKit，勾上这个容器；Push Notifications 一并打开。
3. 新建 App ID `com.gxlself.bangs.ios`（iOS），同样打开 iCloud → CloudKit 和 Push Notifications。
4. 为 Mac 生成 **Mac App Development** profile（联调用）和 **Developer ID** profile（发布用），
   放到 `src-tauri/icloud/`，文件名见该目录的 README。profile 不进 git。

只有 iCloud 能力需要 profile；没有 profile 时 Bangs 照常运行，只是同步开关会提示「这个版本没有 iCloud 权限」。

## 已知限制

- **没有在 Mac 和真机上跑过。** 这一套是在没有 Swift 工具链的环境里写的：Rust 部分有测试并在 macOS / Windows /
  Linux 三个目标上通过了类型检查；Swift 部分没有真正编译过，只做了语法检查（tree-sitter），
  并经过一轮对照 iOS SDK 接口文件逐行核对 API 签名的审查，没有找到编译错误——但第一次在 Mac 上
  `swift test` 和 Xcode Run 时仍可能要修小问题。合并规则有共用的测试向量，CloudKit 那一层只能靠真机联调。
- Mac 会把自己上传的文件架文件在下一次拉取时再下载一次（下载完马上丢掉，不占磁盘）。可以用 `desiredKeys`
  不取 `asset` 来省掉，但加密的 `body` 在指定 `desiredKeys` 时是否照样返回没验证过——取不回来，手机加的待办
  就会被静默丢掉，所以先不冒这个险。真机上确认之后可以改 `CloudEngine.fetchPages`。
- iOS 图标是从桌面端图标生成的（圆角方块放大铺满、四角补渐变、去掉透明通道），能过 App Store 的检查，
  但设计上值得找人出一张正式的。
- 每次更新已有记录都要两个请求：引擎每次都新建 `CKRecord`，第一次必然撞 `serverRecordChanged`，
  拿服务器那份再存一次。可以按 key 缓存 system fields 省掉一次，没做。
- 「Claude 在等你」的手机通知是 App 被静默推送唤醒后发的本地通知：App 在后台时系统可能推迟或合并
  静默推送（低电量模式下更明显），所以不保证秒到；App 在前台或回到前台时一定会到。
- 关闭同步只是停止收发，iCloud 里已经同步的记录保留，手机上还看得到最后的状态。
- iOS 端收到记录和保存变更令牌之间有几毫秒的间隙（事件要切到主线程处理）：恰好在这时被杀，会漏掉那一批记录。
  `store.json` 丢失或损坏时 iOS 端会连令牌一起删掉重新全量拉取，只是这个窗口本身没补。
  Mac 端没有这个问题：回调是同步的，Rust 存完数据才返回，引擎之后才存令牌。

## 不在第一阶段里的东西

- 小组件、Live Activity、键盘扩展（Paste 有，Bangs 的 iOS 端暂时没有）。
- 从 iOS 往 Mac 的文件架投递（分享扩展）。
- 清理 CloudKit 里的旧墓碑；目前只清本地版本表。
- 一台设备离线超过 90 天后再上线，它手里的旧版本可能让已被清掉墓碑的待办复活。
