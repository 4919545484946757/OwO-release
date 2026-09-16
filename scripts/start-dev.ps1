[CmdletBinding()]
param(
    [string]$BuildDirectory,
    [string]$X86BuildDirectory,
    [string]$LexiconPath,
    [string]$LibimeRuntimeDirectory,
    [string]$LibimeModelPath,
    [switch]$BuildLatest,
    [switch]$RestartCore,
    [switch]$DisableModelHost,
    [switch]$SkipRegistration,
    [switch]$OpenSettings,
    [switch]$OpenNotepad
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Stop-OwOProcesses {
    param([string[]]$Names)

    foreach ($name in $Names) {
        $processes = @(Get-Process -Name $name -ErrorAction SilentlyContinue)
        foreach ($process in $processes) {
            $processPath = try { $process.Path } catch { '<path unavailable>' }
            Write-Output "Stopping $($process.ProcessName) PID $($process.Id): $processPath"
            Stop-Process -Id $process.Id -Force -ErrorAction Stop
            try { $process.WaitForExit(3000) } catch { }
        }
    }
    $remaining = @(
        foreach ($name in $Names) {
            Get-Process -Name $name -ErrorAction SilentlyContinue
        }
    )
    if ($remaining.Count -ne 0) {
        $details = ($remaining | ForEach-Object { "$($_.ProcessName):$($_.Id)" }) -join ', '
        throw "OwO processes did not stop cleanly: $details"
    }
}

function Assert-ProcessPath {
    param(
        [System.Diagnostics.Process[]]$Processes,
        [string]$ExpectedPath,
        [string]$Label
    )

    $expected = [IO.Path]::GetFullPath($ExpectedPath)
    foreach ($process in $Processes) {
        $actual = try { [IO.Path]::GetFullPath($process.Path) } catch { '' }
        if (-not [string]::Equals($actual, $expected,
                [StringComparison]::OrdinalIgnoreCase)) {
            throw "$Label started from an unexpected path: $actual (expected $expected)"
        }
    }
}

function Get-Sha256Hex {
    param([string]$Path)

    $stream = [IO.File]::OpenRead($Path)
    try {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try {
            return ([BitConverter]::ToString($algorithm.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
        } finally {
            $algorithm.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($BuildDirectory)) {
    $BuildDirectory = Get-ChildItem -LiteralPath (Join-Path $projectRoot 'build') -Directory |
        ForEach-Object { Join-Path $_.FullName 'Release' } |
        Where-Object {
            (Test-Path -LiteralPath (Join-Path $_ 'owo_core_service.exe') -PathType Leaf) -and
            (Test-Path -LiteralPath (Join-Path $_ 'owo_tsf_profile_check.exe') -PathType Leaf) -and
            (Test-Path -LiteralPath (Join-Path $_ 'owo_ipc_shell.exe') -PathType Leaf)
        } |
        Sort-Object { (Get-Item -LiteralPath (Join-Path $_ 'owo_core_service.exe')).LastWriteTime } `
            -Descending |
        Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($BuildDirectory)) {
        throw 'No complete Release development build was found under build/.'
    }
}
$BuildDirectory = [IO.Path]::GetFullPath($BuildDirectory)
if ([string]::IsNullOrWhiteSpace($X86BuildDirectory)) {
    $X86BuildDirectory = Join-Path $projectRoot 'build\windows-x86-release\Release'
}
$X86BuildDirectory = [IO.Path]::GetFullPath($X86BuildDirectory)

if ($BuildLatest) {
    # Stop the installed launcher first so it cannot respawn an installed Core
    # while the development binaries are being linked.
    Stop-OwOProcesses @('owo_runtime_launcher', 'owo_core_service', 'owo_model_host')
    $buildTree = Split-Path -Parent $BuildDirectory
    if (-not (Test-Path -LiteralPath (Join-Path $buildTree 'CMakeCache.txt') -PathType Leaf)) {
        throw "CMake build tree is missing: $buildTree"
    }
    & cmake --build $buildTree --config Release --target `
        owo_tsf owo_core_service owo_model_host owo_model_shell owo_ipc_shell `
        owo_tsf_profile_check owo_runtime_launcher
    if ($LASTEXITCODE -ne 0) {
        throw "OwO Release build failed: $LASTEXITCODE"
    }
    $x86BuildTree = Split-Path -Parent $X86BuildDirectory
    if (-not (Test-Path -LiteralPath (Join-Path $x86BuildTree 'CMakeCache.txt') -PathType Leaf)) {
        & cmake --preset windows-x86-release
        if ($LASTEXITCODE -ne 0) {
            throw "OwO x86 configure failed: $LASTEXITCODE"
        }
    }
    & cmake --build $x86BuildTree --config Release --target owo_tsf
    if ($LASTEXITCODE -ne 0) {
        throw "OwO x86 TSF build failed: $LASTEXITCODE"
    }
}

$coreService = Join-Path $BuildDirectory 'owo_core_service.exe'
$ipcShell = Join-Path $BuildDirectory 'owo_ipc_shell.exe'
$modelHost = Join-Path $BuildDirectory 'owo_model_host.exe'
$modelShell = Join-Path $BuildDirectory 'owo_model_shell.exe'
$profileCheck = Join-Path $BuildDirectory 'owo_tsf_profile_check.exe'
$buildTsfDll = Join-Path $BuildDirectory 'OwO.TSF.dll'
$buildX86TsfDll = Join-Path $X86BuildDirectory 'OwO.TSF.dll'
foreach ($required in @($coreService, $profileCheck, $buildTsfDll)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required development binary is missing: $required"
    }
}
if (-not (Test-Path -LiteralPath $buildX86TsfDll -PathType Leaf)) {
    throw "Required x86 development TSF is missing: $buildX86TsfDll"
}
$deployDirectory = Join-Path $projectRoot 'build/manual-deploy'
[void](New-Item -ItemType Directory -Path $deployDirectory -Force)
$tsfHash = Get-Sha256Hex $buildTsfDll
$tsfDll = Join-Path $deployDirectory "OwO.TSF.$($tsfHash.Substring(0, 16)).dll"
if (-not (Test-Path -LiteralPath $tsfDll -PathType Leaf)) {
    Copy-Item -LiteralPath $buildTsfDll -Destination $tsfDll -Force
}
$x86TsfHash = Get-Sha256Hex $buildX86TsfDll
$x86TsfDll = Join-Path $deployDirectory "OwO.TSF.x86.$($x86TsfHash.Substring(0, 16)).dll"
if (-not (Test-Path -LiteralPath $x86TsfDll -PathType Leaf)) {
    Copy-Item -LiteralPath $buildX86TsfDll -Destination $x86TsfDll -Force
}

foreach ($required in @($tsfDll, $x86TsfDll)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Required development binary is missing: $required"
    }
}

if ([string]::IsNullOrWhiteSpace($LexiconPath)) {
    $LexiconPath = Join-Path $projectRoot `
        'build/windows-release/rime-ice-cn-2026.06.30-v2.owolx'
}
$LexiconPath = [IO.Path]::GetFullPath($LexiconPath)
if (-not (Test-Path -LiteralPath $LexiconPath -PathType Leaf)) {
    throw "Compiled lexicon is missing: $LexiconPath"
}

if ([string]::IsNullOrWhiteSpace($LibimeRuntimeDirectory)) {
    $LibimeRuntimeDirectory = Join-Path $projectRoot `
        'build/dependencies/libime-1.1.15/runtime-x86_64'
}
if ([string]::IsNullOrWhiteSpace($LibimeModelPath)) {
    $LibimeModelPath = Join-Path $projectRoot `
        'build/dependencies/libime-1.1.15/model-20260804/lib/libime/zh_CN.lm'
}
$LibimeRuntimeDirectory = [IO.Path]::GetFullPath($LibimeRuntimeDirectory)
$LibimeModelPath = [IO.Path]::GetFullPath($LibimeModelPath)
$libimeBridge = Join-Path $LibimeRuntimeDirectory 'owo_libime_bridge.dll'
$modelHostEnabled = -not $DisableModelHost -and
    (Test-Path -LiteralPath $modelHost -PathType Leaf) -and
    (Test-Path -LiteralPath $modelShell -PathType Leaf) -and
    (Test-Path -LiteralPath $libimeBridge -PathType Leaf) -and
    (Test-Path -LiteralPath $LibimeModelPath -PathType Leaf)

if (-not $SkipRegistration) {
    & (Join-Path $PSScriptRoot 'register-dev.ps1') -Configuration Release `
        -DllPath $tsfDll -Architecture x64
    & (Join-Path $PSScriptRoot 'register-dev.ps1') -Configuration Release `
        -DllPath $x86TsfDll -Architecture x86
}

& $profileCheck --activate-session
if ($LASTEXITCODE -ne 0) { throw "OwO input profile could not be activated: $LASTEXITCODE" }

$running = @(Get-Process -Name 'owo_core_service' -ErrorAction SilentlyContinue)
$modelRunning = @(Get-Process -Name 'owo_model_host' -ErrorAction SilentlyContinue)
if ($RestartCore -and $running.Count -ne 0) {
    if (Test-Path -LiteralPath $ipcShell -PathType Leaf) {
        try { & $ipcShell --shutdown *> $null } catch { }
        Start-Sleep -Milliseconds 300
    }
    Stop-OwOProcesses @('owo_runtime_launcher', 'owo_core_service')
    $running = @()
}
if ($RestartCore -and $modelRunning.Count -ne 0) {
    if (Test-Path -LiteralPath $modelShell -PathType Leaf) {
        try { & $modelShell --shutdown *> $null } catch { }
        Start-Sleep -Milliseconds 300
    }
    Stop-OwOProcesses @('owo_model_host')
    $modelRunning = @()
}

$dataRoot = Join-Path $env:LOCALAPPDATA 'OwO/InputMethod'
$frequencyDirectory = Join-Path $dataRoot 'data'
$logDirectory = Join-Path $dataRoot 'logs'
$runDirectory = Join-Path $dataRoot 'run'
foreach ($directory in @($frequencyDirectory, $logDirectory, $runDirectory)) {
    [void](New-Item -ItemType Directory -Path $directory -Force)
}
$frequencyPath = Join-Path $frequencyDirectory 'user-frequency.owuf'

if ($modelHostEnabled -and $modelRunning.Count -eq 0) {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $modelStdout = Join-Path $logDirectory "model-$stamp.stdout.log"
    $modelStderr = Join-Path $logDirectory "model-$stamp.stderr.log"
    $modelArguments = @(
        '--libime-bridge', ('"{0}"' -f $libimeBridge),
        '--libime-model', ('"{0}"' -f $LibimeModelPath)
    )
    $modelProcess = Start-Process -FilePath $modelHost -ArgumentList $modelArguments `
        -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput $modelStdout -RedirectStandardError $modelStderr
    Start-Sleep -Milliseconds 750
    $modelProcess.Refresh()
    if ($modelProcess.HasExited) {
        $detail = if (Test-Path -LiteralPath $modelStderr) {
            (Get-Content -Raw -LiteralPath $modelStderr).Trim()
        } else { '' }
        throw "OwO ModelHost exited during startup (code $($modelProcess.ExitCode)): $detail"
    }
    Set-Content -LiteralPath (Join-Path $runDirectory 'model-host.pid') `
        -Value $modelProcess.Id -Encoding ASCII
    $modelRunning = @($modelProcess)
}

if ($running.Count -eq 0) {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $stdout = Join-Path $logDirectory "core-$stamp.stdout.log"
    $stderr = Join-Path $logDirectory "core-$stamp.stderr.log"
    $arguments = @(
        '--lexicon', ('"{0}"' -f $LexiconPath),
        '--user-frequency', ('"{0}"' -f $frequencyPath)
    )
    if ($modelHostEnabled) { $arguments += '--model-host' }
    $core = Start-Process -FilePath $coreService -ArgumentList $arguments `
        -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    Start-Sleep -Milliseconds 750
    $core.Refresh()
    if ($core.HasExited) {
        $detail = if (Test-Path -LiteralPath $stderr) {
            (Get-Content -Raw -LiteralPath $stderr).Trim()
        } else { '' }
        throw "OwO Core Service exited during startup (code $($core.ExitCode)): $detail"
    }
    Set-Content -LiteralPath (Join-Path $runDirectory 'core-service.pid') `
        -Value $core.Id -Encoding ASCII
    $running = @($core)
}

Assert-ProcessPath -Processes $running -ExpectedPath $coreService -Label 'OwO Core Service'
if ($modelHostEnabled) {
    Assert-ProcessPath -Processes $modelRunning -ExpectedPath $modelHost -Label 'OwO ModelHost'
}

# Verify again after process startup. This catches registration in a different
# Windows user/session and prevents a successful-looking launch with no OwO
# entry in the active input profile list.
& $profileCheck --activate-session
if ($LASTEXITCODE -ne 0) {
    throw "OwO input profile verification failed after startup: $LASTEXITCODE"
}

if ($OpenSettings) {
    & (Join-Path $PSScriptRoot 'open-settings.ps1')
}
if ($OpenNotepad) {
    Start-Process -FilePath "$env:SystemRoot/System32/notepad.exe"
}

$processIds = ($running | ForEach-Object { $_.Id }) -join ', '
Write-Output "OwO manual-test environment is ready."
Write-Output "Core PID: $processIds"
if ($modelHostEnabled) {
    Write-Output "ModelHost PID: $(($modelRunning | ForEach-Object { $_.Id }) -join ', ')"
    Write-Output "libime model: $LibimeModelPath"
} else {
    Write-Output 'ModelHost: disabled or libime runtime/model is incomplete; using base ranking.'
}
Write-Output "TSF DLL: $tsfDll"
Write-Output "TSF DLL (x86): $x86TsfDll"
Write-Output "Lexicon: $LexiconPath"
Write-Output "User frequency: $frequencyPath"
Write-Output "Build updated: $BuildLatest"
Write-Output 'OwO is activated for this Windows session; use Win+Space if the target app kept another profile.'
