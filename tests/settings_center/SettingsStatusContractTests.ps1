param([string]$Repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..')))
$ErrorActionPreference = 'Stop'
$settings = Join-Path $Repository 'apps/settings_center'
[xml]$page = [IO.File]::ReadAllText((Join-Path $settings 'MainPage.xaml'))
$namespaces = [Xml.XmlNamespaceManager]::new($page.NameTable)
$namespaces.AddNamespace('ui', 'http://schemas.microsoft.com/winfx/2006/xaml/presentation')
$namespaces.AddNamespace('x', 'http://schemas.microsoft.com/winfx/2006/xaml')
$bars = $page.SelectNodes('//ui:InfoBar', $namespaces)
if ($bars.Count -ne 1 -or $bars[0].GetAttribute('Name', 'http://schemas.microsoft.com/winfx/2006/xaml') -ne 'Status') {
    throw 'Regular settings XAML must have exactly one window-level InfoBar.'
}
if ($bars[0].ParentNode.ParentNode.ParentNode.GetAttribute('Name', 'http://schemas.microsoft.com/winfx/2006/xaml') -ne 'FooterHost') {
    throw 'Unified status bar must stay inside the existing window footer.'
}
$footer = $bars[0].ParentNode.ParentNode.ParentNode
if ($footer.GetAttribute('MinHeight') -ne '84' -or $footer.GetAttribute('MaxHeight') -ne '240' -or $bars[0].GetAttribute('Height') -ne '64') {
    throw 'Footer and status row need bounded, stable heights.'
}
$download = $page.SelectSingleNode('//*[@x:Name="WindowDownloadHost"]', $namespaces)
if ($null -eq $download -or $download.GetAttribute('MaxHeight') -ne '144') { throw 'Downloads must be height-limited.' }
$main = [IO.File]::ReadAllText((Join-Path $settings 'MainPage.xaml.cs'))
if (-not $main.Contains('ShowPageStatus') -or $main.Contains('Status.IsOpen = false')) { throw 'Expired messages must return to page information, not collapse the footer.' }
$card = [IO.File]::ReadAllText((Join-Path $settings 'InformationCardStyle.cs'))
foreach ($required in @('x:Key="Light"', 'x:Key="Dark"', 'x:Key="HighContrast"', '#EAF4FF', '#D5E9FF', '#17324D')) {
    if (-not $card.Contains($required)) { throw ('Information cards lack blue/high-contrast theme: ' + $required) }
}
$transfer = [IO.File]::ReadAllText((Join-Path $settings 'DownloadProgressView.cs'))
if (-not $transfer.Contains('InformationCardStyle.Apply(_card)')) { throw 'Downloads must use the shared blue information card.' }
foreach ($name in @('DictionaryManager', 'DictionaryDetailEditor', 'DictionaryDiscoveryView', 'UserLexiconEditor', 'OnlineDistributionView', 'PluginDiscoveryView')) {
    $source = [IO.File]::ReadAllText((Join-Path $settings ($name + '.cs')))
    if ($source -match '\bnew\s+InfoBar\b|\bInfoBar\s+_\w+' -or $source -notmatch 'Action<string, InfoBarSeverity>\s+_reportStatus') {
        throw ($name + ' must report to its injected window status sink, without a page-local InfoBar.')
    }
}
$button = [IO.File]::ReadAllText((Join-Path $settings 'DictionaryEncodingDialog.cs'))
foreach ($required in @('AccentButtonStyle', 'x:Key="Light"', 'x:Key="Dark"', 'x:Key="HighContrast"', 'AccentButtonBackgroundPointerOver', 'AccentButtonBackgroundPressed')) {
    if (-not $button.Contains($required)) { throw ('Primary button lacks native theme state: ' + $required) }
}
Write-Output 'PASS: one footer status bar, six injected page sinks, primary button theme/interaction states.'
