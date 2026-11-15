[CmdletBinding()]
param(
    [switch]$RemoveUserData,
    [switch]$Quiet,
    [switch]$Worker,
    [string]$InstallRoot,
    [string]$ExpectedUserSid,
    [string]$ProgressPath
)

$ErrorActionPreference = 'Stop'
if (-not $Worker) {
    # Preserve existing command-line semantics: an explicit deletion switch is
    # consent for the noninteractive path. The normal entry opens the wizard.
    & (Join-Path $PSScriptRoot 'Open-Uninstall-OwO.ps1') -Quiet:($Quiet -or $RemoveUserData) -RemoveUserData:$RemoveUserData
    exit $LASTEXITCODE
}

. (Join-Path $PSScriptRoot 'Uninstall-Common.ps1')
. (Join-Path $PSScriptRoot 'Install-Prerequisites.ps1')
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
# An alternate administrator must not inspect/delete their own HKCU or files
# in place of the account that initiated the uninstall.
if (-not $ExpectedUserSid -or $identity.User.Value -ne $ExpectedUserSid) {
    throw 'Uninstall must be elevated by the same Windows account. No data was changed.'
}
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'OwO TSF unregistration requires administrator approval.'
}
$statusPath = [IO.Path]::GetFullPath($ProgressPath)
$workspace = Assert-OwOUninstallWorkspace ([IO.Path]::GetDirectoryName($statusPath))
if (-not [IO.Path]::GetDirectoryName($statusPath).Equals($workspace, [StringComparison]::OrdinalIgnoreCase) -or
    [IO.Path]::GetFileName($statusPath) -notmatch '^progress-[a-f0-9]{32}\.jsonl$') {
    throw 'Invalid uninstall progress destination.'
}
Assert-OwOPlainPath $statusPath
Set-Location -LiteralPath $workspace

