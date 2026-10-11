# ci-gate.ps1 — agent-sdk CI 核心门禁（PR/Merge 工作流与本地验证共用；幂等、只读为主）
# 步骤顺序：agent-boundary → utf8 → permission-ctor → forbidden-files → fmt → clippy → test →
#          route-contract → node → ts → desktop(src-tauri: fmt/clippy/test) → desktop-selftest
#          → resource-policy-selftest
# R3-D（指南 §3.6）：长步骤经 Invoke-CiLoggedCommand/Invoke-CiCargo 流式执行——每 30s
#          心跳报已运行时长、每步独立超时（超时错误码 124=TIMEOUT）、输出逐行 tee 到
#          LogDir；summary.json 每步增量落盘（running/passed/failed），被中止写
#          interrupted=true。只改善可观察性——不缩短测试、不吞失败。
# §2.4 资源红线（强制）：所有 Rust 步骤经 Invoke-CiCargo——
#          · 完整 workspace（clippy/test/desktop-clippy/desktop-test）自动 strict = -j 1 /
#            --test-threads 1；定向步骤（route-contract、单包 test）normal = -j 2 / 2；
#          · 禁止依赖 Cargo 默认 32 路并发（脚本显式注入，调用方给 >2 直接拒绝）；
#          · 启动前内存门（可用 <6 GB 或已用 ≥80% 拒绝）+ 构建空闲门（已有
#            cargo/rustc/link 时等待，不并发第二组）；运行中心跳复查内存，越线即终止
#            进程树并以退出码 137（resource_limited）收口；
#          · 生效的限制写入 summary.json 的 resources 段（含 guard_events）。
# 约定：块内通过退出码或 $global:LASTEXITCODE = 1 表达失败（避免 throw 吞掉已捕获输出）；
#       前置条件缺失用 return 实现受控跳过（绿色 + SKIP 说明）。
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts\ci-gate.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\ci-gate.ps1 -Step fmt
#   powershell -ExecutionPolicy Bypass -File scripts\ci-gate.ps1 -Step clippy -LogDir "$env:TEMP\ci-gate-logs"
# 参数：
#   -Step <词>           只运行 Id 包含该词的步骤（CI 拆分步骤与本地排查用；-Step desktop 跑壳三步）
#   -SkipFmt/-SkipClippy/-SkipTest/-SkipNode/-SkipWebTests/-SkipTsUnit/-SkipTs/-SkipRouteContract/-SkipUtf8/-SkipPermissionCtor/-SkipDesktop/-SkipDesktopSelftest/-SkipResourceSelftest
#   -ServerOnly          workspace 测试退化为只测 owo-agent-server 单包
#   -LogDir <目录>       把每步输出与 summary.json 落盘（诊断 artifact）
#   -HeartbeatSec <秒>   长步骤心跳间隔（默认 30，R3-D）
#   -ResourceMode <档>   §2.4 资源红线默认档：auto（按步骤自动：全量=strict/定向=normal）
#                        或强制 normal / strict（strict 只会更保守，绝不会被用来提速）
# 退出码：0 = 全过；1 = 存在失败步骤；124 = 某步超时；130 = 被中止；
#         137 = §2.4 内存守护触发（resource_limited）；2 = 前置条件（ORT/资源红线）不满足。
param(
    [string]$Step = "",
    [switch]$SkipFmt,
    [switch]$SkipClippy,
    [switch]$SkipTest,
    [switch]$SkipNode,
    [switch]$SkipWebTests,
    [switch]$SkipTsUnit,
    [switch]$SkipTs,
    [switch]$SkipRouteContract,
    [switch]$SkipUtf8,
    [switch]$SkipPermissionCtor,
    # R3：桌面壳（src-tauri 独立 workspace）三步；本地快速跑主仓时可跳过。
    [switch]$SkipDesktop,
    # R3-D：桌面验收原语负例自测（纯离线合成图，秒级）；--Step desktop-selftest 单跑。
    [switch]$SkipDesktopSelftest,
    # §2.4.4：资源红线脚本层自测（纯离线，秒级）。
    [switch]$SkipResourceSelftest,
    [switch]$ServerOnly,
    # §7.2：新终端直跑 CI 时允许把缺失的 ORT 下载进稳定缓存（默认保守：
    # 探测失败即快速报错，不等链接阶段 LNK1120）。
    [switch]$EnsureOrt,
    [int]$HeartbeatSec = 30,
    [ValidateSet('auto', 'normal', 'strict')][string]$ResourceMode = 'auto',
    [string]$LogDir = ""
)

