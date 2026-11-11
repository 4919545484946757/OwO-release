[CmdletBinding()]
param([string]$PublishDirectory = '')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $PublishDirectory) { $PublishDirectory = Join-Path $project 'build/settings-publish/win-x64' }
$publish = (Resolve-Path -LiteralPath $PublishDirectory).Path
$fixture = Join-Path $project ('build/input-scheme-ui-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $fixture)
$config = Join-Path $fixture 'owo.conf'
$backend = Join-Path $publish 'owo_config_shell.exe'
function Find-Id($window, [string]$id) {
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Wait-Config([string]$line) {
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    do {
        $saved = & $backend $config show
        if ($LASTEXITCODE -eq 0 -and $saved -contains $line) { return }
        Start-Sleep -Milliseconds 200
    } until ([DateTime]::UtcNow -ge $deadline)
    throw "Expected saved config: $line"
}
$start = [Diagnostics.ProcessStartInfo]::new((Join-Path $publish 'OwO.Settings.exe'))
$start.WorkingDirectory = $publish
$start.UseShellExecute = $false
$start.EnvironmentVariables['OWO_CONFIG_PATH'] = $config
$start.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = Join-Path $fixture 'plugins'
$process = [Diagnostics.Process]::Start($start)
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    $window = $null
    $up = $null
    do {
        Start-Sleep -Milliseconds 200
        $process.Refresh()
        if ($process.HasExited) { throw "Settings exited: $($process.ExitCode)" }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
            $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
            $up = Find-Id $window 'InputSchemeMoveUp:english'
        }
    } until (($null -ne $up -and $up.Current.IsEnabled) -or [DateTime]::UtcNow -ge $deadline)
    if ($null -eq $up -or -not $up.Current.IsEnabled) { throw 'English row did not load.' }
    $up.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Wait-Config 'input_scheme=english'
    Wait-Config 'input_scheme_order=english;full_pinyin;!exact_code'
    $exact = Find-Id $window 'InputSchemeToggle:exact_code'
    $exact.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Wait-Config 'input_scheme_order=english;full_pinyin;exact_code'
    $exact = Find-Id $window 'InputSchemeToggle:exact_code'
    $exact.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Wait-Config 'input_scheme_order=english;full_pinyin;!exact_code'
    $pinyin = Find-Id $window 'InputSchemeToggle:full_pinyin'
    $pinyin.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Wait-Config 'input_scheme_order=english;!full_pinyin;!exact_code'
    $english = Find-Id $window 'InputSchemeToggle:english'
    $english.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 300
    if ($english.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -ne [Windows.Automation.ToggleState]::On) {
        throw 'The last enabled provider must remain enabled.'
    }
    Write-Output 'PASS: WinUI scheme rows, reorder, toggles and native autosave (isolated config).'
} finally {
    $process.Refresh()
    if (-not $process.HasExited) {
        [void]$process.CloseMainWindow()
        if (-not $process.WaitForExit(3000)) { Write-Warning "Scoped test window remains open: PID $($process.Id)." }
    }
}
