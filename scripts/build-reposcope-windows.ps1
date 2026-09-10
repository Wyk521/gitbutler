[CmdletBinding()]
param(
    [string]$Version = "0.1.0",
    [string]$OutputDirectory = "dist/windows",
    [switch]$InstallDependencies
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

function Require-Command {
    param([Parameter(Mandatory = $true)][string]$Name)

    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "未找到 $Name。请在 Windows 终端安装并加入 PATH 后重试。"
    }
}

Require-Command "node"
Require-Command "pnpm"
Require-Command "cargo"

if ($InstallDependencies) {
    Write-Host "安装锁定的前端依赖..."
    pnpm install --frozen-lockfile
}

$nodeVersion = (& node --version).Trim()
if ($nodeVersion -notmatch '^v(\d+)') {
    throw "无法识别 Node.js 版本：$nodeVersion"
}
$nodeMajor = [int]$Matches[1]
if ($nodeMajor -lt 24) {
    throw "RepoScope Desktop 需要 Node.js 24 或更高版本，当前为 $nodeVersion。"
}

if ([string]::IsNullOrWhiteSpace($Version)) {
    throw "Version 不能为空。"
}

$releaseConfig = Join-Path $repoRoot "crates/gitbutler-tauri/tauri.conf.release.json"
if (-not (Test-Path -LiteralPath $releaseConfig -PathType Leaf)) {
    throw "找不到 Tauri 发布配置：$releaseConfig"
}

Write-Host "生成 RepoScope Desktop SDK 类型..."
pnpm build:sdk

Write-Host "构建 Windows x64 安装包（RepoScope Desktop $Version）..."
$env:VERSION = $Version
$env:CHANNEL = "release"
pnpm exec tauri build `
    --config $releaseConfig `
    --features offline

$bundleDirectories = @(
    (Join-Path $repoRoot "target/release/bundle/msi"),
    (Join-Path $repoRoot "target/tauri/release/bundle/msi")
)
$installer = $null
foreach ($directory in $bundleDirectories) {
    if (Test-Path -LiteralPath $directory) {
        $installer = Get-ChildItem -LiteralPath $directory -Filter "*.msi" -File |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1
        if ($installer) {
            break
        }
    }
}

if (-not $installer) {
    throw "Tauri 构建完成但没有找到 MSI 安装包。请检查 target/*/release/bundle/msi。"
}

$destination = Join-Path $repoRoot $OutputDirectory
New-Item -ItemType Directory -Force -Path $destination | Out-Null
$destinationFile = Join-Path $destination $installer.Name
Copy-Item -LiteralPath $installer.FullName -Destination $destinationFile -Force

Write-Host "Windows 安装包已生成：$destinationFile"
Write-Host "这是未签名内部版本；如需正式分发，请在 Windows 证书环境中签名。"
