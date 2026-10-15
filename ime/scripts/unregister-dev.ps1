param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Debug',
    [string]$DllPath
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$preset = if ($Configuration -eq 'Debug') { 'windows-debug' } else { 'windows-release' }
if ([string]::IsNullOrWhiteSpace($DllPath)) {
    $DllPath = Join-Path $projectRoot "build/$preset/$Configuration/OwO.TSF.dll"
}
$DllPath = [System.IO.Path]::GetFullPath($DllPath)

if (-not (Test-Path -LiteralPath $DllPath -PathType Leaf)) {
    throw "TSF DLL not found: $DllPath"
}

$process = Start-Process -FilePath "$env:SystemRoot\System32\regsvr32.exe" `
    -ArgumentList @('/s', '/u', $DllPath) -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) {
    throw "regsvr32 /u failed with exit code $($process.ExitCode)"
}
Write-Output "Unregistered development TSF: $DllPath"
