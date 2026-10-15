[CmdletBinding()]
param(
    [string]$CacheRoot = (Join-Path $PSScriptRoot '..\build\dependencies')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$version = '1.28.0'
$expectedSha512 = 'd97d83d031fa744cd67dab61e88deecc8b6c3c11b1a98951b0fcf73852bb4bd4df935d0c71674a19e80d8d3f0a98e02d9129e8df24c5f5ea38de60ef9fdb1a97'
$packageUrl = "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/$version/microsoft.ml.onnxruntime.$version.nupkg"
$packageDirectory = Join-Path $CacheRoot "onnxruntime-$version"
$packagePath = Join-Path $packageDirectory "microsoft.ml.onnxruntime.$version.nupkg"
$sdkRoot = Join-Path $packageDirectory 'sdk'

New-Item -ItemType Directory -Force -Path $packageDirectory | Out-Null
if (-not (Test-Path -LiteralPath $packagePath)) {
    Invoke-WebRequest -UseBasicParsing -Uri $packageUrl -OutFile $packagePath
}

$actualSha512 = (Get-FileHash -Algorithm SHA512 -LiteralPath $packagePath).Hash.ToLowerInvariant()
if ($actualSha512 -ne $expectedSha512) {
    throw "ONNX Runtime package SHA-512 mismatch: $actualSha512"
}

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($packagePath)
try {
    $selectedEntries = @(
        'runtimes/win-x64/native/onnxruntime.dll',
        'runtimes/win-x64/native/onnxruntime.lib',
        'runtimes/win-x64/native/onnxruntime_providers_shared.dll',
        'runtimes/win-x64/native/onnxruntime_providers_shared.lib',
        'LICENSE',
        'ThirdPartyNotices.txt'
    )
    $entries = @($archive.Entries | Where-Object {
        $_.FullName.StartsWith('build/native/include/', [System.StringComparison]::Ordinal) -or
        $selectedEntries -contains $_.FullName
    })
    if ($entries.Count -lt 7) {
        throw 'ONNX Runtime package does not contain the expected native SDK layout'
    }
    foreach ($entry in $entries) {
        $relative = if ($entry.FullName.StartsWith('build/native/include/', [System.StringComparison]::Ordinal)) {
            $entry.FullName.Substring('build/native/'.Length)
        } elseif ($entry.FullName.StartsWith('runtimes/win-x64/native/', [System.StringComparison]::Ordinal)) {
            'bin/' + $entry.Name
        } else {
            'licenses/' + $entry.Name
        }
        $destination = Join-Path $sdkRoot ($relative.Replace('/', [IO.Path]::DirectorySeparatorChar))
        $destinationDirectory = Split-Path -Parent $destination
        New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null
        $sourceStream = $entry.Open()
        try {
            $destinationStream = [IO.File]::Open($destination, [IO.FileMode]::Create,
                                                 [IO.FileAccess]::Write, [IO.FileShare]::None)
            try { $sourceStream.CopyTo($destinationStream) } finally { $destinationStream.Dispose() }
        } finally { $sourceStream.Dispose() }
    }
} finally {
    $archive.Dispose()
}

Write-Output "ONNX Runtime $version SDK ready: $sdkRoot"
Write-Output "Package SHA-512: $actualSha512"
