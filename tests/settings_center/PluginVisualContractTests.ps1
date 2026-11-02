param([string]$Repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..')))
$ErrorActionPreference = 'Stop'
$settings = Join-Path $Repository 'apps/settings_center'
[xml]$page = [IO.File]::ReadAllText((Join-Path $settings 'MainPage.xaml'))
$ns = [Xml.XmlNamespaceManager]::new($page.NameTable)
$ns.AddNamespace('ui', 'http://schemas.microsoft.com/winfx/2006/xaml/presentation')
$ns.AddNamespace('x', 'http://schemas.microsoft.com/winfx/2006/xaml')
$brush = $page.SelectSingleNode('//ui:SolidColorBrush[@x:Key="PluginItemLeftBorderBrush"]', $ns)
if ($brush.GetAttribute('Color') -ne '#78000000') { throw 'Plugin left border must match dictionary ARGB(120,0,0,0).' }
$border = $page.SelectSingleNode('//ui:ItemsControl[@x:Name="PluginVersions"]//ui:Border[@BorderThickness="3,0,0,0"]', $ns)
if ($null -eq $border -or $border.GetAttribute('CornerRadius') -ne '8' -or $border.GetAttribute('IsHitTestVisible') -ne 'False') { throw 'Plugin border must be rounded and not intercept interaction.' }
foreach ($imageName in @('PluginPuzzleBackground', 'PluginManagerPuzzleBackground')) {
$image = $page.SelectSingleNode('//ui:Image[@x:Name="' + $imageName + '"]', $ns)
if ($null -eq $image -or $image.GetAttribute('IsHitTestVisible') -ne 'False' -or
    $image.GetAttribute('Width') -ne '480' -or $image.GetAttribute('Height') -ne '480' -or
    $image.GetAttribute('VerticalAlignment') -ne 'Top' -or $image.GetAttribute('HorizontalAlignment') -ne 'Right' -or
    $image.GetAttribute('Margin') -ne '0,-104,-88,0' -or
    $image.SelectSingleNode('ui:Image.RenderTransform/ui:RotateTransform', $ns).GetAttribute('Angle') -ne '-18') { throw 'Unexpected decorative SVG geometry.' }
if ($null -eq $image.ParentNode.SelectSingleNode('ui:Grid.Clip/ui:RectangleGeometry', $ns) -or
    $null -eq $image.ParentNode.SelectSingleNode('ui:ScrollViewer', $ns)) { throw 'Background must be clipped and independent from the scrolling form.' }
if ($image.GetAttribute('Canvas.ZIndex') -ne '-1' -or
    $image.ParentNode.SelectSingleNode('ui:ScrollViewer', $ns).GetAttribute('Canvas.ZIndex') -ne '0') { throw 'Decoration must remain below the interactive content.' }
}
foreach ($theme in @('Light','Dark')) {
    $background = $page.SelectSingleNode('//ui:ResourceDictionary[@x:Key="' + $theme + '"]/ui:SolidColorBrush[@x:Key="SettingsContentBackgroundBrush"]', $ns)
    if ($background.GetAttribute('Color') -notmatch '^#[0-9A-Fa-f]{6}$') { throw 'Card backing must be opaque.' }
}
$backingStyle = $page.SelectSingleNode('//ui:Style[@x:Key="PluginCardBackingStyle"]', $ns)
if ($backingStyle.SelectSingleNode('ui:Setter[@Property="Background"]', $ns).GetAttribute('Value') -ne '{ThemeResource SettingsContentBackgroundBrush}') { throw 'Card backing must follow the page theme.' }
foreach ($list in @('PluginVersions','RecoveryItems')) {
    $underlay = $page.SelectSingleNode('//ui:ItemsControl[@x:Name="' + $list + '"]//ui:Border[@Background="{ThemeResource SettingsContentBackgroundBrush}"]', $ns)
    if ($null -eq $underlay -or $underlay.GetAttribute('CornerRadius') -ne '8') { throw ('Missing opaque card backing: ' + $list) }
}
foreach ($source in @('PluginSettingsPage.cs','PluginDiscoveryView.cs')) {
    if (-not [IO.File]::ReadAllText((Join-Path $settings $source)).Contains('PluginCardSurface.Wrap(card,')) { throw ('Dynamic cards must obscure the decoration: ' + $source) }
}
$configSource = [IO.File]::ReadAllText((Join-Path $settings 'PluginSettingsPage.cs'))
if (-not $configSource.Contains('card.Child is Border { Child: Expander }')) { throw 'The first color group must still expand by default after wrapping cards.' }
if (-not $configSource.Contains('PluginCardSurface.Wrap(pageSelector,')) { throw 'The configuration page selector must also obscure the decoration.' }
foreach ($theme in @('Light','Dark','HighContrast')) {
    $dictionary = $page.SelectSingleNode('//ui:ResourceDictionary[@x:Key="' + $theme + '"]', $ns)
    $opacity = $dictionary.SelectSingleNode('x:Double[@x:Key="PluginPuzzleOpacity"]', $ns).InnerText
    $value = 0.0
    if (-not [double]::TryParse($opacity, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$value) -or
        $value -lt 0 -or $value -gt 1 -or ($theme -eq 'HighContrast' -and $value -ne 0)) { throw ('Invalid opacity for ' + $theme) }
}
[xml]$light = [IO.File]::ReadAllText((Join-Path $settings 'Assets/Decor/PluginPuzzle.svg'))
[xml]$dark = [IO.File]::ReadAllText((Join-Path $settings 'Assets/Decor/PluginPuzzle.Dark.svg'))
if ($light.svg.path.d -ne $dark.svg.path.d -or $light.svg.path.stroke -ne '#000000' -or $dark.svg.path.stroke -ne '#FFFFFF') { throw 'Theme variants must preserve the supplied vector geometry.' }
if (-not [IO.File]::ReadAllText((Join-Path $settings 'OwO.Settings.csproj')).Contains('Assets\Decor\*.svg')) { throw 'SVG payload must be copied to build/publish output.' }
Write-Output 'PASS: matching rounded left border, SVG geometry, position, opacity, theme variants, clipping and publish contract.'
