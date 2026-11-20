[CmdletBinding()]
param(
    [switch]$NoSettingsStartMenuShortcut,
    [switch]$CreateSettingsDesktopShortcut,
    [string]$InstallDirectory,
    [switch]$ReportProgress
)

$ErrorActionPreference = 'Stop'
if ($ReportProgress) { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) }
function Write-OwOInstallProgress([int]$Percent, [string]$Stage) {
    if ($ReportProgress) {
        [Console]::Out.WriteLine("OWO_SETUP_PROGRESS|$Percent|$Stage")
        [Console]::Out.Flush()
    }
}
. (Join-Path $PSScriptRoot 'Install-Prerequisites.ps1')
Assert-OwOX64OperatingSystem
$nativeSystem = Get-OwONativeSystemDirectory
$nativePowerShell = Join-Path $nativeSystem 'WindowsPowerShell\v1.0\powershell.exe'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $arguments = '-NoLogo -NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '"'
    if ($NoSettingsStartMenuShortcut) { $arguments += ' -NoSettingsStartMenuShortcut' }
    if ($CreateSettingsDesktopShortcut) { $arguments += ' -CreateSettingsDesktopShortcut' }
    if ($ReportProgress) { $arguments += ' -ReportProgress' }
    if (-not [string]::IsNullOrWhiteSpace($InstallDirectory)) {
        $arguments += ' -InstallDirectory "' + $InstallDirectory.Replace('"', '""') + '"'
    }
    try {
        $elevated = Start-Process -FilePath $nativePowerShell -Verb RunAs `
            -ArgumentList $arguments -Wait -PassThru
    } catch {
        throw 'OwO TSF registration requires administrator approval. UAC was cancelled or unavailable.'
    }
    exit $elevated.ExitCode
}

Write-OwOInstallProgress 3 'prepare'
# OwO is registered as a Simplified Chinese TSF profile (LANGID 0x0804).
# Without the corresponding user-language entry Windows can register the COM
# server while still refusing to expose or activate the input profile. Fail
# before copying files so ZIP/script installs receive the same actionable
# diagnosis as the graphical setup wizard.
$profile = Get-ItemProperty -LiteralPath 'HKCU:\Control Panel\International\User Profile' `
    -Name Languages -ErrorAction SilentlyContinue
$languageState = Get-OwOChineseLanguageState -RegistryTags $profile.Languages
$languages = $languageState.Languages
$chinese = $languageState.Chinese
if (-not $languageState.Ready) {
    throw @'
OwO requires the Windows "Chinese (Simplified, China)" language and basic typing support.
Open Settings > Time & language > Language & region, add Chinese (Simplified, China),
wait for the language components to finish downloading, and then run this installer again.
'@
}

$version = '0.2.5'
$packageRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$defaultProgramRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Programs\OwO\InputMethod'))
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OwO Input Method'
$preferredInstallRoot = if ([string]::IsNullOrWhiteSpace($InstallDirectory)) {
    [IO.Path]::GetFullPath((Join-Path $defaultProgramRoot $version))
} else {
    [IO.Path]::GetFullPath($InstallDirectory)
}
$volumeRoot = [IO.Path]::GetPathRoot($preferredInstallRoot)
if ([string]::Equals($preferredInstallRoot.TrimEnd([IO.Path]::DirectorySeparatorChar),
        $volumeRoot.TrimEnd([IO.Path]::DirectorySeparatorChar),
        [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing to install OwO into a filesystem root.'
}
$preferredInstallRoot = $preferredInstallRoot.TrimEnd('\', '/')
$programRoot = [IO.Path]::GetFullPath((Split-Path -Parent $preferredInstallRoot))
$normalizedPackageRoot = $packageRoot.TrimEnd([IO.Path]::DirectorySeparatorChar)
$normalizedInstallRoot = $preferredInstallRoot.TrimEnd([IO.Path]::DirectorySeparatorChar)
if ($normalizedInstallRoot.Equals($normalizedPackageRoot, [StringComparison]::OrdinalIgnoreCase) -or
    $normalizedInstallRoot.StartsWith(
        $normalizedPackageRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase) -or
    $normalizedPackageRoot.StartsWith(
        $normalizedInstallRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing an installation path that overlaps the installer package.'
}
$registeredInstallRoot = $null
$registeredInstall = Get-ItemProperty -LiteralPath $uninstallKey `
    -Name InstallLocation -ErrorAction SilentlyContinue
if ($null -ne $registeredInstall) {
    $registeredInstallRoot = $registeredInstall.InstallLocation
}
$previousInstallRoot = $null
if (-not [string]::IsNullOrWhiteSpace($registeredInstallRoot)) {
    $registeredInstallRoot = [IO.Path]::GetFullPath($registeredInstallRoot)
    if ((Test-Path -LiteralPath $registeredInstallRoot -PathType Container) -and
        (Test-Path -LiteralPath (Join-Path $registeredInstallRoot 'bin\OwO.TSF.dll') -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $registeredInstallRoot 'scripts\Uninstall-OwO.ps1') -PathType Leaf)) {
        $previousInstallRoot = $registeredInstallRoot
    }
} elseif ((Test-Path -LiteralPath $preferredInstallRoot -PathType Container) -and
          (Test-Path -LiteralPath (Join-Path $preferredInstallRoot 'bin\OwO.TSF.dll') -PathType Leaf) -and
          (Test-Path -LiteralPath (Join-Path $preferredInstallRoot 'scripts\Uninstall-OwO.ps1') -PathType Leaf)) {
    $previousInstallRoot = $preferredInstallRoot
}

# Never overwrite an existing version directory. OwO.TSF.dll may remain mapped
# in arbitrary text-input clients after unregistration, and Windows correctly
# rejects replacing a mapped image. Stage this reinstall side-by-side, switch
# registration, then retire the previous instance asynchronously.
$targetPlan = Get-OwOInstallationTarget -PreferredRoot $preferredInstallRoot -PackageRoot $packageRoot
$installRoot = $targetPlan.InstallRoot
if ($targetPlan.ExistingKind -eq 'Incomplete') {
    Write-Warning "A previous incomplete OwO installation was found and preserved: $preferredInstallRoot"
}
Write-Output "OwO installation destination: $installRoot"
if (-not $installRoot.StartsWith($programRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing an installation path outside its selected parent directory.'
}

[void](New-Item -ItemType Directory -Path $installRoot -Force)
$installMarker = Join-Path $installRoot '.owo-install-state.json'
$installState = [ordered]@{ application = 'OwO.InputMethod'; schema = 1; state = 'copying'; version = $version }
[IO.File]::WriteAllText($installMarker, ($installState | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
$copyProgress = @{ bin = 8; data = 20; model = 32; settings = 45; scripts = 60; LICENSES = 65 }
foreach ($directory in @('bin', 'data', 'model', 'settings', 'scripts', 'LICENSES')) {
    Write-OwOInstallProgress $copyProgress[$directory] "copy-$directory"
    $source = Join-Path $packageRoot $directory
    if (-not (Test-Path -LiteralPath $source)) { throw "Package directory is missing: $directory" }
    Copy-Item -LiteralPath $source -Destination $installRoot -Recurse -Force
}
Copy-Item -LiteralPath (Join-Path $packageRoot 'README.zh-CN.md') `
    -Destination $installRoot -Force
Copy-Item -LiteralPath (Join-Path $packageRoot 'THIRD-PARTY-NOTICES.md') `
    -Destination $installRoot -Force
Copy-Item -LiteralPath (Join-Path $packageRoot 'Open-OwO-Settings.cmd') `
    -Destination $installRoot -Force
Copy-Item -LiteralPath (Join-Path $packageRoot 'Uninstall-OwO.cmd') `
    -Destination $installRoot -Force

# Files extracted from a downloaded ZIP can inherit Mark-of-the-Web. The user
# explicitly launched this installer, so remove that stream only from the exact
# copied OwO installation tree before Windows loads its DLLs and executables.
Get-ChildItem -LiteralPath $installRoot -File -Recurse | Unblock-File

Write-OwOInstallProgress 70 'verify'
# Verify the copied v2 lexicon once during installation. Runtime loading uses
# this immutable validation record (format, size and write timestamp) to avoid
# scanning the complete mapped payload after every sign-in.
$sourceLexicon = Join-Path $packageRoot 'data\owo-cn.owolx'
$installedLexicon = Join-Path $installRoot 'data\owo-cn.owolx'
$sourceLexiconHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $sourceLexicon).Hash.ToLowerInvariant()
$installedLexiconHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installedLexicon).Hash.ToLowerInvariant()
if ($sourceLexiconHash -ne $installedLexiconHash) {
    throw 'Installed OwO lexicon failed SHA-256 validation.'
}
$lexiconInfo = Get-Item -LiteralPath $installedLexicon
$lexiconStream = [IO.File]::OpenRead($installedLexicon)
$lexiconReader = $null
try {
    $lexiconReader = [IO.BinaryReader]::new($lexiconStream)
    $lexiconMagic = [Text.Encoding]::ASCII.GetString($lexiconReader.ReadBytes(4))
    $lexiconVersion = $lexiconReader.ReadUInt32()
} finally {
    if ($null -ne $lexiconReader) { $lexiconReader.Dispose() }
    else { $lexiconStream.Dispose() }
}
if ($lexiconMagic -ne 'OWLX' -or $lexiconVersion -ne 2) {
    throw 'Installed OwO lexicon is not the required v2 format.'
}
$lexiconValidation = @(
    'OWO_LEXICON_VALIDATION_V1'
    'version=2'
    "size=$($lexiconInfo.Length)"
    "write_time_utc_ticks=$($lexiconInfo.LastWriteTimeUtc.Ticks)"
    "sha256=$installedLexiconHash"
)
Set-Content -LiteralPath ($installedLexicon + '.validation') `
    -Value $lexiconValidation -Encoding ASCII

# kenlm opens its model through the narrow Windows CRT API. Even though
# ModelHost accepts proper UTF-16 command-line arguments, a model below a
# non-ASCII user profile would still be mojibake inside kenlm. Cache this
# immutable model by content hash below the machine-wide ASCII ProgramData
# path and record the exact location for Start-OwO.ps1.
$installedModel = Join-Path $installRoot 'model\zh_CN.lm'
$modelHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $installedModel).Hash.ToLowerInvariant()
$modelCacheRoot = [IO.Path]::GetFullPath((Join-Path $env:ProgramData 'OwO\InputMethod\models'))
$modelCacheDirectory = [IO.Path]::GetFullPath((Join-Path $modelCacheRoot $modelHash))
if (-not $modelCacheDirectory.StartsWith(
        $modelCacheRoot + [IO.Path]::DirectorySeparatorChar,
        [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Invalid OwO model cache path.'
}
[void](New-Item -ItemType Directory -Path $modelCacheDirectory -Force)
$cachedModel = Join-Path $modelCacheDirectory 'zh_CN.lm'
if (-not (Test-Path -LiteralPath $cachedModel -PathType Leaf) -or
    (Get-FileHash -Algorithm SHA256 -LiteralPath $cachedModel).Hash.ToLowerInvariant() -ne
        $modelHash) {
    Copy-Item -LiteralPath $installedModel -Destination $cachedModel -Force
}
Set-Content -LiteralPath (Join-Path $installRoot 'model\active-model-path.txt') `
    -Value $cachedModel -Encoding ASCII

$oldProfileCheck = if ($null -ne $previousInstallRoot) {
    Join-Path $previousInstallRoot 'bin\owo_tsf_profile_check.exe'
} else { $null }
$oldTsf = if ($null -ne $previousInstallRoot) {
    Join-Path $previousInstallRoot 'bin\OwO.TSF.dll'
} else { $null }
$oldX86Tsf = if ($null -ne $previousInstallRoot) {
    Join-Path $previousInstallRoot 'bin\x86\OwO.TSF.dll'
} else { $null }
Write-OwOInstallProgress 80 'retire'
# Retire services started from old extracted packages as well as registered
# install roots. Their global pipe names can mask the newly installed service.
foreach ($process in Get-CimInstance Win32_Process | Where-Object {
    $_.Name -in @('owo_core_service.exe', 'owo_model_host.exe')
}) {
    Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
}
if ($null -ne $previousInstallRoot) {
    foreach ($process in Get-CimInstance Win32_Process | Where-Object {
        -not [string]::IsNullOrWhiteSpace($_.ExecutablePath) -and
        [IO.Path]::GetFullPath($_.ExecutablePath).StartsWith(
            $previousInstallRoot + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase)
    }) {
        Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
    }
}
if ($null -ne $oldProfileCheck -and (Test-Path -LiteralPath $oldProfileCheck -PathType Leaf)) {
    & $oldProfileCheck --disable | Out-Null
    if ($LASTEXITCODE -notin @(0, 4)) {
        Write-Warning "Previous OwO profile could not be disabled cleanly: $LASTEXITCODE"
    }
}
if ($null -ne $oldTsf -and (Test-Path -LiteralPath $oldTsf -PathType Leaf)) {
    $oldUnregister = Start-Process -FilePath (Join-Path $nativeSystem 'regsvr32.exe') `
        -ArgumentList @('/u', '/s', ('"{0}"' -f $oldTsf)) `
        -WindowStyle Hidden -Wait -PassThru
    if ($oldUnregister.ExitCode -ne 0) {
        Write-Warning "Previous TSF registration could not be removed cleanly: $($oldUnregister.ExitCode)"
    }
}
if ($null -ne $oldX86Tsf -and (Test-Path -LiteralPath $oldX86Tsf -PathType Leaf)) {
    $oldX86Unregister = Start-Process `
        -FilePath "$env:SystemRoot\SysWOW64\regsvr32.exe" `
        -ArgumentList @('/u', '/s', ('"{0}"' -f $oldX86Tsf)) `
        -WindowStyle Hidden -Wait -PassThru
    if ($oldX86Unregister.ExitCode -ne 0) {
        Write-Warning "Previous x86 TSF registration could not be removed cleanly: $($oldX86Unregister.ExitCode)"
    }
}

$tsf = Join-Path $installRoot 'bin\OwO.TSF.dll'
Write-OwOInstallProgress 85 'register'
$x86Tsf = Join-Path $installRoot 'bin\x86\OwO.TSF.dll'
$profileCheck = Join-Path $installRoot 'bin\owo_tsf_profile_check.exe'
$register = Start-Process -FilePath (Join-Path $nativeSystem 'regsvr32.exe') `
    -ArgumentList @('/s', ('"{0}"' -f $tsf)) -WindowStyle Hidden -Wait -PassThru
if ($register.ExitCode -ne 0) { throw "TSF registration failed: $($register.ExitCode)" }
$x86Register = Start-Process -FilePath "$env:SystemRoot\SysWOW64\regsvr32.exe" `
    -ArgumentList @('/s', ('"{0}"' -f $x86Tsf)) -WindowStyle Hidden -Wait -PassThru
if ($x86Register.ExitCode -ne 0) {
    throw "x86 TSF registration failed: $($x86Register.ExitCode)"
}
& $profileCheck --enable
if ($LASTEXITCODE -ne 0) { throw "OwO profile enable failed: $LASTEXITCODE" }

$owoTip = '0804:{6D31C9B1-8978-4F49-89B4-66EB1E741591}{5D9F39C3-BDB4-453C-A7BA-B9EF82487629}'
Write-OwOInstallProgress 89 'preferences'
if ($null -eq $chinese) {
    Write-Warning 'Chinese language readiness was verified through the user registry, but the writable language list is unavailable. Keeping existing language preferences; use Win+Space to select OwO.'
} elseif ($chinese.InputMethodTips -notcontains $owoTip) {
    $chinese.InputMethodTips.Add($owoTip)
    Set-WinUserLanguageList $languages -Force -WarningAction SilentlyContinue
}

$launcher = Join-Path $installRoot 'bin\owo_runtime_launcher.exe'
if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
    throw "OwO native runtime launcher is missing: $launcher"
}
$runCommand = '"' + $launcher + '"'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
Set-ItemProperty -Path $runKey -Name 'OwO Input Method' -Value $runCommand

[void](New-Item -Path $uninstallKey -Force)
$uninstallScript = Join-Path $installRoot 'scripts\Uninstall-OwO.ps1'
$uninstallCommand = 'powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "' +
                    $uninstallScript + '"'
Set-ItemProperty -Path $uninstallKey -Name DisplayName -Value 'OwO Input Method'
Set-ItemProperty -Path $uninstallKey -Name DisplayVersion -Value $version
Set-ItemProperty -Path $uninstallKey -Name Publisher -Value 'OwO Input Method Project'
Set-ItemProperty -Path $uninstallKey -Name InstallLocation -Value $installRoot
Set-ItemProperty -Path $uninstallKey -Name DisplayIcon `
    -Value (Join-Path $installRoot 'settings\Assets\AppIcon.ico')
Set-ItemProperty -Path $uninstallKey -Name UninstallString -Value $uninstallCommand
Set-ItemProperty -Path $uninstallKey -Name QuietUninstallString -Value $uninstallCommand
New-ItemProperty -Path $uninstallKey -Name NoModify -PropertyType DWord -Value 1 `
    -Force | Out-Null
New-ItemProperty -Path $uninstallKey -Name NoRepair -PropertyType DWord -Value 1 `
    -Force | Out-Null
$estimatedSize = [int]([math]::Ceiling(((Get-ChildItem -LiteralPath $installRoot -File -Recurse |
    Measure-Object -Property Length -Sum).Sum / 1KB)))
New-ItemProperty -Path $uninstallKey -Name EstimatedSize -PropertyType DWord `
    -Value $estimatedSize -Force | Out-Null

$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\OwO Input Method'
[void](New-Item -ItemType Directory -Path $startMenu -Force)
try {
    $shell = New-Object -ComObject WScript.Shell
    $settingsExecutable = Join-Path $installRoot 'settings\OwO.Settings.exe'
    $settingsDirectory = Join-Path $installRoot 'settings'
    if (-not $NoSettingsStartMenuShortcut) {
        $settingsShortcut = $shell.CreateShortcut(
            [IO.Path]::Combine($startMenu, 'OwO Input Method Settings.lnk'))
        $settingsShortcut.TargetPath = $settingsExecutable
        $settingsShortcut.WorkingDirectory = $settingsDirectory
        $settingsShortcut.IconLocation = $settingsExecutable + ',0'
        $settingsShortcut.Save()
    }
    $uninstallShortcut = $shell.CreateShortcut(
        [IO.Path]::Combine($startMenu, 'Uninstall OwO Input Method.lnk'))
    $uninstallShortcut.TargetPath = 'powershell.exe'
    $uninstallShortcut.Arguments = '-NoLogo -NoProfile -ExecutionPolicy Bypass -File "' +
                                   $uninstallScript + '"'
    $uninstallShortcut.WorkingDirectory = $installRoot
    $uninstallShortcut.Save()
    if ($CreateSettingsDesktopShortcut) {
        $desktop = [Environment]::GetFolderPath([Environment+SpecialFolder]::DesktopDirectory)
        if ([string]::IsNullOrWhiteSpace($desktop)) {
            throw 'Windows desktop directory is unavailable.'
        }
        $desktopShortcut = $shell.CreateShortcut(
            [IO.Path]::Combine($desktop, 'OwO Input Method Settings.lnk'))
        $desktopShortcut.TargetPath = $settingsExecutable
        $desktopShortcut.WorkingDirectory = $settingsDirectory
        $desktopShortcut.IconLocation = $settingsExecutable + ',0'
        $desktopShortcut.Save()
    }
} catch {
    Write-Warning "OwO was installed, but one or more shortcuts could not be created: $($_.Exception.Message)"
}

Write-OwOInstallProgress 94 'start'
& $launcher
if ($LASTEXITCODE -ne 0) {
    throw "OwO native runtime launcher failed: $LASTEXITCODE"
}
& $profileCheck --activate-session

if ($null -ne $previousInstallRoot -and
    -not $previousInstallRoot.Equals($installRoot, [StringComparison]::OrdinalIgnoreCase) -and
    (Test-Path -LiteralPath $previousInstallRoot -PathType Container)) {
    $cleanupSource = Join-Path $installRoot 'scripts\Finish-Uninstall-OwO.ps1'
    $cleanupRoot = Join-Path ([IO.Path]::GetTempPath()) `
        ("OwO-Uninstall-" + [Guid]::NewGuid().ToString('N'))
    [void](New-Item -ItemType Directory -Path $cleanupRoot)
    $cleanupScript = Join-Path $cleanupRoot 'Finish-Uninstall-OwO.ps1'
    Copy-Item -LiteralPath $cleanupSource -Destination $cleanupScript -Force
    $cleanupArguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass' +
        ' -File "' + $cleanupScript + '"' +
        ' -InstallRoot "' + $previousInstallRoot + '"' +
        ' -ProgramRoot "' + ([IO.Path]::GetFullPath((Split-Path -Parent $previousInstallRoot))) + '"' +
        ' -ParentProcessId ' + $PID
    Start-Process -FilePath 'powershell.exe' -ArgumentList $cleanupArguments `
        -WindowStyle Hidden | Out-Null
}
$installState.state = 'completed'
[IO.File]::WriteAllText($installMarker, ($installState | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
Write-Output "OwO Input Method $version installed: $installRoot"
Write-OwOInstallProgress 99 'complete'
