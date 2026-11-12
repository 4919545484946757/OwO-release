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
function Wait-Id($window, [string]$id) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $item = Find-Id $window $id
        if ($null -ne $item -and $item.Current.IsEnabled) { return $item }
        Start-Sleep -Milliseconds 100
    } until ([DateTime]::UtcNow -ge $deadline)
    throw "Missing or disabled UI control: $id"
}
function Select-Scope($window, [string]$name) {
    $combo = Find-Id $window 'SettingsSchemeSelector'
    if ($null -eq $combo) { throw 'Configuration scheme selector is missing.' }
    $combo.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    Start-Sleep -Milliseconds 200
    $condition = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $name)
    $item = $combo.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
    if ($null -eq $item) { $item = $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition) }
    if ($null -eq $item) { throw "Configuration choice not found: $name" }
    $item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $expanded = $combo.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern)
    if ($expanded.Current.ExpandCollapseState -eq [Windows.Automation.ExpandCollapseState]::Expanded) { $expanded.Collapse() }
    Start-Sleep -Milliseconds 250
}
function Check-ScopeLabel($window, [string]$name) {
    $label = Wait-Id $window 'SettingsPageSchemeLabel'
    if ($label.Current.IsOffscreen -or -not $label.Current.Name.EndsWith($name, [StringComparison]::Ordinal)) {
        throw "Expected a visible configuration label ending with '$name'; got '$($label.Current.Name)'."
    }
}
function Check-NoScopeLabel($window) {
    $label = Find-Id $window 'SettingsPageSchemeLabel'
    if ($null -ne $label -and -not $label.Current.IsOffscreen) { throw 'Configuration label leaked onto an unrelated page.' }
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
$start.EnvironmentVariables['OWO_DICTIONARY_STORE_PATH'] = Join-Path $fixture 'dictionaries'
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
    $fullName = -join (0x5168,0x62fc | ForEach-Object { [char]$_ })
    $englishName = -join (0x82f1,0x6587,0x8f93,0x5165 | ForEach-Object { [char]$_ })
    Check-NoScopeLabel $window
    (Wait-Id $window 'UserLexiconNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $null = Wait-Id $window 'DictionarySearch'
    Check-ScopeLabel $window $fullName
    Select-Scope $window $englishName
    Check-ScopeLabel $window $englishName
    $tabs = Find-Id $window 'DictionarySearch'
    if ($null -ne $tabs -and -not $tabs.Current.IsOffscreen) { throw 'Host dictionary leaked into English scope.' }
    Wait-Config 'input_scheme=full_pinyin'
    foreach ($page in @('CandidateNavigationItem','IntelligenceNavigationItem','ShortcutNavigationItem')) {
        (Wait-Id $window $page).GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
        Check-ScopeLabel $window $englishName
    }
    (Wait-Id $window 'GeneralNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 200
    Check-NoScopeLabel $window
    (Wait-Id $window 'UserLexiconNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Select-Scope $window $fullName
    $null = Wait-Id $window 'DictionarySearch'
    Check-ScopeLabel $window $fullName
    (Wait-Id $window 'SchemeNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 250
    $up = Find-Id $window 'InputSchemeMoveUp:english'
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
    Write-Output 'PASS: WinUI scheme labels, scopes, dictionary isolation, reorder, toggles and native autosave (isolated config).'
} finally {
    $process.Refresh()
    if (-not $process.HasExited) {
        [void]$process.CloseMainWindow()
        if (-not $process.WaitForExit(3000)) { Write-Warning "Scoped test window remains open: PID $($process.Id)." }
    }
}
