# ci-shared.ps1 — CI 脚本公共助手（供 ci-gate.ps1 / ci-nightly.ps1 / ci-weekly.ps1 点源引用）
# 本文件只提供函数与状态，不直接执行外部命令。
# 注意：保持 Windows PowerShell 5.1 兼容语法（仓库规则），同时兼容 PowerShell 7。
# 用法：. (Join-Path $PSScriptRoot "ci-shared.ps1")
#
# 约定：
#   - $script:ciFailures = List[PSCustomObject{name, detail}]；$script:ciSteps = List[string]
#   - $script:ciStepFilter：-Step 过滤词（子串匹配，空=全部执行）
#   - Invoke-CiStep 块内 `return` 表示"受控跳过"（绕过失败标记并在日志说明 SKIP 原因）

function Get-CiRepoRoot {
    # ci-shared.ps1 与 ci-*.ps1 均位于 agent-sdk/scripts/，根目录即上一级
    return Split-Path -Parent $PSScriptRoot
}

function Initialize-CiPath {
    # 保证 cargo 与 npm 可解析；找不到时回退到用户标准安装目录（不写死单机路径）
    $CARGO_HOME = $env:CARGO_HOME
    if (-not $CARGO_HOME) { $CARGO_HOME = Join-Path $env:USERPROFILE ".cargo" }
    $cargoBin = Join-Path $CARGO_HOME "bin"
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path (Join-Path $cargoBin "cargo.exe"))) {
        $env:PATH = "$cargoBin;" + $env:PATH
    }
    if (-not (Get-Command npm -ErrorAction SilentlyContinue) -and (Test-Path (Join-Path $env:ProgramFiles "nodejs\npm.cmd"))) {
        $env:PATH = "$env:ProgramFiles\nodejs;" + $env:PATH
    }
}

# ---------------------------------------------------------------------------
# P0（指南 §8/§9.3）：构建身份与产物身份读取的**唯一实现**。
#
# 为什么放在这里：假通过的根因是验收脚本"自己猜产物"。M15 的 mk-smoke 直接运行
# 现存 target\debug\owo-agent.exe，既没构建当前源码，也没比较 HEAD 与产物的
# build id —— 于是 M0 的旧二进制拿了 M15 的 18/18（证据链断裂）。身份门必须与
# 产物清单（release-artifact-manifest.ps1）同源、可被负例测试直接断言，因此抽成
# 公共函数，禁止各脚本各写一份解析。
#
# 口径（与 owo-build-info/build.rs、release-artifact-manifest.ps1 一致）：
#   * source commit/dirty 作用域 = agent-sdk/ 构建相关树（git status --porcelain -- .）；
#   * 产物身份 = `--version` 一行（编译期烧录，证明"二进制自身主张"）。
# ---------------------------------------------------------------------------

function Get-CiGitIdentity {
    <# 读取当前源码提交身份；git 不可用时 commit=unknown、dirty=true（不谎报干净）。 #>
    param([string]$RepoRoot = '')
    $root = if ($RepoRoot) { $RepoRoot } else { Get-CiRepoRoot }
    $commit = ''
    $dirty = $true
    Push-Location $root
    try {
        $out = & git rev-parse HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and $out) { $commit = ("$out").Trim() }
        $porcelain = & git status --porcelain -uall -- . 2>$null
        $dirty = [bool]($porcelain -and (($porcelain -join "`n").Trim().Length -gt 0))
    } catch { $commit = '' }
    finally { Pop-Location }
    if (-not $commit) { $commit = 'unknown' }
    return [pscustomobject]@{ commit = $commit; dirty = $dirty }
}

function Get-CiExeIdentity {
    <# 读取产物身份：SHA256/大小/mtime + `--version` 自报的 commit/dirty/built_at/api/version。
       产物不可执行或输出不可解析时如实返回空字段（由调用方判定失败，不静默通过）。 #>
    param([Parameter(Mandatory = $true)][string]$ExePath)
    $item = Get-Item -LiteralPath $ExePath -ErrorAction Stop
    $sha = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $line = ''
    try { $line = ((& $ExePath --version 2>&1) | Out-String).Trim() } catch { $line = '' }
    $grab = {
        param([string]$Text, [string]$Key)
        $m = [regex]::Match($Text, "$Key=(\S+)")
        if ($m.Success) { return $m.Groups[1].Value }
        return ''
    }
    $version = ''
    foreach ($tok in ($line -split '\s+')) {
        if ($tok -and [char]::IsDigit($tok[0])) { $version = $tok; break }
    }
    return [pscustomobject]@{
        path         = (Resolve-Path -LiteralPath $ExePath).Path
        sha256       = $sha
        bytes        = [long]$item.Length
        mtime        = $item.LastWriteTimeUtc.ToString('o')
        version_line = $line
        version      = $version
        commit       = (& $grab $line 'commit')
        dirty        = (& $grab $line 'dirty')
        built_at     = (& $grab $line 'built_at')
        api_version  = (& $grab $line 'api')
    }
}

function Test-CiBinaryIdentity {
    <# 身份门：产物 commit 必须等于当前源码 HEAD，dirty 主张必须与源码事实一致。
       返回 [pscustomobject]@{ ok; reason }；不一致的原因可直接进报告（负例据此断言）。 #>
    param(
        [Parameter(Mandatory = $true)]$Source,
        [Parameter(Mandatory = $true)]$Binary
    )
    if (-not $Binary.commit) {
        return [pscustomobject]@{ ok = $false; reason = "产物无法自报 build id（--version 未解析出 commit）：$($Binary.version_line)" }
    }
    if ($Binary.commit -eq 'unknown') {
        return [pscustomobject]@{ ok = $false; reason = '产物 build id 为 unknown（编译时无 git，身份不可验证）' }
    }
    if ($Source.commit -eq 'unknown') {
        return [pscustomobject]@{ ok = $false; reason = '源码 HEAD 不可读（git 不可用），无法验证产物身份' }
    }
    if ($Binary.commit -ne $Source.commit) {
        return [pscustomobject]@{ ok = $false; reason = ("产物过期/来源不符：exe commit={0}，当前 HEAD={1}（先构建当前源码，禁止复用旧产物）" -f $Binary.commit, $Source.commit) }
    }
    $srcDirty = if ($Source.dirty) { 'true' } else { 'false' }
    $binDirty = ("$($Binary.dirty)").ToLowerInvariant()
    if ($binDirty -and $binDirty -ne $srcDirty) {
        return [pscustomobject]@{ ok = $false; reason = ("产物 dirty 主张与源码事实不一致：exe dirty={0}，tree dirty={1}（构建身份作用域可能漂移）" -f $binDirty, $srcDirty) }
    }
    return [pscustomobject]@{ ok = $true; reason = ("commit={0} dirty={1}" -f $Binary.commit, $binDirty) }
}

