# 构建 NSIS 安装程序（含核心服务 sidecar）。
# §6.1.5/§6.2：统一构建门禁——拒绝 dirty 工作树；ORT 原生依赖经 init-dev-env
# 单一实现解析（不再依赖旧终端遗留的 ORT_LIB_PATH）；cargo 构建输出中出现
# LNK4098（CRT 静态/动态混用）按发布失败处理，不允许警告带入安装包。
# 用法：powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1 [-Configuration release|debug] [-OwobotDemo]
param(
    [ValidateSet("release", "debug")]
    [string]$Configuration = "release",
    [switch]$OwobotDemo
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$root = Split-Path $PSScriptRoot -Parent
$electronDir = Join-Path $root "desktop\electron"
$cargo = if ($env:OWO_CARGO) { $env:OWO_CARGO } else { Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe" }
$builder = Join-Path $electronDir 'node_modules\.bin\electron-builder.cmd'
if ($OwobotDemo -and -not (Test-Path -LiteralPath $builder -PathType Leaf)) {
    throw 'Run npm ci in desktop/electron before building the OwOBot demo.'
}

# §6.1.5/§6.2：门禁（clean tree + ORT 解析）；dirty 覆盖开关：OWO_ALLOW_DIRTY_RELEASE=1。
# §7.2：ORT 经统一解析入口 resolve-ort.ps1（内部委托 init-dev-env 单一实现，
# 进程级注入，缺失即报可操作错误，不留到链接阶段）。
. (Join-Path $PSScriptRoot "resolve-ort.ps1")
Assert-OwoCleanTree
try {
    $ortMeta = Resolve-OwoOrtEnv -NoDownload -Quiet
    Write-Host ("[installer] ORT resolved: {0} (source={1}, v{2}, {3})" -f $ortMeta.lib_dir, $ortMeta.source, $ortMeta.version, $ortMeta.crt)
} catch {
    throw "ONNX Runtime not found for release linking. Run: pwsh -File scripts\resolve-ort.ps1 -EnsureOrt （$($_.Exception.Message)）"
}

$configArgs = @()
if ($Configuration -eq "release") { $configArgs = @("--release") }

# §2.4 资源安全红线：安装包链路属"release/原生依赖重链"，一律 strict 档（-j 1），
# 并在启动前过内存门与构建空闲门；输出流式落盘（红线 5 不隐藏长任务进度），
# 完整日志仍交给下面的 linker 零容忍门扫描。
. (Join-Path $PSScriptRoot "ci-shared.ps1")
Initialize-CiPath
$buildLogPath = Join-Path ([IO.Path]::GetTempPath()) ("owo-installer-build-{0}.log" -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
Write-Host "[installer] 构建核心服务（$Configuration，§2.4 strict 档 -j 1）..."
$buildLog = Invoke-CiCargoCapture -Arguments (@('build', '--locked', '-p', 'owo-agent-cli') + $configArgs) `
    -Cwd $root -TimeoutSec 7200 -HeartbeatSec 30 -Label 'installer-build' `
    -PolicyMode 'strict' -LogFile $buildLogPath -CargoExe $cargo
if ($global:LASTEXITCODE -ne 0) {
    throw "核心服务构建失败（cargo exit=$global:LASTEXITCODE；§2.4 日志：$buildLogPath）"
}
# §7.3 零容忍升级：任意 LNK 码（含 LNK4098 CRT 混用、LNK1120 等）或
# linker 警告行都按发布失败处理，不允许带入安装包。
$linkerNoise = @($buildLog | Where-Object { $_ -match 'LNK\d{4}|warning: linker' } |
    Where-Object { $_ -notmatch '正在创建库|Creating library' } | Select-Object -First 5)
if ($linkerNoise.Count -gt 0) {
    throw ("发布构建检测到 {0} 条 linker 告警/错误——按失败处理（§7.3）：{1}" -f $linkerNoise.Count, ($linkerNoise -join ' | '))
}

# R3（§7.3）：随包 core 预置统一走 stage-desktop-sidecar.ps1（**唯一实现**，与
# ci-gate 的 desktop-stage 步、真机验收脚本同源）：清残留 → 按 externalBin 正确
# 命名复制 → SHA-256 核对 → 构建身份核对。
# 历史缺陷：这里曾把新 core 复制成 `owo-agent.exe-<triple>.exe`，而 tauri-build
# 只认 `owo-agent-<triple>.exe`——于是安装包持续打包 binaries/ 里另一份来历不明
# 的旧 core（实测 8/31 残留），"哈希核对通过"核对的却是那个没人读的文件。
. (Join-Path $PSScriptRoot "stage-desktop-sidecar.ps1")
try {
    $staged = Stage-OwoDesktopSidecar -Configuration $Configuration -NoBuild
} catch {
    throw "sidecar 预置失败（§7.3）：$($_.Exception.Message)"
}
Write-Host "[installer] 随包 core 就位：$($staged.destination)"
Write-Host "[installer] sidecar 身份：$($staged.identity)"
Write-Host "[installer] sidecar 复制哈希核对通过：$($staged.sha256)"

# ADR-003：NSIS 打包由 Tauri bundler 改为 electron-builder（配置见
# desktop/electron/electron-builder.yml）。壳不再编译 Rust，因此原先那段
# "临时移开 .cargo/config.toml、清空 rustflags"的补丁随之作废——它是为绕开
# tauri-build 的 static_vcruntime 冲突（/NODEFAULTLIB:LIBCMT 屏蔽 libcmt 导致
# 成批 LNK2001），Electron 侧不存在该链路。
if ($OwobotDemo) {
    & (Join-Path $PSScriptRoot 'stage-owobot.ps1')
    $runtimeManifest = Join-Path $electronDir 'binaries\owobot\owobot-build.json'
    $runtime = Get-Content -LiteralPath $runtimeManifest -Raw | ConvertFrom-Json
    $runtimeDirty = ([bool]$runtime.sourceDirty).ToString().ToLowerInvariant()
    if ($runtime.sourceCommit -notmatch '^[a-f0-9]{40}$' -or
        $staged.identity -notmatch "commit=$($runtime.sourceCommit) dirty=$runtimeDirty(?:\s|$)") {
        throw 'The staged core and OwOBot runtime have different source identities.'
    }
}
$buildExitCode = 0
Push-Location $electronDir
try {
    $packageName = if ($OwobotDemo) { 'OwOBot Demo' } else { 'OwO Agent' }
    Write-Host "[installer] Building the $packageName NSIS installer..."
    # 随包 core 的校验不再在这里单独 stage：npm 的 predist 会自动跑
    # stage-desktop-sidecar.ps1（唯一实现），避免两处实现漂移。
    # ⚠ PowerShell 5.1 陷阱（2026-09-22 实测两次踩到）：顶部设了
    # `$ErrorActionPreference = "Stop"`，而 5.1 会把**外部程序的 stderr**包装成
    # 终止性错误——npm 每次都往 stderr 打 warning，于是命令还没跑起来就被判定
    # 失败，现象是"打包失败"而 bundler 一次都没执行。处置：临时改 Continue，
    # 之后**只认 $LASTEXITCODE**——退出码才是打包成败的唯一权威判据。
    $savedErrorAction = $ErrorActionPreference
    try {
        $ErrorActionPreference = "Continue"
        if ($OwobotDemo) {
            $signingDiscovery = $env:CSC_IDENTITY_AUTO_DISCOVERY
            try {
                $env:CSC_IDENTITY_AUTO_DISCOVERY = 'false'
                $builderArgs = @('--win', '--x64', '--publish', 'never', '--config', 'electron-builder.owobot-demo.yml')
                if ($Configuration -eq 'debug') {
                    $builderArgs += '--config.directories.output=../../dist/owobot-demo-debug'
                }
                & $builder @builderArgs
                $buildExitCode = $LASTEXITCODE
            } finally {
                $env:CSC_IDENTITY_AUTO_DISCOVERY = $signingDiscovery
            }
        } else {
            & npm.cmd run dist
            $buildExitCode = $LASTEXITCODE
        }
    } finally {
        $ErrorActionPreference = $savedErrorAction
    }
} finally {
    Pop-Location
}
if ($buildExitCode -ne 0) {
    throw "NSIS 打包失败（electron-builder exit=$buildExitCode）"
}

$output = if (-not $OwobotDemo) { 'dist\electron' }
    elseif ($Configuration -eq 'debug') { 'dist\owobot-demo-debug' }
    else { 'dist\owobot-demo' }
Write-Host "[installer] Installer built in $output"
