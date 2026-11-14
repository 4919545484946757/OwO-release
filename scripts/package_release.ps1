[CmdletBinding()]
param(
    [string]$Version = '0.3.1',
    [string]$BuildDirectory,
    [string]$X86BuildDirectory,
    [string]$SettingsDirectory,
    [string]$OutputDirectory,
    [string]$BundledPluginStore,
    [string]$BundledPluginDirectory
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Get-Sha256Hex {
    param([Parameter(Mandatory)][string]$LiteralPath)

    $stream = [IO.File]::Open($LiteralPath, [IO.FileMode]::Open,
        [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        try { $bytes = $algorithm.ComputeHash($stream) }
        finally { $algorithm.Dispose() }
    } finally { $stream.Dispose() }
    return [BitConverter]::ToString($bytes).Replace('-', '').ToLowerInvariant()
}
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($BuildDirectory)) {
    $BuildDirectory = Join-Path $root 'build\windows-release\Release'
}
if ([string]::IsNullOrWhiteSpace($X86BuildDirectory)) {
    $X86BuildDirectory = Join-Path $root 'build\windows-x86-release\Release'
}
if ([string]::IsNullOrWhiteSpace($SettingsDirectory)) {
    $SettingsDirectory = Join-Path $root 'build\settings-publish\win-x64'
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $root 'artifacts\release'
}
$build = [IO.Path]::GetFullPath($BuildDirectory)
$x86Build = [IO.Path]::GetFullPath($X86BuildDirectory)
# Build/validate the v3 asset before touching an existing release directory.
& (Join-Path $root 'scripts/prepare-runtime-lexicon.ps1') -BuildDirectory $build
$output = [IO.Path]::GetFullPath($OutputDirectory)
$releaseRoot = Join-Path $output $Version
$stage = Join-Path $releaseRoot "OwO-Input-Method-$Version-windows-x64"
if (-not $releaseRoot.StartsWith($output + [IO.Path]::DirectorySeparatorChar,
                                 [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Invalid release staging path.'
}
if (Test-Path -LiteralPath $releaseRoot) {
    throw 'Release directory already exists. Use a fresh OutputDirectory to preserve previous artifacts.'
}
[void](New-Item -ItemType Directory -Path $stage -Force)

$settings = [IO.Path]::GetFullPath($SettingsDirectory)
# Match start-dev.ps1/stage_libime_runtime.ps1: the legacy P1 bridge
# scores entire sentences as unknown words instead of tokenizing them.
$runtime = Join-Path $root 'build\dependencies\libime-1.1.15\runtime-x86_64'
$model = Join-Path $root 'build\dependencies\libime-1.1.15\model-20260804\lib\libime\zh_CN.lm'
$lexicon = Join-Path $root 'build\windows-release\rime-ice-cn-2026.06.30-v3.owolx'
$vcBuildTools = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools'
$vcRedistBase = Join-Path $vcBuildTools 'VC\Redist\MSVC'
$vcRuntime = Get-ChildItem -LiteralPath $vcRedistBase -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^\d+(\.\d+)+$' } |
    Sort-Object { [Version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\Microsoft.VC143.CRT' } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Container } |
    Select-Object -First 1
$required = @(
    (Join-Path $build 'OwO.TSF.dll'), (Join-Path $build 'owo_core_service.exe'),
    (Join-Path $x86Build 'OwO.TSF.dll'),
    (Join-Path $build 'owo_model_host.exe'), (Join-Path $build 'owo_runtime_launcher.exe'),
    (Join-Path $build 'owo_tsf_profile_check.exe'),
    (Join-Path $settings 'OwO.Settings.exe'),
    (Join-Path $settings 'OwO.Settings.dll'),
    (Join-Path $settings 'App.xbf'),
    (Join-Path $settings 'MainPage.xbf'),
    (Join-Path $settings 'MainWindow.xbf'),
    (Join-Path $settings 'OwO.Settings.pri'),
    (Join-Path $settings 'Assets\AppIcon.ico'),
    (Join-Path $settings 'owo_config_shell.exe'),
    (Join-Path $settings 'owo_plugin_shell.exe'),
    $runtime, $model, $lexicon, $vcRuntime
)
foreach ($path in $required) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Release input is missing: $path" }
}

foreach ($directory in @('bin', 'bin\x86', 'data', 'model\runtime', 'settings', 'scripts', 'LICENSES')) {
    [void](New-Item -ItemType Directory -Path (Join-Path $stage $directory) -Force)
}
Copy-Item -LiteralPath (Join-Path $x86Build 'OwO.TSF.dll') `
    -Destination (Join-Path $stage 'bin\x86\OwO.TSF.dll') -Force
foreach ($name in @('OwO.TSF.dll', 'owo_core_service.exe', 'owo_model_host.exe',
                    'owo_runtime_launcher.exe', 'owo_tsf_profile_check.exe',
                    'owo_config_shell.exe', 'owo_plugin_shell.exe')) {
    Copy-Item -LiteralPath (Join-Path $build $name) -Destination (Join-Path $stage 'bin') -Force
}
Copy-Item -Path (Join-Path $vcRuntime '*.dll') -Destination (Join-Path $stage 'bin') -Force
Copy-Item -Path (Join-Path $settings '*') -Destination (Join-Path $stage 'settings') `
    -Recurse -Force
Copy-Item -Path (Join-Path $runtime '*') -Destination (Join-Path $stage 'model\runtime') `
    -Recurse -Force
Copy-Item -LiteralPath $model -Destination (Join-Path $stage 'model\zh_CN.lm') -Force
Copy-Item -LiteralPath $lexicon -Destination (Join-Path $stage 'data\owo-cn.owolx') -Force

$packaging = Join-Path $root 'packaging\windows'
foreach ($name in @('Install-OwO.cmd', 'Uninstall-OwO.cmd', 'Start-OwO.cmd',
                    'Open-OwO-Settings.cmd',
                    'README.zh-CN.md', 'THIRD-PARTY-NOTICES.md')) {
    Copy-Item -LiteralPath (Join-Path $packaging $name) -Destination $stage -Force
}
foreach ($name in @('Install-OwO.ps1', 'Install-Prerequisites.ps1', 'Uninstall-OwO.ps1', 'Finish-Uninstall-OwO.ps1',
                    'Start-OwO.ps1')) {
    Copy-Item -LiteralPath (Join-Path $packaging $name) -Destination (Join-Path $stage 'scripts') -Force
}
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') `
    -Destination (Join-Path $stage 'LICENSES\OwO-GPL-3.0-only.txt') -Force

$licenseInputs = [ordered]@{
    'libime-LGPL-2.1-or-later.txt' = Join-Path $runtime 'licenses\libime-LGPL-2.1-or-later.txt'
    'libcxx-LICENSE.txt' = 'C:\msys64\clang64\share\licenses\libc++\LICENSE'
    'zstd-LICENSE.txt' = 'C:\msys64\clang64\share\licenses\zstd\LICENSE'
    'libiconv-COPYING.txt' = 'C:\msys64\clang64\share\licenses\libiconv\COPYING'
    'libiconv-COPYING.LIB.txt' = 'C:\msys64\clang64\share\licenses\libiconv\COPYING.LIB'
    'gettext-runtime-COPYING.txt' = 'C:\msys64\clang64\share\licenses\gettext-runtime\COPYING'
    'gettext-runtime-COPYING.LIB.txt' = 'C:\msys64\clang64\share\licenses\gettext-runtime\intl\COPYING.LIB'
    'libuv-LICENSE.txt' = 'C:\msys64\clang64\share\licenses\libuv\LICENSE'
    'winpthreads-COPYING.txt' = 'C:\msys64\clang64\share\licenses\winpthreads\COPYING'
    'dlfcn-LICENSE.txt' = 'C:\msys64\clang64\share\licenses\dlfcn\LICENSE'
    'dotnet-LICENSE.txt' = 'C:\Program Files\dotnet\LICENSE.txt'
    'dotnet-ThirdPartyNotices.txt' = 'C:\Program Files\dotnet\ThirdPartyNotices.txt'
    'Microsoft-VC-Runtime-Redist.txt' = Join-Path $vcBuildTools 'Licenses\2052\Redist.txt'
    'Microsoft-VC-Runtime-ThirdPartyNotices.txt' = Join-Path $vcBuildTools 'Licenses\2052\ThirdPartyNotices.txt'
    'WindowsAppSDK-WinUI-LICENSE.txt' = Join-Path $env:USERPROFILE '.nuget\packages\microsoft.windowsappsdk.winui\2.3.0\license.txt'
    'WindowsAppSDK-WinUI-NOTICE.txt' = Join-Path $env:USERPROFILE '.nuget\packages\microsoft.windowsappsdk.winui\2.3.0\NOTICE.txt'
    'WindowsAppSDK-Foundation-LICENSE.txt' = Join-Path $env:USERPROFILE '.nuget\packages\microsoft.windowsappsdk.foundation\2.3.5\license.txt'
    'WindowsAppSDK-InteractiveExperiences-LICENSE.txt' = Join-Path $env:USERPROFILE '.nuget\packages\microsoft.windowsappsdk.interactiveexperiences\2.1.3\license.txt'
    'WindowsAppSDK-Runtime-LICENSE.txt' = Join-Path $env:USERPROFILE '.nuget\packages\microsoft.windowsappsdk.runtime\2.3.1\license.txt'
    'WindowsAppSDK-Runtime-NOTICE.txt' = Join-Path $env:USERPROFILE '.nuget\packages\microsoft.windowsappsdk.runtime\2.3.1\NOTICE.txt'
    'WebView2-LICENSE.txt' = Join-Path $env:USERPROFILE '.nuget\packages\microsoft.web.webview2\1.0.3719.77\LICENSE.txt'
    'WebView2-NOTICE.txt' = Join-Path $env:USERPROFILE '.nuget\packages\microsoft.web.webview2\1.0.3719.77\NOTICE.txt'
}
foreach ($entry in $licenseInputs.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) {
        throw "Third-party license is missing: $($entry.Value)"
    }
    Copy-Item -LiteralPath $entry.Value -Destination (Join-Path $stage "LICENSES\$($entry.Key)") -Force
}

if ($BundledPluginStore -and $BundledPluginDirectory) { throw 'Choose only one source for bundled plugins.' }
if ($BundledPluginDirectory) {
    . (Join-Path $PSScriptRoot 'plugin-package-name.ps1')
    $bundle = Join-Path $stage 'bundled-plugins'
    [void][IO.Directory]::CreateDirectory((Join-Path $bundle 'packages'))
    $records = @(foreach ($provider in @('candidate-glass', 'unicode-palette')) {
        $plugin = Get-Content -LiteralPath (Join-Path $root "plugins/$provider/manifest.json") -Raw -Encoding UTF8 | ConvertFrom-Json
        $file = Get-OwOPluginPackageName $plugin.name $plugin.version
        $source = Join-Path $BundledPluginDirectory $file
        if (!(Test-Path -LiteralPath $source -PathType Leaf) -or ((Get-Item -LiteralPath $source).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Missing or linked bundled package: $file" }
        # Keep the existing installer catalog's exact internal path contract.
        $path = "packages/$($plugin.id)-$($plugin.version).owopkg"
        $target = Join-Path $bundle $path
        Copy-Item -LiteralPath $source -Destination $target
        $preview = & (Join-Path $build 'owo_plugin_shell.exe') (Join-Path $bundle 'preview-store') inspect-install $target | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0 -or !$preview.ok -or $preview.plugin_id -cne $plugin.id -or $preview.version -cne $plugin.version) { throw "Invalid bundled package: $file" }
        [ordered]@{id=$plugin.id;version=$plugin.version;name=$plugin.name;active=$true;package=$path;sha256=(Get-Sha256Hex -LiteralPath $target);settings=$null;settingsSha256=$null}
    })
    [IO.File]::WriteAllText((Join-Path $bundle 'manifest.json'), ([ordered]@{schemaVersion=1;plugins=$records} | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
} elseif (-not [string]::IsNullOrWhiteSpace($BundledPluginStore)) {
    & (Join-Path $root 'scripts/export-bundled-plugins.ps1') -StorePath $BundledPluginStore `
        -OutputDirectory (Join-Path $stage 'bundled-plugins') -PluginShell (Join-Path $build 'owo_plugin_shell.exe')
}

$hashLines = Get-ChildItem -LiteralPath $stage -File -Recurse |
    Sort-Object FullName | ForEach-Object {
        $relative = $_.FullName.Substring($stage.Length).TrimStart('\', '/').Replace('\', '/')
        $hash = Get-Sha256Hex -LiteralPath $_.FullName
        "$hash  $relative"
    }
[IO.File]::WriteAllLines((Join-Path $stage 'SHA256SUMS.txt'), $hashLines,
                         [Text.UTF8Encoding]::new($false))

$binaryZip = Join-Path $releaseRoot "OwO-Input-Method-$Version-windows-x64.zip"
Compress-Archive -LiteralPath $stage -DestinationPath $binaryZip -CompressionLevel Optimal

$sourceZip = Join-Path $releaseRoot "OwO-Input-Method-$Version-source.zip"
$commit = (& git -C $root rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[a-f0-9]{40}$') { throw 'Cannot determine source commit.' }
# Export only tracked source from the recorded commit, never build output,
# ignored logs or private files that happen to be in the working directory.
& git -C $root archive --format=zip "--prefix=OwO-Input-Method-$Version-source/" "--output=$sourceZip" $commit -- `
    apps benchmarks data docs examples include packaging plugins scripts src tests tools third_party `
    CMakeLists.txt CMakePresets.json global.json vcpkg.json .gitattributes .gitignore LICENSE README.md ':(glob)*.md' ':(glob)*.cmd'
if ($LASTEXITCODE -ne 0) { throw 'Cannot archive release source commit.' }

$releaseHashes = foreach ($file in @($binaryZip, $sourceZip)) {
    $hash = Get-Sha256Hex -LiteralPath $file
    "$hash  $([IO.Path]::GetFileName($file))"
}
[IO.File]::WriteAllLines((Join-Path $releaseRoot 'SHA256SUMS.txt'), $releaseHashes,
                         [Text.UTF8Encoding]::new($false))
Write-Output $releaseRoot
$dirty = @(& git -C $root status --porcelain).Count -ne 0
$provenance = [ordered]@{schema_version=1; version=$Version; git_commit=$commit; dirty=$dirty; built_at_utc=[DateTime]::UtcNow.ToString('o')}
[IO.File]::WriteAllText((Join-Path $releaseRoot 'release-build.json'), ($provenance | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
