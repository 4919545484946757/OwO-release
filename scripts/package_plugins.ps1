[CmdletBinding()]
param(
    [string]$PluginRoot = (Join-Path $PSScriptRoot '..\plugins'),
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\artifacts\plugins')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$plugins = [IO.Path]::GetFullPath($PluginRoot)
$output = [IO.Path]::GetFullPath($OutputDirectory)
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
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    if (Test-Path -LiteralPath $package) { Remove-Item -LiteralPath $package -Force }
    Compress-Archive -Path (Join-Path $directory.FullName '*') -DestinationPath $temporary `
        -CompressionLevel NoCompression
    Move-Item -LiteralPath $temporary -Destination $package
    Write-Output $package
}
