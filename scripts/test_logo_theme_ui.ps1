param([string]$PublishDirectory = '')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $PublishDirectory) { $PublishDirectory = Join-Path $project 'build/settings-publish/win-x64' }
$fixture = Join-Path $project ('build/logo-theme-ui-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $fixture)
$preferences = Join-Path $fixture 'settings-ui.json'
[IO.File]::WriteAllText($preferences, '{"AutoSaveEnabled":false}')
function Find-Id($window, $id) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Find-Name($window, $name) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Start-Fixture([switch]$InstallerPreview) {
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = Join-Path $PublishDirectory 'OwO.Settings.exe'
    $start.WorkingDirectory = $PublishDirectory
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.EnvironmentVariables['OWO_CONFIG_PATH'] = Join-Path $fixture 'owo.conf'
    $start.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = Join-Path $fixture 'plugins'
    if ($InstallerPreview) { $start.Arguments = '--install "' + $fixture + '" --install-preview-stage start' }
    [Diagnostics.Process]::Start($start)
}
function Wait-Window($process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        Start-Sleep -Milliseconds 150
        $process.Refresh()
        if ($process.HasExited) { throw 'Logo fixture exited before startup.' }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
            $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
            $navigation = Find-Name $window '通用设置'
            if ($navigation -and $navigation.Current.IsEnabled) { return $window }
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Logo settings window did not become ready.'
}
function Choose-Mode($window, $name) {
    $selector = Find-Id $window 'LogoTheme'
    $selector.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    Start-Sleep -Milliseconds 100
    $choice = Find-Name $window $name
    if (-not $choice) { throw "Missing logo option: $name" }
    $choice.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 200
}
function Wait-Logo($window) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $logo = Find-Id $window 'OwOBrandLogo'
        if ($logo -and $logo.Current.ItemStatus -eq 'ready') { return $logo }
        if ($logo -and $logo.Current.ItemStatus -eq 'error') { throw 'WinUI failed to decode the SVG logo.' }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'SVG logo did not finish decoding.'
}
$process = Start-Fixture
try {
    $window = Wait-Window $process
    (Find-Name $window '通用设置').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 150
    foreach ($mode in @(@('浅色','light','OwO Logo（浅色）'), @('深色','dark','OwO Logo（深色）'), @('跟随系统','system',''))) {
        Choose-Mode $window $mode[0]
        $saved = Get-Content -LiteralPath $preferences -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($saved.LogoTheme -ne $mode[1] -or $saved.AutoSaveEnabled) { throw 'Logo choice did not persist independently.' }
        $logo = Wait-Logo $window
        if (-not $logo -or ($mode[2] -and $logo.Current.Name -ne $mode[2])) { throw 'Displayed SVG logo did not follow selection.' }
    }
    Choose-Mode $window '浅色'
    $autoSave = (Find-Id $window 'AutoSaveSettings').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
    $autoSave.Toggle()
    Start-Sleep -Milliseconds 200
    $saved = Get-Content -LiteralPath $preferences -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($saved.LogoTheme -ne 'light' -or -not $saved.AutoSaveEnabled) { throw 'Auto-save erased the logo preference.' }
    $process.CloseMainWindow() | Out-Null
    if (-not $process.WaitForExit(5000)) { throw 'Fixture failed to close.' }
    $process = Start-Fixture
    $window = Wait-Window $process
    $logo = Wait-Logo $window
    if (-not $logo -or $logo.Current.Name -ne 'OwO Logo（浅色）') { throw 'Restart lost the logo preference.' }
    $process.CloseMainWindow() | Out-Null
    if (-not $process.WaitForExit(5000)) { throw 'Fixture failed to close before installer preview.' }
    foreach ($mode in @('light', 'dark')) {
        [IO.File]::WriteAllText($preferences, ('{"AutoSaveEnabled":false,"LogoTheme":"' + $mode + '"}'))
        $process = Start-Fixture -InstallerPreview
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        $window = $null
        $logo = $null
        do {
            Start-Sleep -Milliseconds 150
            $process.Refresh()
            if ($process.HasExited) { throw 'Installer logo preview exited.' }
            if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
                $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
                $logo = Find-Id $window 'OwOBrandLogo'
            }
        } while (-not $logo -and [DateTime]::UtcNow -lt $deadline)
        $logo = Wait-Logo $window
        $expected = if ($mode -eq 'light') { 'OwO Logo（浅色）' } else { 'OwO Logo（深色）' }
        if ($logo.Current.Name -ne $expected) { throw 'Installer square logo ignored theme preference.' }
        $process.CloseMainWindow() | Out-Null
        if (-not $process.WaitForExit(5000)) { throw 'Installer preview did not close.' }
    }
    Write-Output 'PASS: live rounded/square SVG decoding, theme switching, persistence, auto-save independence, restart and installer previews.'
} finally {
    if (-not $process.HasExited) { $process.CloseMainWindow() | Out-Null }
    if (-not $process.WaitForExit(5000)) { $process.Kill() }
}
