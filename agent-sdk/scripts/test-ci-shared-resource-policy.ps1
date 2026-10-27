#requires -Version 5.1
<#
test-ci-shared-resource-policy.ps1 — §2.4 资源安全红线脚本层的**离线**负例/正例自测。

为什么需要：§2.4 的红线只有落到脚本层才算被执行；而"限制并发的代码自己写错了"
和历史上"验收脚本假通过"是同一类缺陷（指南 §7.3：每次假绿都要补一个负例）。
本自测不启动任何 cargo/rustc，纯参数归一与门逻辑，秒级完成，可在无桌面会话、
内存紧张时执行。

覆盖：
  正例：cargo test 注入 -j 与 --test-threads；clippy/test 的 -- 分隔保持；
        test 的 --test-threads 归一；非编译子命令不注入 -j；更小的显式值保留（只降不升）。
  负例：-j 4 拒绝；--test-threads 4 拒绝；策略 >2 拒绝；内存快照可读且带阈值；
        内存门在低阈值下确实拒绝；构建空闲门文案可操作。
退出码：0 = 全部通过；1 = 有失败（逐条打印）。
用法：.\scripts\test-ci-shared-resource-policy.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'ci-shared.ps1')

$results = @()
function Add-Result {
    param([string]$Name, [bool]$Ok, [string]$Detail = '')
    $script:results += [pscustomobject]@{ name = $Name; ok = $Ok; detail = $Detail }
    if ($Ok) { Write-Host ("[PASS] {0} — {1}" -f $Name, $Detail) -ForegroundColor Green }
    else { Write-Host ("[FAIL] {0} — {1}" -f $Name, $Detail) -ForegroundColor Red }
}
function Invoke-Guard {
    param([scriptblock]$Block)
    try { $null = & $Block; return '' } catch { return $_.Exception.Message }
}

# ---- 正例：参数归一 -------------------------------------------------------------
$norm = Protect-CiCargoArguments -Arguments @('test', '--workspace', '--locked') -Jobs 1 -TestThreads 1
Add-Result '正例：cargo test 注入 -j 1 与 --test-threads=1' `
    (($norm -join ' ') -eq 'test --workspace --locked -j 1 -- --test-threads=1') "got=$($norm -join ' ')"

$norm = Protect-CiCargoArguments -Arguments @('test', '-p', 'owo-agent-core', '--lib', 'change_set', '--', '--exact') -Jobs 2 -TestThreads 2
Add-Result '正例：已有 -- 分组时把 --test-threads 追加进测试面' `
    (($norm -join ' ') -eq 'test -p owo-agent-core --lib change_set -j 2 -- --exact --test-threads=2') "got=$($norm -join ' ')"

$norm = Protect-CiCargoArguments -Arguments @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings') -Jobs 1 -TestThreads 1
Add-Result '正例：clippy 的 -- -D warnings 原样保留且 -j 在 -- 之前' `
    (($norm -join ' ') -eq 'clippy --workspace --all-targets --locked -j 1 -- -D warnings') "got=$($norm -join ' ')"

$norm = Protect-CiCargoArguments -Arguments @('fmt', '--all', '--', '--check') -Jobs 2 -TestThreads 2
Add-Result '正例：cargo fmt 不注入 -j（实测 cargo fmt 不认该参数）' `
    (($norm -join ' ') -eq 'fmt --all -- --check') "got=$($norm -join ' ')"

$norm = Protect-CiCargoArguments -Arguments @('test', '--locked', '-j', '1', '--', '--test-threads=1') -Jobs 2 -TestThreads 2
Add-Result '正例：显式更小值保留（红线 8 只降不升）' `
    (($norm -join ' ') -eq 'test --locked -j 1 -- --test-threads=1') "got=$($norm -join ' ')"

$norm = Protect-CiCargoArguments -Arguments @('build', '--release') -Jobs 1 -TestThreads 1
Add-Result '正例：release 构建走 -j 1' `
    (($norm -join ' ') -eq 'build --release -j 1') "got=$($norm -join ' ')"

