using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private readonly List<InputSchemeOption> _availableSchemes = [.. InputSchemeSettingsPolicy.BuiltIns];
    private InputSchemeOption CurrentScheme => InputScheme.SelectedItem as InputSchemeOption
        ?? InputSchemeSettingsPolicy.Resolve(_settingsBaseline?.InputScheme ?? "full_pinyin");

    private void SelectInputScheme(string id)
    {
        if (!_availableSchemes.Any(scheme => scheme.Id == id)) {
            _availableSchemes.Add(InputSchemeSettingsPolicy.Resolve(id));
            InputScheme.ItemsSource = _availableSchemes.ToArray();
        }
        InputScheme.SelectedItem = _availableSchemes.First(scheme => scheme.Id == id);
    }

    private async void DiscoverSchemes_Click(object sender, RoutedEventArgs e)
    {
        DiscoverSchemesButton.IsEnabled = false;
        try {
            var previous = CurrentScheme;
            var discovered = await _pluginClient.DiscoverInputSchemesAsync();
            _availableSchemes.Clear();
            _availableSchemes.AddRange(InputSchemeSettingsPolicy.BuiltIns);
            _availableSchemes.AddRange(discovered.Select(s => InputSchemeSettingsPolicy.Plugin(s.Id,
                s.Name + "（插件·实验性）", $"{s.PluginId} · {s.Language}")));
            if (!_availableSchemes.Any(scheme => scheme.Id == previous.Id))
                _availableSchemes.Add(previous with { Description = "方案当前不可用或未获授权；请检查插件页面。" });
            InputScheme.ItemsSource = _availableSchemes.ToArray();
            SelectInputScheme(previous.Id);
            ShowStatus($"发现 {discovered.Count} 个已授权方案；选择后将按保存设置应用。", InfoBarSeverity.Informational);
        } catch (Exception error) { ShowStatus(error.Message, InfoBarSeverity.Error); }
        finally { DiscoverSchemesButton.IsEnabled = true; }
    }

    private void InputScheme_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        CancelShortcutCapture();
        UpdateInputSchemePresentation();
    }

    private void UpdateInputSchemePresentation()
    {
        if (InputSchemeDescription is null || InputSchemeCapabilities is null) return;
        InputSchemeDescription.Text = CurrentScheme.Description;
        InputSchemeCapabilities.Message = CurrentScheme.Capabilities;
        UpdateDependentSettings();
        UpdateSchemeHintVisibility();
    }

    private void UpdateSchemeHintVisibility()
    {
        if (InputSchemeCapabilities is null) return;
        var visible = _currentPage == "schemes" && !string.IsNullOrWhiteSpace(CurrentScheme.Capabilities);
        InputSchemeCapabilities.IsOpen = visible;
        InputSchemeCapabilities.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;
    }

    private void DependentSetting_Toggled(object sender, RoutedEventArgs e) => UpdateDependentSettings();

    private void CancelShortcutCapture()
    {
        if (_shortcutCaptureTarget?.Tag is ShortcutBinding binding) _shortcutCaptureTarget.Content = binding.Value;
        _shortcutCaptureTarget = null;
        StopShortcutCaptureHook();
    }
}
