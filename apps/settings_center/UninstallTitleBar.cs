using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using System.Runtime.InteropServices;
using Windows.UI.ViewManagement;

namespace OwO_Settings;

internal static class UninstallTitleBar
{
    internal static void Bind(Window window, FrameworkElement root)
    {
        if (!AppWindowTitleBar.IsCustomizationSupported()) return;
        var bar = window.AppWindow.TitleBar;
        var accessibility = new AccessibilitySettings();
        var dispatcher = root.DispatcherQueue;
        var closed = false;
        var watchingContrast = false;
        void Apply() {
            if (closed) return;
            bar.PreferredTheme = accessibility.HighContrast ? TitleBarTheme.UseDefaultAppMode :
                root.ActualTheme == ElementTheme.Dark ? TitleBarTheme.Dark : TitleBarTheme.Light;
        }
        void OnLoaded(object sender, RoutedEventArgs args) => Apply();
        void OnThemeChanged(FrameworkElement sender, object args) => Apply();
        void OnActivated(object sender, WindowActivatedEventArgs args) => Apply();
        void OnHighContrastChanged(AccessibilitySettings sender, object args) => dispatcher.TryEnqueue(Apply);
        root.Loaded += OnLoaded;
        root.ActualThemeChanged += OnThemeChanged;
        window.Activated += OnActivated;
        try {
            accessibility.HighContrastChanged += OnHighContrastChanged;
            watchingContrast = true;
        } catch (COMException) {
            // Some desktop sessions don't provide this notification. Loaded,
            // activated and XAML theme changes still refresh the native bar.
        }
        window.Closed += (_, _) => {
            closed = true;
            root.Loaded -= OnLoaded;
            root.ActualThemeChanged -= OnThemeChanged;
            window.Activated -= OnActivated;
            if (watchingContrast) accessibility.HighContrastChanged -= OnHighContrastChanged;
        };
        Apply();
    }
}
