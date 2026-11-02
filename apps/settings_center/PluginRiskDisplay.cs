using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

internal sealed class PluginRiskDisplay
{
    internal StackPanel Content { get; } = new() { Spacing = 12 };
    private readonly string _key;
    private readonly CheckBox _hide = new() { Content = "不再自动展开此版本的风险说明（仍需逐次明确授权）" };
    internal PluginRiskDisplay(string key, string summary, UIElement details) {
        _key = key;
        Content.Children.Add(new TextBlock { Text = summary, TextWrapping = TextWrapping.Wrap });
        Content.Children.Add(new Expander { Header = "权限、隐私与风险详情", Content = details,
            IsExpanded = !PluginRiskPreferences.Hidden(key), HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch });
        _hide.IsChecked = PluginRiskPreferences.Hidden(key);
        Content.Children.Add(_hide);
    }
    internal void Accepted() {
        try { PluginRiskPreferences.Remember(_key, hidden: _hide.IsChecked == true); }
        catch { /* Failed persistence cannot grant or revoke authority. */ }
    }
}
