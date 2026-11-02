[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$testRoot = [IO.Path]::GetFullPath((Join-Path $root 'build/tests/publish'))
$fixture = Join-Path $testRoot ([Guid]::NewGuid().ToString('N'))
$script = Join-Path $root 'scripts/publish_github_release.ps1'
$commit = (& git -C $root rev-parse HEAD).Trim()
$version = '99.0.0'
function Write-Receipt([bool]$dirty, [string]$sourceCommit) {
    $data = @{schema_version=1;version=$version;git_commit=$sourceCommit;dirty=$dirty;built_at_utc=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json
    [IO.File]::WriteAllText((Join-Path $fixture 'release-build.json'), $data)
}
function Run-Plan([bool]$success) {
    # Windows PowerShell turns redirected native stderr into error records.
    # Negative cases deliberately emit these; assert the exit code instead.
    $ErrorActionPreference = 'Continue'
    $result = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $script -Version $version -ReleaseDirectory $fixture 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if (($code -eq 0) -ne $success) { throw "Unexpected publisher outcome: $result" }
    if ($success -and ($result -join "`n") -notmatch 'Validation only') { throw 'Missing dry-run confirmation.' }
    if (Test-Path -LiteralPath (Join-Path $fixture 'owo-resources.json')) { throw 'Dry-run wrote a resource catalog.' }
}
try {
    $null = New-Item -ItemType Directory -Path $fixture -Force
    $names = @("OwO-Input-Method-$version-windows-x64-Setup.exe", "OwO-Input-Method-$version-windows-x64.zip", "OwO-Input-Method-$version-source.zip")
    $hashes = foreach ($name in $names) {
        $path = Join-Path $fixture $name
        [IO.File]::WriteAllBytes($path, [byte[]]@(1,2,3,4))
        "$((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant())  $name"
    }
    [IO.File]::WriteAllLines((Join-Path $fixture 'SHA256SUMS.txt'), $hashes)
    Write-Receipt $false $commit
    Run-Plan $true
    Write-Receipt $true $commit
    Run-Plan $false
    Write-Receipt $false ('0' * 40)
    Run-Plan $false
    Write-Receipt $false $commit
    [IO.File]::WriteAllBytes((Join-Path $fixture $names[0]), [byte[]]@(5,6,7))
    Run-Plan $false
    Write-Output 'Publisher offline tests passed: dry-run, no catalog writes, dirty provenance, wrong commit and modified payload rejected. No network writes.'
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    if (!$resolved.StartsWith($testRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe test cleanup path.' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
