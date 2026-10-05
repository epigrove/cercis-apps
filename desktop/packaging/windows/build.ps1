# 在 Windows 上构建 Cercis（内核 + app）。
#
# 用法（PowerShell）：
#   .\build.ps1                 # 构建 release，装到 %LOCALAPPDATA%\Cercis
#   .\build.ps1 -Prefix C:\Apps\Cercis
#
# 产物：<Prefix>\cercis.exe、<Prefix>\cercis-app.exe
param(
  [string]$Prefix = "$env:LOCALAPPDATA\Cercis"
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Resolve-Path (Join-Path $here "..\..\..\..")   # Cercis\ 仓库根

Write-Host "== 构建内核 =="
Push-Location (Join-Path $repo "kernel"); cargo build --release; Pop-Location
Write-Host "== 构建 app =="
Push-Location (Join-Path $repo "apps\desktop"); cargo build --release; Pop-Location

New-Item -ItemType Directory -Force -Path $Prefix | Out-Null
Copy-Item (Join-Path $repo "kernel\target\release\cercis.exe") $Prefix -Force
Copy-Item (Join-Path $repo "apps\desktop\target\release\cercis-app.exe") $Prefix -Force

Write-Host ""
Write-Host "装好了：$Prefix"
Write-Host "  命令行：$Prefix\cercis.exe --help"
Write-Host "  桌面界面：$Prefix\cercis-app.exe"
Write-Host "  平板/手机：$Prefix\cercis-app.exe --lan   # 打印带令牌的局域网地址"
Write-Host ""
Write-Host "提醒：Windows 上系统钥匙串暂不支持 → 远程凭据用 --password-file 或每次输入（不会静默降级）。"
