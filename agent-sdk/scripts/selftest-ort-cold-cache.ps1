#requires -Version 5.1
# Offline regression: a cold cache must resolve after a successful verified download.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'init-dev-env.ps1')
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('owo-ort-cold-' + [guid]::NewGuid().ToString('N'))
$testLibDir = Join-Path $testRoot "$($script:OwoSherpaAsset)\lib"
$testLib = Join-Path $testLibDir 'onnxruntime.lib'
$originalEnv = @{}
foreach ($name in @('SHERPA_ONNX_LIB_DIR', 'ORT_LIB_PATH')) {
    $originalEnv[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    [Environment]::SetEnvironmentVariable($name, $null, 'Process')
}
$script:OwoRuntimeCache = $testRoot
$script:OwoSdkRoot = Join-Path $testRoot 'sdk-without-legacy-cache'
$script:OwoMinOnnxRuntimeLibBytes = 1
$script:downloadCalls = 0
# Replace only network/extraction. Probe and resolution execute production code.
function Save-OwoSherpaToCache {
    param([string]$ArchivePath)
    if ($ArchivePath -ne (Join-Path $testRoot "$($script:OwoSherpaAsset).tar.bz2")) {
        throw 'Unexpected archive destination'
    }
    $script:downloadCalls++
    [IO.Directory]::CreateDirectory($testLibDir) | Out-Null
    [IO.File]::WriteAllBytes($testLib, [byte[]]@(1))
}
try {
    if (Get-OwoOrtLibDir) { throw 'Fixture unexpectedly starts warm' }
    $resolved = Resolve-OwoOnnxRuntimeLib -AllowDownload
    if ($resolved -ne $testLibDir -or $script:downloadCalls -ne 1) {
        throw 'Cold cache did not resolve the downloaded library'
    }
    $warm = Resolve-OwoOnnxRuntimeLib
    if ($warm -ne $testLibDir -or $script:downloadCalls -ne 1) {
        throw 'Warm cache redownloaded or resolved a different library'
    }
    Write-Host '[PASS] ORT cold-cache resolution and warm-cache reuse'
} finally {
    foreach ($name in $originalEnv.Keys) {
        [Environment]::SetEnvironmentVariable($name, $originalEnv[$name], 'Process')
    }
    # Only delete known fixture entries; no recursive deletion.
    if ([IO.File]::Exists($testLib)) { [IO.File]::Delete($testLib) }
    foreach ($dir in @($testLibDir, (Split-Path $testLibDir -Parent), $testRoot)) {
        if ([IO.Directory]::Exists($dir)) { [IO.Directory]::Delete($dir) }
    }
}
