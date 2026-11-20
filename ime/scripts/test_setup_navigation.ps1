[CmdletBinding(DefaultParameterSetName = 'Assistant')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Assistant')][string]$PackageRoot,
    [Parameter(Mandatory, ParameterSetName = 'Standalone')][switch]$Standalone,
    [Parameter(Mandatory, ParameterSetName = 'Standalone')][string]$SetupPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$log = Join-Path $env:LOCALAPPDATA 'OwO\InputMethod\logs\setup-startup.log'
$targets = @()
if ($Standalone) {
    # Legacy single-file host only. Never pass an Inno bootstrapper here: it
    # would ignore the verification flag and request UAC instead.
    $source = (Resolve-Path -LiteralPath $SetupPath).Path
    if ([Diagnostics.FileVersionInfo]::GetVersionInfo($source).OriginalFilename -notlike 'OwO.Setup.*') {
        throw 'This is not the legacy standalone WinUI host. Use -PackageRoot for the current assistant.'
    }
    $testRoot = Join-Path $root ('build\setup-navigation-check-' + [Guid]::NewGuid().ToString('N'))
    [void](New-Item -ItemType Directory -Path $testRoot)
    foreach ($name in @('OwO.Setup.exe', 'OwO-Input-Method-0.2.5-windows-x64-Setup.exe', 'OwO-Input-Method-0.2.5-windows-x64-Setup (1).exe')) {
        $target = Join-Path $testRoot $name
        Copy-Item -LiteralPath $source -Destination $target
        $targets += [pscustomobject]@{ Path = $target; Arguments = '--verify-installer-navigation' }
    }
} else {
    $package = (Resolve-Path -LiteralPath $PackageRoot).Path
    $target = Join-Path $package 'settings\OwO.Settings.exe'
    if (-not (Test-Path -LiteralPath $target -PathType Leaf) -or
        -not (Test-Path -LiteralPath (Join-Path $package 'scripts\Install-OwO.ps1') -PathType Leaf)) {
        throw 'Package does not contain the WinUI assistant and installer backend.'
    }
    $targets += [pscustomobject]@{ Path = $target; Arguments = '--install "' + $package + '" --verify-installer-navigation' }
}
foreach ($target in $targets) {
    $before = if (Test-Path -LiteralPath $log) { @(Get-Content -LiteralPath $log).Count } else { 0 }
    $process = Start-Process -FilePath $target.Path -ArgumentList $target.Arguments -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(45000)) {
        # Stop only the exact test process created above, not any installed service.
        $process.Kill()
        throw "Navigation verification timed out: $($target.Path)"
    }
    $process.Refresh()
    $entries = @(Get-Content -LiteralPath $log | Select-Object -Skip $before)
    if ($process.ExitCode -ne 0 -or -not ($entries | Where-Object { $_ -match 'Navigation verification passed$' })) {
        throw "Navigation verification failed: $($target.Path) (exit $($process.ExitCode)); see $log"
    }
    Write-Output "PASS: $($target.Path)"
}
Write-Output 'Read-only navigation checks passed; no installation was executed.'
