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
function Check-CardBounds($window, $key) {
    $card = Wait-Id $window ('DictionaryCard_' + $key)
    if ($card.Current.IsOffscreen) { return }
    $bounds = $card.Current.BoundingRectangle
    $horizontalInset = [Math]::Min(6, [Math]::Max(0, ($bounds.Width - 44) / 2))
    foreach ($prefix in @('DictionaryCardEnabled_', 'DictionaryCardMenu_')) {
        $control = Wait-Id $window ($prefix + $key)
        if ($control.Current.IsOffscreen) { continue }
        $rect = $control.Current.BoundingRectangle
        if ($rect.Left -lt $bounds.Left + $horizontalInset -or $rect.Right -gt $bounds.Right - $horizontalInset -or
            $rect.Top -lt $bounds.Top -or $rect.Bottom -gt $bounds.Bottom) {
            # UIA clips a partially scrolled card to the viewport. A visible
            # control may touch that clip edge without escaping the real card.
            throw "Dictionary card control outside visual card bounds: $prefix$key card=$bounds control=$rect"
        }
    }
    $menuControl = Wait-Id $window ('DictionaryCardMenu_' + $key)
    if ($menuControl.Current.IsOffscreen) { return }
    $menu = $menuControl.Current.BoundingRectangle
    if ([Math]::Abs($menu.Width - $menu.Height) -gt 1) { throw 'Dictionary menu is not square.' }
}
function Save-CardScreenshot($window, $key, $name) {
    if ($env:OWO_CAPTURE_DICTIONARY_UI -ne '1') { return }
    Add-Type -AssemblyName System.Drawing
    $rect = (Wait-Id $window ('DictionaryCard_' + $key)).Current.BoundingRectangle
    $bitmap = New-Object Drawing.Bitmap([int][Math]::Ceiling($rect.Width), [int][Math]::Ceiling($rect.Height))
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen([int]$rect.Left, [int]$rect.Top, 0, 0, $bitmap.Size)
        $path = Join-Path $fixture ($name + '.png')
        $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png)
        Write-Output "Isolated card screenshot: $path"
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
function Card-Identities($window) {
    $result = @{}
    foreach ($key in @(('1' * 32), 'user', 'base')) {
        foreach ($prefix in @('DictionaryCard_', 'DictionaryCardEnabled_', 'DictionaryCardMenu_')) {
            $id = $prefix + $key
            $result[$id] = (Wait-Id $window $id).GetRuntimeId() -join ':'
        }
    }
    return $result
}
function Check-CardIdentities($window, $expected) {
    $actual = Card-Identities $window
    foreach ($id in $expected.Keys) {
        if ($actual[$id] -ne $expected[$id]) { throw "Unchanged dictionary control was recreated: $id" }
    }
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
    foreach ($key in @(('1' * 32), 'user', 'base')) { Check-CardBounds $window $key }
    Save-CardScreenshot $window 'user' 'card-normal'
    $search = Wait-Id $window 'DictionarySearch'
    $search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('测试字典')
    Start-Sleep -Milliseconds 150
    if (-not (Find-Id $window $sampleId) -or (Find-Id $window 'DictionaryCard_user')) { throw 'Dictionary search did not filter cards.' }
    $search.GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('')
    $sampleKey = '1' * 32
    $identities = Card-Identities $window
    foreach ($iteration in 1..3) {
        Invoke-Id $window 'DictionaryRefresh'
        Start-Sleep -Milliseconds 150
        Check-CardIdentities $window $identities
    }
    $cardToggle = Wait-Id $window ('DictionaryCardEnabled_' + $sampleKey)
    $cardToggle.SetFocus()
    $cardToggle.GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 350
    if ((Catalog).dictionaries[0].enabled -or (Find-Id $window 'DictionaryBack')) { throw 'Card toggle failed or unexpectedly opened details.' }
    Check-CardIdentities $window $identities
    if (-not (Wait-Id $window ('DictionaryCardEnabled_' + $sampleKey)).Current.HasKeyboardFocus) {
        throw 'Dictionary toggle lost keyboard focus after saving.'
    }
    (Wait-Id $window ('DictionaryCardEnabled_' + $sampleKey)).GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
    Start-Sleep -Milliseconds 350
    Check-CardIdentities $window $identities
    # A rejected save rolls back only the changed toggle; no recursion, extra
    # snapshots, full-card rebuild or stale visual state is permitted.
    $fixtureStore = Join-Path $fixture 'dictionaries'
    $beforeRejected = [IO.File]::ReadAllText((Join-Path $fixtureStore 'current'))
    $lock = [IO.File]::Open((Join-Path $fixtureStore 'store.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        (Wait-Id $window ('DictionaryCardEnabled_' + $sampleKey)).GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Toggle()
        Start-Sleep -Milliseconds 350
        if ((Wait-Id $window ('DictionaryCardEnabled_' + $sampleKey)).GetCurrentPattern([Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -ne [Windows.Automation.ToggleState]::On -or
            [IO.File]::ReadAllText((Join-Path $fixtureStore 'current')) -ne $beforeRejected) { throw 'Failed save did not restore the existing toggle without publishing.' }
        Check-CardIdentities $window $identities
    } finally { $lock.Dispose() }
    Invoke-Id $window ('DictionaryCardMenu_' + $sampleKey)
    $null = Wait-Id $window ('DictionaryCardExport_' + $sampleKey)
    $null = Wait-Id $window ('DictionaryCardDelete_' + $sampleKey)
    Invoke-Id $window ('DictionaryCardRename_' + $sampleKey)
    (Wait-Id $window 'DictionaryRenameInput').GetCurrentPattern([Windows.Automation.ValuePattern]::Pattern).SetValue('改名测试字典')
    Confirm-Dialog $window '保存'
    if ((Catalog).dictionaries[0].name -ne '改名测试字典' -or (Find-Id $window 'DictionaryBack')) { throw 'Card rename failed or opened details.' }
    Check-CardIdentities $window $identities
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
    foreach ($key in @(('1' * 32), 'user', 'base')) { Check-CardBounds $window $key }
    Write-Output 'PASS: stable card/switch/menu identity across refresh/toggle/rename, menus, delete confirmation/cancel, builtin recovery, detail/back, atomic state and responsive layout.'
} finally {
    if (-not $process.HasExited) { $process.CloseMainWindow() | Out-Null }
    if (-not $process.WaitForExit(5000)) { $process.Kill() }
}
