param([string]$PublishDirectory = '')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $PublishDirectory) { $PublishDirectory = Join-Path $project 'build/settings-publish/win-x64' }
$fixture = Join-Path $project ('build/dictionary-manager-ui-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $fixture)
dotnet run --project (Join-Path $project 'tests/settings_center/dictionary_manager_tests.csproj') -c Release --no-restore -- --seed-ui $fixture
if ($LASTEXITCODE -ne 0) { throw 'Dictionary fixture seed failed.' }
[IO.File]::WriteAllText((Join-Path $fixture 'settings-ui.json'), '{"AutoSaveEnabled":false}')
function Find-Id($window, $id) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Wait-Id($window, $id) {
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do { $element = Find-Id $window $id; if ($element) { return $element }; Start-Sleep -Milliseconds 100 } while ([DateTime]::UtcNow -lt $deadline)
    throw "Missing dictionary UI control: $id"
}
function Invoke-Id($window, $id) {
    (Wait-Id $window $id).GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 150
}
function Confirm-Dialog($window, $text) {
    $name = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $text)
    $kind = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::ControlTypeProperty, [Windows.Automation.ControlType]::Button)
    $condition = New-Object Windows.Automation.AndCondition($name, $kind)
    $button = $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
    if (-not $button) { throw "Missing dictionary dialog button: $text" }
    $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 350
}
function Catalog {
    $store = Join-Path $fixture 'dictionaries'
    $snapshot = [IO.File]::ReadAllText((Join-Path $store 'current'))
    return [IO.File]::ReadAllText((Join-Path $store ('snapshots/' + $snapshot + '/catalog.json'))) | ConvertFrom-Json
}
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = Join-Path $PublishDirectory 'OwO.Settings.exe'
$start.WorkingDirectory = $PublishDirectory
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.EnvironmentVariables['OWO_CONFIG_PATH'] = Join-Path $fixture 'owo.conf'
$start.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = Join-Path $fixture 'plugins'
$start.EnvironmentVariables['OWO_DICTIONARY_STORE_PATH'] = Join-Path $fixture 'dictionaries'
$process = [Diagnostics.Process]::Start($start)
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    $window = $null; $navigation = $null
    do {
        Start-Sleep -Milliseconds 150; $process.Refresh()
        if ($process.HasExited) { throw 'Dictionary fixture settings exited.' }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
            $window = [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
            $navigation = Find-Id $window 'UserLexiconNavigationItem'
        }
    } while ((-not $navigation -or -not $navigation.Current.IsEnabled) -and [DateTime]::UtcNow -lt $deadline)
    if (-not $navigation -or $navigation.Current.Name -ne '词库管理器') { throw 'Dictionary manager navigation was not updated.' }
    $navigation.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $sampleId = 'DictionaryCard_' + ('1' * 32)
    $sample = Wait-Id $window $sampleId
    $null = Wait-Id $window 'DictionaryCard_user'
    $null = Wait-Id $window 'DictionaryCard_base'
    $search = Wait-Id $window 'DictionarySearch'
    $search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('测试字典')
    Start-Sleep -Milliseconds 150
    if (-not (Find-Id $window $sampleId) -or (Find-Id $window 'DictionaryCard_user')) { throw 'Dictionary search did not filter cards.' }
    $search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('')
    $sampleKey = '1' * 32
    $cardToggle = Wait-Id $window ('DictionaryCardEnabled_' + $sampleKey)
    $cardToggle.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 350
    if ((Catalog).dictionaries[0].enabled -or (Find-Id $window 'DictionaryBack')) { throw 'Card toggle failed or unexpectedly opened details.' }
    (Wait-Id $window ('DictionaryCardEnabled_' + $sampleKey)).GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 350
    Invoke-Id $window ('DictionaryCardMenu_' + $sampleKey)
    $null = Wait-Id $window ('DictionaryCardExport_' + $sampleKey)
    $null = Wait-Id $window ('DictionaryCardDelete_' + $sampleKey)
    Invoke-Id $window ('DictionaryCardRename_' + $sampleKey)
    (Wait-Id $window 'DictionaryRenameInput').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('改名测试字典')
    Confirm-Dialog $window '保存'
    if ((Catalog).dictionaries[0].name -ne '改名测试字典' -or (Find-Id $window 'DictionaryBack')) { throw 'Card rename failed or opened details.' }
    # Builtin sources expose the same menu/switch, without erasing installed or
    # learned data. Restoring a removed source must leave it disabled.
    Invoke-Id $window 'DictionaryCardMenu_base'
    Invoke-Id $window 'DictionaryCardRename_base'
    (Wait-Id $window 'DictionaryRenameInput').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('基础词典改名')
    Confirm-Dialog $window '保存'
    if (((Catalog).builtins | Where-Object id -eq 'base').name -ne '基础词典改名') { throw 'Builtin rename not persisted.' }
    (Wait-Id $window 'DictionaryCardEnabled_base').GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 350
    if (((Catalog).builtins | Where-Object id -eq 'base').enabled) { throw 'Builtin disable not persisted.' }
    Invoke-Id $window 'DictionaryCardMenu_user'
    $null = Wait-Id $window 'DictionaryCardExport_user'
    Invoke-Id $window 'DictionaryCardDelete_user'
    Confirm-Dialog $window '取消'
    if (((Catalog).builtins | Where-Object id -eq 'user').deleted) { throw 'Cancel removed builtin source.' }
    Invoke-Id $window 'DictionaryCardMenu_user'
    Invoke-Id $window 'DictionaryCardDelete_user'
    Confirm-Dialog $window '删除'
    if (Find-Id $window 'DictionaryCard_user') { throw 'Deleted builtin still visible.' }
    Invoke-Id $window 'DictionaryRestoreBuiltins'
    Invoke-Id $window 'DictionaryRestore_user'
    $restored = (Catalog).builtins | Where-Object id -eq 'user'
    if ($restored.deleted -or $restored.enabled) { throw 'Builtin restore did not keep disabled state.' }
    Invoke-Id $window $sampleId
    $enabled = Wait-Id $window 'DictionaryEnabled'
    $null = Wait-Id $window 'DictionaryWords'
    $enabled.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 350
    $store = Join-Path $fixture 'dictionaries'
    $snapshot = [IO.File]::ReadAllText((Join-Path $store 'current'))
    $active = [IO.File]::ReadAllText((Join-Path $store ('snapshots/' + $snapshot + '/active.tsv')))
    if ($active -ne "OWO_DICTIONARIES_V1`n") { throw 'Disabling the dictionary did not atomically publish empty candidates.' }
    Invoke-Id $window 'DictionaryBack'
    $null = Wait-Id $window $sampleId
    Invoke-Id $window 'DictionaryCard_base'
    $null = Wait-Id $window 'DictionaryBack'
    Invoke-Id $window 'DictionaryBack'
    $window.GetCurrentPattern([Windows.Automation.TransformPattern]::Pattern).Resize(540, 760)
    Start-Sleep -Milliseconds 250
    $bounds = $window.Current.BoundingRectangle
    foreach ($id in @($sampleId, 'DictionaryCard_user', 'DictionaryCard_base')) {
        $card = Wait-Id $window $id
        if (-not $card.Current.IsOffscreen -and $card.Current.BoundingRectangle.Right -gt $bounds.Right + 1) { throw 'Dictionary card overflows narrow window.' }
    }
    Write-Output 'PASS: card switches, imported/builtin menus, rename, delete confirmation/cancel, builtin recovery, detail/back, atomic state and responsive layout.'
} finally {
    if (-not $process.HasExited) { $process.CloseMainWindow() | Out-Null }
    if (-not $process.WaitForExit(5000)) { $process.Kill() }
}
