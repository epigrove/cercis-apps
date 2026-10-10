#!/usr/bin/env bash
# 在 macOS 上构建 Cercis，打成 .app 并压成可分发的 zip。
#
# 签名与公证：**已决定不做**（要花钱）。所以这个包是"未签名"的，用户首次打开
# 要手动放行（右键打开 / 系统设置里"仍要打开"）——文案见 docs/发布说明模板.md。
#
# 用法：bash build.sh [输出目录]
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
packaging="$(cd "$here/.." && pwd)"
repo="$(cd "$here/../../../.." && pwd)"
out="${1:-$repo/build}"
app="$out/Cercis.app"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$repo/apps/desktop/Cargo.toml" | head -1)"
arch="$(uname -m)"

echo "== 构建 =="
(cd "$repo/kernel" && cargo build --release)
(cd "$repo/apps/desktop" && cargo build --release)

echo "== 组装 $app =="
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$repo/apps/desktop/target/release/cercis-app" "$app/Contents/MacOS/cercis-app"
cp "$repo/kernel/target/release/cercis" "$app/Contents/MacOS/cercis"

echo "== 图标（由 app 自己画出来，仓库里不放图片）=="
rm -rf "$out/icon-png" "$out/AppIcon.iconset"
"$app/Contents/MacOS/cercis-app" --write-icons "$out/icon-png" >/dev/null
mkdir -p "$out/AppIcon.iconset"
# iconset 的文件名是 Apple 规定的，改不得
cp "$out/icon-png/icon-16.png"   "$out/AppIcon.iconset/icon_16x16.png"
cp "$out/icon-png/icon-32.png"   "$out/AppIcon.iconset/icon_16x16@2x.png"
cp "$out/icon-png/icon-32.png"   "$out/AppIcon.iconset/icon_32x32.png"
cp "$out/icon-png/icon-64.png"   "$out/AppIcon.iconset/icon_32x32@2x.png"
cp "$out/icon-png/icon-128.png"  "$out/AppIcon.iconset/icon_128x128.png"
cp "$out/icon-png/icon-256.png"  "$out/AppIcon.iconset/icon_128x128@2x.png"
cp "$out/icon-png/icon-256.png"  "$out/AppIcon.iconset/icon_256x256.png"
cp "$out/icon-png/icon-512.png"  "$out/AppIcon.iconset/icon_256x256@2x.png"
cp "$out/icon-png/icon-512.png"  "$out/AppIcon.iconset/icon_512x512.png"
cp "$out/icon-png/icon-1024.png" "$out/AppIcon.iconset/icon_512x512@2x.png"
iconutil -c icns "$out/AppIcon.iconset" -o "$app/Contents/Resources/AppIcon.icns"

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
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# 版本号跟着 Cargo.toml 走，别让 plist 里写死的数字跟实际版本漂移
sed -i '' "s|<string>0.1.0</string>|<string>${version:-0.1.0}</string>|g" "$app/Contents/Info.plist"

echo "== 把基础工具包放进 app（app = 内核 + 基础工具包）=="
# 注意：这里只是**装进安装包里**，不动构建机上的用户目录；
# 用户第一次打开 app 时，由 app 自己把它们装进 ~/.cercis/plugins（见 src/bundled.rs）
bash "$packaging/stage-basic-kits.sh" "$app/Contents/Resources/kits/basic"

echo "== 打包（未签名）=="
zip="$out/Cercis-${version:-0.1.0}-macOS-${arch}.zip"
rm -f "$zip"
ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"

echo
echo "做好了："
echo "  $app"
echo "  $zip"
echo
echo "SHA-256（写进发布说明）："
shasum -a 256 "$zip"
echo
echo "打开：open '$app'"
echo "提醒：这个包**没有签名也没有公证**（已决定不做），别人首次打开要手动放行；"
echo "      发布说明里照 docs/发布说明模板.md 写清这一步。"
