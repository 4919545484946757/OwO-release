# Read-only prerequisite helpers, shared by install/uninstall and testable
# without registering profiles, writing language preferences or copying files.
function Get-OwONativeArchitecture {
    param(
        [string]$ProcessArchitecture = $env:PROCESSOR_ARCHITECTURE,
        [string]$WowArchitecture = $env:PROCESSOR_ARCHITEW6432
    )
    if (-not [string]::IsNullOrWhiteSpace($WowArchitecture)) { return $WowArchitecture.ToUpperInvariant() }
    return $ProcessArchitecture.ToUpperInvariant()
}

function Assert-OwOX64OperatingSystem {
    if ((Get-OwONativeArchitecture) -ne 'AMD64') {
        throw 'This OwO package supports x64 Windows only. ARM64 and 32-bit Windows are not supported; no files or TSF profiles have been installed.'
    }
}

function Get-OwONativeSystemDirectory {
    param(
        [string]$WindowsRoot = $env:SystemRoot,
        [bool]$Is64BitProcess = [Environment]::Is64BitProcess,
        [bool]$Is64BitOperatingSystem = [Environment]::Is64BitOperatingSystem
    )
    if ($Is64BitOperatingSystem -and -not $Is64BitProcess) {
        return Join-Path $WindowsRoot 'Sysnative'
    }
    return Join-Path $WindowsRoot 'System32'
}

function Get-OwOChineseLanguageState {
    param([string[]]$RegistryTags)
    $registryReady = @($RegistryTags | Where-Object { $_ -in @('zh-Hans-CN', 'zh-CN') }).Count -gt 0
    $languages = $null
    $chinese = $null
    $queryError = $null
    # Always obtain the writable language object, even when the registry
    # already proves readiness. Do not call Add/Set on a registry-only result.
    try {
        $languages = Get-WinUserLanguageList -ErrorAction Stop
        foreach ($language in $languages) {
            if ($language.LanguageTag -in @('zh-Hans-CN', 'zh-CN')) {
                $chinese = $language
                break
            }
        }
    } catch {
        $queryError = $_.Exception.Message
        if (-not $registryReady) { throw "OwO language detection failed: $queryError" }
    }
    return [pscustomobject]@{
        Ready = $registryReady -or $null -ne $chinese
        Languages = $languages
        Chinese = $chinese
        QueryError = $queryError
    }
}
