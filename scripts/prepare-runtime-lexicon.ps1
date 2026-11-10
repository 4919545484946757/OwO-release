[CmdletBinding()]
param([string]$BuildDirectory)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([string]::IsNullOrWhiteSpace($BuildDirectory)) {
    $BuildDirectory = Join-Path $projectRoot 'build/windows-release/Release'
}
$source = Join-Path $projectRoot 'build/windows-release/rime-ice-cn-2026.06.30-v2.owolx'
$destination = Join-Path $projectRoot 'build/windows-release/rime-ice-cn-2026.06.30-v3.owolx'
$compiler = Join-Path $BuildDirectory 'owo_lexicon_merge.exe'
if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing source lexicon: $source" }
if (-not (Test-Path -LiteralPath $compiler -PathType Leaf)) {
    throw 'Build the owo_lexicon_merge target before preparing the runtime lexicon.'
}
$sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
$manifest = $destination + '.build.json'
if ((Test-Path -LiteralPath $destination -PathType Leaf) -and (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    $previous = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
    if ($previous.source_sha256 -eq $sourceHash -and $previous.version -eq 3 -and
        $previous.output_sha256 -eq (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash) {
        Write-Output "Runtime v3 lexicon is current: $destination"
        return
    }
}
# Conversion rebuilds row ordering and every row-index posting; changing only
# the version field would falsely claim that exact buckets are frequency sorted.
$temporary = $destination + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
try {
    & $compiler $temporary $source
    if ($LASTEXITCODE -ne 0) { throw "Runtime lexicon conversion failed: $LASTEXITCODE" }
    Move-Item -LiteralPath $temporary -Destination $destination -Force
    @{
        version = 3; source_sha256 = $sourceHash
        output_sha256 = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash
    } | ConvertTo-Json | Set-Content -LiteralPath $manifest -Encoding ASCII
} finally {
    if (Test-Path -LiteralPath $temporary -PathType Leaf) { Remove-Item -LiteralPath $temporary }
}
Write-Output "Runtime v3 lexicon prepared: $destination"