function New-CiFailureState {
    # Scripts using Set-StrictMode -Version Latest cannot read an undeclared
    # script-scoped variable, even in a null comparison. Initialize state by
    # checking the variable table instead of dereferencing an unset variable.
    $defaults = [ordered]@{
        ciFailures = @()
        ciSteps = @()
        ciResourceApplied = @()
        ciResourceEvents = @()
        ciStepFilter = ''
        ciInterrupted = $false
        ciLastCommandResourceLimited = $false
    }
    foreach ($entry in $defaults.GetEnumerator()) {
        if (-not (Get-Variable -Name $entry.Key -Scope Script -ErrorAction SilentlyContinue)) {
            $null = Set-Variable -Name $entry.Key -Scope Script -Value $entry.Value
        }
    }
}

function Add-CiFailure {
    param([Parameter(Mandatory = $true)][string]$Name, [string]$Detail)
    New-CiFailureState
    $script:ciFailures += [pscustomobject]@{ name = $Name; detail = $Detail }
}

# ---------------------------------------------------------------------------
# §2.4 Rust 编译与测试资源安全红线（唯一实现）
# 本机 32 逻辑处理器 / 约 32 GB 内存；Cargo 默认并发会同时起大量 rustc、link.exe
# 与测试线程，导致物理内存耗尽、系统分页、整机卡死。本节把红线变成脚本可执行门：
#   红线 1/2：normal=编译 2 / 测试线程 2；strict（完整 workspace、完整 core、release、
#             原生依赖重链）=1 / 1；
#   红线 3：禁止依赖 Cargo 默认并发——所有 cargo 调用经 Invoke-CiCargo 显式注入 -j；
#   红线 4：禁止两组 Cargo 并行——启动前检测 cargo/rustc/link，繁忙则等待后拒绝；
#   红线 7：可用物理内存 < 6 GB 或已用率 ≥ 80% 时拒绝启动，运行中触发则中止并
#             以 resource_limited / 非零退出码收口（红线 10：保存真实退出码）；
#   §2.4.4：实际生效的限制写入 summary.json，可证明"测试是在安全参数下跑的"。
# 红线 8/9：本层只降不升——没有任何开关可以提高并发来缩短门禁时间。
# ---------------------------------------------------------------------------

function Get-CiRustResourceLimits {
    param([ValidateSet('normal', 'strict')][string]$Mode = 'normal')
    if ($Mode -eq 'strict') {
        return [pscustomobject]@{ mode = 'strict'; jobs = 1; test_threads = 1; reason = '完整 workspace/完整 core/release/原生依赖重链（§2.4 红线 2）' }
    }
    return [pscustomobject]@{ mode = 'normal'; jobs = 2; test_threads = 2; reason = '普通开发与定向测试（§2.4 红线 1）' }
}

# 设置本轮资源策略：注入 CARGO_BUILD_JOBS/RUST_TEST_THREADS 并记录进 summary。
function Set-CiRustResourcePolicy {
    param(
        [ValidateSet('normal', 'strict')][string]$Mode = 'normal',
        [string]$Source = 'script'
    )
    $limits = Get-CiRustResourceLimits -Mode $Mode
    if ($limits.jobs -gt 2 -or $limits.test_threads -gt 2) {
        throw "§2.4 资源红线：本机并发上限 2（收到 jobs=$($limits.jobs) test_threads=$($limits.test_threads)）"
    }
    $env:CARGO_BUILD_JOBS = [string]$limits.jobs
    $env:RUST_TEST_THREADS = [string]$limits.test_threads
    $script:ciRustPolicyMode = $limits.mode
    $script:ciRustPolicySource = $Source
    return $limits
}

function Get-CiRustPolicyMode {
    if ($script:ciRustPolicyMode) { return $script:ciRustPolicyMode }
    return 'normal'
}

# 内存快照：Get-CimInstance 主路径，WMI 兜底；两者都不可用时如实返回 unknown
# （不猜测、不假装通过；调用方按红线 7 拒绝启动新构建）。
function Get-CiMemoryStatus {
    # 阈值可由环境变量覆盖（§2.4 红线 7 的口径不是物理常量，取决于"这台机器此刻谁在用"）：
    #   OWO_CI_MAX_USED_PERCENT  已用率上限，缺省 80
    #   OWO_CI_MIN_FREE_GB       可用内存下限，缺省 6
    # 2026-09-22：机器主人在跑大型游戏（约 9GB 常驻）期间明确要求"已用 95% 以下都行"，
    # 于是把两档一起下调（used<95 / free≥1.5GB）。覆盖是**显式且可追溯**的：
    # 生效时打一行说明，summary 里也记 gate 实际取值——不允许悄悄放宽阈值。
    $maxUsedPercent = 80.0
    $minFreeGb = 6.0
    if ($env:OWO_CI_MAX_USED_PERCENT) {
        $parsed = 0.0
        if ([double]::TryParse($env:OWO_CI_MAX_USED_PERCENT, [ref]$parsed) -and $parsed -gt 0 -and $parsed -le 100) {
            $maxUsedPercent = $parsed
        }
    }
    if ($env:OWO_CI_MIN_FREE_GB) {
        $parsed = 0.0
        if ([double]::TryParse($env:OWO_CI_MIN_FREE_GB, [ref]$parsed) -and $parsed -ge 0) {
            $minFreeGb = $parsed
        }
    }
    $param = [pscustomobject]@{
        min_free_gb      = $minFreeGb
        max_used_percent = $maxUsedPercent
        overridden       = [bool]($env:OWO_CI_MAX_USED_PERCENT -or $env:OWO_CI_MIN_FREE_GB)
    }
    $os = $null
    try { $os = Get-CimInstance -ClassName Win32_OperatingSystem -ErrorAction Stop } catch {
        try { $os = Get-WmiObject -Class Win32_OperatingSystem -ErrorAction Stop } catch { $os = $null }
    }
    if (-not $os) {
        return [pscustomobject]@{ status = 'unknown'; total_gb = 0.0; free_gb = 0.0; used_percent = 0.0; ok = $false; source = 'none'; gate = $param }
    }
    $totalGb = [math]::Round([double]$os.TotalVisibleMemorySize / 1MB, 2)
    $freeGb = [math]::Round([double]$os.FreePhysicalMemory / 1MB, 2)
    if ($totalGb -le 0) {
        return [pscustomobject]@{ status = 'unknown'; total_gb = 0.0; free_gb = $freeGb; used_percent = 0.0; ok = $false; source = 'cim'; gate = $param }
    }
    $usedPct = [math]::Round((1 - ($freeGb / $totalGb)) * 100, 1)
    $limited = ($freeGb -lt $param.min_free_gb) -or ($usedPct -ge $param.max_used_percent)
    return [pscustomobject]@{
        status       = $(if ($limited) { 'limited' } else { 'ok' })
        total_gb     = $totalGb
        free_gb      = $freeGb
        used_percent = $usedPct
        ok           = (-not $limited)
        source       = 'cim'
        gate         = $param
    }
}

