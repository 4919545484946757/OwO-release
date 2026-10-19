[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$HostPath,
    [Parameter(Mandatory = $true)][string]$SdkRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$hostDirectory = Split-Path -Parent (Resolve-Path -LiteralPath $HostPath)
$deployedDll = Join-Path $hostDirectory 'onnxruntime.dll'
$sdkDll = Join-Path $SdkRoot 'bin\onnxruntime.dll'
$deployedLicenseRoot = Join-Path $hostDirectory 'licenses\onnxruntime'
$pairs = @(
    @((Join-Path $SdkRoot 'licenses\LICENSE'), (Join-Path $deployedLicenseRoot 'LICENSE')),
    @((Join-Path $SdkRoot 'licenses\ThirdPartyNotices.txt'),
      (Join-Path $deployedLicenseRoot 'ThirdPartyNotices.txt')),
    @($sdkDll, $deployedDll)
)

foreach ($pair in $pairs) {
    if (-not (Test-Path -LiteralPath $pair[0]) -or -not (Test-Path -LiteralPath $pair[1])) {
        throw "Required ONNX Runtime release file is missing: $($pair[1])"
    }
    $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $pair[0]).Hash
    $deployedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $pair[1]).Hash
    if ($sourceHash -ne $deployedHash) {
        throw "ONNX Runtime release file differs from verified SDK: $($pair[1])"
    }
}

Write-Output 'ONNX Runtime DLL and license release layout verified'
