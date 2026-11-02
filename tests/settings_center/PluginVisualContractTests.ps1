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
$image = $page.SelectSingleNode('//ui:Image[@x:Name="PluginPuzzleBackground"]', $ns)
if ($null -eq $image -or $image.GetAttribute('IsHitTestVisible') -ne 'False' -or
    $image.GetAttribute('Width') -ne '480' -or $image.GetAttribute('Height') -ne '480' -or
    $image.GetAttribute('Margin') -ne '0,0,-88,-104' -or
    $image.SelectSingleNode('ui:Image.RenderTransform/ui:RotateTransform', $ns).GetAttribute('Angle') -ne '-18') { throw 'Unexpected decorative SVG geometry.' }
if ($null -eq $image.ParentNode.SelectSingleNode('ui:Grid.Clip/ui:RectangleGeometry', $ns) -or
    $null -eq $image.ParentNode.SelectSingleNode('ui:ScrollViewer', $ns)) { throw 'Background must be clipped and independent from the scrolling form.' }
foreach ($theme in @('Light','Dark','HighContrast')) {
    $dictionary = $page.SelectSingleNode('//ui:ResourceDictionary[@x:Key="' + $theme + '"]', $ns)
    $opacity = $dictionary.SelectSingleNode('x:Double[@x:Key="PluginPuzzleOpacity"]', $ns).InnerText
    if ($opacity -ne $(if ($theme -eq 'HighContrast') { '0' } else { '0.06' })) { throw ('Unexpected opacity for ' + $theme) }
}
[xml]$light = [IO.File]::ReadAllText((Join-Path $settings 'Assets/Decor/PluginPuzzle.svg'))
[xml]$dark = [IO.File]::ReadAllText((Join-Path $settings 'Assets/Decor/PluginPuzzle.Dark.svg'))
if ($light.svg.path.d -ne $dark.svg.path.d -or $light.svg.path.stroke -ne '#000000' -or $dark.svg.path.stroke -ne '#FFFFFF') { throw 'Theme variants must preserve the supplied vector geometry.' }
if (-not [IO.File]::ReadAllText((Join-Path $settings 'OwO.Settings.csproj')).Contains('Assets\Decor\*.svg')) { throw 'SVG payload must be copied to build/publish output.' }
Write-Output 'PASS: matching rounded left border, SVG geometry, position, opacity, theme variants, clipping and publish contract.'
