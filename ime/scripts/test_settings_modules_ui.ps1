[CmdletBinding()]
param([string]$PublishDirectory = '', [switch]$FreshAutoSaveDefaults)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $PublishDirectory) { $PublishDirectory = Join-Path $projectRoot 'build\settings-publish\win-x64' }
$publish = (Resolve-Path -LiteralPath $PublishDirectory).Path
$shell = Join-Path $publish 'owo_config_shell.exe'
$application = Join-Path $publish 'OwO.Settings.exe'
if (-not (Test-Path -LiteralPath $shell -PathType Leaf) -or
    -not (Test-Path -LiteralPath $application -PathType Leaf)) { throw 'Build/publish settings first.' }
$testRoot = Join-Path $projectRoot ('build\settings-modules-ui-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $testRoot)
$checks = 0

function Find-Id($window, [string]$id) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Find-Name($window, [string]$name) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Navigate($window, [string]$name) {
    $item = Find-Name $window $name
    if ($null -eq $item -or -not $item.Current.IsEnabled) { throw "Missing navigation: $name" }
    $item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 150
    $schemeNotice = Find-Id $window 'SchemeSettingsNotice'
    if ($null -ne $schemeNotice -and -not $schemeNotice.Current.IsOffscreen) { throw 'Passive scheme notice became visible again.' }
    foreach ($message in @(
        '此页管理宿主内置词库，不是插件方案的私有词库。插件词库配置请进入对应插件设置。',
        '词库已加载。编辑会直接保存；导出文件包含个人用词，请妥善保管。',
        '连续调整会合并为一次保存。无效配置不会覆盖已保存内容；安装、启用插件及权限授权仍需明确操作。',
        '支持自动分割、不完整拼音、简拼、混拼与可切换纠错。')) {
        $notice = Find-Name $window $message
        if ($null -ne $notice -and -not $notice.Current.IsOffscreen) { throw 'Hidden informational message became visible again.' }
    }
    $script:checks++
}
function Check-Enabled($window, [string]$id, [bool]$enabled) {
    $element = Find-Id $window $id
    if ($null -eq $element -or $element.Current.IsEnabled -ne $enabled) { throw "Unexpected enabled state: $id (expected $enabled)" }
    $script:checks++
}
function Read-Config([string]$path) {
    $values = @{}
    $lines = & $shell $path show
    if ($LASTEXITCODE -ne 0) { throw 'Isolated config read failed.' }
    foreach ($line in $lines) {
        $pair = $line -split '=', 2
        if ($pair.Length -eq 2) { $values[$pair[0]] = $pair[1] }
    }
    $values
}
function Save-Field([string]$path, [string]$key, [string]$value) {
    & $shell $path set $key $value | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Isolated config seed failed: $key" }
}
function Invoke-Id($window, [string]$id) {
    $element = Find-Id $window $id
    if ($null -eq $element) { throw "Missing control: $id" }
    $element.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
}
function Wait-Enabled($window, [string]$id) {
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    do {
        Start-Sleep -Milliseconds 100
        $element = Find-Id $window $id
        if ($null -ne $element -and $element.Current.IsEnabled) { return }
    } until ([DateTime]::UtcNow -ge $deadline)
    throw "Control did not become ready: $id"
}
function Select-Scheme($window, $process, [string]$name) {
    Navigate $window '输入方案'
    $combo = Find-Id $window 'InputScheme'
    $expand = $combo.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern)
    $expand.Expand()
    Start-Sleep -Milliseconds 150
    $nameCondition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)
    $itemCondition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::ListItem)
    $choiceCondition = New-Object Windows.Automation.AndCondition($nameCondition, $itemCondition)
    $choice = $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $choiceCondition)
    if ($null -eq $choice) {
        $processCondition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ProcessIdProperty, $process.Id)
        $owned = [Windows.Automation.AutomationElement]::RootElement.FindAll([Windows.Automation.TreeScope]::Children, $processCondition)
        foreach ($popup in $owned) {
            $choice = $popup.FindFirst([Windows.Automation.TreeScope]::Descendants, $choiceCondition)
            if ($null -ne $choice) { break }
        }
    }
    if ($null -eq $choice) { throw "Scheme choice unavailable: $name" }
    $choice.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $expand.Collapse()
    Start-Sleep -Milliseconds 300
}

