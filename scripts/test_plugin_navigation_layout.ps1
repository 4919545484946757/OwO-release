[CmdletBinding()]
param([string]$PublishDirectory = '')
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $PublishDirectory) { $PublishDirectory = Join-Path $root 'build/settings-publish/win-x64' }
$fixture = Join-Path $root ('build/plugin-navigation-ui-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($fixture)
$store = Join-Path $fixture 'plugins'
$shell = Join-Path $PublishDirectory 'owo_plugin_shell.exe'
foreach ($index in 1..3) {
    $source = Join-Path $fixture "source-$index"
    Copy-Item -LiteralPath (Join-Path $root 'plugins/candidate-glass') -Destination $source -Recurse
    $manifestPath = Join-Path $source 'manifest.json'
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    $manifest.id = "org.owo.fixture.navigation-$index"
    $manifest.name = "Navigation fixture $index - repeated responsive layout and long name verification"
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $preview = (& $shell $store inspect-install $source | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0 -or -not $preview.ok) { throw 'Fixture inspection failed.' }
    & $shell $store install-risk $source $preview.inventory_sha256 1 I_ACCEPT_PLUGIN_RISK_V1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Fixture installation failed.' }
}
[IO.File]::WriteAllText((Join-Path $fixture 'settings-ui.json'), '{"AutoSaveEnabled":false}')
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = Join-Path $PublishDirectory 'OwO.Settings.exe'
$start.WorkingDirectory = $PublishDirectory
$start.Arguments = '--verify-plugin-navigation'
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.EnvironmentVariables['OWO_CONFIG_PATH'] = Join-Path $fixture 'owo.conf'
$start.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = $store
$process = [Diagnostics.Process]::Start($start)
try {
    if (-not $process.WaitForExit(45000)) { throw 'Plugin navigation layout test timed out.' }
    if ($process.ExitCode -ne 0) { throw 'Plugin navigation layout verification failed; see setup-startup.log.' }
    $report = Join-Path $fixture 'plugin-navigation-report.json'
    $results = Get-Content -LiteralPath $report -Raw -Encoding UTF8 | ConvertFrom-Json
    if (@($results).Count -ne 18) { throw 'Incomplete navigation layout report.' }
    Write-Output "PASS: $($results.Count) row snapshots across window sizes, themes, folding and native-presenter reparenting. System display settings untouched. Report: $report"
} finally {
    if (-not $process.HasExited) { $process.CloseMainWindow() | Out-Null; if (-not $process.WaitForExit(5000)) { $process.Kill() } }
}
