[CmdletBinding()]
param([string]$Version = '0.2.3', [string]$OutputDirectory)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) { $OutputDirectory = Join-Path $root 'artifacts\release' }
if ($Version -notmatch '^\d+\.\d+\.\d+(?:-alpha\.\d+)?$') { throw 'Invalid setup version.' }
$releaseRoot = Join-Path ([IO.Path]::GetFullPath($OutputDirectory)) $Version
$setup = Join-Path $releaseRoot "OwO-Input-Method-$Version-windows-x64-Setup.exe"
if (Test-Path -LiteralPath $setup) { throw 'Setup already exists. Use a new OutputDirectory to preserve previous releases.' }
$payload = Join-Path $releaseRoot "OwO-Input-Method-$Version-windows-x64.zip"
$receipt = Join-Path $releaseRoot 'SHA256SUMS.txt'
if (-not (Test-Path -LiteralPath $payload -PathType Leaf)) { throw "Missing release payload: $payload" }
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $payload).Hash.ToLowerInvariant()
$expected = @(Get-Content -LiteralPath $receipt | Where-Object { $_ -eq "$hash  $([IO.Path]::GetFileName($payload))" })
if ($expected.Count -ne 1) { throw 'Release payload checksum does not match the release receipt.' }
$buildRoot = Join-Path $root "build\standalone-setup\$Version"
[void](New-Item -ItemType Directory -Path $buildRoot -Force)
$payloadHash = Join-Path $buildRoot 'payload.sha256'
[IO.File]::WriteAllText($payloadHash, $hash, [Text.UTF8Encoding]::new($false))
$project = Join-Path $root 'apps\setup_center\OwO.Setup.csproj'
dotnet restore $project -r win-x64 --locked-mode
if ($LASTEXITCODE -ne 0) { throw "Setup restore failed: $LASTEXITCODE" }
dotnet publish $project -c Release -r win-x64 --no-restore -o $buildRoot `
    "-p:Platform=x64" "-p:Version=$Version" `
    "-p:OwOSetupPayload=$payload" "-p:OwOSetupPayloadHash=$payloadHash"
if ($LASTEXITCODE -ne 0) { throw "Standalone WinUI setup publish failed: $LASTEXITCODE" }
$published = Join-Path $buildRoot 'OwO.Setup.exe'
if (-not (Test-Path -LiteralPath $published -PathType Leaf)) { throw 'Setup EXE was not published.' }
$looseFiles = @(Get-ChildItem -LiteralPath $buildRoot -File -Recurse | Where-Object { $_.Extension -in @('.dll', '.xbf', '.pri', '.json') })
if ($looseFiles.Count -ne 0) { throw 'Setup still depends on loose runtime/XAML files; single-file publishing is incomplete.' }
Copy-Item -LiteralPath $published -Destination $setup -Force
$releaseFiles = @($payload, (Join-Path $releaseRoot "OwO-Input-Method-$Version-source.zip"), $setup) |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }
$hashLines = foreach ($file in $releaseFiles) { "$((Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant())  $([IO.Path]::GetFileName($file))" }
[IO.File]::WriteAllLines($receipt, $hashLines, [Text.UTF8Encoding]::new($false))
Write-Output "Standalone WinUI setup ready: $setup"
