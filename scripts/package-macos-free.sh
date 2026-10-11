#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(cd "$script_dir/.." && pwd)"
guide_path="$project_dir/distribution/macos/Open Xone - Read Me First.txt"
background_path="$project_dir/distribution/macos/dmg-background.png"
layout_script="$project_dir/distribution/macos/layout-dmg.applescript"
cargo_env="${CARGO_HOME:-$HOME/.cargo}/env"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "This package currently targets Apple Silicon macOS only." >&2
  exit 1
fi

if [[ -f "$cargo_env" ]]; then
  # shellcheck disable=SC1090
  source "$cargo_env"
fi

cd "$project_dir"
# Keep developer machine paths out of compiled diagnostics and native objects.
path_flags=("--remap-path-prefix=$HOME=/build/home" "--remap-path-prefix=$project_dir=/build/xone")
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  path_flags+=("--remap-path-prefix=$CARGO_TARGET_DIR=/build/target")
fi
for flag in "${path_flags[@]}"; do
  export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS:+${CARGO_ENCODED_RUSTFLAGS}$'\x1f'}$flag"
done
export CFLAGS="${CFLAGS:-} -ffile-prefix-map=$HOME=/build/home -ffile-prefix-map=$project_dir=/build/xone"
export CXXFLAGS="${CXXFLAGS:-} -ffile-prefix-map=$HOME=/build/home -ffile-prefix-map=$project_dir=/build/xone"
npm run tauri -- build --bundles app

version="$(node -p "require('./src-tauri/tauri.conf.json').version")"
app_path="$project_dir/src-tauri/target/release/bundle/macos/Xone.app"
dmg_dir="$project_dir/src-tauri/target/release/bundle/dmg"
dmg_path="$dmg_dir/Xone_${version}_aarch64.dmg"
work_dir="$(mktemp -d /private/tmp/xone-free-dmg.XXXXXX)"
staging_dir="$work_dir/staging"
mount_dir=""
rw_dmg_path="$work_dir/Xone-rw.dmg"
device_path=""

cleanup() {
  if [[ -n "$device_path" ]]; then
    hdiutil detach "$device_path" -force >/dev/null 2>&1 || true
  fi
  if [[ "$work_dir" == /private/tmp/xone-free-dmg.* && -d "$work_dir" ]]; then
    rm -rf -- "$work_dir"
  fi
}
trap cleanup EXIT

codesign --verify --deep --strict --verbose=2 "$app_path"

mkdir -p "$staging_dir/.background"
ditto "$app_path" "$staging_dir/Xone.app"
ln -s /Applications "$staging_dir/Applications"
cp "$guide_path" "$staging_dir/Read Me First.txt"
cp "$background_path" "$staging_dir/.background/dmg-background.png"

mkdir -p "$dmg_dir"
hdiutil create \
  -volname "Xone" \
  -srcfolder "$staging_dir" \
  -ov \
  -fs HFS+ \
  -format UDRW \
  "$rw_dmg_path"

attach_output="$(
  hdiutil attach "$rw_dmg_path" \
    -readwrite \
    -noverify \
    -noautoopen \
    -nobrowse
)"
device_path="$(printf '%s\n' "$attach_output" | awk -F '\t' '$2 ~ /Apple_HFS/ {gsub(/[[:space:]]+$/, "", $1); print $1; exit}')"
mount_dir="$(printf '%s\n' "$attach_output" | awk -F '\t' '$2 ~ /Apple_HFS/ {gsub(/^[[:space:]]+/, "", $3); print $3; exit}')"

if [[ -z "$device_path" || -z "$mount_dir" ]]; then
  echo "Could not mount the writable installer image." >&2
  exit 1
fi

chflags hidden "$mount_dir/.background"
sleep 1
/usr/bin/osascript "$layout_script" "$(basename "$mount_dir")"
sync
hdiutil detach "$device_path"
device_path=""

hdiutil convert "$rw_dmg_path" \
  -ov \
  -format UDZO \
  -imagekey zlib-level=9 \
  -o "$dmg_path"

hdiutil verify "$dmg_path"
echo "Created $dmg_path"
shasum -a 256 "$dmg_path"
