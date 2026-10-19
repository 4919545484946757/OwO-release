[CmdletBinding()]
param(
    [string]$BuildTree,
    [string]$PipeName = '\\.\pipe\OwO.Agent.External.v1'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($BuildTree)) {
    $BuildTree = Join-Path $root 'build\windows-release'
}
$build = [IO.Path]::GetFullPath($BuildTree)
$savedPath = $env:Path
Get-ChildItem Env: | Where-Object { $_.Name -ieq 'Path' } | Remove-Item
$env:Path = $savedPath
if (-not (Test-Path -LiteralPath (Join-Path $build 'CMakeCache.txt') -PathType Leaf)) {
    & cmake --preset windows-release -S $root
    if ($LASTEXITCODE -ne 0) { throw "OwO configure failed: $LASTEXITCODE" }
}
& cmake --build $build --config Release --target owo_agent_ipc_plugin
if ($LASTEXITCODE -ne 0) { throw "OwO Agent connector build failed: $LASTEXITCODE" }
$manifestPath = Join-Path $root 'plugins\agent-ipc\manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
$requiredVersion = [string]$manifest.version
$activeRecord = Join-Path $env:LOCALAPPDATA `
    'OwO\InputMethod\plugins\active\org.owo.agent-ipc.record'
$activeVersion = ''
if (Test-Path -LiteralPath $activeRecord -PathType Leaf) {
    $versionLine = Get-Content -LiteralPath $activeRecord -Encoding UTF8 |
        Where-Object { $_ -like 'version=*' } | Select-Object -First 1
    if ($null -ne $versionLine) { $activeVersion = $versionLine.Substring(8) }
}
if ($activeVersion -ne $requiredVersion) {
    & (Join-Path $root 'scripts\package_plugins.ps1') `
        -BuildDirectory (Join-Path $build 'Release') | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "OwO plugin packaging failed: $LASTEXITCODE" }
    $package = Join-Path $root `
        "artifacts\plugins\org.owo.agent-ipc-$requiredVersion.owopkg"
    throw @"
The active Agent connector is '$activeVersion', but protocol v3 requires '$requiredVersion'.
The updated connector package is ready:
$package

Open OwO Settings > Plugins, install this package, review and approve the updated
context permissions, then run start-owo-dev.cmd once and start-agent-mock.cmd again.
"@
}
& cmake --build $build --config Release --target owo_agent_mock
if ($LASTEXITCODE -ne 0) {
    throw "OwO Agent mock build failed: $LASTEXITCODE. Close any existing Agent mock window and retry."
}
$executable = Join-Path $build 'Release\owo_agent_mock.exe'
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "OwO Agent mock executable is missing: $executable"
}
Write-Output 'Privacy notice: this test console displays input, application information, and local context. Do not enter real sensitive data.'
& $executable --pipe $PipeName
exit $LASTEXITCODE
