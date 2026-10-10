#!/usr/bin/env bash
# Cercis 界面自动验收（Playwright CLI 驱动真浏览器，**无人值守**：不需要人点）。
#
# 它自己干这些事：
#   1. 用内核造一个"有数据的演示项目"（几条记录 + 一条更正 + 三条待办 + 一个录入员会话）
#   2. 起 `cercis-app`，用无头浏览器打开
#   3. 走一遍真实操作：打开项目 → 看表/排序/筛选 → 待办勾选完成 → 历史看差异
#      → 搬运试运行打包 + 共存体检 → 录入员看草稿/原音 → 插件与加密区域页
#   4. 每步断言页面里真的出现该出现的东西，出问题就截图留证
#
# 用法：
#   bash run-ui-acceptance.sh                 # 用 kernel/app 的 debug 二进制
#   APP_BIN=... KERNEL_BIN=... bash run-ui-acceptance.sh
#   PORT=8955 bash run-ui-acceptance.sh
#
# 产物：<仓库>/output/playwright/*.png（截图）与运行日志；退出码 0 = 全部通过。
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"          # Cercis 仓库根（放内核与 app 二进制）
apps_repo="$(cd "$here/../.." && pwd)"        # apps 仓库（验收产物放这里）
KERNEL="${KERNEL_BIN:-$repo/kernel/target/debug/cercis}"
APP="${APP_BIN:-$repo/apps/desktop/target/debug/cercis-app}"
PORT="${PORT:-8949}"
OUT="${OUT_DIR:-$apps_repo/output/playwright}"
WORK="$(mktemp -d)"

export CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
PWCLI="$CODEX_HOME/skills/playwright/scripts/playwright_cli.sh"
# 把 npm 缓存与浏览器数据都放在可写目录，避免动用户的家目录
export npm_config_cache="$WORK/npm-cache"
export PLAYWRIGHT_CLI_SESSION="cercis-ui-$$"
mkdir -p "$OUT" "$npm_config_cache"

pass=0; fail=0; app_pid=""
ok()  { echo "  ✅ $1"; pass=$((pass+1)); }
bad() { echo "  ❌ $1"; fail=$((fail+1)); }

