# Agent V2 定向验证：统一 Task Runtime + Computer Use 可靠性契约。
#
# 只跑受影响范围的单元/契约测试，不跑完整 workspace。资源策略统一走
# scripts/ci-shared.ps1（先点源 resolve-ort.ps1 注入 ORT，防止原生依赖静默挂起）。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts\run-agent-v2-unit.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\run-agent-v2-unit.ps1 -MinFreeGb 4 -MaxUsedPercent 90
#   powershell -ExecutionPolicy Bypass -File scripts\run-agent-v2-unit.ps1 -NoFailFast
param(
    [double]$MinFreeGb = 0,
    [double]$MaxUsedPercent = 0,
    [switch]$NoFailFast
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$root = Split-Path $PSScriptRoot -Parent

. (Join-Path $PSScriptRoot "resolve-ort.ps1")
Resolve-OwoOrtEnv -Quiet | Out-Null
. (Join-Path $PSScriptRoot "ci-shared.ps1")

# 仅在显式传参时放宽资源阈值；默认沿用 ci-shared.ps1 的仓库策略。
if ($MinFreeGb -gt 0) { $env:OWO_CI_MIN_FREE_GB = "$MinFreeGb" }
if ($MaxUsedPercent -gt 0) { $env:OWO_CI_MAX_USED_PERCENT = "$MaxUsedPercent" }
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"

$steps = @(
    @{
        Name = "统一 Task Runtime 单元"
        Arguments = @("test", "-p", "owo-agent-core", "--lib", "task_runtime")
    },
    @{
        Name = "Computer Use 可靠性契约"
        Arguments = @("test", "-p", "owo-agent-core", "--test", "computer_use_tests")
    }
)

$failed = @()
foreach ($step in $steps) {
    $arguments = @($step.Arguments) + @("--locked", "-j", "1")
    if ($NoFailFast) { $arguments += "--no-fail-fast" }
    $arguments += @("--", "--test-threads=1")
    Write-Host ("== " + $step.Name + " ==") -ForegroundColor Cyan
    $exitCode = Invoke-CiCargo -Arguments $arguments -Cwd $root -PolicyMode normal -PassThru
    if ($exitCode -ne 0) {
        $failed += ("{0} (exit={1})" -f $step.Name, $exitCode)
    }
}

if ($failed.Count -gt 0) {
    Write-Host ("Agent V2 定向验证失败：" + ($failed -join "；")) -ForegroundColor Red
    exit 1
}
Write-Host "Agent V2 定向验证 PASS" -ForegroundColor Green
exit 0
