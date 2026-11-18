# run-product-eval-v3-four-groups.ps1 - Phase 4 four-group comparison runner.
#
# Runs the same suite under the four required topologies:
#   A: Single Agent
#   B: WorkSwarm
#   C: Single Agent + Verification/Repair
#   D: WorkSwarm + Verification/Repair
#
# Each group uses the same task definitions, permissions, checkers and budget;
# only the topology and the --repair-attempts flag differ. Reports stay in
# separate --out directories so one group can never overwrite another.
#
# This script does NOT manufacture scores. It only orchestrates real runs.
# Without model credentials the preflight blocks (exit 2) and no report is
# produced. Failed runs are kept in each report; never delete them to recompute
# a success rate.
#
# Usage:
#   pwsh -File scripts\run-product-eval-v3-four-groups.ps1
#   pwsh -File scripts\run-product-eval-v3-four-groups.ps1 -Reps 3 -RepairAttempts 1
#   pwsh -File scripts\run-product-eval-v3-four-groups.ps1 -Suite evals\v3-product\suite.json
#
# Exit code: 0 = all four commands completed, 1 = at least one command failed,
# 2 = preflight blocked.

param(
    [string]$Suite = "evals\v3-product\suite.json",
    [string]$OutRoot = "scratch-eval-runs\v3-four-groups",
    [int]$Reps = 3,
    [string]$Model,
    [int]$RepairAttempts = 1,
    [double]$PriceInPerMTok = 0,
    [double]$PriceOutPerMTok = 0,
    [string]$DesktopDriver = "",
    [string]$FaultDriver = "",
    [string]$DriverConfig = "",
    # Optional category filter. Remaining formal cells use two independent
    # categories so their reports and journals never share an --out tree.
    [string]$Category = "",
    # Full-workspace/native-link batches use strict (-j 1 / one thread).
    [ValidateSet('normal', 'strict')][string]$PolicyMode = "normal",
    # Optional prebuilt CLI path. When set, skip cargo relaunch (useful while the
    # debug exe is held by another process); the binary must still be current.
    [string]$CliPath = "",
    # Wall-clock limit per group command (default 6h; 0 = no limit).
    [int]$CommandTimeoutSec = 21600
)

$ErrorActionPreference = "Continue"
. (Join-Path $PSScriptRoot "ci-shared.ps1")
Initialize-CiPath
$sdkRoot = Split-Path -Parent $PSScriptRoot
Set-Location $sdkRoot
# PS 5.1 Join-Path does not treat a rooted second path as absolute. Resolve
# the output root once so both preflight and reports use the same tree.
$resolvedOutRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutRoot)
Write-Host ("v3 four-group comparison - root: {0}  out: {1}" -f $sdkRoot, $resolvedOutRoot) -ForegroundColor Cyan

$initScript = Join-Path $PSScriptRoot "init-dev-env.ps1"
if (Test-Path $initScript) { . $initScript }

# Gateways may truncate long reports at the default per-request output budget
# (finish_reason=length). Raise the ceiling for evaluation runs unless the
# caller already set one; this is run configuration, not a scoring change.
if (-not $env:OWO_MODEL_MAX_OUTPUT_TOKENS) { $env:OWO_MODEL_MAX_OUTPUT_TOKENS = '64000' }
. (Join-Path $PSScriptRoot "resolve-ort.ps1")
try {
    $ortMeta = Resolve-OwoOrtEnv -NoDownload -Quiet
    Write-Host ("[ORT] resolved (process only): {0}" -f $ortMeta.lib_dir) -ForegroundColor Cyan
}
catch {
    Write-Host "[ORT] not resolved: $($_.Exception.Message)" -ForegroundColor Yellow
}

# Credential passthrough: read user-level registry env into this process only.
if (-not $env:OPENAI_API_KEY) {
    $userKey = [Environment]::GetEnvironmentVariable('OPENAI_API_KEY', 'User')
    if ($userKey) { $env:OPENAI_API_KEY = $userKey }
}
if ($PriceInPerMTok -gt 0) { $env:OWO_EVAL_PRICE_IN_PER_MTOK = "$PriceInPerMTok" }
if ($PriceOutPerMTok -gt 0) { $env:OWO_EVAL_PRICE_OUT_PER_MTOK = "$PriceOutPerMTok" }

$script:failedRuns = 0
$suiteArgs = @("--suite", $Suite)
Set-CiRustResourcePolicy -Mode $PolicyMode -Source 'four-groups-script' | Out-Null
$categoryArgs = @()
if ($Category) { $categoryArgs = @("--category", $Category) }
$modelArgs = @()
if ($Model) { $modelArgs = @("--model", $Model) }

function Invoke-EvalTool {
    param([string[]]$RunArgs, [string]$Label)
    if ($CliPath) {
        Invoke-CiLoggedCommand -Exe $CliPath -Arguments $RunArgs -Cwd $sdkRoot `
            -HeartbeatSec 60 -TimeoutSec $CommandTimeoutSec -Label $Label
    } else {
        Invoke-CiCargo -Arguments (@('run', '-q', '-p', 'owo-agent-cli', '--') + @($RunArgs)) -Cwd $sdkRoot `
            -HeartbeatSec 60 -TimeoutSec $CommandTimeoutSec -Label $Label -PolicyMode $PolicyMode
    }
}

