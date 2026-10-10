# cercis-apps（各平台 app）

Cercis 的 app：各平台界面，同时是**插件总管**（发现、授权、以子进程调用插件）。

```
apps/
└── desktop/                桌面端（macOS 先行；Linux / Windows 跟进）
    ├── crates/cercis-app/  一个可执行文件：本地 HTTP 服务 + 内置网页界面
    │   ├── src/{main,http,api,plugin_host,market_host,icons,bundled}.rs
    │   └── src/web/index.html   界面（编译进二进制，不读磁盘）
    ├── packaging/          各平台打包脚本（linux / macos / windows）
    └── ui-tests/           界面自动验收（Playwright 无头浏览器，无人值守）
```

平板/手机现在不需要单独客户端：电脑上 `cercis-app --lan` 打印带令牌的局域网地址，
用平板浏览器打开、存到主屏幕即可（原生移动客户端待做，评估见 `cercis-docs`）。

## 怎么构建

app **不含数据逻辑**，它直接链接内核库（`cercis-core`）。因此源码构建要求内核仓库
就在**旁边一级**：工作布局是 `<任意目录>/{kernel, kits/basic, apps}`——
`apps/desktop/Cargo.toml` 里写的是 `path = "../../kernel/crates/cercis-core"`。

```bash
# 在 <任意目录>/apps/desktop 下
cargo run -- --no-browser --port 8765     # 浏览器打开 http://127.0.0.1:8765
```

打安装包（内置内核与基础工具包；**未签名**，首次打开要手动放行）：

```bash
bash packaging/macos/build.sh      # 出 Cercis.app 与可分发的 zip
bash packaging/linux/build.sh      # 装到 ~/.local（含桌面菜单项）
```

界面自动验收（自己造演示项目、起 app、用无头浏览器真点一遍，不需要人工）：

```bash
bash ui-tests/run-ui-acceptance.sh; cat output/playwright/UI-REPORT.md
```

插件调用契约（与需求文档 §14.7 接口 A 对应）：

```text
<插件入口> <命令…> --project <项目目录> --json [--cercis <内核命令>] [--allow-write]
```

- **项目内插件必须先被信任**才运行；全局插件（`~/.cercis/plugins`，可用
  `CERCIS_GLOBAL_PLUGINS` 换目录）视为已安装。
- `--allow-write` 只在插件清单声明了 `write` 时才注入：插件的权限由 app 把关。
- 插件是**子进程**：它崩溃或卡住都不影响主界面与数据完整性。
- 缺插件时对应按钮自动隐藏并说明缺什么，打开/浏览/校验等主流程不受影响。

环境变量（开发用）：

| 变量 | 作用 |
|------|------|
| `CERCIS_GLOBAL_PLUGINS` | 全局插件目录（默认 `~/.cercis/plugins`；旧名 `CERCIS_PLUGINS` 仍认） |
| `CERCIS_BIN` | 内核命令行路径（默认找 PATH 或 app 同级目录） |
| `CERCIS_BUNDLED_KITS` | 随包基础工具包的位置（安装包里自带；app 第一次启动会装进全局插件目录） |

## 形态分级（回顾）

| 形态 | 重点 |
|------|------|
| 桌面 GUI | 信息密度、批量编辑、导入导出 |
| 桌面 CLI | 自动化、AI 调用（与 GUI 同权） |
| Pad | 翻阅、手写、分屏、拍照 |
| 手机 | 速记、扫码、拍照、待办；离线优先 |
