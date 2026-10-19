[CmdletBinding()]
param(
    [string]$Version = '1.3.0',
    [string]$BuildDirectory,
    [string]$PluginPackage,
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($BuildDirectory)) {
    $BuildDirectory = Join-Path $root 'build\windows-release\Release'
}
if ([string]::IsNullOrWhiteSpace($PluginPackage)) {
    $PluginPackage = Join-Path $root "artifacts\plugins\org.owo.agent-ipc-$Version.owopkg"
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $root 'artifacts\agent-sdk'
}
$build = [IO.Path]::GetFullPath($BuildDirectory)
$output = [IO.Path]::GetFullPath($OutputDirectory)
$packageInput = [IO.Path]::GetFullPath($PluginPackage)
$stage = Join-Path $output "OwO-Agent-IPC-SDK-$Version-windows-x64"
$archive = Join-Path $output "OwO-Agent-IPC-SDK-$Version-windows-x64.zip"
if (-not $stage.StartsWith($output + [IO.Path]::DirectorySeparatorChar,
                           [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Invalid Agent SDK staging path.'
}
$mock = Join-Path $build 'owo_agent_mock.exe'
foreach ($required in @($mock, $packageInput)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Agent SDK input is missing: $required"
    }
}
[void](New-Item -ItemType Directory -Path $output -Force)
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }
foreach ($directory in @('bin', 'connector', 'docs', 'examples\mock-agent',
                          'include\owo\agent', 'src', 'LICENSES')) {
    [void](New-Item -ItemType Directory -Path (Join-Path $stage $directory) -Force)
}
Copy-Item -LiteralPath $mock -Destination (Join-Path $stage 'bin\owo_agent_mock.exe') -Force
Copy-Item -LiteralPath $packageInput -Destination (Join-Path $stage 'connector') -Force
Copy-Item -LiteralPath (Join-Path $root 'packaging\agent-sdk\Start-Mock-Agent.cmd') `
    -Destination $stage -Force
Copy-Item -LiteralPath (Join-Path $root 'packaging\agent-sdk\README.zh-CN.md') `
    -Destination $stage -Force
Copy-Item -LiteralPath (Join-Path $root 'packaging\agent-sdk\CMakeLists.txt') `
    -Destination $stage -Force
Copy-Item -LiteralPath (Join-Path $root 'docs\plugins\agent-ipc-integration.md') `
    -Destination (Join-Path $stage 'docs') -Force
Copy-Item -LiteralPath (Join-Path $root 'docs\plugins\agent-ipc-privacy.md') `
    -Destination (Join-Path $stage 'docs') -Force
Copy-Item -LiteralPath (Join-Path $root 'docs\plugins\agent-ipc-mock.md') `
    -Destination (Join-Path $stage 'docs') -Force
Copy-Item -LiteralPath (Join-Path $root 'docs\plugins\agent-ipc-v3.schema.json') `
    -Destination (Join-Path $stage 'docs') -Force
Copy-Item -LiteralPath (Join-Path $root 'apps\agent_mock\main.cpp') `
    -Destination (Join-Path $stage 'examples\mock-agent') -Force
foreach ($name in @('agent_protocol.h', 'agent_pipe.h')) {
    Copy-Item -LiteralPath (Join-Path $root "include\owo\agent\$name") `
        -Destination (Join-Path $stage 'include\owo\agent') -Force
}
foreach ($name in @('agent_protocol.cpp', 'agent_pipe.cpp')) {
    Copy-Item -LiteralPath (Join-Path $root "src\agent\$name") `
        -Destination (Join-Path $stage 'src') -Force
}
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') `
    -Destination (Join-Path $stage 'LICENSES\OwO-GPL-3.0-only.txt') -Force
$hashes = Get-ChildItem -LiteralPath $stage -File -Recurse |
    Sort-Object FullName | ForEach-Object {
        $relative = $_.FullName.Substring($stage.Length).TrimStart('\', '/').Replace('\', '/')
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
        "$hash  $relative"
    }
[IO.File]::WriteAllLines((Join-Path $stage 'SHA256SUMS.txt'), $hashes,
                         [Text.UTF8Encoding]::new($false))
Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal
$archiveHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
Write-Output "$archiveHash  $([IO.Path]::GetFileName($archive))"
Write-Output $archive