# 红线 7：低于阈值直接拒绝启动新 Rust 构建（给出可操作说明，不静默降级）。
function Assert-CiMemoryGate {
    param([string]$Context = 'cargo')
    $mem = Get-CiMemoryStatus
    if ($mem.gate.overridden) {
        Write-Host ("    [§2.4] 内存门阈值被环境变量覆盖：可用 ≥ {0} GB 且已用 < {1}%（owner 显式指定）" -f `
                $mem.gate.min_free_gb, $mem.gate.max_used_percent) -ForegroundColor Yellow
    }
    if ($mem.status -eq 'unknown') {
        Write-Host ("    [§2.4] 内存状态不可读取（{0}）——本轮仍按 -j≤2 / --test-threads≤2 受限并发执行" -f $Context) -ForegroundColor Yellow
    } elseif (-not $mem.ok) {
        throw ("§2.4 红线 7（resource_limited）：{0} 拒绝启动 Rust 构建——可用物理内存 {1} GB / 已用 {2}%，低于阈值（要求可用 ≥ {3} GB 且已用 < {4}%）。请关闭占用内存的程序后重试，或等待当前构建结束；禁止用提高超时来绕过内存门。" -f `
                $Context, $mem.free_gb, $mem.used_percent, $mem.gate.min_free_gb, $mem.gate.max_used_percent)
    }
    return $mem
}

# 红线 7 补充（本轮实测事故驱动）：**磁盘和内存一样是资源安全红线**，但指南 §2.4
# 原文只写了内存门。后果是真实的：全 workspace 串行一轮在 target\debug\deps 里堆出
# 数百个 PDB（实测 45.6 GB / 722 个），盘满后 link.exe 报
# `LNK1318 非意外的 PDB 错误: LIMIT`，表面像"编译失败"，实为磁盘耗尽——
# 一整轮 30 分钟验证直接作废，还把用户的盘逼到只剩 0.1 GB。
# 这里在**启动前**就拒绝，并把阈值与处置办法写进异常文案（不靠事后猜）。
function Get-CiDiskStatus {
    <# 读构建卷剩余空间；读不到就如实报 unknown（不猜"空间充足"）。 #>
    param(
        [string]$Path = '',
        [int]$MinFreeGbStrict = 10,
        [int]$MinFreeGbNormal = 4
    )
    $target = if ($Path) { $Path } else { Get-CiRepoRoot }
    $item = Get-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue
    $driveName = if ($item -and $item.PSDrive -and $item.PSDrive.Name) { $item.PSDrive.Name }
                 elseif ($item) { ($item.FullName -replace '^(\$:).*', '$1') -replace ':$', '' }
                 else { '' }
    $pd = if ($driveName) { Get-PSDrive -Name $driveName -ErrorAction SilentlyContinue } else { $null }
    if (-not $pd) {
        return [pscustomobject]@{
            status = 'unknown'; drive = $driveName; free_gb = $null; total_gb = $null
            min_free_gb = $null; ok = $true; source = 'none'
        }
    }
    return [pscustomobject]@{
        status      = 'ok'
        drive       = $driveName
        free_gb     = [Math]::Round($pd.Free / 1GB, 2)
        total_gb    = [Math]::Round((($pd.Free + $pd.Used)) / 1GB, 2)
        min_free_gb = 0          # 由调用方按档位填（此处只做读取，不做策略）
        ok          = $true
        source      = 'psdrive'
    }
}