# ---- 负例：超过 2 的并发参数必须被拒绝 --------------------------------------------
$msg = Invoke-Guard { Protect-CiCargoArguments -Arguments @('test', '--workspace', '-j', '4') -Jobs 2 -TestThreads 2 }
Add-Result '负例：-j 4 必须被拒绝' ($msg -like '*上限 2*') "msg=$msg"

$msg = Invoke-Guard { Protect-CiCargoArguments -Arguments @('build', '--jobs=8') -Jobs 2 -TestThreads 2 }
Add-Result '负例：--jobs=8 必须被拒绝' ($msg -like '*上限 2*') "msg=$msg"

$msg = Invoke-Guard { Protect-CiCargoArguments -Arguments @('test', '--', '--test-threads', '4') -Jobs 2 -TestThreads 2 }
Add-Result '负例：--test-threads 4 必须被拒绝' ($msg -like '*上限 2*') "msg=$msg"

$msg = Invoke-Guard { Protect-CiCargoArguments -Arguments @('test') -Jobs 32 -TestThreads 2 }
Add-Result '负例：策略本身要求 32 路并发必须被拒绝' ($msg -like '*本机并发上限 2*') "msg=$msg"

# ---- 门：策略表与内存快照 ----------------------------------------------------------
$l = Get-CiRustResourceLimits -Mode 'normal'
Add-Result '策略：normal = 编译 2 / 测试线程 2' (($l.jobs -eq 2) -and ($l.test_threads -eq 2)) "jobs=$($l.jobs) threads=$($l.test_threads)"
$l = Get-CiRustResourceLimits -Mode 'strict'
Add-Result '策略：strict = 编译 1 / 测试线程 1' (($l.jobs -eq 1) -and ($l.test_threads -eq 1)) "jobs=$($l.jobs) threads=$($l.test_threads)"

$mem = Get-CiMemoryStatus
Add-Result '内存快照可读取（红线 7 的前提）' ($mem.status -in @('ok', 'limited')) "status=$($mem.status) free=$($mem.free_gb)GB used=$($mem.used_percent)%"
Add-Result '内存阈值与红线一致（6 GB / 80%）' `
    (($mem.gate.min_free_gb -eq 6.0) -and ($mem.gate.max_used_percent -eq 80.0)) "gate=$($mem.gate.min_free_gb)/$($mem.gate.max_used_percent)"
Add-Result '内存读取值不为零（拒绝把 unknown 当通过）' ($mem.total_gb -gt 0) "total=$($mem.total_gb)GB source=$($mem.source)"

$compiles = @(
    (Test-CiCargoCompiles -Arguments @('test', '--workspace')),
    (Test-CiCargoCompiles -Arguments @('clippy')),
    (Test-CiCargoCompiles -Arguments @('build'))
)
Add-Result '编译子命令判定：test/clippy/build 均需资源门' ((@($compiles | Where-Object { $_ -eq $true }).Count) -eq 3) "got=$($compiles -join ',')"
Add-Result '非编译子命令判定：fmt 不占用构建槽' (-not (Test-CiCargoCompiles -Arguments @('fmt', '--all'))) 'fmt=false'

# 红线 7 的适用面：守护保护"rustc/测试分片风暴"，不保护用户拉起的常驻进程。
# `cargo run`（dev serve / product-eval 批次）编译段一过就是业务进程，内存吃紧时
# 杀它等于杀用户程序——红线 8 要的正是"工程让出资源、不打断用户"。
Add-Result '守护范围：cargo test 启用运行中内存守护' (Test-CiCargoGuardRunning -Arguments @('test', '--workspace')) 'test=true'
Add-Result '守护范围：cargo build --release 启用运行中守护' (Test-CiCargoGuardRunning -Arguments @('build', '--release')) 'build=true'
Add-Result '守护范围：cargo run 不启用运行中守护（常驻进程）' (-not (Test-CiCargoGuardRunning -Arguments @('run', '-q', '-p', 'owo-agent-cli', '--', 'serve'))) 'run=false'
Add-Result '守护范围：cargo run 仍属编译子命令（启动前门照常）' (Test-CiCargoCompiles -Arguments @('run', '-p', 'owo-agent-cli', '--', 'serve')) 'run compiles=true'

# ---- 资源状态可落盘进 summary（§2.4.4 最后一条） ------------------------------------
$null = Set-CiRustResourcePolicy -Mode 'strict' -Source 'selftest'
$state = Get-CiResourceState
Add-Result 'summary 资源段含红线标识与真实限制' `
    (($state.redline -like '*2.4*') -and ($state.jobs -eq 1) -and ($state.test_threads -eq 1) -and ($state.mode -eq 'strict')) `
    "mode=$($state.mode) jobs=$($state.jobs) threads=$($state.test_threads) env_jobs=$($state.cargo_env.CARGO_BUILD_JOBS)"
Add-Result 'Set-CiRustResourcePolicy 注入子进程环境变量' `
    (($env:CARGO_BUILD_JOBS -eq '1') -and ($env:RUST_TEST_THREADS -eq '1')) "CARGO_BUILD_JOBS=$env:CARGO_BUILD_JOBS RUST_TEST_THREADS=$env:RUST_TEST_THREADS"

