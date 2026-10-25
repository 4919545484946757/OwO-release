[CmdletBinding()]
param([switch]$RemoveUserData)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'Install-Prerequisites.ps1')
$nativeSystem = Get-OwONativeSystemDirectory
$nativePowerShell = Join-Path $nativeSystem 'WindowsPowerShell\v1.0\powershell.exe'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $arguments = '-NoLogo -NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '"'
    if ($RemoveUserData) { $arguments += ' -RemoveUserData' }
    try {
        $elevated = Start-Process -FilePath $nativePowerShell -Verb RunAs `
            -ArgumentList $arguments -Wait -PassThru
    } catch {
        throw 'OwO TSF unregistration requires administrator approval. UAC was cancelled or unavailable.'
    }
    exit $elevated.ExitCode
}

$version = '0.2.5'
$defaultProgramRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\OwO\InputMethod'))
$scriptPackageRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$installedVersionRoot = [IO.Path]::GetFullPath((Join-Path $defaultProgramRoot $version))
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OwO Input Method'
$registeredInstallRoot = $null
$registeredInstall = Get-ItemProperty -LiteralPath $uninstallKey `
    -Name InstallLocation -ErrorAction SilentlyContinue
if ($null -ne $registeredInstall) {
    $registeredInstallRoot = $registeredInstall.InstallLocation
}
$installRoot = if (-not [string]::IsNullOrWhiteSpace($registeredInstallRoot)) {
    # ZIP-root uninstallers follow the currently registered side-by-side
    # instance, including a user-selected install location.
    [IO.Path]::GetFullPath($registeredInstallRoot)
} elseif ((Test-Path -LiteralPath (Join-Path $scriptPackageRoot 'bin\OwO.TSF.dll') -PathType Leaf) -and
          (Test-Path -LiteralPath (Join-Path $scriptPackageRoot 'scripts\Uninstall-OwO.ps1') -PathType Leaf)) {
    $scriptPackageRoot
} else {
    # The ZIP-root Uninstall-OwO.cmd invokes this copy of the script. Resolve
    # the actual per-user installation instead of trying to delete the ZIP.
    $installedVersionRoot
}
$programRoot = [IO.Path]::GetFullPath((Split-Path -Parent $installRoot))
$volumeRoot = [IO.Path]::GetPathRoot($installRoot)
if ([string]::Equals($installRoot.TrimEnd([IO.Path]::DirectorySeparatorChar),
        $volumeRoot.TrimEnd([IO.Path]::DirectorySeparatorChar),
        [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing to uninstall a filesystem root.'
}
if (-not (Test-Path -LiteralPath $installRoot -PathType Container)) {
    throw "OwO Input Method $version is not installed: $installRoot"
}
foreach ($marker in @('bin\OwO.TSF.dll', 'scripts\Uninstall-OwO.ps1', 'settings\OwO.Settings.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $installRoot $marker) -PathType Leaf)) {
        throw "Refusing to uninstall a directory without the expected OwO files: $installRoot"
    }
}

$modelPointer = Join-Path $installRoot 'model\active-model-path.txt'
$cachedModel = $null
if (Test-Path -LiteralPath $modelPointer -PathType Leaf) {
    $cachedModel = (Get-Content -Raw -LiteralPath $modelPointer).Trim()
}

foreach ($process in Get-CimInstance Win32_Process | Where-Object {
    $_.Name -in @('owo_core_service.exe', 'owo_model_host.exe') -or
    (-not [string]::IsNullOrWhiteSpace($_.ExecutablePath) -and
     [IO.Path]::GetFullPath($_.ExecutablePath).StartsWith(
         $installRoot + [IO.Path]::DirectorySeparatorChar,
         [StringComparison]::OrdinalIgnoreCase))
}) {
    Stop-Process -Id $process.ProcessId -Force
}

$profileCheck = Join-Path $installRoot 'bin\owo_tsf_profile_check.exe'
$tsf = Join-Path $installRoot 'bin\OwO.TSF.dll'
$x86Tsf = Join-Path $installRoot 'bin\x86\OwO.TSF.dll'
if (Test-Path -LiteralPath $profileCheck) {
    & $profileCheck --disable | Out-Null
    if ($LASTEXITCODE -notin @(0, 4)) {
        throw "OwO profile disable failed: $LASTEXITCODE"
    }
}
if (Test-Path -LiteralPath $tsf) {
    $unregister = Start-Process -FilePath (Join-Path $nativeSystem 'regsvr32.exe') `
        -ArgumentList @('/u', '/s', ('"{0}"' -f $tsf)) `
        -WindowStyle Hidden -Wait -PassThru
    if ($unregister.ExitCode -ne 0) {
        throw "TSF unregistration failed: $($unregister.ExitCode)"
    }
}
if (Test-Path -LiteralPath $x86Tsf) {
    $x86Unregister = Start-Process `
        -FilePath "$env:SystemRoot\SysWOW64\regsvr32.exe" `
        -ArgumentList @('/u', '/s', ('"{0}"' -f $x86Tsf)) `
        -WindowStyle Hidden -Wait -PassThru
    if ($x86Unregister.ExitCode -ne 0) {
        throw "x86 TSF unregistration failed: $($x86Unregister.ExitCode)"
    }
}

$owoTip = '0804:{6D31C9B1-8978-4F49-89B4-66EB1E741591}{5D9F39C3-BDB4-453C-A7BA-B9EF82487629}'
$languages = Get-WinUserLanguageList
$changed = $false
foreach ($language in $languages) {
    if ($language.InputMethodTips -contains $owoTip) {
        [void]$language.InputMethodTips.Remove($owoTip)
        $changed = $true
    }
}
if ($changed) {
    Set-WinUserLanguageList $languages -Force -WarningAction SilentlyContinue
}

Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
    -Name 'OwO Input Method' -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $uninstallKey `
    -Recurse -Force -ErrorAction SilentlyContinue
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\OwO Input Method'
if (Test-Path -LiteralPath $startMenu) { Remove-Item -LiteralPath $startMenu -Recurse -Force }
$desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
if (-not [string]::IsNullOrWhiteSpace($desktop)) {
    Remove-Item -LiteralPath (Join-Path $desktop 'OwO Input Method Settings.lnk') `
        -Force -ErrorAction SilentlyContinue
}
if ($RemoveUserData) {
    $userData = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'OwO\InputMethod'))
    if ($userData.EndsWith('OwO\InputMethod', [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $userData)) {
        Remove-Item -LiteralPath $userData -Recurse -Force
    }
}

if (-not [string]::IsNullOrWhiteSpace($cachedModel)) {
    $modelCacheRoot = [IO.Path]::GetFullPath(
        (Join-Path $env:ProgramData 'OwO\InputMethod\models'))
    $cachedModel = [IO.Path]::GetFullPath($cachedModel)
    $cacheDirectory = [IO.Path]::GetFullPath((Split-Path -Parent $cachedModel))
    $cacheUsedByAnotherInstall = $false
    # Installed instances are direct siblings. Avoid a recursive scan because
    # a custom location may make the selected parent a large application or
    # even a drive-root directory.
    $otherPointers = foreach ($sibling in Get-ChildItem -LiteralPath $programRoot `
            -Directory -ErrorAction SilentlyContinue) {
        $pointer = Join-Path $sibling.FullName 'model\active-model-path.txt'
        if (Test-Path -LiteralPath $pointer -PathType Leaf) { Get-Item -LiteralPath $pointer }
    }
    foreach ($otherPointer in $otherPointers) {
        if ($otherPointer.FullName.Equals($modelPointer, [StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        $otherCachedModel = Get-Content -Raw -LiteralPath $otherPointer.FullName `
            -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($otherCachedModel) -and
            [IO.Path]::GetFullPath($otherCachedModel.Trim()).Equals(
                $cachedModel, [StringComparison]::OrdinalIgnoreCase)) {
            $cacheUsedByAnotherInstall = $true
            break
        }
    }
    if ($cachedModel.StartsWith(
            $modelCacheRoot + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase) -and
        [IO.Path]::GetFileName($cachedModel) -eq 'zh_CN.lm' -and
        -not $cacheUsedByAnotherInstall -and
        (Test-Path -LiteralPath $cacheDirectory -PathType Container)) {
        Remove-Item -LiteralPath $cacheDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$cleanupSource = Join-Path $PSScriptRoot 'Finish-Uninstall-OwO.ps1'
if (-not (Test-Path -LiteralPath $cleanupSource -PathType Leaf)) {
    throw "Uninstall cleanup helper is missing: $cleanupSource"
}
$cleanupRoot = Join-Path ([IO.Path]::GetTempPath()) ("OwO-Uninstall-" + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $cleanupRoot)
$cleanupScript = Join-Path $cleanupRoot 'Finish-Uninstall-OwO.ps1'
Copy-Item -LiteralPath $cleanupSource -Destination $cleanupScript -Force
$cleanupArguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass' +
    ' -File "' + $cleanupScript + '"' +
    ' -InstallRoot "' + $installRoot + '"' +
    ' -ProgramRoot "' + $programRoot + '"' +
    ' -ParentProcessId ' + $PID
Start-Process -FilePath 'powershell.exe' -ArgumentList $cleanupArguments `
    -WindowStyle Hidden | Out-Null

Write-Output "OwO Input Method $version has been unregistered. Program files will now be removed."
