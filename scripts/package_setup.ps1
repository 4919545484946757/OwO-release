[CmdletBinding()]
param([string]$Version = '0.2.5', [string]$OutputDirectory,
      [string]$CompilerPath, [switch]$ValidateOnly)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $root 'artifacts\release' }
if ($Version -notmatch '^(\d+)\.(\d+)\.(\d+)(?:-alpha\.(\d+))?$') { throw 'Invalid setup version.' }
$revision = if ([string]::IsNullOrEmpty($Matches[4])) { '0' } else { $Matches[4] }
$numericVersion = "$($Matches[1]).$($Matches[2]).$($Matches[3]).$revision"
$releaseRoot = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) $Version
$setup = Join-Path $releaseRoot "OwO-Input-Method-$Version-windows-x64-Setup.exe"
if (-not $ValidateOnly -and (Test-Path -LiteralPath $setup)) { throw 'Setup already exists. Use a new OutputDirectory to preserve previous releases.' }
$payload = Join-Path $releaseRoot "OwO-Input-Method-$Version-windows-x64.zip"
$receipt = Join-Path $releaseRoot 'SHA256SUMS.txt'
if (-not (Test-Path -LiteralPath $payload -PathType Leaf)) { throw "Missing release payload: $payload" }
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $payload).Hash.ToLowerInvariant()
$expected = @(Get-Content -LiteralPath $receipt | Where-Object { $_ -eq "$hash  $([IO.Path]::GetFileName($payload))" })
if ($expected.Count -ne 1) { throw 'Release payload checksum does not match the release receipt.' }
# Inno packages the staged tree, not the ZIP. Validate both independently.
$stage = Join-Path $releaseRoot "OwO-Input-Method-$Version-windows-x64"
foreach ($name in @('settings\OwO.Settings.exe', 'settings\OwO.Settings.dll',
        'settings\OwO.Settings.pri', 'settings\App.xbf', 'settings\MainWindow.xbf',
        'settings\MainPage.xbf', 'settings\Assets\AppIcon.ico',
        'settings\Assets\SetupProgressBanner.png', 'scripts\Install-OwO.ps1',
        'scripts\Install-Prerequisites.ps1', 'bin\OwO.TSF.dll', 'bin\x86\OwO.TSF.dll')) {
    if (-not (Test-Path -LiteralPath (Join-Path $stage $name) -PathType Leaf)) { throw "Release stage is missing: $name" }
}
if ((Get-Item -LiteralPath $stage).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked setup staging root is not allowed.' }
$entries = @(Get-ChildItem -LiteralPath $stage -Recurse -Force)
if (@($entries | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint }).Count -ne 0) { throw 'Linked setup staging entries are not allowed.' }
$covered = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$manifest = Join-Path $stage 'SHA256SUMS.txt'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($payload)
try {
    $manifestName = [IO.Path]::GetFileName($stage) + '/SHA256SUMS.txt'
    $manifestEntries = @($archive.Entries | Where-Object { $_.FullName.Replace('\', '/') -ceq $manifestName })
    if ($manifestEntries.Count -ne 1) { throw 'Release ZIP has no unique staging manifest.' }
    $entry = $manifestEntries[0]
    if ($entry.Length -gt 4MB) { throw 'Release ZIP staging manifest is too large.' }
    $reader = [IO.StreamReader]::new($entry.Open())
    try { $zipManifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
    if ($zipManifest -cne [IO.File]::ReadAllText($manifest)) { throw 'Setup staging manifest differs from verified release ZIP.' }
} finally { $archive.Dispose() }
foreach ($line in Get-Content -LiteralPath $manifest) {
    if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw 'Invalid setup staging checksum entry.' }
    $expectedHash = $Matches[1]
    $relative = $Matches[2]
    if ([IO.Path]::IsPathRooted($relative) -or $relative.Contains(':')) { throw 'Invalid setup staging path.' }
    $file = [IO.Path]::GetFullPath((Join-Path $stage $relative))
    if (-not $file.StartsWith($stage + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or
        -not $covered.Add($file) -or -not (Test-Path -LiteralPath $file -PathType Leaf)) { throw 'Unsafe, duplicate or missing setup staging file.' }
    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHash) { throw "Setup staging checksum mismatch: $relative" }
}
foreach ($file in $entries | Where-Object { -not $_.PSIsContainer -and $_.FullName -ne $manifest }) {
    if (-not $covered.Contains($file.FullName)) { throw "Unverified setup staging file: $($file.Name)" }
}
if ($ValidateOnly) { Write-Output 'Setup ZIP and staged assistant integrity checks passed.'; return }

if ([string]::IsNullOrWhiteSpace($CompilerPath)) {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 7\ISCC.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 7\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 7\ISCC.exe')
    )
    $CompilerPath = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if ([string]::IsNullOrWhiteSpace($CompilerPath) -or -not (Test-Path -LiteralPath $CompilerPath -PathType Leaf)) {
    throw 'Inno Setup compiler was not found. Install JRSoftware.InnoSetup or pass -CompilerPath.'
}
$definition = Join-Path $root 'packaging\windows\OwO-Setup.iss'
& $CompilerPath "/DMyVersion=$Version" "/DNumericVersion=$numericVersion" "/DStageDir=$stage" "/DReleaseDir=$releaseRoot" $definition
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed: $LASTEXITCODE" }
if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) { throw 'Setup EXE was not produced.' }
$releaseFiles = @($payload, (Join-Path $releaseRoot "OwO-Input-Method-$Version-source.zip"), $setup) |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }
$hashLines = foreach ($file in $releaseFiles) { "$((Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant())  $([IO.Path]::GetFileName($file))" }
[IO.File]::WriteAllLines($receipt, $hashLines, [Text.UTF8Encoding]::new($false))
Write-Output "Setup with WinUI installation assistant ready: $setup"