# 构建前磁盘门：strict（完整 workspace/release/原生依赖重链）要求 ≥20 GB，定向 ≥6 GB。
# 差这么多有实测依据：一轮全量测试的 PDB/中间产物增量实测 13.7 GB，且 debug 依赖树
# 本身还会继续膨胀；门必须高于一轮的真实消耗，才能让"盘满"变成"拒绝启动"而不是
# "跑到链接阶段半途炸掉"（那等于给注定失败的长任务开假绿灯）。
function Assert-CiDiskGate {
    param(
        [ValidateSet('normal', 'strict')][string]$Mode = 'normal',
        [string]$Path = '',
        [string]$Context = 'cargo',
        [int]$MinFreeGb = -1
    )
    # 阈值可覆盖只为让"拒绝路径"能被离线断言测到（负例不能依赖磁盘真的快满）。
    # strict=20 GB 是**实测**定的：全 workspace 串行一轮（88 套件）实测吃掉 13.7 GB
    # （51.4 → 37.7）。原先写 10 GB 会让一轮注定半途炸在 LNK1318 的构建通过检查——
    # 那不是门，是假绿灯；阈值必须高于一轮的实测消耗并留安全余量。
    $min = if ($MinFreeGb -ge 0) { $MinFreeGb } elseif ($Mode -eq 'strict') { 20 } else { 6 }
    $disk = Get-CiDiskStatus -Path $Path -MinFreeGbStrict $min -MinFreeGbNormal $min
    $disk | Add-Member -NotePropertyName min_free_gb -NotePropertyValue $min -Force
    if ($disk.status -eq 'unknown') {
        Write-Host ("    [§2.4] 构建卷剩余空间不可读取（{0}）——本轮仍执行，但请自行确认磁盘余量" -f $Context) -ForegroundColor Yellow
        return $disk
    }
    $disk.ok = ($disk.free_gb -ge $min)
    if (-not $disk.ok) {
        throw ("§2.4 资源红线（resource_limited）：{0} 拒绝启动 Rust 构建——卷 {1}: 仅剩 {2} GB，{3} 档要求 ≥ {4} GB。PDB/中间产物会在链接阶段瞬间吃满磁盘（实测 LNK1318 PDB LIMIT）。请删除可再生产物（target\debug\incremental、*.pdb 均可安全删，重编即恢复）后重试；禁止用提高超时或改小并发来绕过磁盘门。" -f `
                $Context, $disk.drive, $disk.free_gb, $Mode, $min)
    }
    return $disk
}

# 红线 4：不得同时运行两组 Cargo——已有 cargo/rustc/link 时等待，等待超时后拒绝。
function Assert-CiBuildIdle {
    param([int]$WaitSec = 900, [int]$PollSec = 10, [string]$Context = 'cargo')
    $busy = @(Get-Process -Name cargo, rustc, link -ErrorAction SilentlyContinue)
    if ($busy.Count -eq 0) { return $true }
    $deadline = (Get-Date).AddSeconds($WaitSec)
    $groups = (@($busy | Group-Object ProcessName | ForEach-Object { "$($_.Name)x$($_.Count)" }) -join ' ')
    Write-Host ("    [§2.4] 检测到 {0} 个既有构建进程（{1}）——按红线 4 等待其结束，不启动第二组" -f $busy.Count, $groups) -ForegroundColor Yellow
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds $PollSec
        $busy = @(Get-Process -Name cargo, rustc, link -ErrorAction SilentlyContinue)
        if ($busy.Count -eq 0) {
            Write-Host "    [§2.4] 既有构建已结束，继续本轮编译" -ForegroundColor Green
            return $true
        }
    }
    throw ("§2.4 红线 4（build_busy）：{0} 等待 {1}s 后仍有 {2} 个 cargo/rustc/link 进程在跑，拒绝并发启动第二组构建。请等待或显式终止本轮无关的构建进程。" -f `
            $Context, $WaitSec, $busy.Count)
}

# 红线 3 + §2.4.4：显式 -j / --test-threads，且拒绝任何 >2 的本机并发参数。
function Protect-CiCargoArguments {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [int]$Jobs = 2,
        [int]$TestThreads = 2
    )
    if ($Jobs -gt 2 -or $TestThreads -gt 2) {
        throw "§2.4 资源红线：本机并发上限 2（收到 jobs=$Jobs test_threads=$TestThreads）"
    }
    $tokens = @($Arguments | ForEach-Object { [string]$_ })
    # 以第一个 `--` 切 head/tail：head 属 cargo 自身选项，tail 属测试可执行文件。
    $sep = [Array]::IndexOf($tokens, '--')
    if ($sep -ge 0) {
        $head = @($tokens | Select-Object -First $sep)
        $tail = @($tokens | Select-Object -Skip ($sep + 1))
    } else {
        $head = @($tokens)
        $tail = @()
    }
    # 1) head：剥掉调用方自带的 -j/--jobs（>2 直接拒绝，≤2 由本轮策略统一改写）。
    $headClean = New-Object System.Collections.Generic.List[string]
    $skipNext = $false
    $foundJobs = 0
    for ($i = 0; $i -lt $head.Count; $i++) {
        $tok = [string]$head[$i]
        if ($skipNext) { $skipNext = $false; continue }
        if (($tok -eq '-j') -or ($tok -eq '--jobs')) {
            if ($i + 1 -lt $head.Count) {
                if (-not [int]::TryParse([string]$head[$i + 1], [ref]$foundJobs)) {
                    throw "§2.4 资源红线：$tok 之后不是数字（cargo 参数归一失败，拒绝猜测）"
                }
                $skipNext = $true
            }
            continue
        }
        if ($tok -match '^-j(\d+)$') { $foundJobs = [int]$Matches[1]; continue }
        if ($tok -match '^--jobs=(\d+)$') { $foundJobs = [int]$Matches[1]; continue }
        $headClean.Add($tok) | Out-Null
    }
    if ($foundJobs -gt 2) {
        throw "§2.4 红线 3/§2.4.4：检测到本机并发参数 -j $foundJobs（上限 2）。请改用 -j 1（完整测试/release）或 -j 2（定向测试）。"
    }
    # 子命令 = head 中第一个不以 '-' 开头的 token（cargo test/clippy/build/fmt/run/…）。
    $sub = ''
    foreach ($tok in $headClean) { if ($tok -notmatch '^-') { $sub = $tok; break } }
    # 只有会真正编译/rustc 派生的子命令接受 -j（cargo fmt 不认：实测
    # `error: unexpected argument '-j' found`）。非编译子命令跳过注入，不猜测。
    $buildish = @('b', 'build', 'c', 'check', 'clippy', 't', 'test', 'r', 'run', 'bench', 'install', 'package', 'fix', 'doc', 'rustc')
    $injectJobs = ($buildish -contains $sub)
    # 2) tail：仅 cargo test 归一 --test-threads（>2 拒绝，缺失则注入）。
    $effectiveThreads = $TestThreads
    if ($sub -eq 'test') {
        $tailClean = New-Object System.Collections.Generic.List[string]
        $foundThreads = 0
        $skipNext2 = $false
        for ($i = 0; $i -lt $tail.Count; $i++) {
            $tok = [string]$tail[$i]
            if ($skipNext2) { $skipNext2 = $false; continue }
            if ($tok -eq '--test-threads') {
                if (-not [int]::TryParse([string]$tail[$i + 1], [ref]$foundThreads)) {
                    throw "§2.4 资源红线：--test-threads 之后不是数字"
                }
                $skipNext2 = $true
                continue
            }
            if ($tok -match '^--test-threads=(\d+)$') { $foundThreads = [int]$Matches[1]; continue }
            $tailClean.Add($tok) | Out-Null
        }
        if ($foundThreads -gt 2) {
            throw "§2.4 红线 3/§2.4.4：检测到测试并发参数 --test-threads $foundThreads（上限 2）。请改用 1 或 2。"
        }
        # 调用方显式更小时取更小值（红线 8：只降不升）。
        if ($foundThreads -gt 0) { $effectiveThreads = [Math]::Min($foundThreads, $TestThreads) }
        $tail = @($tailClean) + @("--test-threads=$effectiveThreads")
    }
    # 调用方显式更小的 -j 同样取更小值（只降不升）。
    $effectiveJobs = $Jobs
    if ($foundJobs -gt 0) { $effectiveJobs = [Math]::Min($foundJobs, $Jobs) }

    # 3) 重新注入策略值：-j 位于 trailing `--` 之前（cargo 全局选项位置）。
    $result = New-Object System.Collections.Generic.List[string]
    foreach ($tok in $headClean) { $result.Add($tok) | Out-Null }
    if ($injectJobs) {
        $result.Add('-j') | Out-Null
        $result.Add([string]$effectiveJobs) | Out-Null
    }
    if ($tail.Count -gt 0) {
        $result.Add('--') | Out-Null
        foreach ($tok in $tail) { $result.Add($tok) | Out-Null }
    }
    return , @($result)
}

