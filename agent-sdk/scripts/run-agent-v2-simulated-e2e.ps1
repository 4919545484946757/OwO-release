# Agent V2 模拟闭环验证：进程内 axum + 模拟执行面，不触碰真实桌面。
#
# 覆盖：创建任务 → 列表 → 缺 payload 的后端明确失败、不伪装成功 → 事件日志 →
# 验证失败进入有界修复 → 取消；Computer Use 执行后端的审批门禁/闭环/熔断（内存
# Mock 桌面面）；WorkSwarm 执行后端的团队运行/暂停恢复（内置 echo/sleep Worker）；
# Workflow 执行后端的 .owflow 运行/暂停恢复（临时工作区、真实文件后端）；
# CloudRemote 执行后端的模拟传输提交/失败/暂停取消/恢复重提（不联网）。
# 真实桌面验收见 run-agent-v2-real-desktop.ps1。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts\run-agent-v2-simulated-e2e.ps1
param(
    [double]$MinFreeGb = 0,
    [double]$MaxUsedPercent = 0
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$root = Split-Path $PSScriptRoot -Parent

. (Join-Path $PSScriptRoot "resolve-ort.ps1")
Resolve-OwoOrtEnv -Quiet | Out-Null
. (Join-Path $PSScriptRoot "ci-shared.ps1")

if ($MinFreeGb -gt 0) { $env:OWO_CI_MIN_FREE_GB = "$MinFreeGb" }
if ($MaxUsedPercent -gt 0) { $env:OWO_CI_MAX_USED_PERCENT = "$MaxUsedPercent" }
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"

Write-Host "== 统一 Task Runtime HTTP 模拟闭环 ==" -ForegroundColor Cyan
Write-Host "说明：这是进程内 axum 模拟面，不执行真实桌面动作，也不调用真实模型。" -ForegroundColor Yellow

$arguments = @(
    "test", "-p", "owo-agent-server", "--test", "route_contract_tests",
    "agent_task_runtime_routes_create_list_and_never_fake_success",
    "--locked", "-j", "1", "--no-fail-fast", "--", "--test-threads=1"
)
$exitCode = Invoke-CiCargo -Arguments $arguments -Cwd $root -PolicyMode normal -PassThru
if ($exitCode -ne 0) {
    Write-Host ("模拟闭环失败（exit={0}）" -f $exitCode) -ForegroundColor Red
    exit $exitCode
}
Write-Host "== Computer Use 执行后端 HTTP 契约（内存 Mock 桌面面） ==" -ForegroundColor Cyan
$computerArguments = @(
    "test", "-p", "owo-agent-server", "--test", "computer_use_backend_tests",
    "--locked", "-j", "1", "--no-fail-fast", "--", "--test-threads=1"
)
$computerExit = Invoke-CiCargo -Arguments $computerArguments -Cwd $root -PolicyMode normal -PassThru
if ($computerExit -ne 0) {
    Write-Host ("Computer Use 后端契约失败（exit={0}）" -f $computerExit) -ForegroundColor Red
    exit $computerExit
}

Write-Host "== WorkSwarm 执行后端 HTTP 契约（内置 echo/sleep Worker，不调模型） ==" -ForegroundColor Cyan
$swarmArguments = @(
    "test", "-p", "owo-agent-server", "--test", "work_swarm_backend_tests",
    "--locked", "-j", "1", "--no-fail-fast", "--", "--test-threads=1"
)
$swarmExit = Invoke-CiCargo -Arguments $swarmArguments -Cwd $root -PolicyMode normal -PassThru
if ($swarmExit -ne 0) {
    Write-Host ("WorkSwarm 后端契约失败（exit={0}）" -f $swarmExit) -ForegroundColor Red
    exit $swarmExit
}

Write-Host "== Workflow 执行后端 HTTP 契约（临时工作区 .owflow，不调模型） ==" -ForegroundColor Cyan
$workflowArguments = @(
    "test", "-p", "owo-agent-server", "--test", "workflow_task_backend_tests",
    "--test", "workflow_api_tests",
    "--locked", "-j", "1", "--no-fail-fast", "--", "--test-threads=1"
)
$workflowExit = Invoke-CiCargo -Arguments $workflowArguments -Cwd $root -PolicyMode normal -PassThru
if ($workflowExit -ne 0) {
    Write-Host ("Workflow 后端契约失败（exit={0}）" -f $workflowExit) -ForegroundColor Red
    exit $workflowExit
}

Write-Host "== CloudRemote 执行后端 HTTP 契约（本地模拟传输，不联网） ==" -ForegroundColor Cyan
$cloudArguments = @(
    "test", "-p", "owo-agent-server", "--test", "cloud_remote_backend_tests",
    "--test", "cloud_task_api_tests",
    "--locked", "-j", "1", "--no-fail-fast", "--", "--test-threads=1"
)
$cloudExit = Invoke-CiCargo -Arguments $cloudArguments -Cwd $root -PolicyMode normal -PassThru
if ($cloudExit -ne 0) {
    Write-Host ("CloudRemote 后端契约失败（exit={0}）" -f $cloudExit) -ForegroundColor Red
    exit $cloudExit
}

Write-Host "Agent V2 模拟闭环 PASS" -ForegroundColor Green
exit 0
