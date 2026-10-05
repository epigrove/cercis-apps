# 桌面端（desktop）

同一份代码，三个桌面平台 + 平板/手机浏览器：

| 形态 | 怎么跑 | 状态 |
|------|--------|------|
| macOS / Windows / Linux | `cercis-app`（默认只绑 `127.0.0.1:8765`，自动开浏览器） | 三平台**交叉编译检查通过**（macOS 本机跑测试） |
| 平板 / 手机（同一局域网） | `cercis-app --lan`（打印带令牌的地址，平板浏览器直接开） | ✅ 已实现；媒体（原音/原图）也带令牌 |

打包脚本与平台差异见 [`packaging/README.md`](packaging/README.md)。

## 已实现

- **S4 界面**：打开项目、表数据浏览与筛选、插件信任、变更流水、校验、中英双语
- **S5 插件宿主**：发现插件（项目内 + 全局）、按清单授权、以子进程调用，
  并把插件声明的命令渲染成界面入口（新增记录 / 导入 CSV / 导出 / 体检）

## 运行

```bash
cd apps/desktop
cargo run                      # 默认 http://127.0.0.1:8765
cargo run -- --port 9000 --no-browser
cargo run -- --lan             # 局域网（平板/手机），打印带令牌的地址
```

> **局域网模式**：`--lan` 会随机生成令牌；不带令牌的请求一律 401。
> 只在可信网络里开——同网段的人虽然拿不到数据，但能扫到端口在监听。

要看到工具包的功能，需要让 app 找得到插件与内核：

```bash
# 内核（插件用它读写数据）
cd ../../kernel && cargo build

# 插件：放进全局插件目录，或放进项目的 plugins/
mkdir -p ~/.cercis/plugins
cp -r ../../kits/basic/plugins/* ~/.cercis/plugins/

# 插件入口（cercis-plugin-*）需要在 PATH 上，或放在插件目录内
export PATH="$PWD/../../kits/basic/target/debug:$PATH"
export CERCIS_BIN="$PWD/../../kernel/target/debug/cercis"
```

## 设计要点

- 界面是**内置字符串**：不读磁盘、零安装，本地浏览器打开即可用。
- 只监听 `127.0.0.1`；所有数据操作经内核（app 不直接改 CSV）。
- 插件按**子进程**调用，插件崩溃不影响主界面与数据。
- 缺插件时对应入口自动隐藏并说明缺什么，主流程（打开/浏览/校验）照常可用。
