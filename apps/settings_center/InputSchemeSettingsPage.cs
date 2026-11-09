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
            var count = await RefreshInputSchemesAsync();
            ShowStatus($"发现 {count} 个已授权方案；选择后将按保存设置应用。", InfoBarSeverity.Informational);
        } catch (Exception error) { ShowStatus(error.Message, InfoBarSeverity.Error); }
        finally { DiscoverSchemesButton.IsEnabled = true; }
    }

    private async Task<int> RefreshInputSchemesAsync()
    {
        var discovered = await _pluginClient.DiscoverInputSchemesAsync();
        var previous = CurrentScheme;
        var options = InputSchemeSettingsPolicy.Refresh(discovered.Select(s =>
            InputSchemeSettingsPolicy.Plugin(s.Id, s.Name + "（插件）", $"{s.PluginId} · {s.Language}")), previous);
        var loaded = _settingsLoaded;
        _settingsLoaded = false; // Refreshing labels must not save a transient ComboBox selection.
        try {
            _availableSchemes.Clear();
            _availableSchemes.AddRange(options);
            InputScheme.ItemsSource = _availableSchemes.ToArray();
            SelectInputScheme(previous.Id);
            UpdateInputSchemePresentation();
        } finally { _settingsLoaded = loaded; }
        return discovered.Select(s => s.Id).Distinct(StringComparer.Ordinal).Count();
    }

    private async Task RestorePinyinForPluginAsync(string pluginId)
    {
        await _settingsSaveGate.WaitAsync();
        try {
            // Reload before writing so disabling a plugin changes only its persisted scheme.
            var saved = await _client.LoadAsync();
            if (InputSchemeSettingsPolicy.NeedsFallback(saved.InputScheme, pluginId)) {
                await _client.SaveAsync(saved with { InputScheme = "full_pinyin" });
                if (_settingsBaseline is not null)
                    _settingsBaseline = _settingsBaseline with { InputScheme = "full_pinyin" };
            }
            if (InputSchemeSettingsPolicy.NeedsFallback(CurrentScheme.Id, pluginId))
                SelectInputScheme("full_pinyin");
        } catch (Exception error) {
            throw new InvalidOperationException($"插件已停用，但恢复全拼配置失败：{error.Message}", error);
        } finally { _settingsSaveGate.Release(); }
    }

    private void InputScheme_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        CancelShortcutCapture();
        UpdateInputSchemePresentation();
    }

    private void UpdateInputSchemePresentation()
    {
        if (InputSchemeDescription is null) return;
        InputSchemeDescription.Text = CurrentScheme.Description;
        UpdateDependentSettings();
        UpdateSchemeHintVisibility();
    }

    private void UpdateSchemeHintVisibility()
    {
        if (_currentPage == "schemes" && !string.IsNullOrWhiteSpace(CurrentScheme.Capabilities))
            ShowStatus(CurrentScheme.Capabilities, InfoBarSeverity.Informational);
    }

    private void DependentSetting_Toggled(object sender, RoutedEventArgs e) => UpdateDependentSettings();

    private void CancelShortcutCapture()
    {
        if (_shortcutCaptureTarget?.Tag is ShortcutBinding binding) _shortcutCaptureTarget.Content = binding.Value;
        _shortcutCaptureTarget = null;
        StopShortcutCaptureHook();
    }
}
