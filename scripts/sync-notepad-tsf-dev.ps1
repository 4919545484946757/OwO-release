param(
    [Parameter(Mandatory)][string]$DllPath,
    [Parameter(Mandatory)][string]$EvidenceDirectory
)
$ErrorActionPreference = 'Stop'
$DllPath = [IO.Path]::GetFullPath($DllPath)
if (-not (Test-Path -LiteralPath $DllPath -PathType Leaf)) { throw 'Development DLL does not exist' }
$key = 'HKCU:\Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}\InprocServer32'
if ((Get-Item $key).GetValue('') -ne $DllPath) { throw 'Desktop x64 registration does not match requested DLL' }
if (-not (Get-AppxPackage Microsoft.WindowsNotepad)) { return }
$EvidenceDirectory = [IO.Path]::GetFullPath($EvidenceDirectory)
if (Test-Path -LiteralPath (Join-Path $EvidenceDirectory 'request.json')) {
    throw 'Use a fresh evidence directory; existing registration backups must not be overwritten'
}
[void](New-Item -ItemType Directory -Path $EvidenceDirectory -Force)
# Use the documented package-context debugger. Only this development CLSID is
# synchronized; never reset the package, its registry hive, or user documents.
$request = Join-Path $EvidenceDirectory 'request.json'
@{ dll = $DllPath; output = $EvidenceDirectory } | ConvertTo-Json | Set-Content -LiteralPath $request -Encoding UTF8
$worker = Join-Path $EvidenceDirectory 'sync.ps1'
@'
param([string]$Request)
$ErrorActionPreference = 'Stop'
$r = Get-Content -LiteralPath $Request -Raw | ConvertFrom-Json
$result = Join-Path $r.output 'result.json'
try {
    $sub = 'Software\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}\InprocServer32'
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($sub, $true)
    if (-not $key) { throw 'OwO registration absent in package view' }
    try {
        $before = $key.GetValue('')
        $threading = $key.GetValue('ThreadingModel')
        if ($before -ne $r.dll -or $threading -ne 'Apartment') {
            # Exact previous values are saved before either write, for rollback
            # in the same package context. No unrelated values are touched.
            @{ dll=$before; threading=$threading } | ConvertTo-Json |
                Set-Content -LiteralPath (Join-Path $r.output 'before.json') -Encoding UTF8
            $key.SetValue('', $r.dll, [Microsoft.Win32.RegistryValueKind]::String)
            $key.SetValue('ThreadingModel', 'Apartment', [Microsoft.Win32.RegistryValueKind]::String)
        }
        if ($key.GetValue('') -ne $r.dll -or $key.GetValue('ThreadingModel') -ne 'Apartment') {
            throw 'Package registration verification failed'
        }
        @{ status='PASS'; before=$before; after=$key.GetValue('') } | ConvertTo-Json |
            Set-Content -LiteralPath $result -Encoding UTF8
    } finally { $key.Dispose() }
} catch {
    @{ status='FAIL'; error="$PSItem" } | ConvertTo-Json | Set-Content -LiteralPath $result -Encoding UTF8
    exit 1
}
'@ | Set-Content -LiteralPath $worker -Encoding UTF8
Invoke-CommandInDesktopPackage -PackageFamilyName Microsoft.WindowsNotepad_8wekyb3d8bbwe -AppId App `
    -Command "$env:WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" `
    -Args ('-NoProfile -WindowStyle Hidden -File "{0}" -Request "{1}"' -f $worker, $request) -PreventBreakaway
$result = Join-Path $EvidenceDirectory 'result.json'
$deadline = [DateTime]::UtcNow.AddSeconds(15)
while (-not (Test-Path -LiteralPath $result)) {
    if ([DateTime]::UtcNow -ge $deadline) { throw "Package registration sync timed out; evidence: $EvidenceDirectory" }
    Start-Sleep -Milliseconds 100
}
$response = Get-Content -LiteralPath $result -Raw | ConvertFrom-Json
if ($response.status -ne 'PASS') { throw "Package registration sync failed: $($response.error)" }
Write-Output "Notepad OwO registration: $($response.before) -> $($response.after). Backup: $EvidenceDirectory"