$tmp = Join-Path ([IO.Path]::GetTempPath()) ('owo-ci-res-selftest-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
Write-CiStepSummary -LogDir $tmp -CurrentStep 'selftest' -CurrentId 'selftest' -StepState 'passed'
$sum = Get-Content (Join-Path $tmp 'summary.json') -Raw | ConvertFrom-Json
Add-Result 'summary.json 落盘 resources 段（可证明安全参数）' `
    (($sum.resources.jobs -eq 1) -and ($sum.resources.test_threads -eq 1) -and ($sum.resources.memory_gate.min_free_gb -eq 6)) `
    "jobs=$($sum.resources.jobs) threads=$($sum.resources.test_threads) step=$($sum.step_state)"
Remove-Item -LiteralPath $tmp -Recurse -Force

# ---- §2.4 磁盘门（LNK1318 PDB LIMIT 事故驱动）：必须"启动前拒绝"而非"半途炸掉" ----
$disk = Get-CiDiskStatus
Add-Result '磁盘读取：能报出构建卷余量（读不到必须显式 unknown，不得猜充足）' `
    (($disk.status -eq 'unknown') -or (($disk.status -eq 'ok') -and ($disk.free_gb -gt 0) -and $disk.drive)) `
    "status=$($disk.status) drive=$($disk.drive) free=$($disk.free_gb)GB"
$allow = $null; $allowOk = $true
try { $allow = Assert-CiDiskGate -Mode strict -MinFreeGb 0 -Context 'selftest' } catch { $allowOk = $false }
Add-Result '磁盘门：阈值 0 时放行（不误伤正常构建）' ($allowOk -and $allow.ok) "ok=$($allow.ok)"
$rejectMsg = ''
try {
    $huge = [int]((Get-CiDiskStatus).free_gb + 500)
    $null = Assert-CiDiskGate -Mode strict -MinFreeGb $huge -Context 'selftest'
    $rejectMsg = '<未抛出：负例失效>'
} catch { $rejectMsg = $_.Exception.Message }
Add-Result '磁盘门：余量不足必须 throw resource_limited（禁止提高超时/降并发绕过）' `
    ($rejectMsg -match 'resource_limited' -and $rejectMsg -match '拒绝启动') "msg=$($rejectMsg.Substring(0,[Math]::Min(60,$rejectMsg.Length)))"
Add-Result '磁盘门异常文案自带可操作处置（指出可再生产物与恢复方式）' `
    ($rejectMsg -match 'incremental' -and $rejectMsg -match 'pdb') '需提到可删的可再生产物'

# ---- 磁盘门的"输出污染"：§2.4 门自己制造假红的实测事故 ----
# 门在成功时向管道吐一个磁盘状态对象。在"返回单个元数据"的函数里裸调它，调用方按
# 属性取值就会撞到状态对象的同名字段——stage-desktop-sidecar 的 `$staged.source`
# 取到 'psdrive'，Copy-Item 当场炸，4 个故障场景 + 冷启动全红，且红得像产品回归。
function Get-CleanStageLike {
    $null = Assert-CiDiskGate -Mode normal -MinFreeGb 0 -Context 'selftest'
    return [pscustomobject]@{ source = 'real' }
}
function Get-LeakyStageLike {
    Assert-CiDiskGate -Mode normal -MinFreeGb 0 -Context 'selftest'
    return [pscustomobject]@{ source = 'real' }
}
$leaky = @(Get-LeakyStageLike)
$clean = @(Get-CleanStageLike)
Add-Result '门确实向管道吐状态对象（污染来源定性，不是偶发现象）' ($leaky.Count -ge 2) "leaky_count=$($leaky.Count)"
Add-Result '单对象返回契约：函数内必须吞门输出（$null= / Out-Null），否则调用方取到错字段' `
    (($clean.Count -eq 1) -and (@(Get-CleanStageLike).source -eq 'real')) `
    "clean_count=$($clean.Count) source=$(if ($clean.Count -eq 1) { $clean[0].source } else { 'AMBIGUOUS' })"
$resKeys = if ($sum.resources) { ($sum.resources.PSObject.Properties.Name -join ',') } else { 'none' }
Add-Result 'summary.resources 必须带构建卷余量（盘满导致的失败要能被事后归因）' `
    ($null -ne $sum.resources.disk_at_write) "keys=$resKeys"

# ---- P0：分片墙钟硬超时（mk-tests-sharded 依赖的执行器路径） --------------------------
# mk-tests-sharded 把 -ShardTimeoutMin 落成 Invoke-CiCargo -TimeoutSec；这里直接验证
# 底层 Invoke-CiLoggedCommand 的超时确实：① 到时返回 124；② 杀掉整棵进程树。
# 用一个 60s 睡眠进程 + 3s 超时做离线负例，秒级完成，不需要 cargo。
$timeoutCode = 0
try {
    Invoke-CiLoggedCommand -Exe 'powershell' -Arguments @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') `
        -TimeoutSec 3 -HeartbeatSec 1 -Label 'timeout-selftest' | Out-Null
    $timeoutCode = $global:LASTEXITCODE
} catch { $timeoutCode = -1 }
Add-Result '超时门：墙钟超时返回 124（不无限等）' ($timeoutCode -eq 124) "exit=$timeoutCode"
$sleepLeft = @(Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -match 'Start-Sleep -Seconds 60' }).Count
Add-Result '超时门：进程树被 taskkill /T 终止（无残留睡眠进程）' ($sleepLeft -eq 0) "残留睡眠进程=$sleepLeft"

