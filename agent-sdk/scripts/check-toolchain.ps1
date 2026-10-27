<#
.SYNOPSIS
检查 Rust 工具链二进制是否损坏（rustc 随机 0xC0000005 / 0xC000001D 的头号原因）。

.DESCRIPTION
在本机出现过 rustc 在编译任意 crate 时崩溃（STATUS_ACCESS_VIOLATION /
STATUS_ILLEGAL_INSTRUCTION），根因是工具链 DLL 落盘后被破坏（磁盘/内存不稳定）。
本脚本对关键二进制做 SHA256 基线比对：

  -WriteBaseline   记录当前（已知可用的）哈希到 %LOCALAPPDATA%\OwO\toolchain-baseline.json
  默认             逐个比对，任一不符即报错退出码 2
  -Reinstall       检测到不符时用 rsproxy 镜像重装工具链与组件

.EXAMPLE
  .\check-toolchain.ps1 -WriteBaseline
  .\check-toolchain.ps1
  .\check-toolchain.ps1 -Reinstall
#>
[CmdletBinding()]
param(
    [switch]$WriteBaseline,
    [switch]$Reinstall,
    [string]$Channel = '1.97.1'
)

$ErrorActionPreference = 'Stop'
$baselineDir = Join-Path $env:LOCALAPPDATA 'OwO'
$baselinePath = Join-Path $baselineDir 'toolchain-baseline.json'

function Get-ToolchainFiles {
    $sysroot = (& rustc --print sysroot).Trim()
    $files = @(
        (Join-Path $sysroot 'bin\rustc.exe'),
        (Join-Path $sysroot 'bin\cargo.exe'),
        (Join-Path $sysroot 'bin\rustfmt.exe')
    )
    $files += Get-ChildItem (Join-Path $sysroot 'bin') -Filter 'rustc_driver-*.dll' -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty FullName
    $files += Get-ChildItem (Join-Path $sysroot 'bin') -Filter 'rustc_codegen_*.dll' -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty FullName
    $files += Get-ChildItem (Join-Path $sysroot 'lib\rustlib\x86_64-pc-windows-msvc\lib') -Filter 'libstd-*.rlib' -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty FullName
    $files | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Sort-Object -Unique
}

function Get-Hashes([string[]]$paths) {
    $map = [ordered]@{}
    foreach ($p in $paths) {
        $map[$p] = (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash
    }
    $map
}

$paths = Get-ToolchainFiles
if (-not $paths) { throw '未找到工具链文件：请确认 rustc 可用（rustup toolchain list）。' }

$hashes = Get-Hashes -paths $paths

if ($WriteBaseline) {
    New-Item -ItemType Directory -Force -Path $baselineDir | Out-Null
    $payload = [ordered]@{
        channel    = $Channel
        recorded_at = (Get-Date).ToString('o')
        sysroot    = (& rustc --print sysroot).Trim()
        hashes     = $hashes
    }
    $payload | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $baselinePath -Encoding UTF8
    Write-Host ("已写入基线：{0}（{1} 个文件）" -f $baselinePath, $hashes.Count) -ForegroundColor Green
    exit 0
}

if (-not (Test-Path -LiteralPath $baselinePath)) {
    Write-Host '没有基线；先运行 .\check-toolchain.ps1 -WriteBaseline' -ForegroundColor Yellow
    exit 1
}

$baseline = Get-Content -LiteralPath $baselinePath -Raw | ConvertFrom-Json
$bad = @()
foreach ($p in $paths) {
    $expected = $baseline.hashes.$p
    if (-not $expected) { continue }
    if ($hashes[$p] -ne $expected) { $bad += $p }
}

if ($bad.Count -eq 0) {
    Write-Host ("工具链校验通过：{0} 个文件哈希一致（channel={1}）" -f $paths.Count, $baseline.channel) -ForegroundColor Green
    exit 0
}

Write-Host ("工具链文件被改动/损坏 {0} 个：" -f $bad.Count) -ForegroundColor Red
$bad | ForEach-Object { Write-Host ("  " + $_) -ForegroundColor Red }

if (-not $Reinstall) {
    Write-Host '可运行 .\check-toolchain.ps1 -Reinstall 通过 rsproxy 镜像重装。' -ForegroundColor Yellow
    exit 2
}

Write-Host '正在重装工具链（rsproxy 镜像）...' -ForegroundColor Yellow
$env:RUSTUP_DIST_SERVER = 'https://rsproxy.cn'
$env:RUSTUP_UPDATE_ROOT = 'https://rsproxy.cn/rustup'
rustup toolchain uninstall $Channel
rustup toolchain install $Channel --profile minimal
rustup component add rustfmt clippy rust-src llvm-tools-preview --toolchain $Channel
Write-Host '重装完成；重新记录基线前请先验证编译：cargo check -p owo-agent-core --lib' -ForegroundColor Green
exit 0
