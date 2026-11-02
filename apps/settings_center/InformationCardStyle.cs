using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using System.Runtime.CompilerServices;

namespace OwO_Settings;

internal static class InformationCardStyle
{
    private static readonly ConditionalWeakTable<InfoBar, ResourceDictionary> Themes = new();
    internal static void Apply(InfoBar card, bool blue = true) {
        var resources = Themes.GetValue(card, _ => (ResourceDictionary)XamlReader.Load("""
            <ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                                xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">
                <ResourceDictionary.ThemeDictionaries>
                    <ResourceDictionary x:Key="Light">
                        <SolidColorBrush x:Key="InfoBarInformationalSeverityBackgroundBrush" Color="#EAF4FF"/>
                        <SolidColorBrush x:Key="InfoBarInformationalSeverityIconBackground" Color="#005FB8"/>
                        <SolidColorBrush x:Key="InfoBarMessageForeground" Color="#17324D"/>
                        <SolidColorBrush x:Key="InfoBarTitleForeground" Color="#17324D"/>
                        <SolidColorBrush x:Key="OwOInformationText" Color="#17324D"/>
                    </ResourceDictionary>
                    <ResourceDictionary x:Key="Dark">
                        <SolidColorBrush x:Key="InfoBarInformationalSeverityBackgroundBrush" Color="#D5E9FF"/>
                        <SolidColorBrush x:Key="InfoBarInformationalSeverityIconBackground" Color="#005FB8"/>
                        <SolidColorBrush x:Key="InfoBarMessageForeground" Color="#17324D"/>
                        <SolidColorBrush x:Key="InfoBarTitleForeground" Color="#17324D"/>
                        <SolidColorBrush x:Key="OwOInformationText" Color="#17324D"/>
                    </ResourceDictionary>
                    <ResourceDictionary x:Key="HighContrast">
                        <StaticResource x:Key="InfoBarInformationalSeverityBackgroundBrush" ResourceKey="SystemColorWindowColorBrush"/>
                        <StaticResource x:Key="InfoBarInformationalSeverityIconBackground" ResourceKey="SystemColorWindowTextColorBrush"/>
                        <StaticResource x:Key="InfoBarMessageForeground" ResourceKey="SystemColorWindowTextColorBrush"/>
                        <StaticResource x:Key="InfoBarTitleForeground" ResourceKey="SystemColorWindowTextColorBrush"/>
                        <StaticResource x:Key="OwOInformationText" ResourceKey="SystemColorWindowTextColorBrush"/>
                    </ResourceDictionary>
                </ResourceDictionary.ThemeDictionaries>
                <Style x:Key="OwOInformationCardStyle" TargetType="InfoBar">
                    <Setter Property="Foreground" Value="{ThemeResource OwOInformationText}"/>
                </Style>
            </ResourceDictionary>
            """));
        if (blue) {
            if (!card.Resources.MergedDictionaries.Contains(resources)) card.Resources.MergedDictionaries.Add(resources);
            card.Style = (Style)resources["OwOInformationCardStyle"];
        } else {
            card.Resources.MergedDictionaries.Remove(resources);
            card.ClearValue(FrameworkElement.StyleProperty);
        }
        card.CornerRadius = new CornerRadius(8);
    }
}