# 子命令是否会派生 rustc/link（决定是否需要构建空闲门与 -j 注入）。
# 注意：不含 tree/fmt/audit/deny——它们不启动编译，也不接受 -j。
function Test-CiCargoCompiles {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    $buildish = @('b', 'build', 'c', 'check', 'clippy', 't', 'test', 'r', 'run', 'bench', 'install', 'package', 'fix', 'doc', 'rustc')
    foreach ($tok in @($Arguments | ForEach-Object { [string]$_ })) {
        if ($tok -match '^-') { continue }
        return ($buildish -contains $tok)
    }
    return $false
}

# 红线 7 的"运行中内存守护"保护对象是**构建与测试风暴**（rustc 群 + 测试分片），
# 不是用户主动拉起的常驻进程：`cargo run` 编译段一过就是 dev serve / product-eval
# 批次；内存吃紧时杀掉它等于杀用户程序（红线 8 要的正是"工程让出资源、不打断用户"）。
# 因此 run 只过启动前的内存门与构建空闲门，不启用运行中守护。
function Test-CiCargoGuardRunning {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    if (-not (Test-CiCargoCompiles -Arguments $Arguments)) { return $false }
    foreach ($tok in @($Arguments | ForEach-Object { [string]$_ })) {
        if ($tok -match '^-') { continue }
        return (@('run', 'r') -notcontains $tok)
    }
    return $false
}

# §2.4 统一 cargo 执行入口：策略注入 + 内存门 + 构建空闲门 + 参数归一 + 心跳/超时
# + 运行中内存守护（红线 7 的"必要时中止"）+ 真实退出码（红线 10）。
function Invoke-CiCargo {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$Cwd = "",
        [int]$TimeoutSec = 0,
        [int]$HeartbeatSec = 30,
        [string]$LogFile = "",
        [string]$Label = 'cargo',
        [ValidateSet('auto', 'normal', 'strict')][string]$PolicyMode = 'auto',
        # 允许显式指定 cargo 可执行文件（发布脚本用 $env:OWO_CARGO 覆盖；缺省走 PATH）。
        [string]$CargoExe = "",
        # 需要真实退出码时使用：把 cargo 的退出码作为返回值输出，调用方不必依赖
        # `$LASTEXITCODE`（后者只在原生命令结束时由 PowerShell 自动写入，经函数调用
        # 后会丢失，实测会把 101 编译失败错报成 1）。
        [switch]$PassThru,
        [switch]$SkipIdleGate
    )
    $mode = if ($PolicyMode -eq 'auto') { Get-CiRustPolicyMode } else { $PolicyMode }
    $limits = Set-CiRustResourcePolicy -Mode $mode -Source $Label
    $exe = if ($CargoExe) { $CargoExe } elseif ($env:OWO_CARGO) { $env:OWO_CARGO } else { 'cargo' }
    $compiles = Test-CiCargoCompiles -Arguments $Arguments
    if ($compiles) {
        $null = Assert-CiMemoryGate -Context $Label
        $disk = Assert-CiDiskGate -Mode $mode -Path $Cwd -Context $Label
        if (-not $SkipIdleGate) {
            $null = Assert-CiBuildIdle -Context $Label
            # Waiting for another build can invalidate both earlier snapshots.
            # Recheck at the actual launch boundary, through the same gates.
            $null = Assert-CiMemoryGate -Context $Label
            $disk = Assert-CiDiskGate -Mode $mode -Path $Cwd -Context $Label
        }
    }
    $safe = Protect-CiCargoArguments -Arguments $Arguments -Jobs $limits.jobs -TestThreads $limits.test_threads
    if ($compiles) {
        Write-Host ("    [§2.4] {0}：-j {1} / --test-threads {2}（{3}）" -f $Label, $limits.jobs, $limits.test_threads, $limits.reason)
        # 必须丢弃返回值：New-CiFailureState 会 return 布尔；若它留在输出流里，
        # `-PassThru` 的 `return $code` 就变成 @($true, 101)，调用方拿到 "True 101"，
        # 用 $code 去 exit 时会退化成 1，真实的 101（编译失败）被掩盖。实测踩过。
        $null = New-CiFailureState
        $script:ciResourceApplied = @($script:ciResourceApplied) + @(
            "{0}={1}(-j {2}/--test-threads {3})" -f $Label, $limits.mode, $limits.jobs, $limits.test_threads
        )
    } else {
        Write-Host ("    [§2.4] {0}：非编译子命令（不注入 -j；红线 1/2 不适用）" -f $Label)
    }
    $script:ciLastCommandResourceLimited = $false
    # 红线 7 的适用范围：编译/测试才开运行中守护；`cargo run`（常驻服务、eval 批次）
    # 只过启动前门，避免把用户程序当"越线构建"杀掉。
    $guardRunning = Test-CiCargoGuardRunning -Arguments $Arguments
    Invoke-CiLoggedCommand -Exe $exe -Arguments $safe -Cwd $Cwd -TimeoutSec $TimeoutSec `
        -HeartbeatSec $HeartbeatSec -LogFile $LogFile -Label $Label -MemoryGuard:$guardRunning
    $code = $global:LASTEXITCODE
    if ($script:ciLastCommandResourceLimited) {
        # 资源保护触发：非零退出（红线 10），不允许记成通过。
        $global:LASTEXITCODE = 137
        $code = 137
    }
    if ($PassThru) {
        # 只返回标量退出码：即便上游还有别的东西混进输出流，也不能让调用方拿到
        # @($true, 0) 这种数组（实测会把真实退出码掩盖成 1）。
        if ($code -is [array]) { $code = [int]($code | Select-Object -Last 1) }
        return [int]$code
    }
}

# §2.4 变体：既要实时进度（红线 5）又要把日志读回来扫描（发布脚本的 LNK 零容忍门）。
# 实现：流式写 LogFile + 实时回显 → 结束后把日志读回作为返回数组；退出码留在
# $global:LASTEXITCODE（含 124 超时 / 137 资源守护，红线 10）。
function Invoke-CiCargoCapture {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [string]$Cwd = "",
        [int]$TimeoutSec = 0,
        [int]$HeartbeatSec = 30,
        [string]$Label = 'cargo',
        [Parameter(Mandatory = $true)][string]$LogFile,
        [ValidateSet('auto', 'normal', 'strict')][string]$PolicyMode = 'auto',
        [string]$CargoExe = "cargo"
    )
    $dir = Split-Path -Parent $LogFile
    if ($dir -and -not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
    Invoke-CiCargo -Arguments $Arguments -Cwd $Cwd -TimeoutSec $TimeoutSec -HeartbeatSec $HeartbeatSec `
        -LogFile $LogFile -Label $Label -PolicyMode $PolicyMode -CargoExe $CargoExe
    $lines = @()
    if (Test-Path -LiteralPath $LogFile) { $lines = @(Get-Content -LiteralPath $LogFile -Encoding UTF8) }
    return ,$lines
}

