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
. (Join-Path $PSScriptRoot 'tsf-dev-image.ps1')
Assert-TsfImageArchitecture -Path $DllPath -Architecture $Architecture
$evidence=Join-Path $projectRoot ('build/tsf-registration/'+[guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $evidence)
$registryView=if($Architecture -eq 'x64'){'64'}else{'32'}
$classKey='HKCU\Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}'
& reg.exe export $classKey (Join-Path $evidence 'before-com.reg') /y "/reg:$registryView" *> (Join-Path $evidence 'backup.log')
$existed=$LASTEXITCODE -eq 0
foreach($hive in @('HKCU','HKLM')){
    & reg.exe export "$hive\Software\Microsoft\CTF\TIP\{6D31C9B1-8978-4F49-89B4-66EB1E741591}" `
        (Join-Path $evidence "before-tip-$hive.reg") /y "/reg:$registryView" *>> (Join-Path $evidence 'backup.log')
}
@{ dll=$DllPath; sha256=(Get-FileHash -LiteralPath $DllPath).Hash; architecture=$Architecture;
   previous_registration=$existed; scope='HKCU'; created_utc=[DateTime]::UtcNow.ToString('O') } |
    ConvertTo-Json | Set-Content (Join-Path $evidence 'deployment.json') -Encoding UTF8

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
if ($Architecture -eq 'x64') {
    & (Join-Path $PSScriptRoot 'sync-notepad-tsf-dev.ps1') -DllPath $DllPath `
        -EvidenceDirectory (Join-Path $evidence 'notepad-legacy-view')
}
Write-Output "Registration backup: $evidence"
$check = Join-Path $projectRoot "build/$preset/$Configuration/owo_tsf_profile_check.exe"
if (Test-Path -LiteralPath $check -PathType Leaf) {
    & $check --refresh-cache
    if ($LASTEXITCODE -ne 0) {
        Write-Warning 'TSF registration cache refresh failed; signing out may be required. Existing applications may still use the previous DLL.'
    }
}
