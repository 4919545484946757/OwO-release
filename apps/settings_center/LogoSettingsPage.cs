using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private readonly ComboBox _logoThemeSelector = new() {
        ItemsSource = new[] { "跟随系统", "浅色", "深色" },
        MinWidth = 160, VerticalAlignment = VerticalAlignment.Center,
    };
    private Border? _logoThemeCard;
    private bool _loadingLogoPreference;
    private static string LogoMode(int index) => index switch { 1 => "light", 2 => "dark", _ => "system" };
    private void InitializeLogoSettings() {
        if (SettingsNavigation.PaneHeader is StackPanel header && header.Children.OfType<Image>().FirstOrDefault() is Image logo)
            LogoAppearance.BindImage(logo);
        var content = new Grid { ColumnSpacing = 16 };
        content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var labels = new StackPanel { Spacing = 3 };
        labels.Children.Add(new TextBlock { Text = "Logo 外观", Style = (Style)Resources["SettingTitleStyle"] });
        labels.Children.Add(new TextBlock {
            Text = "控制界面 Logo、标题栏和运行中窗口图标；仅改变 Logo，不切换界面或候选框主题。此选项立即保存。",
            Style = (Style)Resources["SettingDescriptionStyle"],
        });
        content.Children.Add(labels);
        Grid.SetColumn(_logoThemeSelector, 1);
        content.Children.Add(_logoThemeSelector);
        _logoThemeCard = new Border { Style = (Style)Resources["SettingsCardStyle"], Child = content };
        AutomationProperties.SetAutomationId(_logoThemeSelector, "LogoTheme");
        AutomationProperties.SetName(_logoThemeSelector, "Logo 外观");
        ((StackPanel)GeneralPreferencesPage.Content).Children.Insert(1, _logoThemeCard);
        RefreshLogoPreference();
        _logoThemeSelector.SelectionChanged += (_, _) => {
            if (_loadingLogoPreference) return;
            var previous = LogoAppearance.Mode;
            try {
                var preferences = SettingsUiPreferences.Load(UiPreferencesPath);
                var mode = LogoMode(_logoThemeSelector.SelectedIndex);
                (preferences with { LogoTheme = mode }).Save(UiPreferencesPath);
                LogoAppearance.SetMode(mode);
            } catch (Exception error) {
                SetLogoSelection(previous);
                ShowStatus("Logo 外观保存失败：" + error.Message, InfoBarSeverity.Error);
            }
        };
    }
    private void SetLogoSelection(string mode) {
        _loadingLogoPreference = true;
        try { _logoThemeSelector.SelectedIndex = mode switch { "light" => 1, "dark" => 2, _ => 0 }; }
        finally { _loadingLogoPreference = false; }
    }
    private void RefreshLogoPreference() {
        var mode = LogoAppearance.Mode;
        try { mode = SettingsUiPreferences.Load(UiPreferencesPath).LogoTheme; }
        catch (Exception error) { ShowStatus("Logo 偏好读取失败：" + error.Message, InfoBarSeverity.Error); }
        LogoAppearance.SetMode(mode);
        SetLogoSelection(mode);
    }
}