foreach ($scheme in @('full_pinyin', 'exact_code', 'plugin:example.input/cangjie')) {
    $fixture = Join-Path $testRoot ($scheme -replace '[^a-zA-Z0-9_]', '_')
    [void](New-Item -ItemType Directory -Path $fixture)
    $config = Join-Path $fixture 'owo.conf'
    Save-Field $config 'input_scheme' $scheme
    foreach ($key in @('user_learning_enabled', 'user_lexicon_completion_enabled', 'model_ranking_enabled',
        'candidate_partition_enabled', 'alt_code_preview_enabled', 'text_navigation_shortcut_enabled')) {
        Save-Field $config $key 'true'
    }
    Save-Field $config 'candidate_page_size' '9'
    Save-Field $config 'chinese_punctuation_mask' '3'
    Save-Field $config 'user_learning_sensitivity' '2'
    $preferences = Join-Path $fixture 'settings-ui.json'
    if (-not $FreshAutoSaveDefaults -or $scheme -ne 'full_pinyin') {
        [IO.File]::WriteAllText($preferences, '{"AutoSaveEnabled":false}')
    }
    $startInfo = New-Object Diagnostics.ProcessStartInfo
    $startInfo.FileName = $application
    $startInfo.WorkingDirectory = $publish
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.EnvironmentVariables['OWO_CONFIG_PATH'] = $config
    $startInfo.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = Join-Path $fixture 'plugins'
    $process = [Diagnostics.Process]::Start($startInfo)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        $window = $null
        do {
            Start-Sleep -Milliseconds 200
            $process.Refresh()
            if ($process.HasExited) { throw "Settings exited: $($process.ExitCode)" }
            if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
                $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
                $nav = Find-Name $window '候选与输入'
            } else { $nav = $null }
        } until (($null -ne $nav -and $nav.Current.IsEnabled) -or [DateTime]::UtcNow -ge $deadline)
        if ($null -eq $window -or $null -eq $nav -or -not $nav.Current.IsEnabled) { throw 'Settings did not become ready.' }
        $builtIn = $scheme -ne 'plugin:example.input/cangjie'
        $phonetic = $scheme -eq 'full_pinyin'
        Navigate $window '候选与输入'
        foreach ($id in @('CandidatePageSize', 'CandidateWrapLength', 'AltCodePreview', 'AltCodePreviewPageSize')) {
            Check-Enabled $window $id $true
        }
        foreach ($id in @('CandidatePartition', 'CandidateImmediateCommit', 'FocusLossCommit', 'ChinesePunctuation')) {
            Check-Enabled $window $id $builtIn
        }
        Check-Enabled $window 'CandidateSingleCharacterFirst' $phonetic
        Navigate $window '学习与排序'
        foreach ($id in @('UserLearning', 'UserLearningSensitivity', 'ModelRanking', 'ModelTimeout')) {
            Check-Enabled $window $id $builtIn
        }
        foreach ($id in @('UserCompletion', 'UserCompletionRange')) { Check-Enabled $window $id $phonetic }
        Navigate $window '快捷键'
        Check-Enabled $window 'CorrectionShortcutEnabled' $phonetic
        foreach ($id in @('LanguageShortcutEnabled', 'RawInputShortcutEnabled', 'CursorLeftShortcutEnabled',
            'CursorRightShortcutEnabled', 'PreviousPageShortcutEnabled', 'NextPageShortcutEnabled',
            'ExpandedPreviousPageShortcutEnabled', 'ExpandedNextPageShortcutEnabled')) {
            Check-Enabled $window $id $true
        }
        foreach ($id in @('PunctuationShortcutEnabled', 'TextNavigationShortcutEnabled', 'TextNavigationSelectEnabled')) {
            Check-Enabled $window $id $builtIn
        }
        $correctionButton = Find-Name $window 'Ctrl+Q'
        if ($null -eq $correctionButton -or $correctionButton.Current.IsEnabled -ne $phonetic) {
            throw 'Dynamically built correction key button bypassed capability gate.'
        }
        $script:checks++
        $beforeSave = Read-Config $config
        Invoke-Id $window 'SaveButton'
        Wait-Enabled $window 'SaveButton'
        $afterSave = Read-Config $config
        foreach ($key in $beforeSave.Keys) {
            if ($key -ne 'generation' -and $beforeSave[$key] -ne $afterSave[$key]) {
                throw "Load/capture/save changed unedited field: $key"
            }
            $script:checks++
        }
        Navigate $window '候选与输入'
        Invoke-Id $window 'ResetPageButton'
        Wait-Enabled $window 'SaveButton'
        Invoke-Id $window 'SaveButton'
        Wait-Enabled $window 'SaveButton'
        $saved = Read-Config $config
        if ($saved['candidate_page_size'] -ne '7' -or $saved['chinese_punctuation_mask'] -ne '3' -or
            $saved['user_learning_sensitivity'] -ne '2' -or $saved['input_scheme'] -ne $scheme) {
            throw 'Reset/save changed unrelated module fields or failed to persist.'
        }
        $script:checks++
        if ($phonetic) {
            Navigate $window '通用设置'
            $autoSave = (Find-Id $window 'AutoSaveSettings').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern)
            if ($FreshAutoSaveDefaults) {
                if ($autoSave.Current.ToggleState -ne [Windows.Automation.ToggleState]::On -or
                    (Test-Path -LiteralPath $preferences)) { throw 'Fresh defaults must enable auto-save without writing a preference on startup.' }
                $script:checks++
            } else {
                if ($autoSave.Current.ToggleState -ne [Windows.Automation.ToggleState]::Off) { throw 'Startup replaced explicit disabled preference.' }
                $autoSave.Toggle()
                $script:checks++
            }
            Start-Sleep -Milliseconds 800
            Navigate $window '学习与排序'
            (Find-Id $window 'UserCompletion').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
            Start-Sleep -Milliseconds 800
            if ((Read-Config $config)['user_lexicon_completion_enabled'] -ne 'false') { throw 'Module registered editor did not auto-save.' }
            $script:checks++
            Select-Scheme $window $process '精确编码（实验性）'
            Navigate $window '学习与排序'
            Check-Enabled $window 'UserCompletion' $false
            if ((Find-Id $window 'UserCompletion').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -ne
                [Windows.Automation.ToggleState]::Off) { throw 'Switch to exact code reset dormant completion value.' }
            Select-Scheme $window $process '全拼'
            Navigate $window '学习与排序'
            Check-Enabled $window 'UserCompletion' $true
            if ((Find-Id $window 'UserCompletion').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -ne
                [Windows.Automation.ToggleState]::Off) { throw 'Switch back to pinyin lost completion preference.' }
            Start-Sleep -Milliseconds 700
            if ((Read-Config $config)['input_scheme'] -ne 'full_pinyin') { throw 'Scheme switch did not auto-save.' }
            $script:checks++
        }
        Write-Output "PASS: $scheme capability gating, page reset isolation and persistence."
    } finally {
        $process.Refresh()
        if (-not $process.HasExited) {
            [void]$process.CloseMainWindow()
            if (-not $process.WaitForExit(5000)) { $process.Kill() }
        }
    }
}
Write-Output "PASS: $checks isolated WinUI checks. Fixtures retained at $testRoot"
