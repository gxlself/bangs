#!/usr/bin/env bash
# Builds what a release needs and collects it in dist/release/v<version>/.
#
#   scripts/build-release.sh            # both platforms
#   scripts/build-release.sh --mac      # this Mac only
#   scripts/build-release.sh --windows  # the Windows box only
#
# macOS is built here; Windows is built over ssh on the box named by
# BANGS_WIN_HOST (default: win-gxl), whose caches all live on D: — see
# scripts/windows-build.ps1.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

host="${BANGS_WIN_HOST:-win-gxl}"
# Signing identity: a Developer ID belonging to this app's team, which is not
# necessarily the first one in the keychain — an employer's certificate can sit
# above it, and notarization then fails because the team does not match the
# credentials. The hash is used rather than the name, which repeats when a
# certificate has been imported twice and then matches ambiguously.
team="${BANGS_TEAM_ID:-W8L8ZJ3N2P}"
identity="${APPLE_SIGNING_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null |
  awk -v team="$team" '/Developer ID Application/ && index($0, team) { print $2; exit }')}"
# The keychain profile holding the notarization credentials; see
# scripts/publish-release.md for the one command that creates it.
notary="${BANGS_NOTARY_PROFILE:-bangs-notary}"
remote_repo='D:\Develop\bangs-src\repo'
version="$(node -p "require('./package.json').version")"
out="$root/dist/release/v$version"

want_mac=1
want_windows=1
case "${1:-}" in
  --mac) want_windows=0 ;;
  --windows) want_mac=0 ;;
  "") ;;
  *) echo "用法: $0 [--mac|--windows]" >&2; exit 1 ;;
esac

echo "== Bangs v$version =="

if [[ $want_mac -eq 1 ]]; then
  echo "-- macOS --"
  if [[ -n "$identity" ]]; then
    echo "签名身份 ${identity}（团队 ${team}）"
  else
    echo "钥匙串里没有 $team 的 Developer ID 证书，打出来的包是未签名的" >&2
  fi
  # Asking for the credentials is the only way to know they are there; without
  # them the build still produces a signed bundle, it just is not notarized.
  # No flags beyond the profile: notarytool has dropped options before, and a
  # check that fails for the wrong reason skips notarization without a word.
  if [[ -n "$notary" ]] && ! xcrun notarytool history --keychain-profile "$notary" >/dev/null 2>&1; then
    echo "钥匙串里没有 $notary 的公证凭据，这次只签名不公证（见 scripts/publish-release.md）" >&2
    notary=""
  fi
  # `pnpm build` empties dist/, which is where $out lives, so it is made after.
  # BANGS_ICLOUD=1 builds the version that can sync with the iPhone app
  # (docs/sync.md): the Swift bridge, the iCloud entitlements and the Developer ID
  # provisioning profile from src-tauri/icloud/. Without it nothing changes.
  extra=()
  if [[ "${BANGS_ICLOUD:-}" == "1" ]]; then
    [[ -f src-tauri/icloud/developer-id.provisionprofile ]] ||
      { echo "BANGS_ICLOUD=1 需要 src-tauri/icloud/developer-id.provisionprofile（见该目录的 README）" >&2; exit 1; }
    extra=(--features icloud --config src-tauri/tauri.icloud.conf.json)
  fi
  APPLE_SIGNING_IDENTITY="$identity" pnpm tauri build --bundles app,dmg ${extra[@]+"${extra[@]}"}
  bundle="$root/src-tauri/target/release/bundle"
  # Tauri's own name for the architecture; older bundles of other versions stay
  # in that directory, so the file is named, never globbed.
  mkdir -p "$out"
  arch="$([[ "$(uname -m)" == "arm64" ]] && echo aarch64 || echo x64)"
  dmg="$bundle/dmg/Bangs_${version}_${arch}.dmg"
  [[ -f "$dmg" ]] || { echo "没找到 $dmg" >&2; exit 1; }
  app="$bundle/macos/Bangs.app"
  if [[ -n "$notary" ]]; then
    # Notarize the app first, staple it, and only then wrap it up: a stapled
    # ticket has to be inside whatever people download.
    echo "-- 公证 --"
    ditto -c -k --keepParent "$app" "$bundle/macos/Bangs-notarize.zip"
    xcrun notarytool submit "$bundle/macos/Bangs-notarize.zip" \
      --keychain-profile "$notary" --wait
    xcrun stapler staple "$app"
    rm -f "$bundle/macos/Bangs-notarize.zip"
    xcrun notarytool submit "$dmg" --keychain-profile "$notary" --wait
    xcrun stapler staple "$dmg"
  else
    echo "没有设置 BANGS_NOTARY_PROFILE，跳过公证（见 scripts/publish-release.md）" >&2
  fi
  # A zipped .app is what people who dislike mounting a disk image want.
  ditto -c -k --keepParent "$app" "$out/Bangs_${version}_${arch}.app.zip"
  cp "$dmg" "$out/Bangs_${version}_${arch}.dmg"

  if [[ -n "$notary" ]]; then
    echo "-- Gatekeeper --"
    spctl --assess --type execute --verbose=2 "$app" 2>&1 | tail -2
  fi
fi

if [[ $want_windows -eq 1 ]]; then
  echo "-- Windows ($host) --"
  mkdir -p "$out"
  bundle="$root/dist/release/.bundle.$$"
  git bundle create "$bundle" HEAD >/dev/null
  scp -q "$bundle" "$host:D:/Develop/bangs-src/release.bundle"
  rm -f "$bundle"
  scp -q "$root/scripts/windows-build.ps1" "$host:D:/Develop/bangs-src/windows-build.ps1"

  set +e
  ssh "$host" "powershell -NoProfile -ExecutionPolicy Bypass -File D:\\Develop\\bangs-src\\windows-build.ps1" |
    iconv -f utf-8 -t utf-8 -c | tail -20
  built=${PIPESTATUS[0]}
  set -e
  [[ $built -eq 0 ]] || { echo "Windows 构建失败（退出码 ${built}），日志在 $host 的 D:\\Develop\\bangs-src\\build.log" >&2; exit 1; }

  for name in "Bangs_${version}_x64-setup.exe" "Bangs_${version}_x64_en-US.msi"; do
    scp -q "$host:D:/Develop/bangs-src/artifacts/$name" "$out/$name" ||
      { echo "缺少 ${name}（Windows 端没打出来）" >&2; exit 1; }
    echo "取回 $name"
  done
fi

echo
echo "产物在 $out"
ls -lh "$out" | tail -n +2 | awk '{ printf "  %-44s %s\n", $9, $5 }'
echo
echo "下一步：scripts/publish-release.md 里是发版清单"