function Write-OwOUninstallStatus([int]$Percent, [string]$Stage, [string]$Detail = '') {
    if ($Detail.Length -gt 3000) { $Detail = $Detail.Substring(0, 3000) }
    $line = @{ percent = $Percent; stage = $Stage; detail = $Detail } | ConvertTo-Json -Compress
    [IO.File]::AppendAllText($statusPath, $line + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
}

$jobLock = [Threading.Mutex]::new($false, "Local\OwO.Uninstall.$ExpectedUserSid")
$lockHeld = $false
try {
    try { $lockHeld = $jobLock.WaitOne(0) }
    catch [Threading.AbandonedMutexException] { $lockHeld = $true }
    if (-not $lockHeld) { throw 'An OwO uninstall is already running.' }
    Write-OwOUninstallStatus 6 'prepare'
    $target = Get-OwOUninstallTarget $InstallRoot
    $root = $target.Root
    $data = Get-OwOUninstallUserData
    if ($RemoveUserData) { Assert-OwOPlainTree $data }
    $nativeSystem = Get-OwONativeSystemDirectory

    Write-OwOUninstallStatus 18 'stop'
    $installedProcesses = @(Get-CimInstance Win32_Process | Where-Object {
        $_.ProcessId -ne $PID -and $_.ExecutablePath -and
        $_.ExecutablePath.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)
    })
    foreach ($process in $installedProcesses) {
        # Scope to the selected deployed version. A development Core or another
        # copy with the same filename must not be stopped.
        $live = Get-Process -Id $process.ProcessId -ErrorAction SilentlyContinue
        if ($live -and $live.Path -and $live.Path.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) {
            Stop-Process -InputObject $live -Force
        }
    }

    Write-OwOUninstallStatus 36 'unregister'
    if (-not $target.Pending) {
        $profileCheck = Join-Path $root 'bin\owo_tsf_profile_check.exe'
        if (Test-Path -LiteralPath $profileCheck -PathType Leaf) {
            Assert-OwOPlainPath $profileCheck
            & $profileCheck --disable | Out-Null
            if ($LASTEXITCODE -notin @(0, 4)) { throw "OwO profile disable failed: $LASTEXITCODE" }
        }
        foreach ($entry in @(
            @{ Dll = 'bin\OwO.TSF.dll'; System = $nativeSystem },
            @{ Dll = 'bin\x86\OwO.TSF.dll'; System = (Join-Path $env:SystemRoot 'SysWOW64') }
        )) {
            $dll = Join-Path $root $entry.Dll
            if (-not (Test-Path -LiteralPath $dll -PathType Leaf)) { continue }
            Assert-OwOPlainPath $dll
            $unregister = Start-Process -FilePath (Join-Path $entry.System 'regsvr32.exe') `
                -ArgumentList @('/u', '/s', ('"{0}"' -f $dll)) -WindowStyle Hidden -Wait -PassThru
            if ($unregister.ExitCode -ne 0) { throw "TSF unregistration failed ($($entry.Dll)): $($unregister.ExitCode)" }
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
    if ($changed) { Set-WinUserLanguageList $languages -Force -WarningAction SilentlyContinue }
    if (-not $target.Pending) {
        $resume = [Guid]::NewGuid().ToString('N')
        $resumeFile = Join-Path $root '.owo-removal-pending'
        Assert-OwOPlainPath $resumeFile
        [IO.File]::WriteAllText($resumeFile, $resume)
        if (-not (Test-Path -LiteralPath $script:OwOUninstallKey)) {
            [void](New-Item -Path $script:OwOUninstallKey -Force)
        }
        Set-ItemProperty -LiteralPath $script:OwOUninstallKey -Name InstallLocation -Value $root
        Set-ItemProperty -LiteralPath $script:OwOUninstallKey -Name OwORemovalPending -Value $resume
    }

    Write-OwOUninstallStatus 54 'shortcuts'
    Remove-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
        -Name 'OwO Input Method' -ErrorAction SilentlyContinue
    $startMenu = Join-Path ([Environment]::GetFolderPath('Programs')) 'OwO Input Method'
    try {
        Assert-OwOPlainPath $startMenu
        foreach ($shortcut in @('OwO Input Method Settings.lnk', 'Uninstall OwO Input Method.lnk')) {
            $link = Join-Path $startMenu $shortcut
            if (Test-Path -LiteralPath $link -PathType Leaf) {
                Assert-OwOPlainPath $link
                Remove-Item -LiteralPath $link -Force
            }
        }
        if ((Test-Path -LiteralPath $startMenu -PathType Container) -and
            @(Get-ChildItem -LiteralPath $startMenu -Force).Count -eq 0) {
            [IO.Directory]::Delete($startMenu, $false)
        }
    } catch {
        Write-OwOUninstallStatus 55 'shortcuts' $_.Exception.Message
    }
    $desktop = [Environment]::GetFolderPath('DesktopDirectory')
    if ($desktop) {
        $link = Join-Path $desktop 'OwO Input Method Settings.lnk'
        if (Test-Path -LiteralPath $link -PathType Leaf) {
            try {
                Assert-OwOPlainPath $link
                Remove-Item -LiteralPath $link -Force
            } catch {
                Write-OwOUninstallStatus 56 'shortcuts' $_.Exception.Message
            }
        }
    }

    if ($RemoveUserData) {
        Write-OwOUninstallStatus 67 'remove-data'
        Remove-OwOOwnedTree $data $data
        Write-OwOUninstallStatus 73 'data-removed'
    } else {
        Write-OwOUninstallStatus 67 'keep-data'
    }

    Write-OwOUninstallStatus 79 'program-files'
    $emptyDirectoryRetained = $false
    if (Test-Path -LiteralPath $root -PathType Container) {
        # Keep entry points intact while in-use binaries are being retried.
        $ordered = @(Get-ChildItem -LiteralPath $root -Force | Sort-Object {
            if ($_.Name -eq '.owo-removal-pending') { 3 }
            elseif ($_.Name -eq 'scripts') { 2 }
            elseif ($_.Name -eq 'settings') { 1 }
            else { 0 }
        })
        $removedEntries = 0
        foreach ($child in $ordered) {
            for ($attempt = 0; $attempt -lt 10; ++$attempt) {
                try { Remove-OwOOwnedTree $child.FullName $root; break }
                catch [IO.IOException] {
                    if ($attempt -eq 9) { throw }
                    Start-Sleep -Milliseconds 200
                } catch [System.Management.Automation.ProviderInvocationException] {
                    if ($attempt -eq 9) { throw }
                    Start-Sleep -Milliseconds 200
                }
            }
            ++$removedEntries
            $percent = 79 + [int][Math]::Floor(17 * $removedEntries / $ordered.Count)
            Write-OwOUninstallStatus $percent 'program-files' $child.Name
        }
        try { [IO.Directory]::Delete($root, $false) }
        catch {
            if (@(Get-ChildItem -LiteralPath $root -Force).Count -ne 0) { throw }
            $emptyDirectoryRetained = $true
        }
    }
    if (-not $emptyDirectoryRetained -and (Test-Path -LiteralPath $root)) {
        throw "Program directory still exists: $root"
    }
    if (Test-Path -LiteralPath $script:OwOUninstallKey) {
        Remove-Item -LiteralPath $script:OwOUninstallKey -Force
    }
    $detail = if ($emptyDirectoryRetained) { 'empty-directory-retained' } else { '' }
    Write-OwOUninstallStatus 100 'complete' $detail
    exit 0
} catch {
    Write-OwOUninstallStatus 0 'failed' $_.Exception.Message
    Write-Error -Message $_.Exception.Message -ErrorAction Continue
    exit 1
} finally {
    if ($lockHeld) { $jobLock.ReleaseMutex() }
    $jobLock.Dispose()
}
