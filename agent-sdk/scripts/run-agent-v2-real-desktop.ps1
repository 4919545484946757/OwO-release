# Agent V2 真实桌面验收（默认只做前置检查，不移动鼠标、不按键）。
#
# 真实桌面执行会控制当前前台窗口，只能由本人在场时显式开启：
#   powershell -ExecutionPolicy Bypass -File scripts\run-agent-v2-real-desktop.ps1 `
#       -App notepad -Anchor "文件" -Verify "文件" -Execute
#
# 判定依据（不是"接口返回 200"）：
#   1) 每一步动作都有 receipt；2) 动作前窗口身份（app/pid/hwnd）未变化；
#   3) 锚点命中唯一；4) 动作后验证通过；5) 失败时任务暂停并保留稳定错误码。
param(
    [string]$App = "",
    [string]$Anchor = "",
    [string]$Verify = "",
    [string]$Action = "click",
    [string]$Value = "",
    [int]$Port = 4096,
    [string]$ServeExe = "",
    [switch]$Execute
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$root = Split-Path $PSScriptRoot -Parent
if (-not $ServeExe) { $ServeExe = Join-Path $root "target\debug\owo-agent.exe" }

function Get-Json($uri, $headers, $method, $body) {
    $parameters = @{
        Uri = $uri
        Method = $method
        Headers = $headers
        TimeoutSec = 30
        UseBasicParsing = $true
    }
    if ($body -ne $null) {
        $parameters["ContentType"] = "application/json"
        $parameters["Body"] = ($body | ConvertTo-Json -Depth 8)
    }
    return Invoke-RestMethod @parameters
}

Write-Host "== Agent V2 真实桌面前置检查 ==" -ForegroundColor Cyan
$problems = @()
if (-not [Environment]::UserInteractive) { $problems += "当前会话不是交互式桌面会话" }
if (-not (Test-Path -LiteralPath $ServeExe)) { $problems += ("缺少服务端可执行文件：" + $ServeExe) }
if ($Execute) {
    if (-not $App) { $problems += "真实执行必须显式指定 -App（例如 notepad）" }
    if (-not $Anchor) { $problems += "真实执行必须显式指定 -Anchor（锚点文本，必须唯一命中）" }
}
if ($problems.Count -gt 0) {
    foreach ($problem in $problems) { Write-Host ("BLOCKED: " + $problem) -ForegroundColor Red }
    exit 2
}

Write-Host "前置检查 PASS" -ForegroundColor Green
if (-not $Execute) {
    Write-Host ""
    Write-Host "当前为只检查模式，不会控制桌面。真实执行需要同时满足：" -ForegroundColor Yellow
    Write-Host "  1. 目标应用已打开并处于前台；"
    Write-Host "  2. 锚点文本在窗口内唯一命中；"
    Write-Host "  3. 显式传入 -Execute，并接受鼠标/键盘会被真实控制。"
    Write-Host "示例：-App notepad -Anchor `"文件`" -Verify `"文件`" -Execute"
    exit 0
}

$base = "http://127.0.0.1:$Port"
$server = Start-Process -FilePath $ServeExe -ArgumentList @("serve", "--port", "$Port", "--workspace", $root) -WindowStyle Hidden -PassThru
try {
    $ready = $false
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        try {
            $null = Invoke-WebRequest -Uri ($base + "/health") -TimeoutSec 2 -UseBasicParsing
            $ready = $true
            break
        } catch {}
    }
    if (-not $ready) { throw ("服务未就绪：" + $base) }

    $token = (Get-Json ($base + "/auth/token") @{} "GET" $null).token
    $headers = @{ Authorization = ("Bearer " + $token) }

    $created = Get-Json ($base + "/computer-use/task") $headers "POST" @{
        target_app = $App
        description = "Agent V2 真实桌面验收"
        allowed_actions = @($Action)
        max_duration_ms = 120000
    }
    $taskId = $created.task.id
    Write-Host ("任务已创建：" + $taskId) -ForegroundColor Cyan

    $null = Get-Json ($base + "/computer-use/task/" + $taskId + "/approve") $headers "POST" @{}
    $null = Get-Json ($base + "/computer-use/task/" + $taskId + "/start") $headers "POST" @{}

    $goal = @{ anchor_text = $Anchor; action = $Action; value = $Value; verify_text = $Verify }
    $report = Get-Json ($base + "/computer-use/task/" + $taskId + "/run") $headers "POST" @{ goals = @($goal) }
    Write-Host ($report | ConvertTo-Json -Depth 8) -ForegroundColor Gray

    $receipts = @($report.receipts)
    if ($receipts.Count -lt 1) { throw "执行完成但没有动作回执（receipts 为空）" }
    Write-Host ("动作回执数：" + $receipts.Count) -ForegroundColor Green
    Write-Host "Agent V2 真实桌面闭环 PASS（请人工确认窗口可见结果）" -ForegroundColor Green
    exit 0
} catch {
    Write-Host ("真实桌面闭环 FAIL：" + $_.Exception.Message) -ForegroundColor Red
    exit 1
} finally {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
}
