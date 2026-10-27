#requires -Version 5.1
<#
init-dev-env.ps1 - Unified development environment initializer (v1 closeout, Lane 1).

Purpose:
  Give a brand-new PowerShell terminal a deterministic, validated dev environment:
  toolchain sanity, ONNX Runtime native dependency (single shared copy for both
  sherpa-onnx and ort/onnx_ocr), optional OCR model presence, and a reproducible
  build-info.json (version + git commit + dirty) that the whole surface
  (server /health, TS client, desktop dev build, eval reports) corresponds to.

Rules (per Lane-1 mandate):
  - Sets PROCESS-SCOPED environment variables only. Never writes user/registry
    env (no setx, no [Environment]::SetEnvironmentVariable(...,'User')).
  - Never commits machine absolute paths; all caches live under a per-machine
    runtime dir (%LOCALAPPDATA%\OwO\Agent\runtime) resolvable at script time.
  - Missing dependencies => clear, actionable error (no reliance on stale
    leftovers from an old terminal / target dir / environment variable).
  - PowerShell 5.1 compatible syntax, ASCII-only comments (same convention as
    run-product-eval-live.ps1).

Usage:
  # dot-source to expose functions and cached state, then call:
  . .\scripts\init-dev-env.ps1
  Initialize-OwoDevEnv -EnsureOrt -EnsureModels

  # or run directly (full flow, good for CI logs):
  pwsh -File scripts\init-dev-env.ps1 [-OrtCacheDir <dir>] [-EnsureModels] [-NoDownload]

Exit codes (direct run): 0 = OK; 2 = missing required dependency (actionable).
#>

[CmdletBinding()]
param(
    # ⚠ dot-source 参数卫生（与 resolve-ort.ps1 同一条纪律）：dot-source 时
    # param 变量会注入**调用方**作用域——裸名 $EnsureOrt/$NoDownload 曾把
    # ci-gate 自己的 -EnsureOrt 静默重置为 $false（下载开关失效），裸名
    # $Json 类冲突更会让调用方对同名变量的普通赋值触发类型转换异常
    # （PowerShell 变量名不区分大小写、首次绑定定型）。内部变量一律带
    # $Owo 前缀；CLI 面经 Alias 保持原样（-File 直调/文档口径不变）。
    # Override the shared runtime cache root (default %LOCALAPPDATA%\OwO\Agent\runtime).
    [Alias('OrtCacheDir')]
    [string]$OwoOrtCacheDir,
    # Download the sherpa-onnx prebuilt archive when missing (needs network).
    [Alias('EnsureOrt')]
    [switch]$OwoEnsureOrt,
    # Download OCR models via the existing downloader when missing.
    [Alias('EnsureModels')]
    [switch]$OwoEnsureModels,
    # Never download anything; fail when a required piece is absent.
    [Alias('NoDownload')]
    [switch]$OwoNoDownload
)

# ---------------------------------------------------------------------------
# Pinned facts (validated this round against the official release / ort dist.tsv)
# ---------------------------------------------------------------------------
$script:OwoSherpaVersion = "1.13.5"
$script:OwoSherpaAsset = "sherpa-onnx-v1.13.5-win-x64-static-MT-Release-lib"
# sha256 of the official release archive (k2-fsa/sherpa-onnx v1.13.5 release asset).
$script:OwoSherpaSha256 = "b7080b6f470bac96ef0afe56b25ae9b2f9f0ca82d10dad19bf3a2fc5ffd6cffc"
$script:OwoSherpaUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.5/$($script:OwoSherpaAsset).tar.bz2"
# Expected rust build target (sherpa prebuilt is win-x64).
$script:OwoRequiredTarget = "x86_64-pc-windows-msvc"
# OcR model files + minimum sizes (bytes) validated when -EnsureModels/available.
$script:OwoOcrModels = @(
    @{ Name = "ch_PP-OCRv4_det_infer.onnx"; MinBytes = 3MB },
    @{ Name = "ch_PP-OCRv4_rec_infer.onnx"; MinBytes = 8MB },
    @{ Name = "ppocr_keys_v1.txt"; MinBytes = 10KB }
)
$script:OwoMinOnnxRuntimeLibBytes = 100MB

