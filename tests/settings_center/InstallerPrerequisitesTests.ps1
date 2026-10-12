$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot '../../packaging/windows/Install-Prerequisites.ps1')
function Assert($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
Assert ((Get-OwONativeArchitecture 'x86' 'AMD64') -eq 'AMD64') 'WOW64 must identify x64 OS'
Assert ((Get-OwONativeArchitecture 'AMD64' 'ARM64') -eq 'ARM64') 'Emulated x64 must identify ARM64 OS'
Assert ((Get-OwONativeArchitecture 'x86' '') -eq 'X86') '32-bit OS must stay x86'
Assert ((Get-OwONativeSystemDirectory 'C:\Windows' $false $true) -eq 'C:\Windows\Sysnative') '32-bit host needs Sysnative'
Assert ((Get-OwONativeSystemDirectory 'C:\Windows' $true $true) -eq 'C:\Windows\System32') '64-bit host needs System32'
if ([Environment]::Is64BitOperatingSystem) {
    $nativeRegistrar = Join-Path (Get-OwONativeSystemDirectory) 'regsvr32.exe'
    $stream = [IO.File]::OpenRead($nativeRegistrar)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        $stream.Position = 0x3c
        $header = $reader.ReadInt32()
        $stream.Position = $header + 4
        Assert ($reader.ReadUInt16() -eq 0x8664) 'Actual native registrar must be x64, even under 32-bit PowerShell'
    } finally { $reader.Dispose() }
}
function Get-OwONativeArchitecture { return $script:MockArchitecture }
foreach ($script:MockArchitecture in @('ARM64', 'X86', 'UNKNOWN')) {
    $rejected = $false
    try { Assert-OwOX64OperatingSystem } catch { $rejected = $true }
    Assert $rejected "Unsupported OS must be rejected: $script:MockArchitecture"
}
$script:MockArchitecture = 'AMD64'
Assert-OwOX64OperatingSystem

# Stub only the read API: these tests never modify system language preferences.
function Get-WinUserLanguageList {
    param($ErrorAction)
    if ($script:FailQuery) { throw 'simulated query failure' }
    return $script:MockLanguages
}
$script:FailQuery = $false
$script:MockLanguages = [Collections.Generic.List[object]]::new()
$script:MockLanguages.Add([pscustomobject]@{ LanguageTag = 'en-US'; InputMethodTips = [Collections.Generic.List[string]]::new() })
$script:MockLanguages.Add([pscustomobject]@{ LanguageTag = 'zh-Hans-CN'; InputMethodTips = [Collections.Generic.List[string]]::new() })
$state = Get-OwOChineseLanguageState @('zh-Hans-CN', 'en-US')
Assert ($state.Ready -and $null -ne $state.Chinese) 'Registry success must still retrieve writable Chinese object'
$state.Chinese.InputMethodTips.Add('test-tip')
Assert ($state.Chinese.InputMethodTips.Contains('test-tip')) 'Retrieved object must support tip additions'
$state = Get-OwOChineseLanguageState @('en-US')
Assert ($state.Ready -and $null -ne $state.Chinese) 'Language-list-only readiness must work'
$script:FailQuery = $true
$state = Get-OwOChineseLanguageState @('zh-CN')
Assert ($state.Ready -and $null -eq $state.Chinese -and $null -ne $state.QueryError) 'Registry fallback must not fabricate mutable object'
$failed = $false
try { Get-OwOChineseLanguageState @('en-US') | Out-Null } catch { $failed = $true }
Assert $failed 'Query failure without registry evidence must stop install'
$script:FailQuery = $false
$script:MockLanguages.Clear()
$state = Get-OwOChineseLanguageState @('en-US')
Assert (-not $state.Ready) 'Missing Chinese must fail readiness'

foreach ($name in @('Install-OwO.ps1', 'Uninstall-OwO.ps1', 'Install-Prerequisites.ps1')) {
    $tokens = $null; $parseErrors = $null
    [Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $PSScriptRoot "../../packaging/windows/$name"), [ref]$tokens, [ref]$parseErrors) | Out-Null
    Assert ($parseErrors.Count -eq 0) "Syntax errors in $name"
}
Write-Output 'Installer architecture, language object/fallback, and script syntax tests passed.'
