#!/usr/bin/env bash
# 把**基础工具包装进用户级插件目录**（需求 D35：app = 内核 + 基础工具包）。
#
# 为什么要有这一步：`cercis-app` 自己不含插件代码，插件是**独立可执行文件**，
# 它按"全局插件目录"去找。安装包不装这一步，用户装完打开一看——一个插件也没有。
#
# 装的过程走**内核自己的 `market install`**（写安装台账、记指纹、标 bundled），
# 所以"装了什么、哪一版、能不能删"有据可查，不是把文件拷过去了事。
#
# 用法：
#   bash install-basic-kits.sh                 # 装到 ~/.cercis/plugins
#   CERCIS_GLOBAL_PLUGINS=/tmp/x bash install-basic-kits.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"   # packaging → desktop → apps → 仓库根
kits="$repo/kits/basic"
cercis="${CERCIS_BIN:-$repo/kernel/target/release/cercis}"
plugins_dir="${CERCIS_GLOBAL_PLUGINS:-$HOME/.cercis/plugins}"

echo "== 构建内核与基础工具包（release）=="
if [ "${SKIP_BUILD:-0}" = "1" ]; then
  echo "   跳过构建（SKIP_BUILD=1）"
else
  (cd "$repo/kernel" && cargo build --release)
fi

[ -x "$cercis" ] || { echo "没找到内核命令行：$cercis" >&2; exit 1; }

# 暂存一份"已下载好的市场仓库"（形状见 stage-basic-kits.sh）：
# 内核的 market install --from 就吃这个，和 app 从 git 下载后交给内核的完全一样。
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
bash "$here/stage-basic-kits.sh" "$stage"

echo "== 装机（写成市场安装记录，来源记 basic）=="
echo "   插件目录：$plugins_dir"
ids="$(sed -n 's/^ *"id": "\([^"]*\)".*/\1/p' "$stage/marketplace.json")"
[ -n "$ids" ] || { echo "marketplace.json 里没读到插件 id" >&2; exit 1; }
for id in $ids; do
  "$cercis" market --global-dir "$plugins_dir" install \
      --from "$stage" --plugin "$id" --market basic --ref bundled >/dev/null
  echo "  ✅ $id"
done

echo
echo "基础工具包装好了（随包交付，不能删除；不想要就停用或屏蔽）。"
echo "看一眼：$cercis market --global-dir '$plugins_dir' list"
