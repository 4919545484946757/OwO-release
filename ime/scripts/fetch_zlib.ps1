param(
    [string]$DependencyRoot = (Join-Path $PSScriptRoot "..\build\dependencies")
)

$ErrorActionPreference = "Stop"
$version = "1.3.2"
$expectedSha256 = "e8bf55f3017aa181690990cb58a994e77885da140609fc8f94abe9b65d2cae28"
$resolvedRoot = [System.IO.Path]::GetFullPath($DependencyRoot)
$target = Join-Path $resolvedRoot "zlib-$version\source"
$header = Join-Path $target "zlib.h"
$receipt = Join-Path (Split-Path -Parent $target) "source-archive.sha256"
if (Test-Path -LiteralPath $header) {
    if ((Test-Path -LiteralPath $receipt) -and
        ([System.IO.File]::ReadAllText($receipt).Trim().ToLowerInvariant() -eq $expectedSha256)) {
        Write-Output $target
        exit 0
    }
    throw "Existing zlib source has no matching verified archive receipt: $target"
}
if (Test-Path -LiteralPath $target) {
    throw "Refusing to replace incomplete existing dependency directory: $target"
}

New-Item -ItemType Directory -Path $resolvedRoot -Force | Out-Null
$work = Join-Path $resolvedRoot ("zlib-download-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $archive = Join-Path $work "zlib-$version.zip"
    Invoke-WebRequest -Uri "https://zlib.net/zlib132.zip" -OutFile $archive
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    if ($actual -ne $expectedSha256) {
        throw "zlib archive SHA-256 mismatch: expected $expectedSha256, got $actual"
    }
    Expand-Archive -LiteralPath $archive -DestinationPath $work
    $extracted = Join-Path $work "zlib-$version"
    if (-not (Test-Path -LiteralPath (Join-Path $extracted "zlib.h"))) {
        throw "Verified zlib archive does not contain the expected source tree"
    }
    $targetParent = Split-Path -Parent $target
    New-Item -ItemType Directory -Path $targetParent | Out-Null
    Move-Item -LiteralPath $extracted -Destination $target
    [System.IO.File]::WriteAllText($receipt, $expectedSha256 + [Environment]::NewLine,
                                  [System.Text.UTF8Encoding]::new($false))
    Write-Output $target
} finally {
    if (Test-Path -LiteralPath $work) {
        Remove-Item -Recurse -Force -LiteralPath $work
    }
}