function Invoke-EvalRun {
    param([string]$Label, [string[]]$RunArgs)
    Write-Host ""
    Write-Host ("== [four-groups] {0} ==" -f $Label) -ForegroundColor Cyan
    Invoke-EvalTool -RunArgs $RunArgs -Label ("v3-" + ($Label -replace '[^\w]', '_'))
    if ($global:LASTEXITCODE -ne 0) {
        Write-Host ("    run FAILED (exit={0}): {1}" -f $global:LASTEXITCODE, $Label) -ForegroundColor Red
        $script:failedRuns++
        return $false
    }
    return $true
}

# 0) Preflight: blocked means no live run and no reference substitution.
Write-Host "== [four-groups] preflight ==" -ForegroundColor Cyan
Invoke-EvalTool -RunArgs (@('product-eval', 'preflight') + @($suiteArgs) + @('--out', $resolvedOutRoot)) `
    -Label 'v3-four-groups-preflight'

if ($global:LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host ("PREFLIGHT BLOCKED (exit={0}): four-group comparison NOT executed." -f $global:LASTEXITCODE) -ForegroundColor Red
    Write-Host "Reference (dry) results are NOT a substitute for live runs." -ForegroundColor Red
    exit 2
}

$batchLabel = "v3-four-groups-" + (Get-Date -Format "yyyyMMdd-HHmmss")
$driverArgs = @()
if ($DesktopDriver) { $driverArgs += @("--desktop-driver", $DesktopDriver) }
if ($FaultDriver) { $driverArgs += @("--fault-driver", $FaultDriver) }
if ($DriverConfig) { $driverArgs += @("--driver-config", $DriverConfig) }
if ($driverArgs.Count -gt 0) {
    Write-Host "Registered external drivers are active; unregistered or failed cells remain Error." -ForegroundColor Cyan
} else {
    Write-Host "No external drivers registered: desktop and long-horizon cells remain fail-closed Error." -ForegroundColor Yellow
}
$groups = @(
    @{ Name = "A-single"; Description = "Single Agent"; Args = @("--exec", "agent", "--agents", "single") },
    @{ Name = "B-workswarm"; Description = "WorkSwarm"; Args = @("--exec", "workswarm", "--agents", "single", "--team-mode", "auto") },
    @{ Name = "C-single-repair"; Description = "Single Agent + Verification/Repair"; Args = @("--exec", "agent", "--agents", "single", "--repair-attempts", "$RepairAttempts") },
    @{ Name = "D-workswarm-repair"; Description = "WorkSwarm + Verification/Repair"; Args = @("--exec", "workswarm", "--agents", "single", "--team-mode", "auto", "--repair-attempts", "$RepairAttempts") }
)

foreach ($group in $groups) {
    $outDir = Join-Path $resolvedOutRoot $group.Name
    if ($Category) { $outDir = Join-Path $outDir $Category }
    $runArgs = @("product-eval", "run") + @($suiteArgs) + @($categoryArgs) + $group.Args + @(
        "--reps", "$Reps",
        "--out", $outDir,
        "--label", $batchLabel,
        "--tag", "phase4-four-groups",
        "--tag", ("group=" + $group.Name)
    ) + $driverArgs + $modelArgs
    Invoke-EvalRun ("{0} - {1}" -f $group.Name, $group.Description) $runArgs | Out-Null
}

# Summary: read only report.json produced by the real runs. Missing reports are
# shown as MISSING, never filled in with reference or estimated numbers.
Write-Host ""
Write-Host ("== [four-groups] summary (batch {0}) ==" -f $batchLabel) -ForegroundColor Cyan
$rows = @()
foreach ($group in $groups) {
    $groupOut = Join-Path $resolvedOutRoot $group.Name
    if ($Category) { $groupOut = Join-Path $groupOut $Category }
    $reportPath = Join-Path $groupOut "report.json"
    if (-not (Test-Path $reportPath)) {
        $rows += [pscustomobject]@{
            Group = $group.Name; Runs = "MISSING"; Passed = "-"; SuccessRate = "-"; Tokens = "-"; CostUsd = "-"
        }
        continue
    }
    $report = Get-Content -Raw -Encoding UTF8 $reportPath | ConvertFrom-Json
    $tokens = if ($null -eq $report.metrics.total_tokens) { "n/a" } else { "$($report.metrics.total_tokens)" }
    $cost = if ($null -eq $report.metrics.estimated_cost_usd) { "n/a" } else { "{0:N4}" -f $report.metrics.estimated_cost_usd }
    $rows += [pscustomobject]@{
        Group = $group.Name
        Runs = $report.metrics.runs_total
        Passed = $report.metrics.passed
        SuccessRate = ("{0:P1}" -f [double]$report.metrics.success_rate)
        Tokens = $tokens
        CostUsd = $cost
    }
}
$rows | Format-Table -AutoSize | Out-String -Width 160 | Write-Host

Write-Host "Notes:" -ForegroundColor Cyan
Write-Host "  - Each group keeps its own --out directory; compare only same-suite/same-model reports."
Write-Host "  - Failed, timeout and cancelled runs stay in the denominator."
Write-Host "  - Desktop (Computer Use) tasks must be judged on a real Windows desktop and reported separately."
Write-Host "  - Preflight, report and summary are evidence only; this script does not claim an acceptance score."

if ($script:failedRuns -gt 0) {
    Write-Host ("FAILED: {0} group command(s) exited non-zero." -f $script:failedRuns) -ForegroundColor Red
    exit 1
}
Write-Host "OK: all four group commands completed." -ForegroundColor Green
exit 0
