[CmdletBinding()]
param([string]$SettingsExe)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $SettingsExe) {
    $SettingsExe = Join-Path $root 'apps/settings_center/bin/x64/Release/net10.0-windows10.0.26100.0/win-x64/OwO.Settings.exe'
}
$path = (Resolve-Path -LiteralPath $SettingsExe).ProviderPath
$assembly = Join-Path ([IO.Path]::GetDirectoryName($path)) 'OwO.Settings.dll'
if (-not (Test-Path -LiteralPath $assembly -PathType Leaf) -or
    -not [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($assembly)).Contains('VerifySettingsSearchAsync')) {
    throw 'Build a current OwO.Settings.exe before running the search UI test.'
}
$fixture = Join-Path $root ('build/settings-search-ui-' + [Guid]::NewGuid().ToString('N'))
$config = Join-Path $fixture 'config'
$plugins = Join-Path $fixture 'plugins'
[void][IO.Directory]::CreateDirectory($config)
[void][IO.Directory]::CreateDirectory($plugins)
[IO.File]::WriteAllText((Join-Path $config 'settings-ui.json'), '{"AutoSaveEnabled":false}')
$report = Join-Path $fixture 'report.txt'
$start = [Diagnostics.ProcessStartInfo]::new($path, '--verify-settings-search')
$start.UseShellExecute = $false
$start.WorkingDirectory = [IO.Path]::GetDirectoryName($path)
$start.EnvironmentVariables['OWO_CONFIG_PATH'] = Join-Path $config 'owo.conf'
$start.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = $plugins
$start.EnvironmentVariables['OWO_SEARCH_TEST_REPORT'] = $report
$process = [Diagnostics.Process]::Start($start)
try {
    if (-not $process.WaitForExit(45000)) {
        $process.Kill()
        throw "Settings search test timed out: $report"
    }
    $checks = if (Test-Path -LiteralPath $report) { @(Get-Content -LiteralPath $report -Encoding UTF8) } else { @() }
    $checks | Write-Output
    if ($process.ExitCode -ne 0 -or @($checks | Where-Object { $_ -like 'PASS *' }).Count -ne 15) {
        throw "Settings search verification failed: $report"
    }
    Write-Output "PASS: isolated settings search. Report: $report"
} finally { $process.Dispose() }
