#!/usr/bin/env bash
# Checks this Mac has everything the iPhone app and iCloud sync need, before the
# first build — one list of what is missing instead of one failed build at a time.
#
#   scripts/sync-preflight.sh           # check only
#   scripts/sync-preflight.sh --test    # also run the Swift and Rust merge tests and generate the Xcode project
#
# See docs/sync-testing.md for what to do once everything here is green.
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

container="iCloud.com.gxlself.bangs"
team="${BANGS_TEAM_ID:-W8L8ZJ3N2P}"
app_id="$team.com.gxlself.bangs"
profile="src-tauri/icloud/dev.provisionprofile"
failures=0

ok() { printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad() { printf '  \033[31m✗\033[0m %s\n' "$1"; [[ -n "${2:-}" ]] && printf '      → %s\n' "$2"; failures=$((failures + 1)); }
warn() { printf '  \033[33m!\033[0m %s\n' "$1"; [[ -n "${2:-}" ]] && printf '      → %s\n' "$2"; }

if [[ "$(uname)" != "Darwin" ]]; then
  echo "这个脚本要在 Mac 上跑" >&2
  exit 1
fi

echo "== 工具 =="
if xcode_version="$(xcodebuild -version 2>/dev/null | head -1)"; then
  major="$(echo "$xcode_version" | awk '{print $2}' | cut -d. -f1)"
  if [[ "${major:-0}" -ge 15 ]]; then ok "$xcode_version"; else bad "$xcode_version" "需要 Xcode 15 或更新"; fi
else
  bad "没有完整的 Xcode（只有命令行工具不够）" "从 App Store 装 Xcode，然后 sudo xcode-select -s /Applications/Xcode.app"
fi
if swift_version="$(swift --version 2>/dev/null | head -1)"; then ok "$swift_version"; else bad "没有 swift"; fi
if command -v xcodegen >/dev/null; then ok "xcodegen $(xcodegen --version 2>/dev/null | awk '{print $NF}')"; else bad "没有 xcodegen" "brew install xcodegen"; fi
if command -v pnpm >/dev/null; then ok "pnpm $(pnpm --version)"; else bad "没有 pnpm" "npm install -g pnpm"; fi
if command -v cargo >/dev/null; then ok "$(cargo --version)"; else bad "没有 Rust" "https://rustup.rs"; fi
[[ -d node_modules ]] && ok "node_modules 已装" || warn "还没装前端依赖" "pnpm install"

echo "== 签名 =="
identity="$(security find-identity -v -p codesigning 2>/dev/null | awk -F'"' '/Apple Development/ { print $2; exit }')"
if [[ -n "$identity" ]]; then ok "开发证书：$identity"; else bad "钥匙串里没有 Apple Development 证书" "Xcode → Settings → Accounts → Manage Certificates → + Apple Development"; fi

if [[ -f "$profile" ]]; then
  plist="$(security cms -D -i "$profile" 2>/dev/null)"
  if [[ -z "$plist" ]]; then
    bad "$profile 读不出来" "重新下载这个 profile"
  else
    tmp="$(mktemp)"; printf '%s' "$plist" > "$tmp"
    read_key() { /usr/libexec/PlistBuddy -c "Print :$1" "$tmp" 2>/dev/null; }
    ok "Mac 开发 profile：$(read_key Name)"
    [[ "$(read_key Entitlements:com.apple.application-identifier)" == "$app_id" ]] \
      && ok "App ID $app_id" \
      || bad "profile 的 App ID 是 $(read_key Entitlements:com.apple.application-identifier)，不是 $app_id" "用 App ID com.gxlself.bangs 重新生成"
    if read_key Entitlements:com.apple.developer.icloud-container-identifiers | grep -q "$container"; then
      ok "profile 里有容器 $container"
    else
      bad "profile 里没有容器 $container" "App ID 打开 iCloud → CloudKit 并勾上这个容器，然后重新生成 profile"
    fi
    expiry="$(read_key ExpirationDate)"
    if [[ -n "$expiry" ]] && [[ "$(date -j -f '%a %b %d %T %Z %Y' "$expiry" +%s 2>/dev/null || echo 0)" -lt "$(date +%s)" ]]; then
      bad "profile 已过期（$expiry）" "重新生成"
    else
      ok "有效期到 $expiry"
    fi
    # A development profile only runs on the Macs it lists.
    udid="$(system_profiler SPHardwareDataType 2>/dev/null | awk -F': ' '/Provisioning UDID/ {print $2}')"
    [[ -z "$udid" ]] && udid="$(system_profiler SPHardwareDataType 2>/dev/null | awk -F': ' '/Hardware UUID/ {print $2}')"
    if read_key ProvisionedDevices | grep -qi "$udid"; then
      ok "这台 Mac（$udid）在 profile 的设备列表里"
    else
      bad "这台 Mac（$udid）不在 profile 的设备列表里" "developer.apple.com → Devices 加上它，再重新生成 profile"
    fi
    rm -f "$tmp"
  fi
else
  bad "缺 $profile" "见 src-tauri/icloud/README.md：Mac App Development profile，App ID com.gxlself.bangs"
fi

echo "== iCloud =="
if defaults read MobileMeAccounts Accounts 2>/dev/null | grep -q AccountID; then
  ok "这台 Mac 登录了 iCloud"
else
  warn "看不出这台 Mac 有没有登录 iCloud" "系统设置 → 登录 Apple ID；没登录时托盘会显示「这台 Mac 没有登录 iCloud」"
fi
warn "iPhone 要登录同一个 Apple ID" "第一次运行后，设置 → [你的名字] → iCloud → 使用 iCloud 的 App 里 Bangs 要是打开的"

if [[ "${1:-}" == "--test" ]]; then
  echo "== 测试 =="
  if (cd packages/BangsCloud && swift test 2>&1 | tail -3); then ok "swift test"; else bad "swift test 失败" "把输出贴给我"; fi
  if (cd src-tauri && cargo test --lib -- sync:: todos:: 2>&1 | tail -3); then ok "cargo test"; else bad "cargo test 失败" "把输出贴给我"; fi
  if (cd ios && xcodegen generate >/dev/null); then ok "ios/Bangs.xcodeproj 已生成"; else bad "xcodegen generate 失败"; fi
fi

echo
if [[ $failures -eq 0 ]]; then
  echo "全部就绪。下一步见 docs/sync-testing.md。"
else
  echo "$failures 项要先处理。"
  exit 1
fi
