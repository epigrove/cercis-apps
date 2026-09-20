# cercis-apps（各平台 app）

Cercis 的 app 插件（各平台界面），同时是**插件总管**（管理全部插件）。

```
apps/
├── desktop/    桌面端（macOS 先行；Windows / Linux 跟进）
│   ├── gui/    图形界面
│   └── cli/    命令行入口（复用内核 cercis 命令）
└── mobile/     Pad / 手机（iOS · iPadOS · Android · HarmonyOS）
```

## 当前状态

- **S4 已完成**：桌面端本地网页界面（打开项目 / 数据浏览与筛选 / 插件信任 / 变更流水 / 校验 / 中英双语）。
- **S5 进行中**：app 侧补齐**插件宿主**——发现、授权、调用工具包插件；
  界面按插件清单渲染入口（新增记录、导入 CSV、导出、体检）。

```
apps/desktop/crates/cercis-app/src/
├── main.rs          本地服务与启动
├── http.rs          极简 HTTP（只监听本机）
├── api.rs           界面 ↔ 内核 / 插件的通道
├── plugin_host.rs   插件宿主：发现 → 授权 → 以子进程调用
└── web/index.html   内置界面（零安装、内置字符串，不读磁盘）
```

插件调用契约（与需求文档 §14.7 接口 A 对应）：

```text
<插件入口> <命令…> --project <项目目录> --json [--cercis <内核命令>] [--allow-write]
```

- **项目内插件必须先被信任**才运行；全局插件（`~/.cercis/plugins` 或 `CERCIS_PLUGINS`）视为已安装。
- `--allow-write` 只在插件清单声明了 `write` 时才注入：插件的权限由 app 把关。
- 插件是**子进程**：它崩溃或卡住都不影响主界面与数据完整性。
- 缺插件时对应按钮自动隐藏并说明缺什么，打开/浏览/校验等主流程不受影响。

环境变量（开发用）：

| 变量 | 作用 |
|------|------|
| `CERCIS_PLUGINS` | 全局插件目录（默认 `~/.cercis/plugins`） |
| `CERCIS_BIN` | 内核命令行路径（默认找 PATH 或 app 同级目录） |

## 形态分级（回顾）

| 形态 | 重点 |
|------|------|
| 桌面 GUI | 信息密度、批量编辑、导入导出 |
| 桌面 CLI | 自动化、AI 调用（与 GUI 同权） |
| Pad | 翻阅、手写、分屏、拍照 |
| 手机 | 速记、扫码、拍照、待办；离线优先 |
