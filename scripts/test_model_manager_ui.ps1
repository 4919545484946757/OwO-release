[CmdletBinding()]
param([string]$PublishDirectory = '', [string]$DotnetExe = '', [switch]$VerifyOwOInput)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
if ($VerifyOwOInput) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class OwODebugTypingProbe {
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder name, int capacity);
    private delegate bool Visit(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(Visit visit, IntPtr data);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] private static extern uint MapVirtualKey(uint key, uint mode);
    [DllImport("user32.dll")] private static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    public static uint ForegroundPid() {
        uint process; GetWindowThreadProcessId(GetForegroundWindow(), out process); return process;
    }
    public static void Tap(byte key) {
        byte scan = (byte)MapVirtualKey(key, 0);
        keybd_event(key, scan, 0, UIntPtr.Zero);
        keybd_event(key, scan, 2, UIntPtr.Zero);
    }
    public static bool CandidateVisible(uint pid) {
        bool visible = false;
        EnumWindows(delegate(IntPtr window, IntPtr data) {
            uint process; GetWindowThreadProcessId(window, out process);
            if (process != pid || !IsWindowVisible(window)) return true;
            var name = new StringBuilder(64);
            GetClassName(window, name, name.Capacity);
            visible = name.ToString() == "OwO.P1.CandidateWindow";
            return !visible;
        }, IntPtr.Zero);
        return visible;
    }
}
'@
}
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$PublishDirectory) { $PublishDirectory = Join-Path $root 'build/settings-publish/win-x64' }
$fixture = Join-Path $root ('build/model-manager-ui-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $fixture)
$sdk = Join-Path $root 'build/dependencies/dotnet-10.0.403'
if ($DotnetExe) { $sdk = Split-Path -Parent $DotnetExe }
else {
    $DotnetExe = Join-Path $sdk 'dotnet.exe'
    if (!(Test-Path -LiteralPath $DotnetExe)) { $DotnetExe = 'dotnet' }
}
$env:DOTNET_ROOT = $sdk
$env:PATH = "$sdk;$env:PATH"
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
& $DotnetExe run --project (Join-Path $root 'tests/settings_center/model_manager_tests.csproj') -c Release --no-restore -- --seed-ui $fixture
if ($LASTEXITCODE) { throw 'Cannot seed isolated synthetic model profile.' }
$env:OWO_CONFIG_PATH = Join-Path $fixture 'config/owo.conf'
$env:OWO_PLUGIN_STORE_PATH = Join-Path $fixture 'plugins'
$process = Start-Process (Join-Path $PublishDirectory 'OwO.Settings.exe') -WindowStyle Hidden -PassThru
$checks = 0
function Find-Id($window, [string]$id) {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Find-Name($window, [string]$name) {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $name)
    $window.FindAll([Windows.Automation.TreeScope]::Descendants, $condition) | Where-Object { !$_.Current.IsOffscreen } | Select-Object -First 1
}
function Check([bool]$value, [string]$label) { if (!$value) { throw $label }; $script:checks++ }
function Invoke($element) { if (!$element) { throw 'Missing button.' }; $element.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
function Assert-OwOComposition($editorWindow, [string]$id) {
    function Target-Is-Focused {
        if ([OwODebugTypingProbe]::ForegroundPid() -ne $process.Id) { return $false }
        $control = [Windows.Automation.AutomationElement]::FocusedElement
        return $null -ne $control -and $control.Current.ProcessId -eq $process.Id -and $control.Current.AutomationId -eq $id
    }
    [void][OwODebugTypingProbe]::SetForegroundWindow([IntPtr]$editorWindow.Current.NativeWindowHandle)
    $editor = Find-Id $editorWindow $id
    $value = $editor.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern)
    $editor.SetFocus()
    Start-Sleep -Milliseconds 250
    try {
        foreach ($key in @(0x4e,0x49)) {
            if (!(Target-Is-Focused) -or (0x10,0x11,0x12,0x5b,0x5c | Where-Object {
                [OwODebugTypingProbe]::GetAsyncKeyState($_) -lt 0 })) { throw 'Test editor lost focus or a modifier is held. No key was sent.' }
            [OwODebugTypingProbe]::Tap([byte]$key)
            Start-Sleep -Milliseconds 75
        }
        $deadline = [DateTime]::UtcNow.AddSeconds(2)
        while (![OwODebugTypingProbe]::CandidateVisible($process.Id) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 60 }
        Check ([OwODebugTypingProbe]::CandidateVisible($process.Id)) "OwO candidate window did not open for $id. An active full-pinyin profile is required."
        if (!(Target-Is-Focused)) { throw 'Test editor lost focus before commit.' }
        # Enter commits only the raw spelling; it does not record learning.
        [OwODebugTypingProbe]::Tap([byte]0x0d)
        Start-Sleep -Milliseconds 250
        Check ($value.Current.Value -eq 'ni') "OwO could not type into $id."
        $value.SetValue('')
    } finally {
        # Cancel the synthetic composition; never commit to the user's Core.
        if (Target-Is-Focused) { [OwODebugTypingProbe]::Tap([byte]0x1b) }
    }
    Start-Sleep -Milliseconds 160
}
try {
    $window = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        Start-Sleep -Milliseconds 150
        $process.Refresh()
        if ($process.HasExited) { throw 'Settings exited during startup.' }
        if ($process.MainWindowHandle -ne 0) { $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle) }
    } while (!$window -and [DateTime]::UtcNow -lt $deadline)
    Check ($null -ne $window) 'Settings window did not open.'
    $navigation = Find-Id $window 'ModelManagerNavigationItem'
    Check ($null -ne $navigation) 'Model manager navigation missing.'
    $navigation.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 500
    foreach ($id in @('TextAssistantEnabled','TextAssistantContext','ZeroCandidateEnabled','ZeroCandidateMode','ModelPreferredLanguage','ModelSecondaryLanguage','ModelPolishLevel','ModelCodeLevel','AddModelApi')) {
        $element = Find-Id $window $id
        Check ($null -ne $element) "Missing model control: $id"
    }
    Check ($null -ne (Find-Id $window 'DownloadLocalModel-qwen3-4b-q4km')) 'Missing Qwen3-4B download.'
    Check ($null -ne (Find-Id $window 'LocalModelStoragePicker')) 'Missing local model storage picker.'
    Check ($null -eq (Find-Id $window 'DownloadLocalModel-qwen35-08b-q4')) 'Legacy model must not be offered for new downloads.'
    Invoke (Find-Id $window 'DebugLocalModel-qwen3-4b-q4km')
    $debugWindow = $null
    $debugDeadline = [DateTime]::UtcNow.AddSeconds(4)
    $processWindows = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ProcessIdProperty, $process.Id)
    do {
        Start-Sleep -Milliseconds 100
        $debugWindow = [Windows.Automation.AutomationElement]::RootElement.FindAll(
            [Windows.Automation.TreeScope]::Children, $processWindows) |
                Where-Object { $null -ne (Find-Id $_ 'LocalModelDebugInput') } | Select-Object -First 1
    } while (!$debugWindow -and [DateTime]::UtcNow -lt $debugDeadline)
    Check ($null -ne $debugWindow) 'Local model debug window did not open.'
    foreach ($id in @('LocalModelDebugInput','LocalModelDebugSystemPrompt','LocalModelDebugSend','LocalModelDebugCandidateMode')) {
        Check ($null -ne (Find-Id $debugWindow $id)) "Missing dialog control: $id"
    }
    Check (!(Find-Id $debugWindow 'LocalModelDebugSend').Current.IsEnabled) 'No model must not start inference.'
    $closeDebug = Find-Id $debugWindow 'LocalModelDebugClose'
    Check ($null -ne $closeDebug) 'Local model debug window has no close button.'
    if ($VerifyOwOInput) {
        Assert-OwOComposition $debugWindow 'LocalModelDebugSystemPrompt'
        Assert-OwOComposition $debugWindow 'LocalModelDebugInput'
    }
    Invoke $closeDebug
    Start-Sleep -Milliseconds 300
    $enabled = Find-Id $window 'TextAssistantEnabled'
    Check ($enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -eq [Windows.Automation.ToggleState]::Off) 'Must default disabled.'
    $zero = Find-Id $window 'ZeroCandidateEnabled'
    $zeroMode = Find-Id $window 'ZeroCandidateMode'
    Check (!$zeroMode.Current.IsEnabled) 'Modes remain disabled while zero candidate is off.'
    $zeroToggle = $zero.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
    Check ($zeroToggle.Current.ToggleState -eq [Windows.Automation.ToggleState]::Off) 'Zero candidate must default disabled.'
    $zeroToggle.Toggle()
    Start-Sleep -Milliseconds 250
    Check ($null -ne (Find-Name $window '启用')) 'Zero candidate requires separate explicit opt-in.'
    Invoke (Find-Name $window '取消')
    Start-Sleep -Milliseconds 250
    Check (!(Test-Path -LiteralPath (Join-Path $fixture 'models/zero-candidate.mode'))) 'Cancelling must not allow automatic requests.'
    $zeroToggle.Toggle()
    Start-Sleep -Milliseconds 250
    Invoke (Find-Name $window '启用')
    Start-Sleep -Milliseconds 250
    Check ((Get-Content (Join-Path $fixture 'models/zero-candidate.mode') -Raw) -eq 'lattice-eco') 'Consent must enable local energy mode.'
    Check ($zeroMode.Current.IsEnabled) 'Core ranking modes do not depend on the linked API.'
    $zeroToggle.Toggle()
    Start-Sleep -Milliseconds 250
    Check (!(Test-Path -LiteralPath (Join-Path $fixture 'models/zero-candidate.mode'))) 'Disable must revoke zero candidate authorization.'
    Invoke (Find-Name $window '编辑')
    Start-Sleep -Milliseconds 250
    foreach ($id in @('ModelApiName','ModelApiEndpoint','ModelApiModel','ModelApiKey')) { Check ($null -ne (Find-Id $window $id)) "Profile editor field missing: $id" }
    $name = Find-Id $window 'ModelApiName'
    $name.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('Fixture edited')
    Invoke (Find-Name $window '保存')
    Start-Sleep -Milliseconds 300
    $config = Get-Content (Join-Path $fixture 'models/profiles.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    Check ($config.Profiles[0].Name -eq 'Fixture edited') 'Editing profile did not persist.'
    Check (!$config.Enabled -and !$config.PrivacyConsent) 'Editing must not implicitly enable network.'
    Check (!(Get-Content (Join-Path $fixture 'models/profiles.json') -Raw).Contains('unit-test-not-a-real-api-key')) 'Secret leaked to ordinary JSON.'
    $enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 250
    Check ($null -ne (Find-Name $window '我同意并启用')) 'Explicit privacy consent missing.'
    Invoke (Find-Name $window '取消')
    Start-Sleep -Milliseconds 250
    Check (!(Test-Path -LiteralPath (Join-Path $fixture 'models/selection-assistant.enabled'))) 'Cancel must not enable native trigger.'
    $enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 250
    Invoke (Find-Name $window '我同意并启用')
    Start-Sleep -Milliseconds 250
    Check (Test-Path -LiteralPath (Join-Path $fixture 'models/selection-assistant.enabled')) 'Consent did not enable isolated trigger.'
    $enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 250
    Check (!(Test-Path -LiteralPath (Join-Path $fixture 'models/selection-assistant.enabled'))) 'Disable did not clear trigger.'
    Write-Output "PASS: $checks model-manager UI checks; synthetic profile only; no model API requests. Fixture: $fixture"
} finally {
    if (!$process.HasExited -and $window) { $window.GetCurrentPattern([Windows.Automation.WindowPattern]::Pattern).Close(); [void]$process.WaitForExit(5000) }
}