# 报告里**不写本机绝对路径**（仓库要公开，别把"谁在什么目录下跑的"带出去）
rel() { case "$1" in "$repo"/*) echo "${1#"$repo"/}";; "$apps_repo"/*) echo "${1#"$apps_repo"/}";; *) echo "$(basename "$1")";; esac; }

pw() { "$PWCLI" "$@" 2>&1; }
snap() { pw snapshot; }
# 断言：快照里出现某段文字
expect() {
  if snap | grep -qF -- "$1"; then ok "$2"; else bad "$2（页面上没找到「$1」）"; fi
}
expect_absent() {
  if snap | grep -qF -- "$1"; then bad "$2（不该出现的「$1」出现了）"; else ok "$2"; fi
}
# 按元素上的文字/占位符找 ref（eNN）
ref() { snap | grep -F -- "$1" | head -1 | grep -oE 'e[0-9]+' | head -1; }
shot() {  # 截图存到 $OUT/<名字>.png（Playwright CLI 自己决定临时路径，这里搬过来）
  local out path
  out="$(pw screenshot)"
  # 输出形如： - [Screenshot of viewport](.playwright-cli/page-…Z.png)
  path="$(echo "$out" | grep -oE '\(([^)]+\.png)\)' | head -1 | tr -d '()')"
  # CLI 可能给相对路径（相对当前目录），补成绝对路径
  case "$path" in
    /*) ;;
    "") ;;
    *) path="$PWD/$path" ;;
  esac
  if [ -n "$path" ] && [ -f "$path" ]; then
    cp "$path" "$OUT/$1.png" && echo "     📷 $OUT/$1.png"
  else
    echo "     （截图没拿到，原始输出：$(echo "$out" | head -2 | tr '\n' ' ')）"
  fi
}

cleanup() {
  [ -n "$app_pid" ] && kill "$app_pid" 2>/dev/null
  pw close >/dev/null 2>&1
  # Playwright CLI 的临时目录（截图已复制到 output/playwright，这里清干净，别污染仓库）
  rm -rf "$PWD/.playwright-cli"
  # 全绿时不留截图（二进制会污染"发布前扫描"）；失败时保留，或显式 KEEP_SCREENSHOTS=1
  if [ "$fail" -eq 0 ] && [ "${KEEP_SCREENSHOTS:-0}" != "1" ]; then
    rm -f "$OUT"/*.png
  fi
}
trap cleanup EXIT

echo "== Cercis 界面自动验收 =="
echo "内核：$KERNEL"
echo "app：$APP"
echo "工作目录：$WORK"
echo

for f in "$KERNEL" "$APP"; do
  [ -x "$f" ] || { echo "缺少可执行文件：${f}（先 cargo build）" >&2; exit 2; }
done
if [ ! -x "$PWCLI" ]; then
  echo "缺少 Playwright 包装脚本：$PWCLI" >&2
  exit 2
fi

echo "== 1/8 造演示项目 =="
P="$WORK/演示项目"
"$KERNEL" init "$P" --name "界面验收演示" >/dev/null
for i in 1 2 3 4 5; do
  "$KERNEL" append "$P" --table records --set "record_id=R-$i" --set "value_num=$((10+i))" \
    --set "notes=第 $i 条" --allow-new-columns >/dev/null
done
"$KERNEL" correct "$P" --table records --target R-1 --field value_num --old 11 --new 12.7 --reason 笔误 >/dev/null
today="$(date +%F)"
yesterday="$(date -v-1d +%F 2>/dev/null || date -d yesterday +%F)"
later="$(date -v+40d +%F 2>/dev/null || date -d "+40 days" +%F)"
"$KERNEL" append "$P" --table tasks --set task_id=TK-1 --set "title=昨天的活" --set "due_date=$yesterday" \
  --set status=待办 --set priority=高 --allow-new-columns >/dev/null
"$KERNEL" append "$P" --table tasks --set task_id=TK-2 --set "title=今天的活" --set "due_date=$today" \
  --set status=待办 --set priority=中 >/dev/null
"$KERNEL" append "$P" --table tasks --set task_id=TK-3 --set "title=下个月的活" --set "due_date=$later" \
  --set status=待办 --set priority=低 >/dev/null
S="$P/_labnote/scribe/20261005-101010-abcd"
mkdir -p "$S/原音" "$S/原图"
printf '%s\n' '{"schema":"cercis.scribe_session/1","session_id":"20261005-101010-abcd","label":"小区 3 号地","created_at":"2026-10-05T10:10:10Z","status":"draft","mode":"offline","table":"records","rows":1,"must_confirm":1,"low_confidence":1,"model_name":"演示模型"}' > "$S/session.json"
printf 'record_id,value_num,notes\nR-9,62.3,现场念的\n' > "$S/records.draft.csv"
printf '%s\n' '{"format":"cercis-scribe-confidence/1","threshold":0.7,"rows":[{"index":1,"cells":{"value_num":{"score":0.45,"source":"model","reason":"听不清 62.3 还是 60.3","value":"62.3"},"record_id":{"score":0.9,"source":"heuristic","reason":"普通文本","value":"R-9"}}}]}' > "$S/records.confidence.json"
printf '第 1 条 value_num：听不清\n' > "$S/待确认.md"
echo "12.5" > "$S/原音/录音.m4a"; printf 'PNGfake' > "$S/原图/手稿.png"
ok "演示项目就绪（5 条记录 + 1 条更正 + 3 条待办 + 1 个录入会话）"

echo "== 2/8 起 app =="
CERCIS_SHARE_HOME="$WORK/share" "$APP" --port "$PORT" --no-browser >"$WORK/app.log" 2>&1 &
app_pid=$!
up=""
for _ in $(seq 1 20); do
  if curl -sf -o /dev/null "http://127.0.0.1:$PORT/"; then up=1; break; fi
  sleep 0.5
done
[ -n "$up" ] && ok "app 起来了（端口 ${PORT}）" || { bad "app 没起来（见 $WORK/app.log）"; exit 1; }

echo "== 3/8 打开界面 =="
pw open "http://127.0.0.1:$PORT/" >/dev/null
pw resize 1440 900 >/dev/null
expect "Cercis" "首页能打开"
shot "01-首页"

echo "== 4/8 打开项目 → 看表/排序/筛选 =="
r="$(ref '项目文件夹路径')"
[ -n "$r" ] && pw fill "$r" "$P" >/dev/null
r="$(ref '打开项目')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
expect "records" "表出现在界面里"
expect "R-1" "能看到数据行"
shot "02-数据表"

# 点表头排序（表头带 aria-label「按这一列排序 value_num」，按它找才点得准）
r="$(snap | grep -F '按这一列排序 value_num' | head -1 | grep -oE 'e[0-9]+' | head -1)"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
expect "value_num:asc" "点表头能排序"

# 筛选
r="$(ref '筛选：列:运算符:值')"
[ -n "$r" ] && pw fill "$r" "value_num:gt:13" >/dev/null
pw press Enter >/dev/null
sleep 1
expect "R-5" "筛选后剩下符合的行"
expect_absent "R-1" "被筛掉的行不显示"
pw fill "$(ref '筛选：列:运算符:值')" "" >/dev/null
pw press Enter >/dev/null

echo "== 5/8 待办：分桶 + 勾选完成 =="
r="$(ref '待办')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
expect "逾期" "逾期分组在"
expect "今天的活" "今天的待办在"
shot "03-待办"
# 勾选"昨天的活"（动态列表没有稳定 ref，这里按 DOM 找那一行的复选框）
pw eval "(() => { const li = [...document.querySelectorAll('#todoBuckets li')].find(x => x.textContent.includes('昨天的活')); if (!li) return 'not-found'; li.querySelector('input[type=checkbox]').click(); return 'clicked'; })()" >/dev/null
sleep 1
expect "已完成" "勾选后进入已完成分组"

echo "== 6/8 历史：版本链 + 差异 =="
r="$(ref '变更流水')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
r="$(snap | grep -F '表名，如 records' | head -1 | grep -oE 'e[0-9]+' | head -1)"
[ -n "$r" ] && pw fill "$r" "records" >/dev/null
r="$(snap | grep -F '记录编号，如 R-1' | head -1 | grep -oE 'e[0-9]+' | head -1)"
[ -n "$r" ] && pw fill "$r" "R-1" >/dev/null
r="$(ref '看版本链')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
expect "R-1" "版本链能看"
r="$(ref '看差异')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
expect "12.7" "差异里给出生效值"
expect "笔误" "差异里给出更正理由"
shot "04-历史与差异"

echo "== 7/8 搬运：试运行打包 + 共存体检 + 录入员 =="
r="$(ref '搬运')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
r="$(snap | grep -F '输出目录' | head -1 | grep -oE 'e[0-9]+' | head -1)"
[ -n "$r" ] && pw fill "$r" "$WORK/包" >/dev/null
r="$(ref '试运行')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
expect "试运行" "打包试运行有结果"
r="$(ref '跑一次体检')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
expect "共存体检" "共存体检有结果"
shot "05-搬运"

r="$(ref '录入员')"
[ -n "$r" ] && pw click "$r" >/dev/null
sleep 1
expect "小区 3 号地" "录入会话列出来"
expect "62.3" "草稿里的值显示出来"
expect "录音.m4a" "原音挂上了（能重放）"
shot "06-录入员"

echo "== 8/8 其余页面能打开 + 控制台无错 =="
for tab in 插件 插件市场 加密区域 工具; do
  r="$(ref "$tab")"
  if [ -n "$r" ]; then pw click "$r" >/dev/null; sleep 0.5; expect "$tab" "「${tab}」页能打开"; fi
done
console_out="$(pw console error)"
# Playwright CLI 的格式是 "Total messages: N (Errors: M, Warnings: K)"，直接看 M
nerr="$(echo "$console_out" | grep -oE 'Errors: [0-9]+' | head -1 | grep -oE '[0-9]+')"
if [ "${nerr:-1}" = "0" ]; then ok "控制台没有报错（Errors: 0）"; else
  bad "控制台有报错（原样贴出，便于定位）"
  echo "$console_out" | head -12 | sed 's/^/      /'
fi

echo
echo "== 结果：通过 $pass ｜ 失败 $fail =="
echo "截图与日志：$OUT"
echo "（app 日志：$WORK/app.log）"

# 留一份报告（文本，可提交；截图是二进制、可复跑生成，不入库）
report="$OUT/UI-REPORT.md"
{
  echo "# 界面自动验收报告"
  echo
  echo "- 时间：$(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo "- 内核：\`$(rel "$KERNEL")\`"
  echo "- app：\`$(rel "$APP")\`"
  echo "- 结果：**通过 $pass ｜ 失败 $fail**"
  echo "- 截图：$(ls "$OUT"/*.png 2>/dev/null | wc -l | tr -d ' ') 张（全绿时自动清掉、失败时才保留；要留就 KEEP_SCREENSHOTS=1 跑）"
  echo
  echo "## 这一轮验了什么"
  echo
  echo "- 打开项目（界面里填路径 → 点「打开项目」）"
  echo "- 数据表：能看到行、点表头排序、筛选（含"被筛掉的行不显示"）"
  echo "- 待办：逾期/今天分桶、勾选完成后进「已完成」"
  echo "- 历史：版本链 + 记录差异（生效值 + 更正理由）"
  echo "- 搬运：试运行打包、共存体检"
  echo "- 录入员：会话列表、草稿值、原音挂载"
  echo "- 插件/插件市场/加密区域/工具页能打开；**浏览器控制台无报错**"
  echo
  echo "> 复跑：\`bash apps/desktop/ui-tests/run-ui-acceptance.sh\`"
  echo "> （自动造演示项目、起 app、开无头浏览器点一遍；不需要人工操作）"
} > "$report"
echo "报告：$report"
[ "$fail" -eq 0 ]
