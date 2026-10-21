param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Debug',
    [string]$DllPath,
    [ValidateSet('x64', 'x86')]
    [string]$Architecture = 'x64'
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$preset = if ($Configuration -eq 'Debug') { 'windows-debug' } else { 'windows-release' }
if ([string]::IsNullOrWhiteSpace($DllPath)) {
    $DllPath = Join-Path $projectRoot "build/$preset/$Configuration/OwO.TSF.dll"
}
$DllPath = [System.IO.Path]::GetFullPath($DllPath)

if (-not (Test-Path -LiteralPath $DllPath -PathType Leaf)) {
    throw "TSF DLL not found: $DllPath. Build the selected preset first."
}

$regsvr32 = if ($Architecture -eq 'x86') {
    Join-Path $env:SystemRoot 'SysWOW64\regsvr32.exe'
} else {
    Join-Path $env:SystemRoot 'System32\regsvr32.exe'
}
$process = Start-Process -FilePath $regsvr32 `
    -ArgumentList @('/s', $DllPath) -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) {
    throw "regsvr32 failed with exit code $($process.ExitCode)"
}
Write-Output "Registered development TSF ($Architecture): $DllPath"
$check = Join-Path $projectRoot "build/$preset/$Configuration/owo_tsf_profile_check.exe"
if (Test-Path -LiteralPath $check -PathType Leaf) {
    & $check --refresh-cache
    if ($LASTEXITCODE -ne 0) {
        Write-Warning 'TSF registration cache refresh failed; signing out may be required. Existing applications may still use the previous DLL.'
    }
}