# Reproduce native stderr after successful termination, without Cargo or a
# resource threshold override. The mock still kills the real sleeping process.
$actualTaskkill = (Get-Command taskkill.exe -CommandType Application).Source
$priorFailures = @($script:ciFailures)
$originalMemoryStatus = (Get-Command Get-CiMemoryStatus).ScriptBlock
try {
    function taskkill {
        & $actualTaskkill @args | Out-Null
        & powershell.exe -NoProfile -EncodedCommand WwBDAG8AbgBzAG8AbABlAF0AOgA6AEUAcgByAG8AcgAuAFcAcgBpAHQAZQBMAGkAbgBlACgAIgBjAGgAaQBsAGQAIABhAGwAcgBlAGEAZAB5ACAAZQB4AGkAdABlAGQAIAAoAHMAZQBsAGYAdABlAHMAdAApACIAKQA7ACAAZQB4AGkAdAAgADEAMgA4AA==
    }
    $raceCode = -1
    try {
        Invoke-CiLoggedCommand -Exe 'powershell' -Arguments @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') `
            -TimeoutSec 1 -HeartbeatSec 1 -Label 'timeout-stderr-race' | Out-Null
        $raceCode = $global:LASTEXITCODE
    } catch { }
    Add-Result 'taskkill stderr race preserves timeout result under Stop preference' `
        (($raceCode -eq 124) -and ($ErrorActionPreference -eq 'Stop')) "exit=$raceCode"
    function Get-CiMemoryStatus {
        [pscustomobject]@{ status = 'limited'; free_gb = 0; used_percent = 100; gate = [pscustomobject]@{ min_free_gb = 6; max_used_percent = 80 } }
    }
    $raceCode = -1
    try {
        Invoke-CiLoggedCommand -Exe 'powershell' -Arguments @('-NoProfile', '-Command', 'Start-Sleep -Seconds 60') `
            -MemoryGuard -HeartbeatSec 1 -Label 'memory-stderr-race' | Out-Null
        $raceCode = $global:LASTEXITCODE
    } catch { }
    Add-Result 'taskkill stderr race preserves resource_limited result and reason' `
        (($raceCode -eq 137) -and $script:ciLastCommandResourceLimited) "exit=$raceCode"
    $diagnostics = @($script:ciFailures | Where-Object { $_.name -like '*-stderr-race-termination' })
    Add-Result 'nonzero taskkill remains visible as unverified termination' `
        (($diagnostics.Count -eq 2) -and ($diagnostics[0].detail -like '*taskkill exit=128 root_exited=True*') -and ($diagnostics[1].detail -like '*taskkill exit=128 root_exited=True*')) "diagnostics=$($diagnostics.Count)"
} finally {
    Remove-Item Function:taskkill -ErrorAction SilentlyContinue
    $null = Set-Item Function:Get-CiMemoryStatus -Value $originalMemoryStatus
    $script:ciFailures = $priorFailures
}
$sleepLeft = @(Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" -ErrorAction SilentlyContinue |
    Where-Object { $_.CommandLine -match 'Start-Sleep -Seconds 60' }).Count
