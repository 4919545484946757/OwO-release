[CmdletBinding()]
param([string]$PublishDirectory = '', [string]$EnglishPackage = '', [switch]$RequirePackageIcon, [switch]$VerifyResponsiveLayout)
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
$pluginFixture = Join-Path $fixture 'plugins'
if ($EnglishPackage) {
    $EnglishPackage = (Resolve-Path -LiteralPath $EnglishPackage).Path
    $pluginBackend = Join-Path $publish 'owo_plugin_shell.exe'
    $preview = & $pluginBackend $pluginFixture inspect-install $EnglishPackage | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or !$preview.ok -or $preview.plugin_id -ne 'owo.plugin.english') { throw 'Invalid English test package.' }
    # Consent applies only to the unique, isolated automation store above.
    $installed = & $pluginBackend $pluginFixture install-risk $EnglishPackage $preview.inventory_sha256 1 I_ACCEPT_PLUGIN_RISK_V1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or !$installed.ok) { throw "English fixture failed to install: $($installed.diagnostic)" }
    $activated = & $pluginBackend $pluginFixture activate $preview.plugin_id $preview.version | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or !$activated.ok) { throw 'English fixture failed to activate.' }
    if ($RequirePackageIcon) {
        $scheme = (& $pluginBackend $pluginFixture input-schemes | ConvertFrom-Json).schemes |
            Where-Object id -eq 'plugin:owo.plugin.english/english'
        if ($LASTEXITCODE -ne 0 -or !$scheme -or @($scheme.icon).Count -ne 16 -or !$scheme.icon[8]) {
            throw 'English package icon was not discovered.'
        }
    }
}
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
function Assert-Columns($window, [string]$firstId, [string]$secondId, [bool]$wide) {
    $first = (Wait-Id $window $firstId).Current.BoundingRectangle
    $second = (Wait-Id $window $secondId).Current.BoundingRectangle
    if ($first.Height -le 0 -or $second.Height -le 0) { throw "Missing section geometry: $firstId, $secondId" }
    if ($wide -and ($second.Left -le $first.Left + 50 -or [Math]::Abs($first.Top - $second.Top) -gt 3)) {
        throw "Expected two columns for $firstId and $secondId; got $first / $second."
    }
    if (!$wide -and ([Math]::Abs($first.Left - $second.Left) -gt 3 -or $second.Top -le $first.Bottom)) {
        throw "Expected a stacked layout for $firstId and $secondId; got $first / $second."
    }
}
function Select-Nav($window, [string]$id, [string]$name) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $element = Find-Id $window $id
        if ($null -eq $element) {
            $label = [Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::NameProperty, $name)
            $element = $window.FindAll([Windows.Automation.TreeScope]::Descendants, $label) | Where-Object {
                $pattern = $null
                $_.TryGetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)
            } | Select-Object -First 1
        }
        if ($null -ne $element -and $element.Current.IsEnabled) {
            $element.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
            return
        }
        Start-Sleep -Milliseconds 100
    } until ([DateTime]::UtcNow -ge $deadline)
    throw "Missing navigation item: $id"
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
$start.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = $pluginFixture
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
    $englishName = -join (0x7eaf,0x51c0,0x952e,0x76d8 | ForEach-Object { [char]$_ })
    Check-NoScopeLabel $window
    if ($VerifyResponsiveLayout) {
        Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class OwOTestDpi { [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd); }'
        $scale = [OwOTestDpi]::GetDpiForWindow($process.MainWindowHandle) / 96.0
        $resize = $window.GetCurrentPattern([Windows.Automation.TransformPattern]::Pattern)
        $resize.Resize(1320 * $scale, 850 * $scale)
        Start-Sleep -Milliseconds 550
        $candidateName = -join (0x5019,0x9009,0x4e0e,0x8f93,0x5165 | ForEach-Object { [char]$_ })
        $learningName = -join (0x5b66,0x4e60,0x4e0e,0x6392,0x5e8f | ForEach-Object { [char]$_ })
        Select-Nav $window 'CandidateNavigationItem' $candidateName
        Start-Sleep -Milliseconds 250
        Assert-Columns $window 'CandidateOrderLayout' 'CandidateCommitLayout' $true
        Select-Nav $window 'IntelligenceNavigationItem' $learningName
        Start-Sleep -Milliseconds 200
        Assert-Columns $window 'LearningFrequencyLayout' 'LearningCompletionLayout' $true
        $resize.Resize(560 * $scale, 850 * $scale)
        Start-Sleep -Milliseconds 550
        Assert-Columns $window 'LearningFrequencyLayout' 'LearningCompletionLayout' $false
        $resize.Resize(1080 * $scale, 760 * $scale)
        Start-Sleep -Milliseconds 300
        Select-Nav $window 'CandidateNavigationItem' $candidateName
        $punctuation = Wait-Id $window 'ChinesePunctuationSettingsButton'
        $punctuation.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
        foreach ($group in @('PunctuationSentenceLayout','PunctuationQuotationLayout','PunctuationBracketLayout','PunctuationSymbolsLayout')) {
            $null = Wait-Id $window $group
        }
        Write-Output 'PASS: candidate and learning sections reflow between two columns and one; punctuation groups are reachable.'
    }
    (Wait-Id $window 'UserLexiconNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $null = Wait-Id $window 'DictionarySearch'
    Check-ScopeLabel $window $fullName
    if ($EnglishPackage) {
        $pluginName = -join (0x82f1,0x6587,0x8f93,0x5165 | ForEach-Object { [char]$_ })
        Select-Scope $window $pluginName
        Check-ScopeLabel $window $pluginName
        foreach ($control in @('SchemeExportLearned', 'SchemeClearLearned', 'SchemeDownload:libreoffice-en-us', 'SchemeDownload:libreoffice-en-gb')) {
            $null = Wait-Id $window $control
        }
        (Wait-Id $window 'CandidateNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
        $null = Wait-Id $window 'SchemeField:show_completions'
        $null = Wait-Id $window 'SchemeField:show_spelling_suggestions'
        if ($preview.version -eq '1.0.2') {
            $placement = Wait-Id $window 'SchemePresentationPreview'
            $parked = @($placement.GetCurrentPattern([Windows.Automation.SelectionPattern]::Pattern).Current.GetSelection())
            $inlineName = -join (0x8f93,0x5165,0x4f4d,0x7f6e | ForEach-Object { [char]$_ })
            if ($parked.Count -ne 1 -or $parked[0].Current.Name -ne $inlineName) { throw 'English inline preedit is not the default.' }
            $null = Wait-Id $window 'SchemePresentationPageSize'
            $null = Wait-Id $window 'SchemePresentationWrapLength'
        }
        (Wait-Id $window 'IntelligenceNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
        $null = Wait-Id $window 'SchemeField:learn_selected_words'
        (Wait-Id $window 'UserLexiconNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    }
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
    $up = Wait-Id $window 'InputSchemeMoveUp:english'
    $up.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Wait-Config 'input_scheme=english'
    $pluginSuffix = if ($EnglishPackage) { ';!plugin:owo.plugin.english/english' } else { '' }
    Wait-Config ('input_scheme_order=english;full_pinyin;!exact_code' + $pluginSuffix)
    $exact = Wait-Id $window 'InputSchemeToggle:exact_code'
    $exact.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Wait-Config ('input_scheme_order=english;full_pinyin;exact_code' + $pluginSuffix)
    $exact = Wait-Id $window 'InputSchemeToggle:exact_code'
    $exact.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Wait-Config ('input_scheme_order=english;full_pinyin;!exact_code' + $pluginSuffix)
    $pinyin = Wait-Id $window 'InputSchemeToggle:full_pinyin'
    $pinyin.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Wait-Config ('input_scheme_order=english;!full_pinyin;!exact_code' + $pluginSuffix)
    $english = Wait-Id $window 'InputSchemeToggle:english'
    $english.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 300
    if ($english.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -ne [Windows.Automation.ToggleState]::On) {
        throw 'The last enabled provider must remain enabled.'
    }
    if ($EnglishPackage) {
        $pluginToggle = Wait-Id $window 'InputSchemeToggle:plugin:owo.plugin.english/english'
        $pluginToggle.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
        Wait-Config 'input_scheme_order=english;!full_pinyin;!exact_code;plugin:owo.plugin.english/english'
    }
    (Wait-Id $window 'ShortcutNavigationItem').GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $picker = Wait-Id $window 'InputSchemePickerEnabled'
    $pattern = $picker.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
    if ($pattern.Current.ToggleState -ne [Windows.Automation.ToggleState]::On) { throw 'Scheme picker is not on by default.' }
    $pattern.Toggle()
    Wait-Config 'input_scheme_picker_enabled=false'
    (Wait-Id $window 'InputSchemePickerEnabled').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Wait-Config 'input_scheme_picker_enabled=true'
    Write-Output 'PASS: WinUI scheme labels, scopes, dictionary isolation, reorder, toggles and native autosave (isolated config).'
} finally {
    $process.Refresh()
    if (-not $process.HasExited) {
        [void]$process.CloseMainWindow()
        if (-not $process.WaitForExit(3000)) { Write-Warning "Scoped test window remains open: PID $($process.Id)." }
    }
}
