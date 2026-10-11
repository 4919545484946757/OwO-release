# Start the desktop source with the core built during the six-item validation.
param([switch]$DevTools)
$ErrorActionPreference = "Stop"
$sdkRoot = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$corePath = Join-Path $sdkRoot "target\debug\owo-agent.exe"
if (-not (Test-Path -LiteralPath $corePath)) { throw "Build the debug CLI core first, or pass an explicit core to start.ps1." }
$env:OWO_WEB_UI_DIR = Join-Path $sdkRoot "desktop\web"
& (Join-Path $PSScriptRoot "start.ps1") -CoreExe $corePath -DevTools:$DevTools
