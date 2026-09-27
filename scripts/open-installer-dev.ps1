[CmdletBinding()]
param(
    [string]$Version = '0.2.3',
    [string]$PackageRoot,
    [switch]$SkipBuild,
    [ValidateSet('start', 'options', 'progress', 'checking', 'ready', 'installing', 'success', 'error')]
    [string]$PreviewStage
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Import-Module Microsoft.PowerShell.Utility -ErrorAction Stop

$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

function Stop-SettingsProcessAtPath {
    param([Parameter(Mandatory)][string]$ExpectedPath)

    $expected = [IO.Path]::GetFullPath($ExpectedPath)
    foreach ($process in @(Get-Process -Name 'OwO.Settings' -ErrorAction SilentlyContinue)) {
        $actual = try { [IO.Path]::GetFullPath($process.Path) } catch { '' }
        if (-not [string]::Equals($actual, $expected,
                [StringComparison]::OrdinalIgnoreCase)) { continue }
        Stop-Process -Id $process.Id -Force -ErrorAction Stop
        try { [void]$process.WaitForExit(3000) } catch { }
    }
}

if ([string]::IsNullOrWhiteSpace($PackageRoot)) {
    $publishedSettings = Join-Path $projectRoot `
        'build\settings-publish\win-x64\OwO.Settings.exe'
    Stop-SettingsProcessAtPath -ExpectedPath $publishedSettings
    if (-not $SkipBuild) {
        & (Join-Path $PSScriptRoot 'build_settings_center.ps1') `
            -Configuration Release -RuntimeIdentifier win-x64 -NoRestore
    }

    $previewOutput = Join-Path $projectRoot 'build\installer-preview'
    $previousPreviewSettings = Join-Path $previewOutput `
        "$Version\OwO-Input-Method-$Version-windows-x64\settings\OwO.Settings.exe"
    Stop-SettingsProcessAtPath -ExpectedPath $previousPreviewSettings
    # Run packaging in a clean PowerShell process. The self-contained WinUI
    # publish payload includes framework assemblies whose names can interfere
    # with Windows PowerShell module command discovery in a nested script scope.
    & (Join-Path $PSHOME 'powershell.exe') -NoLogo -NoProfile -NonInteractive `
        -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'package_release.ps1') `
        -Version $Version -OutputDirectory $previewOutput
    if ($LASTEXITCODE -ne 0) {
        throw "Installer preview staging failed: $LASTEXITCODE"
    }
    $PackageRoot = Join-Path $previewOutput `
        "$Version\OwO-Input-Method-$Version-windows-x64"
}

$package = [IO.Path]::GetFullPath($PackageRoot)
$settings = Join-Path $package 'settings\OwO.Settings.exe'
$installerBackend = Join-Path $package 'scripts\Install-OwO.ps1'
foreach ($required in @($settings, $installerBackend)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "Installer preview input is missing: $required"
    }
}

Stop-SettingsProcessAtPath -ExpectedPath $settings

$quotedPackage = '"' + $package + '"'
$arguments = @('--install', $quotedPackage)
if (-not [string]::IsNullOrWhiteSpace($PreviewStage)) {
    $arguments += @('--install-preview-stage', $PreviewStage)
}
$process = Start-Process -FilePath $settings `
    -ArgumentList $arguments -WorkingDirectory $package -PassThru
Write-Output "OwO WinUI installer opened."
Write-Output "Package root: $package"
if (-not [string]::IsNullOrWhiteSpace($PreviewStage)) {
    Write-Output "Preview stage: $PreviewStage (no installation will be performed)"
}
Write-Output "Process ID: $($process.Id)"
