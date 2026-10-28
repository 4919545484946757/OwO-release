using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Media.Imaging;

namespace OwO_Settings;

// Branding follows the application/system light palette unless explicitly
// overridden. This never changes the application's or candidate plugin's theme.
internal static class LogoAppearance
{
    internal static string Mode { get; private set; } = "system";
    private static event Action? Changed;
    internal static void Initialize() {
        try { SetMode(SettingsUiPreferences.Load(SettingsUiPreferences.DefaultPath).LogoTheme); }
        catch (Exception error) { App.LogStage("Logo preference fallback: " + error.Message); }
    }
    internal static void SetMode(string mode) {
        if (!LogoAssets.IsValid(mode)) throw new InvalidDataException("Logo 外观选项无效。");
        if (Mode == mode) return;
        Mode = mode;
        Changed?.Invoke();
    }
    private static string PathFor(string name) => Path.Combine(App.ResourceDirectory, "Assets", "Brand", name);
    private static SvgImageSource Source(FrameworkElement owner, bool rounded) =>
        new(new Uri(PathFor(LogoAssets.Svg(Mode, owner.ActualTheme == ElementTheme.Light, rounded))));

    internal static void BindImage(Image image, bool rounded = true) {
        var subscribed = false;
        void Apply() {
            var light = LogoAssets.IsLight(Mode, image.ActualTheme == ElementTheme.Light);
            AutomationProperties.SetName(image, light ? "OwO Logo（浅色）" : "OwO Logo（深色）");
            AutomationProperties.SetAutomationId(image, "OwOBrandLogo");
            AutomationProperties.SetItemStatus(image, "loading");
            var source = Source(image, rounded);
            source.Opened += (_, _) => {
                if (ReferenceEquals(image.Source, source)) AutomationProperties.SetItemStatus(image, "ready");
            };
            source.OpenFailed += (_, error) => {
                if (ReferenceEquals(image.Source, source)) AutomationProperties.SetItemStatus(image, "error");
                App.LogStage("SVG logo load failed: " + error.Status);
            };
            image.Source = source;
        }
        image.ActualThemeChanged += (_, _) => Apply();
        image.Loaded += (_, _) => {
            if (!subscribed) { Changed += Apply; subscribed = true; }
            Apply();
        };
        image.Unloaded += (_, _) => {
            if (subscribed) { Changed -= Apply; subscribed = false; }
        };
        Apply();
    }

    internal static void BindWindow(Window window, TitleBar titleBar) {
        if (window.Content is not FrameworkElement root) return;
        void Apply() {
            var light = root.ActualTheme == ElementTheme.Light;
            titleBar.IconSource = new ImageIconSource { ImageSource = Source(root, true) };
            window.AppWindow.SetIcon(PathFor(LogoAssets.Icon(Mode, light)));
        }
        void ThemeChanged(FrameworkElement sender, object args) => Apply();
        root.ActualThemeChanged += ThemeChanged;
        Changed += Apply;
        window.Closed += (_, _) => {
            root.ActualThemeChanged -= ThemeChanged;
            Changed -= Apply;
        };
        Apply();
    }
}
