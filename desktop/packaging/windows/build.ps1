# 在 Windows 上构建 Cercis（内核 + app + 基础工具包），装到本机或压成可分发的 zip。
#
# 用法（PowerShell，在仓库工作区里跑：<根>\{kernel, kits\basic, apps}）：
#   .\build.ps1                        # 构建 release，装到 %LOCALAPPDATA%\Cercis
#   .\build.ps1 -Prefix C:\Apps\Cercis # 换安装位置
#   .\build.ps1 -Zip                   # 另外压一个可分发的 zip（含基础工具包）
#
# 注意：这个脚本**没有在真机上验过**（本项目手上只有 macOS 与 Linux 机器）；
# 逻辑与 macos/build.sh、linux/build.sh 对齐，第一次在 Windows 上跑可能要微调路径。
#
# 未签名：Windows 会弹 SmartScreen「Windows 已保护你的电脑」——
# 让用户点「更多信息 → 仍要运行」，别去绕过（发布说明里写清即可）。
param(
  [string]$Prefix = "$env:LOCALAPPDATA\Cercis",
  [switch]$Zip
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$packaging = Resolve-Path (Join-Path $here "..")
$repo = Resolve-Path (Join-Path $here "..\..\..\..")   # Cercis\ 仓库根
$kits = Join-Path $repo "kits\basic"

Write-Host "== 构建内核与 app =="
Push-Location (Join-Path $repo "kernel"); cargo build --release; Pop-Location
Push-Location (Join-Path $repo "apps\desktop"); cargo build --release; Pop-Location

Write-Host "== 摆基础工具包（stage-basic-kits.sh 的 PowerShell 版）=="
$stage = Join-Path $Prefix "kits\basic"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item (Join-Path $kits "marketplace.json") $stage -Force
$entrypoints = @{}
Get-ChildItem (Join-Path $kits "plugins") -Directory | ForEach-Object {
  $id = $_.Name
  $yaml = Get-Content (Join-Path $_.FullName "plugin.yaml") -Raw
  $entry = ([regex]::Match($yaml, "(?m)^entrypoint:\s*(\S+)")).Groups[1].Value
  if (-not $entry) { Write-Warning "跳过 $id：plugin.yaml 里没有 entrypoint"; return }
  $bin = Join-Path $kits "target\release\$entry.exe"
  if (-not (Test-Path $bin)) { Write-Warning "跳过 $id：没找到 $entry.exe"; return }
  $dest = Join-Path $stage "plugins\$id"
  New-Item -ItemType Directory -Force -Path $dest | Out-Null
  Copy-Item (Join-Path $_.FullName "plugin.yaml") $dest -Force
  if (Test-Path (Join-Path $_.FullName "README.md")) {
    Copy-Item (Join-Path $_.FullName "README.md") $dest -Force
  }
  Copy-Item $bin (Join-Path $dest "$entry.exe") -Force
  Write-Host "  + $id（$entry.exe）"
}

New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
Copy-Item (Join-Path $repo "kernel\target\release\cercis.exe") $Prefix -Force
Copy-Item (Join-Path $repo "apps\desktop\target\release\cercis-app.exe") $Prefix -Force

Write-Host ""
Write-Host "装好了：$Prefix"
Write-Host "  命令行：$Prefix\cercis.exe --help"
Write-Host "  桌面界面：$Prefix\cercis-app.exe        # 第一次打开时它会把基础工具包装进 %USERPROFILE%\.cercis\plugins"
Write-Host "  平板/手机：$Prefix\cercis-app.exe --lan   # 打印带令牌的局域网地址"
Write-Host ""
Write-Host "提醒：Windows 上系统钥匙串暂不支持 → 远程凭据用 --password-file 或每次输入（不会静默降级）。"

if ($Zip) {
  $version = (Select-String -Path (Join-Path $repo "apps\desktop\Cargo.toml") -Pattern '^version = "(.+)"' |
              Select-Object -First 1).Matches.Groups[1].Value
  if (-not $version) { $version = "0.0.0" }
  $zip = Join-Path (Split-Path -Parent $Prefix) "Cercis-$version-Windows-x64.zip"
  if (Test-Path $zip) { Remove-Item $zip -Force }
  Compress-Archive -Path (Join-Path $Prefix "*") -DestinationPath $zip
  Write-Host ""
  Write-Host "分发包：$zip"
  Write-Host "SHA-256（写进发布说明）："
  (Get-FileHash $zip -Algorithm SHA256).Hash
}