# ---------------------------------------------------------------------------
# Paths / state (all computed at runtime; nothing hard-coded to this machine)
# ---------------------------------------------------------------------------
$script:OwoSdkRoot = Split-Path -Parent $PSScriptRoot
$script:OwoRepoRoot = Split-Path -Parent $script:OwoSdkRoot
$script:OwoRuntimeCache = if ($OwoOrtCacheDir) { (Join-Path (Resolve-Path $OwoOrtCacheDir -ErrorAction SilentlyContinue).Path $script:OwoSherpaVersion) } else {
    $base = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "OwO\Agent\runtime" } else { Join-Path $env:TEMP "owo-runtime" }
    Join-Path $base $script:OwoSherpaVersion
}
$script:OwoSherpaLibDir = Join-Path $script:OwoRuntimeCache "$($script:OwoSherpaAsset)\lib"
$script:OwoBuildInfoPath = Join-Path $script:OwoSdkRoot "build-info.json"
$script:OwoModelsDir = Join-Path $script:OwoSdkRoot "models\ocr"

# Persist resolved values for later phases in the same process.
function Get-OwoSdkRoot { return $script:OwoSdkRoot }
function Get-OwoBuildInfoPath { return $script:OwoBuildInfoPath }
function Get-OwoSherpaLibDir { return $script:OwoSherpaLibDir }

# ---------------------------------------------------------------------------
# Toolchain sanity
# ---------------------------------------------------------------------------
function Assert-OwoToolchain {
    foreach ($tool in @('git', 'cargo', 'rustc')) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            throw "Required tool missing: $tool (install Rust toolchain via rustup and ensure git is on PATH)."
        }
    }
    $hostTriple = (rustc -vV) 2>$null | Select-String -Pattern '^host:\s*(.+)$' | ForEach-Object { $_.Matches.Groups[1].Value }
    if ($hostTriple -ne $script:OwoRequiredTarget) {
        throw "Unsupported build target: '$hostTriple'. Expected '$($script:OwoRequiredTarget)' (sherpa-onnx prebuilt is win-x64 only)."
    }
    Write-Host "[init] toolchain OK: $hostTriple" -ForegroundColor Green
}

# ---------------------------------------------------------------------------
# ONNX Runtime native dependency (shared by sherpa-onnx and ort-sys)
# ---------------------------------------------------------------------------
function Test-OwoOnnxRuntimeLib {
    param([string]$LibDir)
    if (-not $LibDir -or -not (Test-Path $LibDir)) { return $false }
    $lib = Join-Path $LibDir "onnxruntime.lib"
    if (-not (Test-Path $lib)) { return $false }
    return (Get-Item $lib).Length -ge $script:OwoMinOnnxRuntimeLibBytes
}

