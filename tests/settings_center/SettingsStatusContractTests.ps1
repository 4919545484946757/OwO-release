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