function Get-CiResourceState {
    New-CiFailureState
    $mem = Get-CiMemoryStatus
    $disk = Get-CiDiskStatus
    $mode = Get-CiRustPolicyMode
    $limits = Get-CiRustResourceLimits -Mode $mode
    return [ordered]@{
        redline         = '§2.4 Rust 编译与测试资源安全红线'
        mode            = $limits.mode
        jobs            = $limits.jobs
        test_threads    = $limits.test_threads
        policy_source   = if ($script:ciRustPolicySource) { $script:ciRustPolicySource } else { 'default' }
        cargo_env       = [ordered]@{ CARGO_BUILD_JOBS = $env:CARGO_BUILD_JOBS; RUST_TEST_THREADS = $env:RUST_TEST_THREADS }
        cpu_logical     = [int]$env:NUMBER_OF_PROCESSORS
        memory_gate     = [ordered]@{ min_free_gb = 6.0; max_used_percent = 80.0 }
        memory_at_write  = [ordered]@{ status = $mem.status; total_gb = $mem.total_gb; free_gb = $mem.free_gb; used_percent = $mem.used_percent; source = $mem.source }
        # §2.4 补充门：构建卷余量（PDB/中间产物会在链接阶段瞬间吃满磁盘）。
        disk_at_write    = [ordered]@{ status = $disk.status; drive = $disk.drive; free_gb = $disk.free_gb; total_gb = $disk.total_gb; source = $disk.source }
        applied         = @($script:ciResourceApplied)
        guard_events    = @($script:ciResourceEvents)
    }
}

# 断言工具可用；不可用时 -Required 抛错（CI 中安全类门禁必须真实执行），否则返回 $false 供调用方显式 SKIP。
function Assert-CiTool {
    param(
        [Parameter(Mandatory = $true)][string]$Tool,
        [string]$InstallHint = "CI 应通过 taiki-e/install-action 安装",
        [switch]$Required
    )
    if (Get-Command $Tool -ErrorAction SilentlyContinue) { return $true }
    if ($Required) {
        throw "$Tool 不可用（$InstallHint）；安全类门禁不可跳过"
    }
    return $false
}

# 执行单个 CI 步骤：输出回显 + 可选落盘 $LogDir\<id>.log；支持 -Step 过滤。
# R3-D（指南 §3.6）：summary.json 每步增量落盘；Ctrl+C 中断写 interrupted=true，
# 绝不留下"没有结论的空报告"。长任务的实时心跳/超时由块内 Invoke-CiLoggedCommand 负责。
function Invoke-CiStep {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Id,
        [Parameter(Mandatory = $true)][scriptblock]$Block,
        [string]$Cwd = "",
        [string]$LogDir = ""
    )
    New-CiFailureState
    if ($script:ciStepFilter -and -not $Id.Contains($script:ciStepFilter)) {
        return
    }
    $script:ciSteps += $Name
    Write-Host ("==> {0}" -f $Name) -ForegroundColor Cyan
    $logFile = ""
    if ($LogDir) {
        New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
        $logFile = Join-Path $LogDir ($Id + ".log")
        Write-CiStepSummary -LogDir $LogDir -CurrentStep $Name -CurrentId $Id -StepState "running"
    }
    $outText = ""
    if ($Cwd) { Push-Location $Cwd }
    try {
        $global:LASTEXITCODE = $null
        $outText = (& $Block 2>&1 | Out-String)
        $exit = $global:LASTEXITCODE
        if ($null -eq $exit) { $exit = 0 }
        if ($outText) {
            $trimmed = $outText.TrimEnd("`r", "`n")
            if ($trimmed) { Write-Host $trimmed }
        }
        if ($logFile) { $outText | Out-File -LiteralPath $logFile -Encoding UTF8 }
        if ($exit -ne 0) {
            Add-CiFailure $Name ("exit = {0}" -f $exit)
            Write-Host ("    [FAIL] {0} (exit={1})" -f $Name, $exit) -ForegroundColor Red
            if ($logFile) { Write-CiStepSummary -LogDir $LogDir -CurrentStep $Name -CurrentId $Id -StepState "failed" }
        } else {
            Write-Host ("    [OK]   {0}" -f $Name) -ForegroundColor Green
            if ($logFile) { Write-CiStepSummary -LogDir $LogDir -CurrentStep $Name -CurrentId $Id -StepState "passed" }
        }
    } catch [Management.Automation.PipelineStoppedException] {
        # Ctrl+C：中断必须成为可判定的终态（R3-D），不允许"跑没跑完看不出来"。
        $script:ciInterrupted = $true
        Write-CiStepSummary -LogDir $LogDir -CurrentStep $Name -CurrentId $Id -StepState "interrupted"
        Write-Host "    [INTERRUPTED] $Name —— 用户中止，summary.json 已标记 interrupted=true" -ForegroundColor Yellow
        if ($LogDir) {
            exit 130
        }
        throw
    } catch {
        $err = "EXCEPTION: $($_.Exception.Message)`n$($_.ScriptStackTrace)"
        if ($outText) { $err = $outText + $err }
        Write-Host $err -ForegroundColor Red
        if ($logFile) { $err | Out-File -LiteralPath $logFile -Encoding UTF8 }
        Add-CiFailure $Name $_.Exception.Message
        Write-Host ("    [FAIL] {0} : {1}" -f $Name, $_.Exception.Message) -ForegroundColor Red
        if ($logFile) { Write-CiStepSummary -LogDir $LogDir -CurrentStep $Name -CurrentId $Id -StepState "failed" }
    } finally {
        if ($Cwd) { Pop-Location }
    }
}