function Save-OwoSherpaToCache {
    # Download the official archive, verify sha256, extract lib/ into the cache.
    param([string]$ArchivePath)
    Write-Host "[init] downloading ONNX Runtime (sherpa-onnx $($script:OwoSherpaVersion) win-x64 static)..." -ForegroundColor Cyan
    Write-Host "[init]   $($script:OwoSherpaUrl)"
    Invoke-WebRequest -Uri $script:OwoSherpaUrl -OutFile $ArchivePath -UseBasicParsing
    $hash = (Get-FileHash $ArchivePath -Algorithm SHA256).Hash.ToLower()
    if ($hash -ne $script:OwoSherpaSha256) {
        Remove-Item $ArchivePath -Force -ErrorAction SilentlyContinue
        throw "ONNX Runtime archive integrity check FAILED (sha256 $hash != expected $($script:OwoSherpaSha256)). Refusing to use an unverified binary."
    }
    Write-Host "[init] archive sha256 verified" -ForegroundColor Green
    $extractRoot = Split-Path $ArchivePath -Parent
    if (-not (Get-Command tar -ErrorAction SilentlyContinue)) {
        throw "tar is required to extract the prebuilt archive (part of Windows 10+ / Git for Windows)."
    }
    # Windows 10+ tar handles .tar.bz2; extract into cache dir.
    Push-Location $extractRoot
    try {
        tar -xjf $ArchivePath -C $extractRoot | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "tar extraction failed with code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
    if (-not (Test-OwoOnnxRuntimeLib (Join-Path $extractRoot "$($script:OwoSherpaAsset)\lib"))) {
        throw "Extracted archive does not contain a valid onnxruntime.lib; cache is corrupt."
    }
}

function Get-OwoOrtLibDir {
    # Single non-throwing ORT probe (audit 6.2): session env -> machine cache ->
    # legacy target cache. Returns $null when absent; callers decide whether to
    # warn / fail / download. All other scripts MUST reuse this instead of
    # keeping their own copies of the probe logic.
    $envLib = if ($env:SHERPA_ONNX_LIB_DIR) { $env:SHERPA_ONNX_LIB_DIR } else { $env:ORT_LIB_PATH }
    if (Test-OwoOnnxRuntimeLib $envLib) { return $envLib }
    $cached = Join-Path $script:OwoRuntimeCache "$($script:OwoSherpaAsset)\lib"
    if (Test-OwoOnnxRuntimeLib $cached) { return $cached }
    $legacy = Join-Path $script:OwoSdkRoot "target\sherpa-onnx-prebuilt\$($script:OwoSherpaAsset)\lib"
    if (Test-OwoOnnxRuntimeLib $legacy) { return $legacy }
    return $null
}

function Resolve-OwoOnnxRuntimeLib {
    # Priority: session-provided -> machine cache -> legacy target cache -> download.
    param([switch]$AllowDownload)
    $probed = Get-OwoOrtLibDir
    if ($probed) {
        Write-Host "[init] ONNX Runtime: resolved ($probed)" -ForegroundColor Green
        return $probed
    }
    # Download (explicit or implied).
    if ($AllowDownload -or $EnsureOrt) {
        if ($NoDownload) {
            throw "ONNX Runtime dependency missing and downloads are disabled (-NoDownload). Run: pwsh -File scripts\init-dev-env.ps1 -EnsureOrt"
        }
        New-Item -ItemType Directory -Force -Path $script:OwoRuntimeCache | Out-Null
        $archive = Join-Path $script:OwoRuntimeCache "$($script:OwoSherpaAsset).tar.bz2"
        Save-OwoSherpaToCache $archive
        $cached = Get-OwoOrtLibDir
        if (Test-OwoOnnxRuntimeLib $cached) { return $cached }
        throw "Download succeeded but the extracted lib is invalid; remove '$($script:OwoRuntimeCache)' and retry."
    }
    throw "ONNX Runtime dependency not found. Obtain the official sherpa-onnx v$($script:OwoSherpaVersion) win-x64 static prebuilt package (or run: pwsh -File scripts\init-dev-env.ps1 -EnsureOrt), then either place it under '$($script:OwoRuntimeCache)' or set SHERPA_ONNX_LIB_DIR / ORT_LIB_PATH to its lib directory. Do NOT rely on a leftover from an old terminal."
}

# ---------------------------------------------------------------------------
# OCR models (optional for builds; required for OCR feature/tests)
# ---------------------------------------------------------------------------
function Test-OwoOcrModels {
    param([string]$Dir)
    if (-not $Dir -or -not (Test-Path $Dir)) { return $false }
    foreach ($m in $script:OwoOcrModels) {
        $f = Join-Path $Dir $m.Name
        if (-not (Test-Path $f)) { return $false }
        if ((Get-Item $f).Length -lt $m.MinBytes) { return $false }
    }
    return $true
}

function Resolve-OwoOcrModels {
    if (Test-OwoOcrModels $script:OwoModelsDir) {
        Write-Host "[init] OCR models: present ($($script:OwoModelsDir))" -ForegroundColor Green
        return $true
    }
    if ($EnsureModels) {
        Write-Host "[init] OCR models missing; downloading via download-onnx-ocr-models.ps1..." -ForegroundColor Cyan
        & (Join-Path $PSScriptRoot "download-onnx-ocr-models.ps1")
        if (Test-OwoOcrModels $script:OwoModelsDir) {
            Write-Host "[init] OCR models downloaded and validated" -ForegroundColor Green
            return $true
        }
        throw "OCR model download failed validation; check network to GitHub/Gitee."
    }
    Write-Host "[init] OCR models missing (optional for builds; OCR feature disabled). Enable via: pwsh -File scripts\init-dev-env.ps1 -EnsureModels" -ForegroundColor Yellow
    return $false
}

# ---------------------------------------------------------------------------
# Release gate (audit 6.1.5): release entries refuse a dirty work tree
# ---------------------------------------------------------------------------
function Assert-OwoCleanTree {
    # Called by release entry points (package-desktop / build-installer).
    # Explicit override: OWO_ALLOW_DIRTY_RELEASE=1 (recorded in the log).
    param([switch]$AllowDirty)
    if ($AllowDirty) { return }
    if ($env:OWO_ALLOW_DIRTY_RELEASE -eq "1") {
        Write-Host "[release] OWO_ALLOW_DIRTY_RELEASE=1: clean-tree gate skipped (recorded)" -ForegroundColor Yellow
        return
    }
    Push-Location $script:OwoRepoRoot
    # §7.3 统一口径（与 owo-build-info/build.rs、release manifest 一致）：
    # dirty 作用域 = agent-sdk/（构建相关树）；仓根个人文档/素材等
    # 与构建无关的 untracked 资产不拦截发布。
    try { $status = @(git status --porcelain -uall -- agent-sdk 2>$null) } finally { Pop-Location }
    if ($status.Count -gt 0) {
        throw ("Release build refuses a dirty work tree (audit 6.1.5): {0} uncommitted/untracked entries under agent-sdk/. Commit all changes first, or set OWO_ALLOW_DIRTY_RELEASE=1 to override explicitly." -f $status.Count)
    }
    Write-Host "[release] clean-tree gate passed" -ForegroundColor Green
}

# ---------------------------------------------------------------------------
# Version / build info (single source: workspace Cargo.toml)
# ---------------------------------------------------------------------------
function Get-OwoAppVersion {
    $cargo = Join-Path $script:OwoSdkRoot "Cargo.toml"
    $line = Get-Content $cargo | Select-String -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if (-not $line) { throw "workspace version not found in $cargo" }
    return $line.Matches.Groups[1].Value
}

function Write-OwoBuildInfo {
    pushd $script:OwoRepoRoot
    try {
        $commit = (git rev-parse --short=12 HEAD 2>$null | Out-String).Trim()
        $dirty = (git status --porcelain -uall -- agent-sdk 2>$null | Measure-Object).Count -gt 0
    } finally {
        popd
    }
    $info = [ordered]@{
        app_version  = Get-OwoAppVersion
        git_commit   = if ($commit) { $commit } else { "unknown" }
        git_dirty    = [bool]$dirty
        built_at     = (Get-Date).ToUniversalTime().ToString("o")
        rust_target  = $script:OwoRequiredTarget
        onnx_runtime = "$($script:OwoSherpaAsset) sha256:$($script:OwoSherpaSha256)"
    }
    # BOM-less UTF-8：PS 5.1 的 Set-Content -Encoding UTF8 会带 BOM，
    # serde_json 读取时会在第 1 列报 expected value（BOM 不是空白）。
    [IO.File]::WriteAllText($script:OwoBuildInfoPath, ($info | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    Write-Host "[init] build-info.json written: v$($info.app_version) @$($info.git_commit) dirty=$($info.git_dirty)" -ForegroundColor Green
    return $info
}

# ---------------------------------------------------------------------------
# Main flow
# ---------------------------------------------------------------------------
function Initialize-OwoDevEnv {
    [CmdletBinding()]
    param(
        [switch]$EnsureOrt,
        [switch]$EnsureModels,
        [switch]$NoDownload
    )
    Assert-OwoToolchain
    $libDir = Resolve-OwoOnnxRuntimeLib -AllowDownload:($EnsureOrt -and -not $NoDownload)
    # Export session-scoped only.
    $env:SHERPA_ONNX_LIB_DIR = $libDir
    $env:ORT_LIB_PATH = $libDir
    $env:ORT_LIB_LOCATION = $libDir
    Write-Host "[init] exported SHERPA_ONNX_LIB_DIR / ORT_LIB_PATH / ORT_LIB_LOCATION = $libDir" -ForegroundColor Cyan
    $null = Resolve-OwoOcrModels
    return Write-OwoBuildInfo
}

# Direct run: execute the full flow.
if ($MyInvocation.InvocationName -ne ".") {
    try {
        $null = Initialize-OwoDevEnv -EnsureModels:$OwoEnsureModels -NoDownload:(($OwoNoDownload) -or (-not $OwoEnsureOrt))
        Write-Host "[init] init-dev-env OK. Repo: $script:OwoSdkRoot" -ForegroundColor Green
        exit 0
    } catch {
        Write-Host "[init] ERROR: $($_.Exception.Message)" -ForegroundColor Red
        exit 2
    }
}