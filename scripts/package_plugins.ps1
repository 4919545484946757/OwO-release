[CmdletBinding()]
param(
    [string]$PluginRoot = (Join-Path $PSScriptRoot '..\plugins'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\artifacts\plugins'),
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '..\build\windows-release\Release')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$plugins = [IO.Path]::GetFullPath($PluginRoot)
$output = [IO.Path]::GetFullPath($OutputDirectory)
$build = [IO.Path]::GetFullPath($BuildDirectory)
[void](New-Item -ItemType Directory -Path $output -Force)

foreach ($directory in Get-ChildItem -LiteralPath $plugins -Directory | Sort-Object Name) {
    $manifestPath = Join-Path $directory.FullName 'manifest.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { continue }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace($manifest.id) -or
        [string]::IsNullOrWhiteSpace($manifest.version)) {
        throw "Invalid plugin identity: $manifestPath"
    }
    $baseName = "$($manifest.id)-$($manifest.version)"
    $temporary = Join-Path $output "$baseName.zip"
    $package = Join-Path $output "$baseName.owopkg"
    $packageSource = $directory.FullName
    $staging = $null
    if ($manifest.id -eq 'org.owo.agent-ipc') {
        $agentBinary = Join-Path $build 'owo_agent_ipc_plugin.exe'
        if (-not (Test-Path -LiteralPath $agentBinary -PathType Leaf)) {
            throw "Agent IPC plugin binary is missing: $agentBinary"
        }
        $staging = Join-Path $output ".$baseName-stage"
        if (-not $staging.StartsWith($output + [IO.Path]::DirectorySeparatorChar,
                                     [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Invalid plugin staging path.'
        }
        if (Test-Path -LiteralPath $staging) {
            Remove-Item -LiteralPath $staging -Recurse -Force
        }
        [void](New-Item -ItemType Directory -Path (Join-Path $staging 'bin') -Force)
        foreach ($name in @('manifest.json', 'config.schema.json', 'README.md')) {
            Copy-Item -LiteralPath (Join-Path $directory.FullName $name) `
                -Destination $staging -Force
        }
        Copy-Item -LiteralPath $agentBinary `
            -Destination (Join-Path $staging 'bin\owo-agent-ipc-plugin.exe') -Force
        $packageSource = $staging
    }
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    if (Test-Path -LiteralPath $package) { Remove-Item -LiteralPath $package -Force }
    Compress-Archive -Path (Join-Path $packageSource '*') -DestinationPath $temporary `
        -CompressionLevel NoCompression
    Move-Item -LiteralPath $temporary -Destination $package
    if ($null -ne $staging -and (Test-Path -LiteralPath $staging)) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
    Write-Output $package
}
