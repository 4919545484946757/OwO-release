$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '../../packaging/windows/Install-Prerequisites.ps1')
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$fixtures = Join-Path $root ('build\install-directory-tests-' + [Guid]::NewGuid().ToString('N'))
$utf8 = [Text.UTF8Encoding]::new($false)
function Write-Fixture([string]$Path, [string]$Content = 'fixture') {
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($Path))
    [IO.File]::WriteAllText($Path, $Content, $utf8)
}
function Assert($Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
$package = Join-Path $fixtures 'package'
foreach ($file in @('bin/OwO.TSF.dll', 'bin/owo_core_service.exe')) { Write-Fixture (Join-Path $package $file) $file }
foreach ($scenario in @('new', 'empty', 'installed', 'partial', 'marked', 'foreign', 'bad-marker', 'tampered', 'file')) {
    $preferred = Join-Path $fixtures $scenario
    switch ($scenario) {
        'empty' { [void][IO.Directory]::CreateDirectory($preferred) }
        'installed' {
            Write-Fixture (Join-Path $preferred 'bin/OwO.TSF.dll') 'older version'
            Write-Fixture (Join-Path $preferred 'scripts/Uninstall-OwO.ps1')
        }
        'partial' {
            foreach ($file in @('bin/OwO.TSF.dll', 'bin/owo_core_service.exe')) { Write-Fixture (Join-Path $preferred $file) $file }
            Write-Fixture (Join-Path $preferred 'settings/unfinished.dll') 'preserve partial files'
        }
        'marked' { Write-Fixture (Join-Path $preferred '.owo-install-state.json') '{"application":"OwO.InputMethod","schema":1,"state":"copying"}' }
        'foreign' { Write-Fixture (Join-Path $preferred 'notes.txt') 'user data - preserve'
        }
        'bad-marker' { Write-Fixture (Join-Path $preferred '.owo-install-state.json') '{"application":"other","schema":1,"state":"copying"}' }
        'tampered' {
            Write-Fixture (Join-Path $preferred 'bin/OwO.TSF.dll') 'bin/OwO.TSF.dll'
            Write-Fixture (Join-Path $preferred 'bin/owo_core_service.exe') 'not the packaged core'
        }
        'file' { Write-Fixture $preferred }
    }
    $before = if (Test-Path -LiteralPath $preferred -PathType Container) {
        @(Get-ChildItem -LiteralPath $preferred -File -Recurse -Force | ForEach-Object { $_.FullName + ':' + (Get-FileHash -LiteralPath $_.FullName).Hash })
    } else { @() }
    $rejected = $false
    try { $plan = Get-OwOInstallationTarget -PreferredRoot $preferred -PackageRoot $package }
    catch { $rejected = $true }
    if ($scenario -in @('foreign', 'bad-marker', 'tampered', 'file')) {
        Assert $rejected "Unsafe directory must be rejected: $scenario"
    } else {
        Assert (-not $rejected) "Valid directory must be accepted: $scenario"
        if ($scenario -in @('installed', 'partial', 'marked')) {
            Assert ($plan.InstallRoot -ne $preferred -and (Split-Path -Parent $plan.InstallRoot) -eq $fixtures) 'Retry must use a sibling directory'
            Assert (-not (Test-Path -LiteralPath $plan.InstallRoot)) 'Read-only planning must not create the target'
            $second = Get-OwOInstallationTarget -PreferredRoot $preferred -PackageRoot $package
            Assert ($second.InstallRoot -ne $plan.InstallRoot) 'Repeated plans must avoid target collisions'
        } else { Assert ($plan.InstallRoot -eq $preferred) 'New/empty target should not move' }
        if ($scenario -in @('partial', 'marked')) { Assert ($plan.ExistingKind -eq 'Incomplete') 'Incomplete installs must not become uninstall/cleanup targets' }
        $trailing = Get-OwOInstallationTarget -PreferredRoot ($preferred + '\') -PackageRoot $package
        Assert ((Split-Path -Parent $trailing.InstallRoot) -eq $fixtures) 'Trailing separator must not change the installation parent'
    }
    $after = if (Test-Path -LiteralPath $preferred -PathType Container) {
        @(Get-ChildItem -LiteralPath $preferred -File -Recurse -Force | ForEach-Object { $_.FullName + ':' + (Get-FileHash -LiteralPath $_.FullName).Hash })
    } else { @() }
    Assert (($before -join "`n") -ceq ($after -join "`n")) 'Planning must preserve every existing file'
    Write-Output "PASS: $scenario"
}
$rejected = $false
try { Get-OwOInstallationTarget -PreferredRoot ([IO.Path]::GetPathRoot($fixtures)) -PackageRoot $package | Out-Null } catch { $rejected = $true }
Assert $rejected 'Volume root must be rejected'
Write-Output "Installer directory retry/preservation tests passed. Fixtures: $fixtures"
