[CmdletBinding()]
param([string]$PublishDirectory = '')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $PublishDirectory) { $PublishDirectory = Join-Path $projectRoot 'build/settings-publish/win-x64' }
$publish = (Resolve-Path -LiteralPath $PublishDirectory).Path
$testRoot = Join-Path $projectRoot ('build/candidate-diagnostics-ui-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $testRoot)
function Find-Id($window, [string]$id) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Find-Name($window, [string]$name) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
$startInfo = New-Object Diagnostics.ProcessStartInfo
$startInfo.FileName = Join-Path $publish 'OwO.Settings.exe'
$startInfo.WorkingDirectory = $publish
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.EnvironmentVariables['OWO_CONFIG_PATH'] = Join-Path $testRoot 'owo.conf'
$startInfo.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = Join-Path $testRoot 'plugins'
$process = [Diagnostics.Process]::Start($startInfo)
$checks = 0
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $window = $null
    $navigation = $null
    do {
        Start-Sleep -Milliseconds 200
        $process.Refresh()
        if ($process.HasExited) { throw "Settings exited: $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
            $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
            $navigation = Find-Name $window '通用设置'
        }
    } until ($null -ne $navigation -or [DateTime]::UtcNow -ge $deadline)
    if ($null -eq $navigation) { throw 'General settings navigation did not load.' }
    if ($null -ne (Find-Name $window '调试与性能')) { throw 'Diagnostics must not appear in the sidebar.' }
    $navigation.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 500
    $entry = Find-Id $window 'OpenCandidateDiagnostics'
    if ($null -eq $entry) { throw 'General settings diagnostics entry did not load.' }
    $entry.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 500
    $checks += 2
    foreach ($id in @('CandidateTimingStart', 'CandidateTimingStop', 'CandidateTimingExport', 'CandidateTimingRefresh',
        'CandidateTimingState', 'CandidateTimingSummary', 'CandidateTimingRequests', 'CandidateTimingDetails')) {
        $element = Find-Id $window $id
        if ($null -eq $element) { throw "Missing diagnostics control: $id" }
        $checks++
    }
    if (-not (Find-Id $window 'CandidateTimingStart').Current.IsEnabled) { throw 'Start is disabled by default.' }
    if ((Find-Id $window 'CandidateTimingStop').Current.IsEnabled) { throw 'Capture enabled without explicit start.' }
    if ((Find-Id $window 'CandidateTimingExport').Current.IsEnabled) { throw 'Export enabled without data.' }
    $checks += 3
    # Never click Start: do not capture the user's running TSF/Core. Store and
    # native capture behavior have separate isolated automated contracts.
    (Find-Id $window 'CandidateTimingRefresh').GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 300
    (Find-Id $window 'CandidateDiagnosticsBack').GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 200
    $hiddenStart = Find-Id $window 'CandidateTimingStart'
    if ($null -ne $hiddenStart -and -not $hiddenStart.Current.IsOffscreen) { throw 'Diagnostics overlaps other pages.' }
    if ($null -eq (Find-Id $window 'OpenCandidateDiagnostics')) { throw 'Back did not restore general settings.' }
    $checks++
    Write-Output "Candidate diagnostics real WinUI smoke: $checks checks passed; capture remained disabled."
} finally {
    $process.Refresh()
    if (-not $process.HasExited) {
        [void]$process.CloseMainWindow()
        if (-not $process.WaitForExit(3000)) { Write-Warning "Scoped test window remains open: PID $($process.Id). Close it manually." }
    }
}
