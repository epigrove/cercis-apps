# 打包与多端（S14）

`Cercis app` 是**一个可执行文件 + 内置网页界面**：内核与插件以子进程方式调用，
界面是同一份 HTML（中英双语）。所以"多端"分三层看：

| 形态 | 怎么用 | 现在的状态 |
|------|--------|-----------|
| 桌面（macOS / Windows / Linux） | 直接跑 `cercis-app`，浏览器自动打开 `127.0.0.1:8765` | ✅ 三个平台的代码都编译通过（macOS 本机跑测试；Linux/Windows 交叉 `cargo check`） |
| 平板 / 手机（同一局域网） | `cercis-app --lan`，用平板浏览器打开启动时打印的地址（**带令牌**） | ✅ 已实现：随机令牌 + `?token=` + `X-Cercis-Token` 头，媒体（原音/原图）也带令牌 |
| 独立安装包（dmg/msi/deb…） | `packaging/<平台>/` 下的脚本 | 🚧 有脚本骨架；**签名与公证已决定不做**（要花钱），用户首次打开按下面"未签名放行"手动放行 |

## 安装包未签名：首次打开怎么放行（重要）

我们的安装包**没有代码签名、也没有公证**（个人/学术项目，证书与公证要花钱，已决定发布前不做）。
所以从网上下载后，系统会拦一下。这不是软件有问题，按下面放行即可；文案模板见 `docs/发布说明模板.md`。

| 系统 | 会看到什么 | 放行办法 |
|------|-----------|---------|
| macOS | "Apple 无法检查其是否包含恶意软件" | Finder 里**按住 Control 点图标 → 打开 → 再点"打开"**；或「系统设置 → 隐私与安全性」→ **"仍要打开"**；仍被拦则 `xattr -dr com.apple.quarantine /Applications/Cercis.app` |
| Windows | SmartScreen："Windows 已保护你的电脑" | **"更多信息" → "仍要运行"** |
| Linux | 解包后的二进制没有执行权限 | `chmod +x cercis-app cercis` |

> 打包脚本**不要**试图绕过系统检查（例如自动清 quarantine 属性）：那会掩盖"未签名"这个事实。
> 正确做法是把上面这张表放进发布说明，让用户自己确认。

## 目录

```
packaging/
├── linux/     build.sh + cercis.desktop + README（NAS 完整界面按文档放 Linux 客户端）
├── windows/   build.ps1 + README
└── macos/     build.sh + README（产出一个 .app 目录骨架，签名/公证另说）
```

## 局域网模式（田间/实验室最常用）

```bash
cercis-app --lan                 # 令牌随机生成，地址与令牌会打印出来
cercis-app --lan --token mytok   # 自己指定令牌（脚本/固定书签用）
```

- 启动时会打印形如 `http://192.168.3.218:8765/?token=abcd1234` 的地址——**平板直接开这个**。
- 没有令牌的请求一律 `401`；`cercis-app` 默认只绑 `127.0.0.1`，**不加 `--lan` 就不会暴露到网络**。
- 令牌也支持请求头 `X-Cercis-Token`（给脚本用），界面的每个请求（含原音/原图）都会自动带上。
- 只在**可信网络**里开：同网段的人虽然拿不到数据，但能扫到端口在监听。

## 跨平台检查（开发者）

```bash
# 交叉编译检查（不需要目标机器）
cd kernel      && cargo check --target x86_64-unknown-linux-gnu -p cercis-core -p cercis-cli
cd apps/desktop && cargo check --target x86_64-unknown-linux-gnu
cd kernel      && cargo check --target x86_64-pc-windows-msvc -p cercis-core -p cercis-cli
cd apps/desktop && cargo check --target x86_64-pc-windows-msvc
```

> 首次要装目标工具链：`rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc`。

## 平台差异（代码里已经处理）

- **打开浏览器**：macOS `open`、Linux `xdg-open`、Windows `cmd /C start`。
- **钥匙串**：macOS `security`、Linux `secret-tool`；Windows 暂不支持（用完即输或口令文件）。
- **路径**：一律 `std::path`，不假设分隔符；文件名用 UTF-8，NAS 上避免大小写只差一个字母的同名文件。
- **换行**：写 CSV 一律 LF、无 BOM（Excel 打开中文不乱码靠 UTF-8）。
- **插件入口**：Windows 自动加 `.exe` 后缀。
