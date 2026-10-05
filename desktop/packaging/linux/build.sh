#!/usr/bin/env bash
# 在 Linux 上构建 Cercis（内核 + app），并装一份到 ~/.local（不需要 root）。
#
# 用法：
#   bash build.sh              # 构建 + 安装到 ~/.local
#   PREFIX=/usr/local bash build.sh   # 换安装前缀（可能要 sudo）
#   SKIP_BUILD=1 BIN_SRC=<目录> bash build.sh   # 跳过构建，用现成二进制（验收/打包机用）
#
# 产物：$PREFIX/bin/cercis、$PREFIX/bin/cercis-app、桌面菜单项 cercis.desktop
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../../.." && pwd)"     # Cercis/ 仓库根（apps/desktop/packaging/linux → 上四级）
PREFIX="${PREFIX:-$HOME/.local}"

if [ "${SKIP_BUILD:-0}" = "1" ]; then
  : "${BIN_SRC:?SKIP_BUILD=1 时要给 BIN_SRC=<放 cercis 与 cercis-app 的目录>}"
  kernel_bin="$BIN_SRC/cercis"
  app_bin="$BIN_SRC/cercis-app"
  echo "== 跳过构建（SKIP_BUILD=1）：用 $BIN_SRC 里的现成二进制 =="
else
  echo "== 构建内核 =="
  (cd "$repo/kernel" && cargo build --release)
  echo "== 构建 app =="
  (cd "$repo/apps/desktop" && cargo build --release)
  kernel_bin="$repo/kernel/target/release/cercis"
  app_bin="$repo/apps/desktop/target/release/cercis-app"
fi

for f in "$kernel_bin" "$app_bin"; do
  [ -x "$f" ] || { echo "找不到可执行文件：$f" >&2; exit 2; }
done

echo "== 安装到 $PREFIX =="
mkdir -p "$PREFIX/bin" "$PREFIX/share/applications"
install -m 755 "$kernel_bin" "$PREFIX/bin/cercis"
install -m 755 "$app_bin" "$PREFIX/bin/cercis-app"
sed "s|__EXEC__|$PREFIX/bin/cercis-app|" "$here/cercis.desktop" \
  > "$PREFIX/share/applications/cercis.desktop"

echo
echo "装好了："
echo "  命令行：$PREFIX/bin/cercis --help"
echo "  桌面界面：$PREFIX/bin/cercis-app        # 只绑本机"
echo "  平板/手机：$PREFIX/bin/cercis-app --lan   # 打印带令牌的局域网地址"
echo
echo "提醒：$PREFIX/bin 要加进 PATH（大多数发行版已默认包含 ~/.local/bin）。"
