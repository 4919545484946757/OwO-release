[CmdletBinding()]
param()

# Requires a desktop with the current OwO TSF registered, active and at least
# two enabled schemes. This touches no document, config, plugin grant or clipboard.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
public static class OwOSchemePickerProbe {
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    private static extern int GetClassName(IntPtr h, StringBuilder name, int length);
    private delegate bool EnumWindow(IntPtr h, IntPtr arg);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindow fn, IntPtr arg);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vk);
    [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint map);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    public static bool PickerVisible() {
        bool visible = false;
        var pid = (uint)Process.GetCurrentProcess().Id;
        EnumWindows(delegate(IntPtr h, IntPtr arg) {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner != pid || !IsWindowVisible(h)) return true;
            var name = new StringBuilder(64);
            GetClassName(h, name, name.Capacity);
            visible = name.ToString() == "OwO.SchemePickerWindow";
            return !visible;
        }, IntPtr.Zero);
        return visible;
    }
}
'@

function Pump([int] $Milliseconds) {
    $end = [Diagnostics.Stopwatch]::StartNew()
    do {
        [Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 10
    } while ($end.ElapsedMilliseconds -lt $Milliseconds)
}
function Key([byte] $Vk, [bool] $Up = $false) {
    if (!$Up -and [OwOSchemePickerProbe]::GetForegroundWindow() -ne $form.Handle) {
        throw 'Probe no longer has focus; no key was sent.'
    }
    $scan = [byte]([OwOSchemePickerProbe]::MapVirtualKey($Vk, 0) -band 0xff)
    [OwOSchemePickerProbe]::keybd_event($Vk, $scan, [uint32] $(if ($Up) { 2 } else { 0 }), [UIntPtr]::Zero)
    Pump 40
}
function Visible {
    return [OwOSchemePickerProbe]::PickerVisible()
}
function Check-Visible([bool] $Expected, [string] $Step, [bool] $ShellMayTakeFocus = $false) {
    $wait = [Diagnostics.Stopwatch]::StartNew()
    while ((Visible) -ne $Expected -and $wait.ElapsedMilliseconds -lt 2000) { Pump 20 }
    if ((Visible) -ne $Expected) { throw "Unexpected picker visibility at ${Step}. Check the active OwO profile and enabled schemes." }
    if ([OwOSchemePickerProbe]::GetForegroundWindow() -ne $form.Handle) {
        if (!$ShellMayTakeFocus) { throw "Probe lost focus at $Step." }
        Write-Output 'Windows consumed Ctrl+Escape; focus loss cancelled the picker.'
    }
}

$form = New-Object Windows.Forms.Form
$form.Text = 'OwO scheme picker test'
$form.StartPosition = 'CenterScreen'
$text = New-Object Windows.Forms.TextBox
$text.Text = 'OwO probe'
$text.Dock = 'Top'
$form.Controls.Add($text)
$ctrlDown = $false
$spaceDown = $false
try {
    $form.Show()
    $form.Activate()
    [void]$text.Focus()
    $text.SelectionStart = $text.TextLength
    Pump 600
    $ctrlDown = $true; Key 0x11
    if ([OwOSchemePickerProbe]::GetAsyncKeyState(0x11) -ge 0) { throw 'The injected Control key did not reach the desktop.' }
    $spaceDown = $true; Key 0x20
    Check-Visible $true 'first Space'
    Key 0x20 $true; $spaceDown = $false
    $spaceDown = $true; Key 0x20
    Key 0x20 $true; $spaceDown = $false
    Check-Visible $true 'second Space with Ctrl held'
    Key 0x11 $true; $ctrlDown = $false
    Check-Visible $false 'Ctrl release'
    if ($text.Text -ne 'OwO probe') { throw 'Shortcut leaked into test text.' }

    $ctrlDown = $true; Key 0x11
    $spaceDown = $true; Key 0x20
    Key 0x20 $true; $spaceDown = $false
    Check-Visible $true 'reopen'
    Key 0x41; Key 0x41 $true
    Check-Visible $false 'Ctrl+A'
    if ($text.SelectionLength -ne $text.TextLength) { throw 'Picker swallowed Ctrl+A.' }
    $spaceDown = $true; Key 0x20
    Key 0x20 $true; $spaceDown = $false
    Check-Visible $true 'reopen for Escape'
    Key 0x1b; Key 0x1b $true
    Check-Visible $false 'Escape' $true
    Key 0x11 $true; $ctrlDown = $false
    Write-Output 'PASS: picker opens, retains focus, accepts another tap, closes on release/Escape, and does not swallow Ctrl+A.'
} finally {
    if ($spaceDown) { Key 0x20 $true }
    if ($ctrlDown) { Key 0x11 $true }
    $form.Dispose()
}