$ErrorActionPreference = "Continue"
. (Join-Path $PSScriptRoot "ci-shared.ps1")
Initialize-CiPath

# §2.4 资源红线前置门（在任何 Rust 步骤之前）：内存不足或已有构建在跑时直接拒绝，
# 不留下"跑了一半把机器拖死"的中间态；结论与限制一律落进 summary.json 的 resources 段。
try {
    $preflightMode = if ($ResourceMode -eq 'auto') { 'normal' } else { $ResourceMode }
    $limits = Set-CiRustResourcePolicy -Mode $preflightMode -Source 'ci-gate-preflight'
    Write-Host ("[§2.4] 资源红线：默认档 {0}（编译 {1} 路 / 测试 {2} 线程；全量步骤自动降为 1）" -f `
            $limits.mode, $limits.jobs, $limits.test_threads) -ForegroundColor Cyan
    $mem = Get-CiMemoryStatus
    Write-Host ("[§2.4] 内存：status={0} total={1}GB free={2}GB used={3}%（要求可用 ≥{4}GB 且已用 <{5}%）" -f `
            $mem.status, $mem.total_gb, $mem.free_gb, $mem.used_percent, $mem.gate.min_free_gb, $mem.gate.max_used_percent)
    $null = Assert-CiMemoryGate -Context 'ci-gate'
    $null = Assert-CiBuildIdle -Context 'ci-gate' -WaitSec 60 -PollSec 10
} catch {
    Write-Host ("[§2.4] 资源红线拒绝启动本轮门禁：{0}" -f $_.Exception.Message) -ForegroundColor Red
    if ($LogDir) {
        New-Item -ItemType Directory -Force -Path $LogDir | Out-Null
        [ordered]@{
            timestamp  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
            ok         = $false
            completed  = $true
            interrupted = $false
            blocked_by = 'resource_redline'
            reason     = $_.Exception.Message
            resources  = Get-CiResourceState
            steps      = @()
            failures   = @("preflight: $($_.Exception.Message)")
        } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $LogDir "summary.json") -Encoding UTF8
    }
    exit 2
}

# §7.2：ORT 统一解析入口（进程级注入；与 dev/package/build-installer 同源）。
. (Join-Path $PSScriptRoot "resolve-ort.ps1")
try {
    $null = Resolve-OwoOrtEnv -EnsureOrt:$EnsureOrt -NoDownload:(-not $EnsureOrt) -Quiet
} catch {
    Write-Host "[ci] ONNX Runtime 解析失败：$($_.Exception.Message)" -ForegroundColor Red
    exit 2
}

$script:ciStepFilter = $Step.ToLowerInvariant()
$root = Get-CiRepoRoot

# 0.1) 根目录 manifest 防回潮（指南 F-15）：agent-sdk 是唯一 Cargo workspace，
#      仓库根不允许出现会劫持 Cargo 查找的临时 Cargo.toml。
Invoke-CiStep -Name "根目录 Cargo manifest 清理检查（F-15）" -Id "root-manifest" -Cwd $root -LogDir $LogDir -Block {
    $repoParent = Split-Path -Parent $root
    $rootManifest = Join-Path $repoParent "Cargo.toml"
    if (Test-Path -LiteralPath $rootManifest) {
        Write-Host "    [root-manifest] 禁止的仓库根 Cargo.toml：$rootManifest" -ForegroundColor Red
        $global:LASTEXITCODE = 1
    } else {
        Write-Host "    [root-manifest] 通过：仓库根无 Cargo.toml，workspace=$root"
        $global:LASTEXITCODE = 0
    }
}

