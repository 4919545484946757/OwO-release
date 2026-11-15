#requires -Version 5.1
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'stage-desktop-sidecar.ps1') -Configuration release -NoBuild
if ($LASTEXITCODE -ne 0) { throw 'Core staging failed' }
& (Join-Path $PSScriptRoot 'stage-ime-gateway.ps1')
if ($LASTEXITCODE -ne 0) { throw 'Intent gateway staging failed' }
& (Join-Path $PSScriptRoot 'stage-owobot.ps1')
