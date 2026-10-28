param([string]$PublishDirectory = '')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $PublishDirectory) { $PublishDirectory = Join-Path $project 'build/settings-publish/win-x64' }
$fixture = Join-Path $project ('build/theme-pages-ui-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $fixture)
$store = Join-Path $fixture 'plugins'
$shell = Join-Path $PublishDirectory 'owo_plugin_shell.exe'
$source = Join-Path $project 'plugins/candidate-glass'
$preview = (& $shell $store inspect-install $source | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0 -or -not $preview.ok) { throw 'Fixture plugin inspection failed.' }
& $shell $store install-risk $source $preview.inventory_sha256 1 I_ACCEPT_PLUGIN_RISK_V1 | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Fixture plugin installation failed.' }
$config = Join-Path $fixture 'owo.conf'
[IO.File]::WriteAllText((Join-Path $fixture 'settings-ui.json'), '{"AutoSaveEnabled":false}')
function Find-Id($window, $id) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::AutomationIdProperty, $id)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Find-Name($window, $name) {
    $condition = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)
    $window.FindFirst([Windows.Automation.TreeScope]::Descendants, $condition)
}
function Wait-Id($window, $id) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $element = Find-Id $window $id
        if ($element) { return $element }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Missing control: $id"
}
function Choose-Page($window, $name) {
    $selector = Wait-Id $window 'PluginConfigPageSelector'
    $selector.GetCurrentPattern([Windows.Automation.ExpandCollapsePattern]::Pattern).Expand()
    Start-Sleep -Milliseconds 100
    $item = Find-Name $window $name
    if (-not $item) { throw "Missing page option: $name" }
    $item.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    Start-Sleep -Milliseconds 100
}
$start = New-Object Diagnostics.ProcessStartInfo
$start.FileName = Join-Path $PublishDirectory 'OwO.Settings.exe'
$start.WorkingDirectory = $PublishDirectory
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.EnvironmentVariables['OWO_CONFIG_PATH'] = $config
$start.EnvironmentVariables['OWO_PLUGIN_STORE_PATH'] = $store
$process = [Diagnostics.Process]::Start($start)
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 200
        $process.Refresh()
        if ($process.HasExited) { throw 'Fixture settings process exited.' }
        $window = if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
            [Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
        } else { $null }
        $nav = if ($window) { Find-Name $window 'OwO 亚克力候选框' } else { $null }
    } while (-not $nav -and [DateTime]::UtcNow -lt $deadline)
    if (-not $nav) { throw 'Fixture plugin navigation missing.' }
    $nav.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select()
    $null = Wait-Id $window 'PluginConfigPageSelector'
    $settingsPath = Join-Path $store 'data/org.owo.candidate-glass/settings.json'
    foreach ($name in @('浅色配色', '深色配色', '公共效果', '浅色配色')) {
        Choose-Page $window $name
        $visibleName = switch ($name) {
            '浅色配色' { '浅色 · 背景颜色' }
            '深色配色' { '深色 · 背景颜色' }
            default { '高斯模糊背景' }
        }
        $visible = Find-Name $window $visibleName
        if (-not $visible -or $visible.Current.IsOffscreen) {
            $window.FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition) |
                Where-Object { $_.Current.AutomationId -like 'Plugin*' } |
                ForEach-Object { Write-Output ("id={0} name={1} offscreen={2}" -f $_.Current.AutomationId, $_.Current.Name, $_.Current.IsOffscreen) }
            throw "Selected page control is hidden: $name"
        }
        foreach ($title in @('浅色 · 背景颜色', '深色 · 背景颜色', '高斯模糊背景')) {
            if ($title -eq $visibleName) { continue }
            $hidden = Find-Name $window $title
            if ($hidden -and -not $hidden.Current.IsOffscreen) { throw "Unselected page remains visible: $title" }
        }
        if (Test-Path -LiteralPath $settingsPath) { throw 'Switching editor pages unexpectedly saved settings.' }
    }
    $save = Wait-Id $window 'PluginConfigSaveButton'
    $ancestor = $save
    while ($ancestor) {
        try {
            $scroll = $ancestor.GetCurrentPattern([Windows.Automation.ScrollPattern]::Pattern)
            $scroll.SetScrollPercent(-1, 100)
            break
        } catch { $ancestor = [Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($ancestor) }
    }
    $save.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
    Start-Sleep -Milliseconds 400
    $saved = Get-Content -LiteralPath $settingsPath -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($key in @('background_color', 'light_background_color', 'caret_color', 'light_caret_color',
                       'hover_block_opacity', 'light_hover_block_opacity', 'button_border_color', 'light_button_border_color')) {
        if (-not $saved.PSObject.Properties[$key]) { throw "Hidden-page setting lost during save: $key" }
    }
    if ($saved.theme_mode -ne 'system') { throw 'Editor page selection changed the runtime theme mode.' }
    Write-Output 'PASS: page selector visibility, no save on page switch, independent hidden-page values and unchanged runtime theme.'
} finally {
    if (-not $process.HasExited) { $process.CloseMainWindow() | Out-Null }
    if (-not $process.WaitForExit(5000)) { $process.Kill() }
}
