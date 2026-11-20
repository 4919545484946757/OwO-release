[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Debug',
    [string]$DllPath,
    [ValidateSet('x64', 'x86')]
    [string]$Architecture = 'x64',
    [ValidateSet('CurrentUser', 'LocalMachine')]
    [string]$Scope = 'CurrentUser'
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$preset = if ($Architecture -eq 'x86') {
    if ($Configuration -eq 'Debug') { 'windows-x86-debug' } else { 'windows-x86-release' }
} else {
    if ($Configuration -eq 'Debug') { 'windows-debug' } else { 'windows-release' }
}
if ([string]::IsNullOrWhiteSpace($DllPath)) {
    $DllPath = Join-Path $projectRoot "build/$preset/$Configuration/OwO.TSF.dll"
}
$DllPath = [System.IO.Path]::GetFullPath($DllPath)

if (-not (Test-Path -LiteralPath $DllPath -PathType Leaf)) {
    throw "TSF DLL not found: $DllPath. Build the selected preset first."
}

# Reject a mismatched in-process server before touching COM/profile registration.
$reader = New-Object IO.BinaryReader([IO.File]::OpenRead($DllPath))
try {
    if ($reader.ReadUInt16() -ne 0x5A4D) { throw "Not a PE image: $DllPath" }
    $reader.BaseStream.Position = 0x3C
    $peOffset = $reader.ReadInt32()
    if ($peOffset -lt 0 -or $peOffset -gt $reader.BaseStream.Length - 6) {
        throw "Invalid PE header: $DllPath"
    }
    $reader.BaseStream.Position = $peOffset
    if ($reader.ReadUInt32() -ne 0x00004550) { throw "Invalid PE signature: $DllPath" }
    $machine = $reader.ReadUInt16()
    $expectedMachine = if ($Architecture -eq 'x86') { 0x014C } else { 0x8664 }
    if ($machine -ne $expectedMachine) {
        throw ("TSF architecture mismatch: requested {0}, PE machine 0x{1:X4}: {2}" -f $Architecture, $machine, $DllPath)
    }
} finally {
    $reader.Dispose()
}

# DllRegisterServer currently registers COM in HKCU even when elevated.
# Machine scope is explicit, so development registration never silently changes
# availability for other users. WhatIf validates the PE and reports this plan
# before invoking regsvr32 or touching any registry/list entry.
if (-not $PSCmdlet.ShouldProcess(
    "$DllPath ($Architecture, $Scope)",
    'Register OwO COM/profile and ensure current user input-list entry')) {
    return
}
if ($Scope -eq 'LocalMachine') {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = New-Object Security.Principal.WindowsPrincipal($identity)
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'LocalMachine registration requires an administrator-elevated process.'
        }
    } finally {
        $identity.Dispose()
    }
}

$regsvr32 = if ($Architecture -eq 'x86') {
    Join-Path $env:SystemRoot 'SysWOW64\regsvr32.exe'
} else {
    Join-Path $env:SystemRoot 'System32\regsvr32.exe'
}
$process = Start-Process -FilePath $regsvr32 `
    -ArgumentList @('/s', ('"{0}"' -f $DllPath)) -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) {
    throw "regsvr32 failed with exit code $($process.ExitCode)"
}
if ($Scope -eq 'LocalMachine') {
    $registryView = if ($Architecture -eq 'x86') {
        [Microsoft.Win32.RegistryView]::Registry32
    } else {
        [Microsoft.Win32.RegistryView]::Registry64
    }
    $machineRegistry = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::LocalMachine, $registryView)
    $classKey = $null
    $serverKey = $null
    try {
        $classKey = $machineRegistry.CreateSubKey(
            'SOFTWARE\Classes\CLSID\{6D31C9B1-8978-4F49-89B4-66EB1E741591}')
        $classKey.SetValue('', 'OwO Input Method Text Service', [Microsoft.Win32.RegistryValueKind]::String)
        $serverKey = $classKey.CreateSubKey('InprocServer32')
        $serverKey.SetValue('', $DllPath, [Microsoft.Win32.RegistryValueKind]::String)
        $serverKey.SetValue('ThreadingModel', 'Apartment', [Microsoft.Win32.RegistryValueKind]::String)
        if ($serverKey.GetValue('') -ne $DllPath -or $serverKey.GetValue('ThreadingModel') -ne 'Apartment') {
            throw 'Machine COM registration verification failed.'
        }
    } finally {
        if ($serverKey) { $serverKey.Dispose() }
        if ($classKey) { $classKey.Dispose() }
        $machineRegistry.Dispose()
    }
}
Write-Output "Registered development TSF ($Architecture, $Scope): $DllPath"

# COM/profile registration alone does not add an IME to Windows' selectable list.
# InstallLayoutOrTip appends this TIP without rewriting the user's complete list.
# Flags=0 preserves the default and does not remove any other keyboard/TIP.
if (-not ('OwODevelopmentInputListNative' -as [type])) {
    Add-Type -TypeDefinition @"
using System.Runtime.InteropServices;
public static class OwODevelopmentInputListNative {
    [DllImport("input.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool InstallLayoutOrTip(string profile, uint flags);
}
"@
}
$owoTip = '0804:{6D31C9B1-8978-4F49-89B4-66EB1E741591}{5D9F39C3-BDB4-453C-A7BA-B9EF82487629}'
$existingTips = @(foreach ($language in (Get-WinUserLanguageList)) {
    foreach ($tip in $language.InputMethodTips) { $tip }
})
if ($existingTips -notcontains $owoTip) {
    if (-not [OwODevelopmentInputListNative]::InstallLayoutOrTip($owoTip, 0)) {
        throw 'TSF registered, but enabling OwO in the current user input list failed.'
    }
}
$enabledTips = @(foreach ($language in (Get-WinUserLanguageList)) {
    foreach ($tip in $language.InputMethodTips) { $tip }
})
if ($enabledTips -notcontains $owoTip) {
    throw 'TSF registered, but OwO is still absent from the current user input list.'
}
foreach ($tip in $existingTips) {
    if ($enabledTips -notcontains $tip) {
        throw 'An existing user input method disappeared; inspect the input list before continuing.'
    }
}
Write-Output 'Verified OwO in the current user input list; existing input methods preserved.'
$check = Join-Path $projectRoot "build/$preset/$Configuration/owo_tsf_profile_check.exe"
if (Test-Path -LiteralPath $check -PathType Leaf) {
    & $check --refresh-cache
    if ($LASTEXITCODE -ne 0) {
        Write-Warning 'TSF registration cache refresh failed; signing out may be required. Existing applications may still use the previous DLL.'
    }
}
