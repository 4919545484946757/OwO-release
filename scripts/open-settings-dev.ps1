[CmdletBinding()]
param([switch]$SkipRegistration)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$buildDirectory = Join-Path $projectRoot 'build/windows-release/Release'
$settingsCenter = Join-Path $projectRoot 'build/settings-publish/win-x64/OwO.Settings.exe'
$targets = @(
    'owo_core_service', 'owo_ipc_shell', 'owo_tsf_profile_check', 'owo_tsf',
    'owo_model_host', 'owo_model_shell', 'owo_config_shell', 'owo_plugin_shell'
)

# Settings, Core and TSF share the configuration and candidate protocols. Keep
# all three on the same Release build so saving a newer schema cannot silently
# leave an older running Core on its in-memory defaults.
cmake --build --preset windows-release --target $targets -- /m:1
if ($LASTEXITCODE -ne 0) {
    throw "OwO Release runtime build failed: $LASTEXITCODE"
}

Get-Process -Name 'OwO.Settings' -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -eq $settingsCenter } |
    Stop-Process -Force

& (Join-Path $PSScriptRoot 'build_settings_center.ps1') `
    -Configuration Release -RuntimeIdentifier win-x64 -NoRestore

$startArguments = @{
    BuildDirectory = $buildDirectory
    RestartCore = $true
    OpenSettings = $true
}
if ($SkipRegistration) { $startArguments.SkipRegistration = $true }
& (Join-Path $PSScriptRoot 'start-dev.ps1') @startArguments