# R3-D（§3.6）：增量 summary——每步开始前/结束后都落盘一次，任意时刻被杀都有结论。
# §2.4.4：同时落盘 resources 段（模式/jobs/test_threads/内存快照/守护事件），
# 使报告能自证"本轮测试是在安全并发参数下跑的"。
function Write-CiStepSummary {
    param(
        [Parameter(Mandatory = $true)][string]$LogDir,
        [string]$CurrentStep = "",
        [string]$CurrentId = "",
        [string]$StepState = "running"
    )
    if (-not $LogDir) { return }
    New-CiFailureState
    $summary = [ordered]@{
        updated_at   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
        ok           = (@($script:ciFailures).Count -eq 0) -and ($StepState -ne "failed") -and ($StepState -ne "interrupted")
        completed    = $false
        interrupted  = [bool]$script:ciInterrupted
        current_step = $CurrentStep
        current_id   = $CurrentId
        step_state   = $StepState
        resources    = Get-CiResourceState
        steps        = @($script:ciSteps)
        failures     = @($script:ciFailures | ForEach-Object { "{0}: {1}" -f $_.name, $_.detail })
    }
    $json = $summary | ConvertTo-Json -Depth 5
    $tmp = Join-Path $LogDir "summary.json.tmp"
    [System.IO.File]::WriteAllText($tmp, $json, (New-Object System.Text.UTF8Encoding($false)))
    Move-Item -LiteralPath $tmp -Destination (Join-Path $LogDir "summary.json") -Force
}

# R3-D（§3.6）：外部命令流式执行器——30s 心跳、独立超时（超时错误码 124=TIMEOUT）、
# 输出逐行 tee 到文件与终端（替代"整步缓冲到结束才见输出"）。
# 用法：块内 `Invoke-CiLoggedCommand -Exe cargo -Arguments @('test','--workspace') -TimeoutSec 5400 -LogFile ...`
# 结束把退出码写入 $global:LASTEXITCODE（超时=124，§2.4 内存守护=137）；
# 本函数自身不 throw，保持门禁统计语义。Rust 构建一律走 Invoke-CiCargo（含红线门）。
# R3-C 缺陷修复（R3-BUG-25）：.cmd 垫片必须用绝对路径启动。
# ProcessStartInfo 用裸名（npm.cmd / tsc.cmd）启动批处理时，cmd 的 %~dp0 会退化成
# 工作目录，垫片随后去找 <工作目录>\node_modules\npm\bin\npm-cli.js —— 报
# "Cannot find module"，看起来像依赖坏了，实际是启动方式坏了。门禁 ts/ts-unit
# 两步就是这么全红的（npm --version 同命令行用绝对路径立刻成功）。
function Resolve-CiCommandPath {
    param([Parameter(Mandatory = $true)][string]$Exe, [string]$Cwd = "")
    if ($Exe -match '[\\/]') { return $Exe }
    # 本地工具优先（node_modules\.bin 里的 .cmd 垫片只有按目录解析才存在）
    if ($Cwd) {
        $local = Join-Path (Join-Path $Cwd "node_modules\.bin") $Exe
        if (Test-Path -LiteralPath $local) { return (Resolve-Path -LiteralPath $local).Path }
    }
    $found = Get-Command $Exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found -and $found.Source) { return $found.Source }
    return $Exe
}

# taskkill can report an already-exited child on stderr. Under the caller's
# ErrorActionPreference=Stop that is a NativeCommandError, not a build result.
# Keep its diagnostics and nonzero status, but always finish the command log.
function Stop-CiLoggedProcessTree {
    param([System.Diagnostics.Process]$Process, [string]$Label)
    $savedPreference = $ErrorActionPreference
    $killOutput = @()
    $killExit = -1
    try {
        $ErrorActionPreference = 'Continue'
        $killOutput = @(& taskkill /PID $Process.Id /T /F 2>&1)
        $killExit = $LASTEXITCODE
    } catch {
        $killOutput = @($_)
    } finally {
        $ErrorActionPreference = $savedPreference
    }
    foreach ($line in $killOutput) { Write-Host ("    [{0}] taskkill: {1}" -f $Label, $line) }
    $rootExited = $false
    try { $rootExited = $Process.WaitForExit(10000) } catch { }
    if (($killExit -ne 0) -or (-not $rootExited)) {
        $detail = "termination_unverified: taskkill exit=$killExit root_exited=$rootExited pid=$($Process.Id)"
        Add-CiFailure -Name ($Label + '-termination') -Detail $detail
        Write-Host ("    [{0}] {1}; inspect remaining processes before another build" -f $Label, $detail) -ForegroundColor Yellow
    }
}

