[CmdletBinding()]
param([Parameter(Mandatory)][string]$SetupPath)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$source = (Resolve-Path -LiteralPath $SetupPath).Path
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$testRoot = Join-Path $root ('build\setup-navigation-check-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $testRoot)
$log = Join-Path $env:LOCALAPPDATA 'OwO\InputMethod\logs\setup-startup.log'
$names = @('OwO.Setup.exe', 'OwO-Input-Method-0.2.3-windows-x64-Setup.exe', 'OwO-Input-Method-0.2.3-windows-x64-Setup (1).exe')
foreach ($name in $names) {
    $target = Join-Path $testRoot $name
    Copy-Item -LiteralPath $source -Destination $target
    $before = if (Test-Path -LiteralPath $log) { @(Get-Content -LiteralPath $log).Count } else { 0 }
    $process = Start-Process -FilePath $target -ArgumentList '--verify-installer-navigation' -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(45000)) {
        # Stop only the exact test process created above, not any installed service.
        $process.Kill()
        throw "Navigation verification timed out: $name"
    }
    $process.Refresh()
    $entries = @(Get-Content -LiteralPath $log | Select-Object -Skip $before)
    if ($process.ExitCode -ne 0 -or -not ($entries | Where-Object { $_ -match 'Navigation verification passed$' })) {
        throw "Navigation verification failed: $name (exit $($process.ExitCode)); see $log"
    }
    Write-Output "PASS: $name"
}
Write-Output "Read-only navigation checks passed. Test copies: $testRoot"
