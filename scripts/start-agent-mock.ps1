[CmdletBinding()]
param(
    [string]$BuildTree = (Join-Path $PSScriptRoot '..\build\windows-release'),
    [string]$PipeName = '\\.\pipe\OwO.Agent.External.v1'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$build = [IO.Path]::GetFullPath($BuildTree)
$savedPath = $env:Path
Get-ChildItem Env: | Where-Object { $_.Name -ieq 'Path' } | Remove-Item
$env:Path = $savedPath
if (-not (Test-Path -LiteralPath (Join-Path $build 'CMakeCache.txt') -PathType Leaf)) {
    & cmake --preset windows-release -S $root
    if ($LASTEXITCODE -ne 0) { throw "OwO configure failed: $LASTEXITCODE" }
}
& cmake --build $build --config Release --target owo_agent_mock
if ($LASTEXITCODE -ne 0) { throw "OwO Agent mock build failed: $LASTEXITCODE" }
$executable = Join-Path $build 'Release\owo_agent_mock.exe'
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "OwO Agent mock executable is missing: $executable"
}
Write-Output '提示：此测试窗口会显示输入、应用信息和局部上下文，请勿输入真实敏感数据。'
& $executable --pipe $PipeName
exit $LASTEXITCODE
