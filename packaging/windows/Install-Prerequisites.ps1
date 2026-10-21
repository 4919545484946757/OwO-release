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

# Read-only plan. Never delete or reuse a partial installation: a failed copy
# may have left mapped binaries or diagnostics worth preserving.
function Get-OwOInstallationTarget {
    param([Parameter(Mandatory)][string]$PreferredRoot,
          [Parameter(Mandatory)][string]$PackageRoot)
    $preferred = [IO.Path]::GetFullPath($PreferredRoot).TrimEnd('\', '/')
    $parent = Split-Path -Parent $preferred
    if ([string]::IsNullOrWhiteSpace($parent)) { throw 'Refusing a filesystem root as an installation directory.' }
    $kind = 'New'
    if (Test-Path -LiteralPath $preferred) {
        $directory = Get-Item -LiteralPath $preferred -Force -ErrorAction Stop
        if (-not $directory.PSIsContainer -or ($directory.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "The installation destination must be a regular directory: $preferred"
        }
        $files = @(Get-ChildItem -LiteralPath $preferred -Force -ErrorAction Stop)
        if ($files.Count -eq 0) { $kind = 'Empty' }
        elseif ((Test-Path -LiteralPath (Join-Path $preferred 'bin\OwO.TSF.dll') -PathType Leaf) -and
                (Test-Path -LiteralPath (Join-Path $preferred 'scripts\Uninstall-OwO.ps1') -PathType Leaf)) { $kind = 'Installed' }
        else {
            $owned = $false
            $markerPath = Join-Path $preferred '.owo-install-state.json'
            if (Test-Path -LiteralPath $markerPath -PathType Leaf) {
                $markerFile = Get-Item -LiteralPath $markerPath -Force
                if ($markerFile.Length -lt 4096 -and -not ($markerFile.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                    try {
                        $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
                        $owned = $marker.application -eq 'OwO.InputMethod' -and $marker.schema -eq 1 -and
                            $marker.state -in @('copying', 'completed')
                    } catch { $owned = $false }
                }
            }
            # Older failed installers had no ownership marker. Only recognize
            # their partial tree if both OwO binaries match this trusted package.
            if (-not $owned) {
                $owned = $true
                foreach ($relative in @('bin\OwO.TSF.dll', 'bin\owo_core_service.exe')) {
                    $existing = Join-Path $preferred $relative
                    $source = Join-Path $PackageRoot $relative
                    if (-not (Test-Path -LiteralPath $existing -PathType Leaf) -or
                        -not (Test-Path -LiteralPath $source -PathType Leaf) -or
                        ((Get-Item -LiteralPath $existing -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                        $owned = $false; break
                    }
                    if ((Get-FileHash -LiteralPath $existing -Algorithm SHA256).Hash -ne
                        (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash) { $owned = $false; break }
                }
            }
            if (-not $owned) {
                throw "The selected installation directory is not empty and cannot be verified as an OwO installation. Existing files were preserved. Choose an empty directory: $preferred"
            }
            $kind = 'Incomplete'
        }
    }
    $target = $preferred
    if ($kind -in @('Installed', 'Incomplete')) {
        do {
            $instance = (Get-Date -Format 'yyyyMMddHHmmss') + '-' + [Guid]::NewGuid().ToString('N')
            $target = [IO.Path]::GetFullPath((Join-Path $parent ([IO.Path]::GetFileName($preferred) + '-' + $instance)))
        } while (Test-Path -LiteralPath $target)
    }
    if (-not $target.StartsWith($parent.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase)) { throw 'Refusing an installation path outside its selected parent directory.' }
    return [pscustomobject]@{ InstallRoot = $target; ExistingKind = $kind }
}