function Invoke-CiLoggedCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Exe,
        [string[]]$Arguments = @(),
        [string]$Cwd = "",
        [int]$TimeoutSec = 0,
        [int]$HeartbeatSec = 30,
        [string]$LogFile = "",
        [string]$Label = "",
        # §2.4 红线 7：运行中内存守护——每次心跳复查，越线即终止进程树并以
        # resource_limited 收口（不允许"跑着跑着把机器拖死"）。
        [switch]$MemoryGuard
    )
    $tag = if ($Label) { $Label } else { $Exe }
    $script:ciLastCommandResourceLimited = $false
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = Resolve-CiCommandPath -Exe $Exe -Cwd $Cwd
    # PS5.1/.NET Framework 无 ArgumentList：逐 token 安全引用拼 Arguments。
    $psi.Arguments = ($Arguments | ForEach-Object {
        if ($_ -match '[\s"]') { '"' + ($_ -replace '"', '\"') + '"' } else { $_ }
    }) -join ' '
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    if ($Cwd) { $psi.WorkingDirectory = $Cwd }
    if ($psi.FileName -ne $Exe) {
        Write-Host ("    [{0}] 可执行文件解析：{1} → {2}" -f $tag, $Exe, $psi.FileName)
    }
    $outQ = New-Object System.Collections.Concurrent.ConcurrentQueue[string]
    $errQ = New-Object System.Collections.Concurrent.ConcurrentQueue[string]
    $proc = $null
    try {
        $proc = [System.Diagnostics.Process]::Start($psi)
    } catch {
        Write-Host ("    [{0}] 启动失败：{1}" -f $tag, $_.Exception.Message) -ForegroundColor Red
        $global:LASTEXITCODE = 1
        return
    }
    $outEvt = Register-ObjectEvent -InputObject $proc -EventName OutputDataReceived -Action {
        if ($null -ne $EventArgs.Data) { $Event.MessageData.Enqueue([string]$EventArgs.Data) }
    } -MessageData $outQ
    $errEvt = Register-ObjectEvent -InputObject $proc -EventName ErrorDataReceived -Action {
        if ($null -ne $EventArgs.Data) { $Event.MessageData.Enqueue([string]$EventArgs.Data) }
    } -MessageData $errQ
    $proc.BeginOutputReadLine()
    $proc.BeginErrorReadLine()
    $writer = $null
    if ($LogFile) {
        $logDir = Split-Path -Parent $LogFile
        if ($logDir -and -not (Test-Path -LiteralPath $logDir)) { New-Item -ItemType Directory -Force -Path $logDir | Out-Null }
        $writer = [System.IO.StreamWriter]::new($LogFile, $false, (New-Object System.Text.UTF8Encoding($false)))
    }
    function Drain-Queues {
        $line = $null
        while ($outQ.TryDequeue([ref]$line)) {
            if ($null -ne $line) { Write-Host $line; if ($writer) { $writer.WriteLine($line) } }
        }
        while ($errQ.TryDequeue([ref]$line)) {
            if ($null -ne $line) { Write-Host $line; if ($writer) { $writer.WriteLine($line) } }
        }
    }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $lastBeat = [DateTime]::Now
    $timedOut = $false
    $resourceLimited = $false
    try {
        while (-not $proc.HasExited) {
            Drain-Queues
            if (($sw.Elapsed.TotalSeconds -ge $TimeoutSec) -and ($TimeoutSec -gt 0)) {
                $timedOut = $true
                Write-Host ("    [{0}] TIMEOUT：已运行 {1}s ≥ 上限 {2}s，终止进程树（错误码 124）" -f `
                        $tag, [int]$sw.Elapsed.TotalSeconds, $TimeoutSec) -ForegroundColor Red
                Stop-CiLoggedProcessTree -Process $proc -Label $tag
                break
            }
            if (([DateTime]::Now - $lastBeat).TotalSeconds -ge $HeartbeatSec) {
                $lastBeat = [DateTime]::Now
                Write-Host ("    [{0}] beat：已运行 {1}s（timeout={2}s）" -f $tag, [int]$sw.Elapsed.TotalSeconds, $(if ($TimeoutSec -gt 0) { $TimeoutSec } else { 'none' }))
                if ($MemoryGuard) {
                    # §2.4 红线 7：心跳同时是内存复查点；越线即停本轮构建（错误码 137），
                    # 不允许把"整机卡顿"当作可接受的测试代价（红线 8）。
                    $mem = Get-CiMemoryStatus
                    if ($mem.status -eq 'limited') {
                        $resourceLimited = $true
                        Write-Host ("    [{0}] RESOURCE_LIMITED：可用内存 {1} GB / 已用 {2}%（阈值 ≥{3} GB 且 <{4}%）——终止进程树，退出码 137" -f `
                                $tag, $mem.free_gb, $mem.used_percent, $mem.gate.min_free_gb, $mem.gate.max_used_percent) -ForegroundColor Red
                        Stop-CiLoggedProcessTree -Process $proc -Label $tag
                        break
                    }
                }
            }
            Start-Sleep -Milliseconds 500
        }
        try { if ($proc.HasExited) { $proc.WaitForExit() } } catch { }
        Start-Sleep -Milliseconds 200
        Drain-Queues
        $code = if ($resourceLimited) {
            137
        } elseif ($timedOut) {
            124
        } else {
            try { $proc.ExitCode } catch { 124 }
        }
    } finally {
        if ($resourceLimited) {
            New-CiFailureState
            $script:ciResourceEvents = @($script:ciResourceEvents) + @((Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ") + " " + $tag + " resource_limited（§2.4 红线 7：运行中内存越线，已触发终止；结果见日志）")
            $script:ciLastCommandResourceLimited = $true
        }
        if ($writer) { $writer.Flush(); $writer.Dispose() }
        Unregister-Event -SubscriptionId $outEvt.Id -ErrorAction SilentlyContinue
        Unregister-Event -SubscriptionId $errEvt.Id -ErrorAction SilentlyContinue
        Remove-Event -SourceIdentifier $outEvt.Name -ErrorAction SilentlyContinue
        Remove-Event -SourceIdentifier $errEvt.Name -ErrorAction SilentlyContinue
        if ($proc) { $proc.Dispose() }
    }
    Write-Host ("    [{0}] 完成：exit={1} 用时 {2}s" -f $tag, $code, [int]$sw.Elapsed.TotalSeconds)
    $global:LASTEXITCODE = $code
}

# 汇总并退出：0 = 全过（含受控 SKIP）；1 = 存在失败（含必装工具缺失）。
function Write-CiSummary {
    param([string]$LogDir = "")
    New-CiFailureState
    $fail = @($script:ciFailures)
    $steps = @($script:ciSteps)
    Write-Host ""
    Write-Host ("==== CI 汇总（{0} 步，{1} 失败）====" -f $steps.Count, $fail.Count) -ForegroundColor Cyan
    foreach ($s in $steps) {
        $matches = @($fail | Where-Object { $_.name -eq $s })
        $mark = if ($matches.Count -gt 0) { "X" } else { "v" }
        Write-Host ("  [{0}] {1}" -f $mark, $s)
    }
    foreach ($f in $fail) {
        Write-Host ("  - FAIL {0} : {1}" -f $f.name, $f.detail) -ForegroundColor Red
    }
    $summary = [ordered]@{
        timestamp   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
        ok          = ($fail.Count -eq 0)
        completed   = $true
        interrupted = [bool]$script:ciInterrupted
        resources   = Get-CiResourceState
        steps       = $steps
        failures    = @($fail | ForEach-Object { "{0}: {1}" -f $_.name, $_.detail })
    }
    if ($LogDir) {
        New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
        $summaryJson = $summary | ConvertTo-Json -Depth 5
        $summaryJson | Set-Content -LiteralPath (Join-Path $LogDir "summary.json") -Encoding UTF8
    }
    if ($fail.Count -gt 0) {
        Write-Host "存在失败步骤，退出码 1" -ForegroundColor Red
        exit 1
    }
    Write-Host "全部通过" -ForegroundColor Green
    exit 0
}