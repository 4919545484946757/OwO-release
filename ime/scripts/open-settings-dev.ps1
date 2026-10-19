[CmdletBinding()]
param([switch]$SkipRegistration)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$buildTree = Join-Path $projectRoot 'build/windows-release'
$buildDirectory = Join-Path $projectRoot 'build/windows-release/Release'
$settingsCenter = Join-Path $projectRoot 'build/settings-publish/win-x64/OwO.Settings.exe'
$targets = @(
    'owo_core_service', 'owo_ipc_shell', 'owo_tsf_profile_check', 'owo_tsf',
    'owo_model_host', 'owo_model_shell', 'owo_config_shell', 'owo_plugin_shell'
)

function Stop-LockingDevelopmentProcess {
    param(
        [string]$Name,
        [string]$ExpectedPath
    )

    $expected = [IO.Path]::GetFullPath($ExpectedPath)
    foreach ($process in @(Get-Process -Name $Name -ErrorAction SilentlyContinue)) {
        $actual = try { [IO.Path]::GetFullPath($process.Path) } catch { '' }
        if (-not [string]::Equals($actual, $expected,
                [StringComparison]::OrdinalIgnoreCase)) { continue }
        Write-Output "Stopping locking development process $Name PID $($process.Id)"
        Stop-Process -Id $process.Id -Force -ErrorAction Stop
        try { $process.WaitForExit(3000) } catch { }
    }
    foreach ($process in @(Get-Process -Name $Name -ErrorAction SilentlyContinue)) {
        $actual = try { [IO.Path]::GetFullPath($process.Path) } catch { '' }
        if ([string]::Equals($actual, $expected,
                [StringComparison]::OrdinalIgnoreCase)) {
            throw "Development process still locks its build output: $actual (PID $($process.Id))"
        }
    }
}

# Settings, Core and TSF share the configuration and candidate protocols. Keep
# all three on the same Release build so saving a newer schema cannot silently
# leave an older running Core on its in-memory defaults.
$ipcShell = Join-Path $buildDirectory 'owo_ipc_shell.exe'
$modelShell = Join-Path $buildDirectory 'owo_model_shell.exe'
if (Test-Path -LiteralPath $ipcShell -PathType Leaf) {
    try { & $ipcShell --shutdown *> $null } catch { }
}
if (Test-Path -LiteralPath $modelShell -PathType Leaf) {
    try { & $modelShell --shutdown *> $null } catch { }
}
Start-Sleep -Milliseconds 300
Stop-LockingDevelopmentProcess -Name 'owo_core_service' `
    -ExpectedPath (Join-Path $buildDirectory 'owo_core_service.exe')
Stop-LockingDevelopmentProcess -Name 'owo_model_host' `
    -ExpectedPath (Join-Path $buildDirectory 'owo_model_host.exe')

cmake --build $buildTree --config Release --target $targets -- /m:1
if ($LASTEXITCODE -ne 0) {
    throw "OwO Release runtime build failed: $LASTEXITCODE"
}

# A new configuration schema must be understood by both in-process TSF builds.
# Never register a stale x86 DLL after saving settings from the latest x64 UI.
$x86BuildTree = Join-Path $projectRoot 'build/windows-x86-release'
if (-not (Test-Path -LiteralPath (Join-Path $x86BuildTree 'CMakeCache.txt'))) {
    cmake --preset windows-x86-release -S $projectRoot
    if ($LASTEXITCODE -ne 0) { throw "OwO x86 configure failed: $LASTEXITCODE" }
}
cmake --build $x86BuildTree --config Release --target owo_tsf -- /m:1
if ($LASTEXITCODE -ne 0) { throw "OwO x86 TSF build failed: $LASTEXITCODE" }

Get-Process -Name 'OwO.Settings' -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $settingsCenter } |
    Stop-Process -Force

& (Join-Path $PSScriptRoot 'build_settings_center.ps1') `
    -Configuration Release -RuntimeIdentifier win-x64 -NoRestore

$startArguments = @{
    BuildDirectory = $buildDirectory
    RestartCore = $true
    OpenSettings = $true
}
if ($SkipRegistration) { $startArguments.SkipRegistration = $true }
& (Join-Path $PSScriptRoot 'start-dev.ps1') @startArguments

if (-not $SkipRegistration) {
    $buildTsf = Join-Path $buildDirectory 'OwO.TSF.dll'
    $buildHash = (Get-FileHash -LiteralPath $buildTsf -Algorithm SHA256).Hash.ToLowerInvariant()
    $expectedTsf = [IO.Path]::GetFullPath((Join-Path $projectRoot `
        "build/manual-deploy/OwO.TSF.$($buildHash.Substring(0, 16)).dll"))
    $registrationKey = 'Registry::HKEY_CURRENT_USER\Software\Classes\CLSID\' +
        '{6D31C9B1-8978-4F49-89B4-66EB1E741591}\InprocServer32'
    $registeredTsf = [IO.Path]::GetFullPath(
        (Get-ItemProperty -LiteralPath $registrationKey -ErrorAction Stop).'(default)')
    if (-not [string]::Equals($registeredTsf, $expectedTsf,
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "TSF registration verification failed: $registeredTsf (expected $expectedTsf)"
    }
    Write-Output "Verified current TSF registration: $registeredTsf"
}
