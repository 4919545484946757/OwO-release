$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
. (Join-Path $root 'packaging\windows\Uninstall-Common.ps1')
$fixture = Join-Path $root ('build\uninstall-path-test-' + [Guid]::NewGuid().ToString('N'))
$program = Join-Path $fixture 'App'
$outside = Join-Path $fixture 'UserData'
$link = Join-Path $program 'LinkedData'
[void][IO.Directory]::CreateDirectory($program)
[void][IO.Directory]::CreateDirectory($outside)
[IO.File]::WriteAllText((Join-Path $outside 'untouched.txt'), 'outside')
try {
    $denied = $false
    try { Remove-OwOOwnedTree $outside $program } catch { $denied = $true }
    if (-not $denied) { throw 'Unexpected access outside the uninstall boundary.' }
    [void](New-Item -ItemType Junction -Path $link -Target $outside)
    $denied = $false
    try { Remove-OwOOwnedTree $program $program } catch { $denied = $true }
    if (-not $denied -or -not (Test-Path -LiteralPath (Join-Path $outside 'untouched.txt'))) {
        throw 'Junction protection failed.'
    }
    [IO.Directory]::Delete($link, $false) # Delete this fixture's link, not its target.
    $nested = Join-Path $program 'payload'
    [void][IO.Directory]::CreateDirectory($nested)
    [IO.File]::WriteAllText((Join-Path $nested 'file.txt'), 'contained')
    Remove-OwOOwnedTree $program $program
    if (Test-Path -LiteralPath $program) { throw 'Owned test tree was not removed.' }
    if (-not (Test-Path -LiteralPath (Join-Path $outside 'untouched.txt'))) { throw 'Outside sentinel was lost.' }
    Write-Output 'PASS: real test files, boundary enforcement, junction refusal and safe cleanup.'
} finally {
    if (Test-Path -LiteralPath $link) { [IO.Directory]::Delete($link, $false) }
    Remove-OwOOwnedTree $fixture $fixture
}
