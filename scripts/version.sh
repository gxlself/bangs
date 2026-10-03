#!/usr/bin/env bash
# Sets the version everywhere it is written down. The app reads its own from
# Cargo.toml (update.rs), the bundles from tauri.conf.json, and the site from
# the GitHub release — so these three must never drift apart.
#
#   scripts/version.sh            # print the current version
#   scripts/version.sh 0.2.0      # set it
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

current() {
  node -p "require('./package.json').version"
}

if [[ $# -eq 0 ]]; then
  current
  exit 0
fi

next="$1"
if [[ ! "$next" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "版本号要是 x.y.z，比如 0.2.0（收到的是：${next}）" >&2
  exit 1
fi

node -e '
  const fs = require("fs");
  const next = process.argv[1];
  for (const file of ["package.json", "src-tauri/tauri.conf.json"]) {
    const json = JSON.parse(fs.readFileSync(file, "utf8"));
    json.version = next;
    fs.writeFileSync(file, JSON.stringify(json, null, 2) + "\n");
  }
' "$next"

# Only the package version at the top of the manifest, never a dependency's.
perl -0pi -e 's/^(\[package\](?:.|\n)*?\nversion = ")[^"]+(")/${1}'"$next"'${2}/m' src-tauri/Cargo.toml
# Cargo.lock repeats it; patching the entry keeps this script offline and quick.
perl -0pi -e 's/(\[\[package\]\]\nname = "bangs"\nversion = ")[^"]+(")/${1}'"$next"'${2}/' src-tauri/Cargo.lock

echo "版本已设为 v$(current)"
echo "改动的文件：package.json  src-tauri/tauri.conf.json  src-tauri/Cargo.toml  src-tauri/Cargo.lock"
