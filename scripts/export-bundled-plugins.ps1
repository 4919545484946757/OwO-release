[CmdletBinding()]
param([Parameter(Mandatory)][string]$StorePath,
      [Parameter(Mandatory)][string]$OutputDirectory,
      [Parameter(Mandatory)][string]$PluginShell,
      [string[]]$PluginIds = @('org.owo.candidate-glass', 'org.owo.unicode-palette'))
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
if (-not ('OwOBundleStoredZip' -as [type])) {
    Add-Type -Path (Join-Path $PSScriptRoot '../tools/BundleStoredZip.cs')
}
$store = [IO.Path]::GetFullPath($StorePath)
$output = [IO.Path]::GetFullPath($OutputDirectory)
if ($output.Equals($store, [StringComparison]::OrdinalIgnoreCase) -or
    $output.StartsWith($store + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
    $store.StartsWith($output + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Plugin export output must not overlap the store.' }
if (Test-Path -LiteralPath $output) { throw 'Bundled plugin output already exists; refusing to overwrite it.' }
$snapshot = & $PluginShell $store list | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $snapshot.schema_version -ne 1) { throw 'Cannot read plugin store.' }
foreach ($id in $PluginIds) { if (-not @($snapshot.plugins | Where-Object id -eq $id).Count) { throw "Requested bundled plugin is missing: $id" } }
[void][IO.Directory]::CreateDirectory((Join-Path $output 'packages'))
[void][IO.Directory]::CreateDirectory((Join-Path $output 'config'))
$plugins = @()
foreach ($plugin in $snapshot.plugins) {
    if ($plugin.id -notin $PluginIds) { continue }
    if ($plugin.id -notmatch '^[a-z0-9][a-z0-9.-]{0,127}$' -or $plugin.id.Contains('..') -or
        $plugin.version -notmatch '^[0-9A-Za-z][0-9A-Za-z.+-]{0,63}$') { throw 'Unsafe plugin identity.' }
    $source = [IO.Path]::GetFullPath((Join-Path $store "versions/$($plugin.id)/$($plugin.version)"))
    if (-not $source.StartsWith($store + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Plugin source escaped store.' }
    $current = $source
    while ($current.Length -ge $store.Length) {
        if ((Get-Item -LiteralPath $current -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked plugin source rejected.' }
        if ($current -eq $store) { break }; $current = Split-Path -Parent $current
    }
    if (@(Get-ChildItem -LiteralPath $source -Recurse -Force | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count) { throw 'Linked plugin files rejected.' }
    $relative = "packages/$($plugin.id)-$($plugin.version).owopkg"
    $package = Join-Path $output $relative
    [OwOBundleStoredZip]::Create($source, $package)
    $preview = & $PluginShell $store inspect-install $package | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or -not $preview.ok -or $preview.plugin_id -ne $plugin.id -or $preview.version -ne $plugin.version) { throw 'Bundled plugin inspection failed.' }
    $entry = [ordered]@{ id = $plugin.id; version = $plugin.version; name = $plugin.name; active = [bool]$plugin.active;
        package = $relative; sha256 = (Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant(); settings = $null; settingsSha256 = $null }
    # Deliberately exclude authorizations, active/records, backups, logs, session
    # data and private Agent context. Only the declared current settings travel.
    $settings = Join-Path $store "data/$($plugin.id)/settings.json"
    if (Test-Path -LiteralPath $settings -PathType Leaf) {
        foreach ($path in @((Join-Path $store 'data'), (Split-Path -Parent $settings), $settings)) {
            if ((Get-Item -LiteralPath $path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked plugin settings rejected.' }
        }
        if ((Get-Item -LiteralPath $settings).Length -gt 256KB) { throw 'Plugin settings exceed size limit.' }
        $null = Get-Content -LiteralPath $settings -Raw -Encoding UTF8 | ConvertFrom-Json
        $entry.settings = "config/$($plugin.id).json"
        Copy-Item -LiteralPath $settings -Destination (Join-Path $output $entry.settings)
        $entry.settingsSha256 = (Get-FileHash -LiteralPath (Join-Path $output $entry.settings) -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $plugins += $entry
}
$catalog = [ordered]@{ schemaVersion = 1; plugins = @($plugins) }
[IO.File]::WriteAllText((Join-Path $output 'manifest.json'), ($catalog | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
Write-Output "Bundled plugins: $($plugins.Count); requested active: $(@($plugins | Where-Object active).Count). No authorizations or history exported."
