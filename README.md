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

S1 阶段：仅建立目录与说明。界面开发在内核完成后开始（S4，桌面先行）。

## 形态分级（回顾）

| 形态 | 重点 |
|------|------|
| 桌面 GUI | 信息密度、批量编辑、导入导出 |
| 桌面 CLI | 自动化、AI 调用（与 GUI 同权） |
| Pad | 翻阅、手写、分屏、拍照 |
| 手机 | 速记、扫码、拍照、待办；离线优先 |
