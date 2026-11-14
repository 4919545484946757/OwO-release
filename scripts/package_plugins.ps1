[CmdletBinding()]
param(
    [string]$PluginRoot,
    [string]$OutputDirectory,
    [string]$BuildDirectory,
    [string[]]$PluginIds
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $PSScriptRoot 'plugin-package-name.ps1')
if ([string]::IsNullOrWhiteSpace($PluginRoot)) {
    $PluginRoot = Join-Path $root 'plugins'
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $root 'artifacts\plugins'
}
if ([string]::IsNullOrWhiteSpace($BuildDirectory)) {
    $BuildDirectory = Join-Path $root 'build\windows-release\Release'
}

$plugins = [IO.Path]::GetFullPath($PluginRoot)
$output = [IO.Path]::GetFullPath($OutputDirectory)
$build = [IO.Path]::GetFullPath($BuildDirectory)
[void](New-Item -ItemType Directory -Path $output -Force)
if (-not ('OwOBundleStoredZip' -as [type])) { Add-Type -Path (Join-Path $root 'tools/BundleStoredZip.cs') }

$packagedIds = @()
$names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
foreach ($directory in Get-ChildItem -LiteralPath $plugins -Directory | Sort-Object Name) {
    $manifestPath = Join-Path $directory.FullName 'manifest.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { continue }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 |
        ConvertFrom-Json
    if ($PluginIds -and $manifest.id -notin $PluginIds) { continue }
    if (!$PluginIds -and $manifest.id -eq 'owo.plugin.input-method-example') { continue }
    if ([string]::IsNullOrWhiteSpace($manifest.id) -or
        [string]::IsNullOrWhiteSpace($manifest.version)) {
        throw "Invalid plugin identity: $manifestPath"
    }
    if ($manifest.id -cnotmatch '^[a-z0-9][a-z0-9.-]+$' -or $manifest.id.Contains('..')) { throw 'Invalid plugin ID.' }
    $fileName = Get-OwOPluginPackageName $manifest.name $manifest.version
    if (!$names.Add($fileName)) { throw "Plugins declare the same package filename: $fileName" }
    $temporary = Join-Path $output ('.' + [Guid]::NewGuid().ToString('N') + '.tmp')
    $package = Join-Path $output $fileName
    if (Test-Path -LiteralPath $package) { throw "Plugin package already exists: $package" }
    $packageSource = $directory.FullName
    $staging = $null
    if ($manifest.id -in @('owo.plugin.english', 'owo.plugin.wubi86') -and
        -not (Test-Path -LiteralPath (Join-Path $packageSource $manifest.entry) -PathType Leaf)) {
        $configuration = Split-Path -Leaf $build
        $provider = if ($manifest.id -eq 'owo.plugin.english') { 'english' } else { 'wubi86' }
        $packageSource = Join-Path (Split-Path -Parent $build) "$provider-plugin/$configuration"
        if (-not (Test-Path -LiteralPath (Join-Path $packageSource $manifest.entry) -PathType Leaf) -or
            (Get-FileHash -LiteralPath $manifestPath).Hash -ne
            (Get-FileHash -LiteralPath (Join-Path $packageSource 'manifest.json')).Hash) {
            throw "Build owo_${provider}_package for the requested configuration before packaging."
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $packageSource $manifest.entry) -PathType Leaf)) { throw "Plugin entry is missing: $($manifest.id)" }
    if ($manifest.id -eq 'org.owo.agent-ipc') {
        $agentBinary = Join-Path $build 'owo_agent_ipc_plugin.exe'
        if (-not (Test-Path -LiteralPath $agentBinary -PathType Leaf)) {
            throw "Agent IPC plugin binary is missing: $agentBinary"
        }
        $staging = Join-Path $output ('.stage-' + [Guid]::NewGuid().ToString('N'))
        if (-not $staging.StartsWith($output + [IO.Path]::DirectorySeparatorChar,
                                     [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Invalid plugin staging path.'
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
    $licensed = $null
    try {
        if ($plugins.Equals((Join-Path $root 'plugins'), [StringComparison]::OrdinalIgnoreCase)) {
            $licensed = Join-Path $output ('.license-' + [Guid]::NewGuid().ToString('N'))
            Copy-Item -LiteralPath $packageSource -Destination $licensed -Recurse
            Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination (Join-Path $licensed 'LICENSE')
            $packageSource = $licensed
        }
        [OwOBundleStoredZip]::Create($packageSource, $temporary)
        Move-Item -LiteralPath $temporary -Destination $package
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
        if ($licensed -and (Test-Path -LiteralPath $licensed)) { Remove-Item -LiteralPath $licensed -Recurse -Force }
        if ($staging -and (Test-Path -LiteralPath $staging)) { Remove-Item -LiteralPath $staging -Recurse -Force }
    }
    Write-Output $package
    $packagedIds += $manifest.id
}
foreach ($id in $PluginIds) { if ($id -notin $packagedIds) { throw "Requested plugin not found: $id" } }
