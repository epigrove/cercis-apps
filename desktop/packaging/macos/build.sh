#!/usr/bin/env bash
# 在 macOS 上构建 Cercis，并打一个 .app 目录骨架（签名与公证留到发布准备，S15）。
#
# 用法：bash build.sh [输出目录]
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
out="${1:-$repo/build}"
app="$out/Cercis.app"

echo "== 构建 =="
(cd "$repo/kernel" && cargo build --release)
(cd "$repo/apps/desktop" && cargo build --release)

echo "== 组装 $app =="
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$repo/apps/desktop/target/release/cercis-app" "$app/Contents/MacOS/cercis-app"
cp "$repo/kernel/target/release/cercis" "$app/Contents/MacOS/cercis"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Cercis</string>
  <key>CFBundleDisplayName</key><string>Cercis</string>
  <key>CFBundleIdentifier</key><string>org.epigrove.cercis</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>cercis-app</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo
echo "做好了：$app"
echo "  打开：open '$app'"
echo "  提醒：对外分发要签名 + 公证（S15 发布准备）；本机自用直接 open 即可。"
