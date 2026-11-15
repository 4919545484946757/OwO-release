[CmdletBinding()]
param(
    [string]$SettingsExe,
    [ValidateSet('options','progress','success','error')][string]$Preview
)

$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $SettingsExe) {
    $SettingsExe = @(
        (Join-Path $root 'apps\settings_center\bin\x64\Release\net10.0-windows10.0.26100.0\win-x64\OwO.Settings.exe'),
        (Join-Path $root 'build\settings-publish\win-x64\OwO.Settings.exe')
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | ForEach-Object {
        Get-Item -LiteralPath $_
    } | Sort-Object { (Get-Item -LiteralPath (Join-Path $_.DirectoryName 'OwO.Settings.dll')).LastWriteTimeUtc } -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $SettingsExe) { throw 'Build the settings center or pass -SettingsExe.' }
$app = (Resolve-Path -LiteralPath $SettingsExe).ProviderPath
if ([IO.Path]::GetFileName($app) -ine 'OwO.Settings.exe') { throw 'Expected an OwO.Settings.exe, not a setup bootstrapper.' }
$assembly = Join-Path ([IO.Path]::GetDirectoryName($app)) 'OwO.Settings.dll'
if (-not (Test-Path -LiteralPath $assembly -PathType Leaf) -or
    -not [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($assembly)).Contains('UninstallPage')) {
    throw 'This settings build does not support safe uninstall preview. Build the current source first.'
}
if ($Preview) {
    Start-Process -FilePath $app -ArgumentList @('--uninstall-preview', $Preview)
    return
}
$reports = Join-Path $root 'build\uninstall-ui-tests'
[void][IO.Directory]::CreateDirectory($reports)
$report = Join-Path $reports ("ui-" + [Guid]::NewGuid().ToString('N') + '.txt')
$process = Start-Process -FilePath $app -ArgumentList ('--verify-uninstall-ui --uninstall-test-report "' + $report + '"') -PassThru
if (-not $process.WaitForExit(35000)) {
    $process.Kill() # Only the isolated test process created above.
    throw 'Uninstall UI verification timed out.'
}
$process.Refresh()
$entries = if (Test-Path -LiteralPath $report) { @(Get-Content -LiteralPath $report -Encoding UTF8) } else { @() }
if ($process.ExitCode -ne 0 -or @($entries | Where-Object { $_ -like 'PASS *' }).Count -lt 8) {
    $entries | Write-Output
    throw "Uninstall UI verification failed (exit $($process.ExitCode)); report: $report"
}
$entries | Write-Output
Write-Output "PASS: isolated WinUI verification; no uninstall was executed. Report: $report"
