#!/usr/bin/env bash
# Builds Bangs with iCloud sync, signed for development, and runs it, so this
# Mac can talk to the iPhone app run from Xcode (both are then on the CloudKit
# Development environment — see docs/sync.md). A plain `pnpm tauri dev` has no
# iCloud entitlements and cannot sync; use it for everything else.
#
#   scripts/dev-icloud.sh            # build, sign, run (logs stay in this terminal)
#   scripts/dev-icloud.sh --no-run   # build and sign only
#
# Needs src-tauri/icloud/dev.provisionprofile (src-tauri/icloud/README.md) and an
# "Apple Development" certificate in the keychain; BANGS_DEV_IDENTITY picks one
# when there are several.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

profile="src-tauri/icloud/dev.provisionprofile"
if [[ ! -f "$profile" ]]; then
  echo "缺 $profile —— 在 developer.apple.com 生成 Mac App Development profile（App ID com.gxlself.bangs），" >&2
  echo "放到这个路径，细节见 src-tauri/icloud/README.md" >&2
  exit 1
fi
identity="${BANGS_DEV_IDENTITY:-$(security find-identity -v -p codesigning | awk '/Apple Development/ { print $2; exit }')}"
if [[ -z "$identity" ]]; then
  echo "钥匙串里没有 Apple Development 证书（Xcode → Settings → Accounts → Manage Certificates 可以建）" >&2
  exit 1
fi
echo "签名身份 $identity"

pnpm tauri build --debug --bundles app --features icloud --config src-tauri/tauri.icloud.dev.conf.json

app="$root/src-tauri/target/debug/bundle/macos/Bangs.app"
[[ -d "$app" ]] || { echo "没找到 $app" >&2; exit 1; }
cp "$profile" "$app/Contents/embedded.provisionprofile"

# Nested code first (the MediaRemote helper), then the app with its entitlements.
# No --deep: it would sign the helper again without the identity's options.
while IFS= read -r library; do
  codesign --force --sign "$identity" "$library"
done < <(find "$app/Contents" -name '*.dylib')
codesign --force --sign "$identity" --entitlements src-tauri/entitlements.icloud.dev.plist "$app"
codesign --verify --verbose "$app"

echo "-- 签名里的 iCloud 权限 --"
codesign -d --entitlements - "$app" 2>/dev/null | grep -A2 icloud || echo "（没有找到，签名不对）" >&2

[[ "${1:-}" == "--no-run" ]] && exit 0
echo "-- 运行：托盘菜单里打开「同步到 iPhone（iCloud）」 --"
exec "$app/Contents/MacOS/$(ls "$app/Contents/MacOS" | head -1)"