# 0.2) Agent / historical IME boundary. This also covers aliased target dependencies.
Invoke-CiStep -Name "Agent workspace boundary" -Id "agent-boundary" -Cwd $root -LogDir $LogDir -Block {
    Invoke-CiLoggedCommand -Exe "python" -Arguments @('scripts/check-agent-boundary.py', '--self-test') `
        -Cwd $root -TimeoutSec 60 -HeartbeatSec $HeartbeatSec -Label 'agent-boundary'
}

# 0) UTF-8 扫描（AGENTS.md 硬性要求：源文件 UTF-8；.ps1 需带 UTF-8 BOM）
if (-not $SkipUtf8) {
    Invoke-CiStep -Name "UTF-8 校验（rs/ts/js/html/json/css/md/ps1）" -Id "utf8" -Cwd $root -LogDir $LogDir -Block {
        $bad = @()
        $exts = @(".rs", ".ts", ".js", ".html", ".json", ".css", ".md", ".ps1")
        $files = Get-ChildItem $root -Recurse -File |
            Where-Object { $_.FullName -notmatch "\\target\\" -and $_.FullName -notmatch "\\node_modules\\" -and $_.FullName -notmatch "\\.git\\" -and $_.FullName -notmatch "\\scratch-" } |
            Where-Object { $exts -contains $_.Extension }
        $strict = New-Object System.Text.UTF8Encoding($false, $true)
        foreach ($f in $files) {
            try {
                $bytes = [System.IO.File]::ReadAllBytes($f.FullName)
                $null = $strict.GetString($bytes)
                if ($f.Extension -eq ".ps1" -and -not ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF)) {
                    $bad += "$($f.FullName)（.ps1 需带 UTF-8 BOM）"
                }
            } catch {
                $bad += "$($f.FullName)（$($_.Exception.Message)）"
            }
        }
        Write-Host ("    [utf8] scanned={0} bad={1}" -f $files.Count, $bad.Count)
        if ($bad.Count -gt 0) {
            $badList = $bad | Sort-Object -Unique
            Write-Host "    UTF-8 违规文件：" -ForegroundColor Red
            foreach ($b in $badList) {
                Write-Host ("      - {0}" -f $b) -ForegroundColor Red
                Write-Output $b
            }
            $global:LASTEXITCODE = 1
        } else {
            $global:LASTEXITCODE = 0
        }
    }
}

# 0.5) PermissionRequest 构造检查（重构方案 3.1：业务模块禁止直接拼装 request_id；
#      除 permissions.rs 与协议类型定义外一律经 PermissionRequest::new 创建））
if (-not $SkipPermissionCtor) {
    Invoke-CiStep -Name "构造检查（PermissionRequest { 只允许出现在 permissions.rs）" -Id "permission-ctor" -Cwd $root -LogDir $LogDir -Block {
        # 合法出现点：permissions.rs（定义/求值）、owo-agent-protocol（协议类型定义）、
        # `-> PermissionRequest {`（返回类型）、`SseEvent::PermissionRequest {`（模式匹配）。
        $allowed = @(
            (Join-Path $root "crates\owo-agent-core\src\permissions.rs"),
            # M12：权限策略已下沉到独立 policy crate；该文件保留 PermissionRequest
            # 的唯一业务构造点，不能被旧 core-only 白名单误报。
            (Join-Path $root "crates\owo-agent-policy\src\permissions.rs"),
            (Join-Path $root "crates\owo-agent-protocol\src\lib.rs")
        )
        $violations = @()
        $rsFiles = Get-ChildItem (Join-Path $root "crates") -Recurse -Filter *.rs |
            Where-Object { $_.FullName -notmatch "\\target\\" }
        foreach ($f in $rsFiles) {
            if ($allowed -contains $f.FullName) { continue }
            $lineno = 0
            foreach ($line in [System.IO.File]::ReadAllLines($f.FullName)) {
                $lineno++
                if ($line -match "PermissionRequest\s*\{") {
                    # 返回类型与模式匹配不是字面量构造，跳过（防误报）。
                    if ($line -match "->\s*PermissionRequest\s*\{" -or $line -match "::\s*PermissionRequest\s*\{") { continue }
                    $violations += "$($f.FullName):$lineno"
                }
            }
        }
        Write-Host ("    [permission-ctor] scanned={0} violations={1}" -f $rsFiles.Count, $violations.Count)
        if ($violations.Count -gt 0) {
            Write-Host "    业务模块直接构造 PermissionRequest（应改用构造函数）：" -ForegroundColor Red
            foreach ($v in $violations) {
                Write-Host ("      - {0}" -f $v) -ForegroundColor Red
                Write-Output $v
            }
            $global:LASTEXITCODE = 1
        } else {
            $global:LASTEXITCODE = 0
        }
    }
}

# 0.6) forbidden-files 检查（重构方案 10.6：拒绝编辑器 workspace、密钥文件、日志、
#      临时数据库与构建缓存进入源码目录；误放文件由人工移出，本步骤防止回潮；已忽略运行产物跳过）
if (-not $SkipPermissionCtor) {
    Invoke-CiStep -Name "forbidden-files（.code-workspace/.log/.db+密钥/构建缓存不得进源码目录）" -Id "forbidden-files" -Cwd $root -LogDir $LogDir -Block {
        $forbidden = @()
        $allFiles = Get-ChildItem $root -Recurse -File |
            Where-Object { $_.FullName -notmatch "\\target\\" -and $_.FullName -notmatch "\\node_modules\\" -and $_.FullName -notmatch "\\.git\\" -and $_.FullName -notmatch "\\scratch-" }
        # 源码扩展名：文件名含 credentials 之类是合法模块命名，不做密钥匹配；
        # 但 .json/.ps1/.md 等仍可能承载真实密钥（保持检查）。
        $sourceExts = @(".rs", ".ts", ".js", ".css", ".html")
        $skipped = 0
        foreach ($f in $allFiles) {
            $name = $f.Name
            # PS 5.1 无 [IO.Path]::GetRelativePath：手动算根相对路径（大小写不敏感）。
            $full = $f.FullName
            $rel = $full
            if ($full.StartsWith($root, [System.StringComparison]::OrdinalIgnoreCase)) {
                $rel = $full.Substring($root.Length).TrimStart('\', '/')
            }
            # 被 git 忽略的运行产物（sim/logs、.owo-agent/、基线/门禁日志等）不在源码目录。
            git -C $root check-ignore -q $full 2>$null
            if ($LASTEXITCODE -eq 0) { $skipped++; continue }
            $lower = $name.ToLowerInvariant()
            # docs/qa/evidence/** 是 §8 验收要求的**归档证据**位置（截图 + 真机 core 日志），
            # 不是源码目录：.log 规则对它放行；密钥/数据库/构建缓存三条规则仍全程生效
            # （证据里出现 .db 或密钥文件照样红，这不是可以绕的口子）。
            $isEvidence = $rel -match '^docs[\\/]qa[\\/]evidence[\\/]'
            if ($lower -match "\.code-workspace$") { $forbidden += "$rel（编辑器 workspace 误入源码）" }
            elseif (($lower -match "\.log$" -or $lower -match "\.log\.\d+$") -and -not $isEvidence) { $forbidden += "$rel（日志文件误入源码）" }
            elseif ($lower -match "\.(db|sqlite|sqlite3|db-wal|db-shm)$") { $forbidden += "$rel（临时数据库误入源码）" }
            elseif ($sourceExts -notcontains $f.Extension -and $lower -match "(api[_-]?key|secret|\.pem|\.pfx|credential|id_rsa|id_ed25519)") { $forbidden += "$rel（疑似密钥/凭据文件）" }
        }
        Write-Host ("    [forbidden-files] scanned={0} skipped-ignored={1} violations={2}" -f $allFiles.Count, $skipped, $forbidden.Count)
        if ($forbidden.Count -gt 0) {
            Write-Host "    forbidden 文件：" -ForegroundColor Red
            foreach ($ff in $forbidden) {
                Write-Host ("      - {0}" -f $ff) -ForegroundColor Red
                Write-Output $ff
            }
            $global:LASTEXITCODE = 1
        } else {
            $global:LASTEXITCODE = 0
        }
    }
}

# 1) fmt
if (-not $SkipFmt) {
    Invoke-CiStep -Name "cargo fmt --all -- --check" -Id "fmt" -Cwd $root -LogDir $LogDir -Block {
        $log = if ($LogDir) { Join-Path $LogDir "fmt.stream.log" } else { "" }
        Invoke-CiCargo -Arguments @('fmt', '--all', '--', '--check') `
            -Cwd $root -TimeoutSec 600 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'fmt' -PolicyMode 'normal'
    }
}

# 2) clippy（workspace 全量；--locked 保证与 Cargo.lock 一致）
#    §2.4 红线 2：完整 workspace lint 属 strict 档（-j 1），不得为省时间放宽。
if (-not $SkipClippy) {
    Invoke-CiStep -Name "cargo clippy --workspace --all-targets --locked -- -D warnings" -Id "clippy" -Cwd $root -LogDir $LogDir -Block {
        $log = if ($LogDir) { Join-Path $LogDir "clippy.stream.log" } else { "" }
        Invoke-CiCargo `
            -Arguments @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings') `
            -Cwd $root -TimeoutSec 7200 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'clippy' -PolicyMode 'strict'
    }
}

# 3) test（默认 workspace 全量；-ServerOnly 时只测 owo-agent-server 单包）
#    §2.4 红线 2：全量测试 strict；红线 1：单包定向测试 normal。
if (-not $SkipTest) {
    $testName = if ($ServerOnly) { "cargo test -p owo-agent-server --locked" } else { "cargo test --workspace --locked" }
    $testId = if ($ServerOnly) { "test-server" } else { "test" }
    $testArgs = if ($ServerOnly) { @('test', '-p', 'owo-agent-server', '--locked') } else { @('test', '--workspace', '--locked') }
    $testPolicy = if ($ServerOnly) { 'normal' } else { 'strict' }
    Invoke-CiStep -Name $testName -Id $testId -Cwd $root -LogDir $LogDir -Block {
        $log = if ($LogDir) { Join-Path $LogDir "test.stream.log" } else { "" }
        Invoke-CiCargo -Arguments $testArgs `
            -Cwd $root -TimeoutSec 10800 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'test' -PolicyMode $testPolicy
    }
}

# 4) 路由契约（HTTP 契约面，AGENTS.md 要求同步 route_contract_tests）
if (-not $SkipRouteContract) {
    Invoke-CiStep -Name "cargo test -p owo-agent-server --test route_contract_tests --locked" -Id "route-contract" -Cwd $root -LogDir $LogDir -Block {
        $log = if ($LogDir) { Join-Path $LogDir "route-contract.stream.log" } else { "" }
        Invoke-CiCargo `
            -Arguments @('test', '-p', 'owo-agent-server', '--test', 'route_contract_tests', '--locked') `
            -Cwd $root -TimeoutSec 2400 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'route-contract' -PolicyMode 'normal'
    }
}

# 5) Node 语法检查（app.js + core/*.js + views/*.view.js + 全部 panel）
if (-not $SkipNode) {
    Invoke-CiStep -Name "node --check（app.js + core/*.js + views/*.view.js + panels/*.panel.js）" -Id "node" -Cwd $root -LogDir $LogDir -Block {
        $jsFiles = @(Join-Path $root "desktop\web\app.js")
        $coreJs = Get-ChildItem (Join-Path $root "desktop\web\core") -Filter *.js -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
        if ($coreJs) { $jsFiles += @($coreJs) }
        $views = Get-ChildItem (Join-Path $root "desktop\web\views") -Filter *.view.js -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
        if ($views) { $jsFiles += @($views) }
        $panels = Get-ChildItem (Join-Path $root "desktop\web\panels") -Filter *.panel.js -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
        if ($panels) { $jsFiles += @($panels) }
        Write-Host ("    [node] files={0}" -f $jsFiles.Count)
        $bad = $false
        foreach ($f in $jsFiles) {
            node --check $f
            if ($global:LASTEXITCODE -ne 0) { $bad = $true }
        }
        if ($bad) { $global:LASTEXITCODE = 1 } else { $global:LASTEXITCODE = 0 }
    }
}

# 5b) 桌面 web 契约测试（node --test）。
#     历史缺口：R3.3 报告写"web node 301 通过"，但门禁里没有这一步——只有
#     `node --check`（语法）。语法绿 ≠ 契约绿：断连卡动作表、错误码归因、首屏
#     请求预算这些契约全靠 node --test 断言，一旦漂移无人拦（AGENTS.md：契约测试
#     随功能提交）。§2.4 不适用：纯 JavaScript 不调用 Rust 编译器。
if (-not $SkipWebTests) {
    Invoke-CiStep -Name "桌面 web + Electron 壳契约测试（node --test）" -Id "web-tests" -Cwd $root -LogDir $LogDir -Block {
        $log = if ($LogDir) { Join-Path $LogDir "web-tests.stream.log" } else { "" }
        Invoke-CiLoggedCommand -Exe node -Arguments @('--test', 'desktop/web/tests/*.test.mjs', 'desktop/electron/tests/*.test.mjs') `
            -Cwd $root -TimeoutSec 900 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'web-tests'
    }
}

# 5c) TS 客户端单测（含 /diagnostics/requests 六字段白名单与三桶聚合契约）。
#     此前门禁只做 typecheck（类型面），契约断言从未在门禁里跑过。
if (-not $SkipTsUnit) {
    Invoke-CiStep -Name "TS 客户端单测（clients\ts npm run test:unit）" -Id "ts-unit" -Cwd $root -LogDir $LogDir -Block {
        $tsDir = Join-Path $root "clients\ts"
        $log = if ($LogDir) { Join-Path $LogDir "ts-unit.stream.log" } else { "" }
        if (-not (Test-Path (Join-Path $tsDir "node_modules\typescript"))) {
            Invoke-CiLoggedCommand -Exe npm.cmd -Arguments @('ci', '--ignore-scripts') `
                -Cwd $tsDir -TimeoutSec 900 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'npm-ci'
            if ($global:LASTEXITCODE -ne 0) { return }
        }
        Invoke-CiLoggedCommand -Exe npm.cmd -Arguments @('run', 'test:unit') `
            -Cwd $tsDir -TimeoutSec 900 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'ts-unit'
    }
}

# 6) TS 类型检查（clients/ts；package-lock.json 锁定依赖）
if (-not $SkipTs) {
    Invoke-CiStep -Name "TS 类型检查（clients\ts npm run typecheck）" -Id "ts" -Cwd $root -LogDir $LogDir -Block {
        $tsDir = Join-Path $root "clients\ts"
        $log = if ($LogDir) { Join-Path $LogDir "ts.stream.log" } else { "" }
        if (-not (Test-Path (Join-Path $tsDir "node_modules\typescript"))) {
            Invoke-CiLoggedCommand -Exe npm.cmd -Arguments @('ci', '--ignore-scripts') `
                -Cwd $tsDir -TimeoutSec 900 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'npm-ci'
            if ($global:LASTEXITCODE -ne 0) { return }
        }
        Invoke-CiLoggedCommand -Exe npm.cmd -Arguments @('run', 'typecheck') `
            -Cwd $tsDir -TimeoutSec 600 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'tsc'
    }
}

# 7) 桌面壳（src-tauri 独立 workspace）：stage → fmt → clippy → test
#    R3（§8.1/§8.2）补位：桌面壳此前不在任何门禁内——R2 把 OWO_API_VERSION 改为
#    re-export 后，壳的 build.rs 文本抓取静默失效（壳根本编译不过）却无人报警。
#    冷启动/错误恢复的真实验收必须有壳侧编译与单测护栏。
if (-not $SkipDesktop) {
    $shellDir = Join-Path $root "desktop\tauri\src-tauri"
    $shellPresent = Test-Path (Join-Path $shellDir "Cargo.toml")
    if (-not $shellPresent) {
        Write-Host "    [desktop] 无 src-tauri Cargo.toml，跳过壳侧四步"
    }
    Invoke-CiStep -Name "桌面壳随包 core 预置（externalBin 命名+身份+SHA-256）" -Id "desktop-stage" -Cwd $root -LogDir $LogDir -Block {
        if (-not $shellPresent) { $global:LASTEXITCODE = 0; return }
        # Tauri 硬要求 binaries/owo-agent-<triple>.exe 存在。历史上开发者用手工复制
        # 旧 core 救火，制造了安装包错包与开发期同目录劫持（build-installer 还曾用错
        # 文件名，tauri-build 根本不读）。门禁内自动预置 = 每次都校验该产物来自当前
        # 源码、命名正确、哈希一致，等价于把那条门禁视野外的隐藏路径纳入门禁。
        . (Join-Path $PSScriptRoot "stage-desktop-sidecar.ps1")
        try {
            $null = Stage-OwoDesktopSidecar -Configuration debug
            $global:LASTEXITCODE = 0
        } catch {
            Write-Host "    [desktop-stage] $($_.Exception.Message)"
            $global:LASTEXITCODE = 1
        }
    }
    Invoke-CiStep -Name "桌面壳 cargo fmt --check（src-tauri）" -Id "desktop-fmt" -Cwd $shellDir -LogDir $LogDir -Block {
        if (-not $shellPresent) { $global:LASTEXITCODE = 0; return }
        $log = if ($LogDir) { Join-Path $LogDir "desktop-fmt.stream.log" } else { "" }
        Invoke-CiCargo -Arguments @('fmt', '--check') `
            -Cwd $shellDir -TimeoutSec 300 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'desktop-fmt' -PolicyMode 'normal'
    }
    Invoke-CiStep -Name "桌面壳 cargo clippy --all-targets --locked -- -D warnings（src-tauri）" -Id "desktop-clippy" -Cwd $shellDir -LogDir $LogDir -Block {
        if (-not $shellPresent) { $global:LASTEXITCODE = 0; return }
        $log = if ($LogDir) { Join-Path $LogDir "desktop-clippy.stream.log" } else { "" }
        # 壳 workspace 全量 lint + 链接 tauri/webview 原生依赖 → §2.4 红线 2 strict 档。
        Invoke-CiCargo `
            -Arguments @('clippy', '--all-targets', '--locked', '--', '-D', 'warnings') `
            -Cwd $shellDir -TimeoutSec 7200 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'desktop-clippy' -PolicyMode 'strict'
    }
    Invoke-CiStep -Name "桌面壳 cargo test --locked（src-tauri）" -Id "desktop-test" -Cwd $shellDir -LogDir $LogDir -Block {
        if (-not $shellPresent) { $global:LASTEXITCODE = 0; return }
        $log = if ($LogDir) { Join-Path $LogDir "desktop-test.stream.log" } else { "" }
        Invoke-CiCargo -Arguments @('test', '--locked') `
            -Cwd $shellDir -TimeoutSec 7200 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'desktop-test' -PolicyMode 'strict'
    }
}

# 8) R3-A1：桌面验收原语负例自测（纯离线合成图，秒级；§3.3.2 要求黑图/纯色/失效
#    句柄必须判失败——把"防假通过"本身变成持续回归，不依赖桌面会话）。
if (-not $SkipDesktopSelftest) {
    Invoke-CiStep -Name "桌面验收原语负例自测（test-desktop-acceptance-common.ps1）" -Id "desktop-selftest" -Cwd $root -LogDir $LogDir -Block {
        $log = if ($LogDir) { Join-Path $LogDir "desktop-selftest.stream.log" } else { "" }
        Invoke-CiLoggedCommand -Exe powershell.exe `
            -Arguments @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'test-desktop-acceptance-common.ps1')) `
            -Cwd $root -TimeoutSec 300 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'desktop-selftest'
    }
}

# 9) §2.4.4：资源红线脚本层自测（纯离线，秒级，不启动任何 cargo）。
#    没有这一步，"限制并发的代码写错了"和历史上"验收脚本假通过"是同一类缺陷。
if (-not $SkipResourceSelftest) {
    Invoke-CiStep -Name "§2.4 资源红线脚本层自测（test-ci-shared-resource-policy.ps1）" -Id "resource-selftest" -Cwd $root -LogDir $LogDir -Block {
        $log = if ($LogDir) { Join-Path $LogDir "resource-selftest.stream.log" } else { "" }
        Invoke-CiLoggedCommand -Exe powershell.exe `
            -Arguments @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'test-ci-shared-resource-policy.ps1')) `
            -Cwd $root -TimeoutSec 300 -HeartbeatSec $HeartbeatSec -LogFile $log -Label 'resource-selftest'
    }
}

Write-CiSummary -LogDir $LogDir