Add-Result 'stderr race tests leave no sleeping process' ($sleepLeft -eq 0) "remaining=$sleepLeft"

# An idle wait can outlive the resource snapshot. Simulate a resource
# changing during that wait; no real Cargo or executable is launched.
$memoryProvider = (Get-Command Get-CiMemoryStatus).ScriptBlock
$diskProvider = (Get-Command Get-CiDiskStatus).ScriptBlock
$idleProvider = (Get-Command Assert-CiBuildIdle).ScriptBlock
try {
    function Get-CiMemoryStatus {
        $limited = $script:idleObserved -and ($script:changedResource -eq 'memory')
        [pscustomobject]@{
            status = $(if ($limited) { 'limited' } else { 'ok' })
            ok = (-not $limited)
            free_gb = $(if ($limited) { 1 } else { 20 })
            used_percent = $(if ($limited) { 97 } else { 37.5 })
            gate = [pscustomobject]@{ min_free_gb = 6; max_used_percent = 80 }
        }
    }
    function Get-CiDiskStatus {
        param($Path, $MinFreeGbStrict, $MinFreeGbNormal)
        [pscustomobject]@{
            status = 'ok'; drive = 'T'; source = 'offline-selftest'; total_gb = 200
            free_gb = $(if ($script:idleObserved -and ($script:changedResource -eq 'disk')) { 1 } else { 100 })
            min_free_gb = 0; ok = $true
        }
    }
    function Assert-CiBuildIdle {
        param($WaitSec, $PollSec, $Context)
        $script:idleObserved = $true
        return $true
    }
    foreach ($changed in @('memory', 'disk')) {
        $script:changedResource = $changed
        $script:idleObserved = $false
        $msg = Invoke-Guard {
            Invoke-CiCargo -Arguments @('check', '-j', '1') -PolicyMode strict `
                -CargoExe 'owo-ci-offline-missing-exe.exe' -Label ('idle-recheck-' + $changed) -PassThru
        }
        Add-Result ('idle wait rechecks current ' + $changed + ' before launching') `
            ($script:idleObserved -and ($msg -match 'resource_limited')) "msg=$msg"
    }
} finally {
    $null = Set-Item Function:Get-CiMemoryStatus -Value $memoryProvider
    $null = Set-Item Function:Get-CiDiskStatus -Value $diskProvider
    $null = Set-Item Function:Assert-CiBuildIdle -Value $idleProvider
}

$fail = @($results | Where-Object { -not $_.ok })
Write-Host ("[selftest] {0}/{1} 通过" -f (@($results).Count - $fail.Count), @($results).Count)
if ($fail.Count -gt 0) { exit 1 }
exit 0
