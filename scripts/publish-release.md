# 发一个版本

每次发版就这几步，脚本都在这个目录里。

```bash
scripts/version.sh 0.2.0        # 三个文件一起改（package.json / tauri.conf.json / Cargo.toml）
git commit -am "Release v0.2.0"
git tag v0.2.0 && git push origin main --tags

scripts/build-release.sh        # mac 本机打包 + ssh 到 Windows 打包，产物收到 dist/release/v0.2.0/
scripts/publish-site.sh         # 站点推到 gh-pages
```

产物：

| 文件 | 平台 |
| --- | --- |
| `Bangs_<版本>_aarch64.dmg` | macOS，拖进「应用程序」 |
| `Bangs_<版本>_aarch64.app.zip` | macOS，不想挂载磁盘映像的人 |
| `Bangs_<版本>_x64-setup.exe` | Windows，NSIS 安装包 |
| `Bangs_<版本>_x64_en-US.msi` | Windows，MSI（给有策略要求的场合） |

然后在 GitHub 上建 Release：标签填 `v0.2.0`，把 `dist/release/v0.2.0/` 里的四个文件全传上去。
**资产文件名必须保留 `.dmg` / `.app.zip` / `setup.exe` / `.msi` 后缀** —— 官网靠后缀认哪个是哪个平台的包
（见 `site/app.js` 里的 `MAC` / `WINDOWS` 两个正则），应用内的「检查更新」只认 `tag_name`。

用 `gh` 一条命令就能建好：

```bash
gh release create v0.2.0 dist/release/v0.2.0/* --title "Bangs v0.2.0" --notes-file notes.md
```

## 发布说明：中英双语

每个版本的说明都是**中文在前、英文在后**，中间用一条 `---` 隔开，两段内容对应：

- 中文：一句话开头，然后 `## 改了什么`、`## 安装`，最后一行 `官网：https://gxlself.github.io/bangs/`
- 英文：同样的结构，`## What's new`、`## Install`，最后一行 `Website: https://gxlself.github.io/bangs/`

英文里提到界面上的东西，用应用英文界面里的原词（Board、To-do、Display、"Follow the main display"……），
在 `src/` 里搜 `t("中文", "English")`、托盘的文字在 `src-tauri/src/tray.rs` 和 `update.rs`。
「安装」一段照抄上一个版本，除非安装方式变了。

回头改旧版本的说明时加上 `--latest=false`：

```bash
gh release edit v0.1.0 --latest=false --notes-file notes.md
```

不加的话 GitHub 可能把被改的旧版本标成 Latest，官网的下载按钮和应用里的「有新版本」就都指错了。

## 签名与公证（macOS）

`build-release.sh` 会自动拿钥匙串里属于本项目团队（`W8L8ZJ3N2P`，可用 `BANGS_TEAM_ID` 改）的
**Developer ID Application** 证书签名 —— 不能随便拿第一张：钥匙串里还有公司的证书，
用它签出来的包团队和公证凭据对不上，公证会直接被拒。
它连带签 app 里那个 MediaRemote 桥接 dylib（公证不接受只有 ad-hoc 签名的二进制）。
硬化运行时（hardened runtime）是打开的，`src-tauri/entitlements.plist` 里那条
`com.apple.security.automation.apple-events` 不能删 —— 少了它，公证后的版本
控制 Spotify 会被系统直接拒掉。

公证凭据存一次就够，**密码只经过你的手**：

```bash
xcrun notarytool store-credentials bangs-notary \
  --apple-id <你的 Apple ID> --team-id W8L8ZJ3N2P --password <App 专用密码>
```

App 专用密码在 https://account.apple.com 的「登录与安全 → App 专用密码」里生成，
**只显示一次**，那一屏关掉就只能重建一个。凭据存在钥匙串里，`build-release.sh`
默认就找 `bangs-notary` 这个名字（换名字用 `BANGS_NOTARY_PROFILE`），
自动公证 `.app` 和 `.dmg` 并 staple 票据，最后跑一次 `spctl` 验证。
钥匙串里没有这份凭据时，它会说一声然后只签名、不公证。

装好之后确认一句话就够：`spctl -a -vv /Applications/Bangs.app` 说
`accepted / source=Notarized Developer ID` 就对了。

## Windows 没签名

SmartScreen 会拦，点「更多信息 → 仍要运行」。Windows 的签名要另买证书（EV 或 OV），
目前没有。

## Windows 构建机

`scripts/build-release.sh` 默认用 ssh 别名 `win-gxl`（改 `BANGS_WIN_HOST` 可以换别的机器）。
那台机器上**所有缓存都在 D 盘**：`CARGO_HOME`、`CARGO_TARGET_DIR`、pnpm store，
连 Tauri 自己下载 NSIS/WiX 的 `%LOCALAPPDATA%` 都被指到了 `D:\Develop\build-cache`。
它需要预装：Rust（MSVC toolchain）、Node + pnpm、Git、WebView2 运行时。

ssh 登进 Windows 落在 session 0，画不出界面，所以要在那台机器桌面上**看**刘海得用交互式计划任务：

```powershell
schtasks /create /tn Bangs /tr "D:\Develop\Bangs\bangs.exe" /sc once /st 00:00 /it /f
schtasks /run /tn Bangs
```
