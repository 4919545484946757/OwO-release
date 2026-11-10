[CmdletBinding()]
param([string]$PublishDirectory = '')
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$PublishDirectory) { $PublishDirectory = Join-Path $root 'build/settings-publish/win-x64' }
$fixture = Join-Path $root ('build/plugin-catalog-ui-' + [Guid]::NewGuid().ToString('N'))
$store = Join-Path $fixture 'plugins'
$config = Join-Path $fixture 'config'
[void][IO.Directory]::CreateDirectory($config)
$shell = Join-Path $PublishDirectory 'owo_plugin_shell.exe'
$id = 'org.owo.fixture.catalog-' + [Guid]::NewGuid().ToString('N')
foreach ($version in @('1.2.0', '1.4.1')) {
    $source = Join-Path $fixture "source-$version"
    Copy-Item -LiteralPath (Join-Path $root 'plugins/candidate-glass') -Destination $source -Recurse
    $manifestPath = Join-Path $source 'manifest.json'
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $manifest.id = $id; $manifest.name = 'Catalog fixture'; $manifest.version = $version
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $preview = (& $shell $store inspect-install $source | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0 -or !$preview.ok) { throw 'Fixture inspection failed.' }
    & $shell $store install-risk $source $preview.inventory_sha256 1 I_ACCEPT_PLUGIN_RISK_V1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Fixture installation failed.' }
}
& $shell $store activate $id '1.4.1' | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Fixture activation failed.' }
[IO.File]::WriteAllText((Join-Path $config 'settings-ui.json'), '{"AutoSaveEnabled":false}')
$start = [Diagnostics.ProcessStartInfo]::new()
$start.FileName = Join-Path $PublishDirectory 'OwO.Settings.exe'
$start.WorkingDirectory = $PublishDirectory
$start.Arguments = '--verify-plugin-catalog'
$start.UseShellExecute = $false; $start.CreateNoWindow = $true
$start.EnvironmentVariables['OWO_CONFIG_PATH'] = Join-Path $config 'owo.conf'
$start.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = $store
$process = [Diagnostics.Process]::Start($start)
try {
    if (!$process.WaitForExit(45000)) { throw 'Plugin catalog UI test timed out.' }
    if ($process.ExitCode -ne 0) { throw 'Plugin catalog UI regression failed; see setup-startup.log.' }
    $report = Join-Path $config 'plugin-catalog-report.json'
    $checks = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
    if ($checks.Count -lt 16) { throw 'Incomplete plugin catalog report.' }
    Write-Output "PASS: $($checks.Count) real WinUI checks: history, exact versions, stable refresh, stop/uninstall, stale editor and pending saves. Report: $report"
} finally {
    if (!$process.HasExited) { [void]$process.CloseMainWindow(); [void]$process.WaitForExit(5000) }
    $process.Dispose()
}
