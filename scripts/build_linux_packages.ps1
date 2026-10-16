# ==============================================================================
# build_linux_packages.ps1 - 在 Windows 下一键调用 WSL 构建 Linux 发布包 (AppImage, deb, rpm)
# ==============================================================================
[CmdletBinding()]
param(
    [string]$Distro = "Ubuntu"
)

$ErrorActionPreference = "Stop"

$Workspace = (Get-Item -Path $PSScriptRoot\..).FullName
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " 开始构建 Linux 平台 Release 发布包 (AppImage / deb / rpm)" -ForegroundColor Cyan
Write-Host " 工作目录: $Workspace" -ForegroundColor Gray
Write-Host " 目标发行版: WSL $Distro" -ForegroundColor Gray
Write-Host "============================================================" -ForegroundColor Cyan

# 检查 WSL 是否可用
$wslCheck = wsl -l -q 2>$null
if (-not $wslCheck) {
    Write-Error "未检测到可用的 WSL 发行版，请确认已安装 WSL 并配置了 $Distro"
    exit 1
}

# 转换工作目录为 WSL 路径
$wslWorkspace = wsl -d $Distro wslpath -u ($Workspace -replace '\\', '/')

# 执行 Linux 构建脚本
Write-Host "`n>> 正在 WSL 中执行打包流程..." -ForegroundColor Green
$cmd = "chmod +x $wslWorkspace/scripts/build_linux_packages.sh && $wslWorkspace/scripts/build_linux_packages.sh"
wsl -d $Distro bash -lc "$cmd"

if ($LASTEXITCODE -ne 0) {
    Write-Error "Linux 发布包构建失败，退出码: $LASTEXITCODE"
    exit $LASTEXITCODE
}

Write-Host "`n>> 打包完成！输出文件如下：" -ForegroundColor Green
Get-ChildItem -Path "$Workspace\dist" -Filter "vpn-parser*" | Select-Object Name, Length, LastWriteTime | Format-Table -AutoSize
