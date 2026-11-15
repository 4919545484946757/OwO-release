using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private readonly ScrollNeutralComboBox _logoThemeSelector = new() {
        ItemsSource = new[] { "跟随系统", "浅色", "深色" },
        MinWidth = 160, VerticalAlignment = VerticalAlignment.Center,
    };
    private readonly ScrollNeutralComboBox _settingsThemeSelector = new() {
        ItemsSource = new[] { "跟随系统", "浅色", "深色" },
        MinWidth = 160, VerticalAlignment = VerticalAlignment.Center,
    };
    private Border? _logoThemeCard;
    private Border? _settingsThemeCard;
    private Button? _settingsThemeLogo;
    private string _settingsThemeMode = "system";
    private bool _loadingLogoPreference;
    private static string AppearanceMode(int index) => index switch { 1 => "light", 2 => "dark", _ => "system" };
    private static int AppearanceIndex(string mode) => mode switch { "light" => 1, "dark" => 2, _ => 0 };
    internal static ElementTheme SettingsElementTheme(string mode) => mode switch {
        "light" => ElementTheme.Light, "dark" => ElementTheme.Dark, _ => ElementTheme.Default,
    };
    private void InitializeLogoSettings() {
        if (SettingsNavigation.PaneHeader is StackPanel header && header.Children.OfType<Image>().FirstOrDefault() is Image logo)
            LogoAppearance.BindImage(logo);
        InitializeSearchLogo();
        _logoThemeCard = AppearanceCard(_logoThemeSelector, "Logo 外观",
            "控制搜索栏 Logo、安装界面和运行中窗口图标；不改变界面主题。此选项立即保存。", "LogoTheme");
        _settingsThemeCard = AppearanceCard(_settingsThemeSelector, "设置中心主题",
            "点击左上角 Logo 可在深色与浅色间切换。可在此恢复跟随系统；不改变输入法候选框主题。", "SettingsTheme");
        RefreshLogoPreference();
        Loaded += (_, _) => ApplySettingsWindowTheme();
        _settingsThemeSelector.SelectionChanged += (_, _) => {
            if (!_loadingLogoPreference && _settingsThemeSelector.SelectedIndex >= 0)
                ChangeSettingsWindowTheme(AppearanceMode(_settingsThemeSelector.SelectedIndex));
        };
        _logoThemeSelector.SelectionChanged += (_, _) => {
            if (_loadingLogoPreference || _logoThemeSelector.SelectedIndex < 0) return;
            var previous = LogoAppearance.Mode;
            try {
                var preferences = SettingsUiPreferences.Load(UiPreferencesPath);
                var mode = AppearanceMode(_logoThemeSelector.SelectedIndex);
                (preferences with { LogoTheme = mode }).Save(UiPreferencesPath);
                LogoAppearance.SetMode(mode);
            } catch (Exception error) {
                SetAppearanceSelection(_logoThemeSelector, previous);
                ShowStatus("Logo 外观保存失败：" + error.Message, InfoBarSeverity.Error);
            }
        };
    }
    private Border AppearanceCard(ScrollNeutralComboBox selector, string title, string description, string id) {
        var content = new Grid { ColumnSpacing = 16 };
        content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var labels = new StackPanel { Spacing = 3 };
        labels.Children.Add(new TextBlock { Text = title, Style = (Style)Resources["SettingTitleStyle"] });
        labels.Children.Add(new TextBlock {
            Text = description,
            Style = (Style)Resources["SettingDescriptionStyle"],
        });
        content.Children.Add(labels);
        Grid.SetColumn(selector, 1);
        content.Children.Add(selector);
        AutomationProperties.SetAutomationId(selector, id);
        AutomationProperties.SetName(selector, title);
        return new Border { Style = (Style)Resources["SettingsCardStyle"], Child = content };
    }
    private void InitializeSearchLogo() {
        const double logoSize = 48, gap = 16;
        var logo = new Image {
            Width = logoSize, Height = logoSize,
            VerticalAlignment = VerticalAlignment.Center,
        };
        LogoAppearance.BindImage(logo);
        var button = _settingsThemeLogo = new Button {
            Content = logo, Padding = new Thickness(0), BorderThickness = new Thickness(0),
            Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent),
            MinWidth = 0, MinHeight = 0,
        };
        AutomationProperties.SetAutomationId(button, "SettingsThemeLogo");
        AutomationProperties.SetName(button, "切换设置中心深浅色主题");
        void UpdateHint() => ToolTipService.SetToolTip(button, button.ActualTheme == ElementTheme.Dark ? "切换到浅色主题" : "切换到深色主题");
        UpdateHint();
        button.ActualThemeChanged += (_, _) => UpdateHint();
        button.Click += (_, _) => ChangeSettingsWindowTheme(button.ActualTheme == ElementTheme.Dark ? "light" : "dark");
        // Sibling containers: branding is anchored to the page's left edge,
        // while search keeps its original independent, centered layout.
        var logoHost = new Border {
            Child = button, Margin = GlobalSearchArea.Margin,
            HorizontalAlignment = HorizontalAlignment.Left,
            VerticalAlignment = VerticalAlignment.Center,
            Visibility = Visibility.Collapsed,
        };
        Grid.SetRow(logoHost, Grid.GetRow(GlobalSearchArea));
        PageRoot.Children.Add(logoHost);
        SettingsSearchBox.VerticalAlignment = VerticalAlignment.Center;
        bool? wasVisible = null;
        void Update() {
            var minimumSearchWidth = Math.Max(SettingsSearchBox.MinWidth, SettingsSearchBox.FontSize * 24 + 24);
            var searchLeft = SettingsSearchBox.TransformToVisual(PageRoot).TransformPoint(default).X;
            var visible = SettingsSearchBox.ActualWidth >= minimumSearchWidth &&
                searchLeft >= logoHost.Margin.Left + logoSize + gap;
            if (wasVisible == visible) return;
            wasVisible = visible;
            logoHost.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;
        }
        GlobalSearchArea.SizeChanged += (_, _) => Update();
        SettingsSearchBox.SizeChanged += (_, _) => Update();
        GlobalSearchArea.Loaded += (_, _) => Update();
    }
    private void SetAppearanceSelection(ScrollNeutralComboBox selector, string mode) {
        _loadingLogoPreference = true;
        try { selector.SelectedIndex = AppearanceIndex(mode); }
        finally { _loadingLogoPreference = false; }
    }
    private void ApplySettingsWindowTheme() {
        if (XamlRoot?.Content is FrameworkElement root)
            root.RequestedTheme = SettingsElementTheme(_settingsThemeMode);
    }
    private void ChangeSettingsWindowTheme(string mode) {
        try {
            (SettingsUiPreferences.Load(UiPreferencesPath) with { SettingsTheme = mode }).Save(UiPreferencesPath);
            _settingsThemeMode = mode;
            ApplySettingsWindowTheme();
        } catch (Exception error) { ShowStatus("设置中心主题保存失败：" + error.Message, InfoBarSeverity.Error); }
        SetAppearanceSelection(_settingsThemeSelector, _settingsThemeMode);
    }
    private void RefreshLogoPreference() {
        var mode = LogoAppearance.Mode;
        try {
            var preferences = SettingsUiPreferences.Load(UiPreferencesPath);
            mode = preferences.LogoTheme;
            _settingsThemeMode = preferences.SettingsTheme;
        }
        catch (Exception error) { ShowStatus("Logo 偏好读取失败：" + error.Message, InfoBarSeverity.Error); }
        LogoAppearance.SetMode(mode);
        SetAppearanceSelection(_logoThemeSelector, mode);
        SetAppearanceSelection(_settingsThemeSelector, _settingsThemeMode);
        ApplySettingsWindowTheme();
    }
}
