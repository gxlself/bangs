# iCloud 同步的签名材料

这里放 provisioning profile，**不进 git**（`.gitignore` 已经排除了 `*.provisionprofile`）。

| 文件名 | 什么 profile | 谁用 |
| --- | --- | --- |
| `dev.provisionprofile` | Mac App Development，App ID `com.gxlself.bangs`，包含你这台 Mac 的设备和开发证书 | `scripts/dev-icloud.sh` |
| `developer-id.provisionprofile` | Developer ID，App ID `com.gxlself.bangs` | `BANGS_ICLOUD=1 scripts/build-release.sh`，由 `tauri.icloud.conf.json` 塞进 `.app` |

两个 profile 都要在 [developer.apple.com](https://developer.apple.com/account/resources) 里生成，App ID 必须已经打开
iCloud（CloudKit，容器 `iCloud.com.gxlself.bangs`）和 Push Notifications。一次性配置的步骤见
[docs/sync.md](../../docs/sync.md#环境签名与权限)。
