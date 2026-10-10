#!/usr/bin/env bash
# 把基础工具包**摆成一个"已下载好的市场仓库"**：marketplace.json + plugins/<id>/（清单 + 编好的二进制）。
#
# 这个形状同时给两处用：
#   1. 安装包（把整个目录塞进 app 里，第一次打开由 app 装进用户目录 —— 见 src/bundled.rs）；
#   2. 源码安装（接着跑 install-basic-kits.sh，用内核的 `market install --from` 装台账）。
#
# 用法：bash stage-basic-kits.sh <目标目录>
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"   # packaging → desktop → apps → 仓库根
kits="$repo/kits/basic"
dest="${1:?用法: bash stage-basic-kits.sh <目标目录>}"

echo "== 构建基础工具包（release）=="
if [ "${SKIP_BUILD:-0}" = "1" ]; then
  echo "   跳过构建（SKIP_BUILD=1）"
else
  (cd "$kits" && cargo build --release)
fi

echo "== 摆到 $dest =="
rm -rf "$dest"
mkdir -p "$dest"
cp "$kits/marketplace.json" "$dest/"
for dir in "$kits"/plugins/*/; do
  id="$(basename "$dir")"
  entry="$(awk '/^entrypoint:/{print $2; exit}' "$dir/plugin.yaml")"
  if [ -z "$entry" ]; then
    echo "跳过 ${id}：plugin.yaml 里没有 entrypoint" >&2
    continue
  fi
  bin="$kits/target/release/$entry"
  if [ ! -x "$bin" ]; then
    echo "跳过 ${id}：没找到可执行文件 ${entry}" >&2
    continue
  fi
  mkdir -p "$dest/plugins/$id"
  cp "$dir/plugin.yaml" "$dest/plugins/$id/"
  [ -f "$dir/README.md" ] && cp "$dir/README.md" "$dest/plugins/$id/"
  cp "$bin" "$dest/plugins/$id/$entry"
  echo "  + ${id}（${entry}）"
done
