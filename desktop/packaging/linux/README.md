# Linux 客户端

按需求 FR-N10，**NAS 的完整图形入口放 Linux 客户端**（实验室常有一台 Linux 机器或 NAS 主机）。

## 装上

```bash
bash build.sh                 # 构建 release 并装到 ~/.local（不需要 root）
PREFIX=/usr/local bash build.sh   # 想装到系统目录就加前缀（可能要 sudo）
```

装好后：

- `cercis`：命令行（内核全部能力）
- `cercis-app`：图形界面（默认只绑本机 `127.0.0.1:8765`）
- `cercis-app --lan`：局域网模式，打印带令牌的地址，平板/手机浏览器直接开
- 桌面菜单里会出现 **Cercis**（`cercis.desktop`）

## Linux 上特有的注意点

- **钥匙串**：远程/NAS 凭据走 `secret-tool`（libsecret）。没装的话会提示"钥匙串不可用"，
  改用 `--password-file` 或每次输入，**不会静默降级**。
- **挂载 NAS**：用桌面环境的文件管理器挂载 SMB/WebDAV（或 `mount -t cifs` / `gio mount`），
  挂好之后把**挂载点**登记给内核：`cercis remote add <项目> --name nas --path /mnt/lab-nas`。
- **NAS 直连 vs 工作副本**：先 `cercis remote test --name nas` 做网络目录体检——
  不支持原子改名或写入很慢时，按提示改用工作副本（`cercis workcopy create …`）。
- **防火墙**：`--lan` 需要放行端口（`sudo ufw allow 8765/tcp`），否则平板连不上。
