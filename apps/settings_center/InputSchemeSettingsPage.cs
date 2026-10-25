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
        UpdateSchemeSettingsNotice();
    }

    private void UpdateSchemeSettingsNotice()
    {
        if (SchemeSettingsNotice is null) return;
        SchemeSettingsNotice.IsOpen = _currentPage is "schemes" or "candidates" or "intelligence" or
            "shortcuts" or "punctuation" or "user-lexicon";
        SchemeSettingsNotice.Message = _currentPage == "user-lexicon"
            ? "此页管理宿主内置词库，不是插件方案的私有词库。插件词库配置请进入对应插件设置。"
            : $"当前方案：{CurrentScheme.Name}。通用功能由宿主管理；不支持的功能会禁用并保留原值，切回支持的方案后恢复。";
    }

    private void DependentSetting_Toggled(object sender, RoutedEventArgs e) => UpdateDependentSettings();

    private void CancelShortcutCapture()
    {
        if (_shortcutCaptureTarget?.Tag is ShortcutBinding binding) _shortcutCaptureTarget.Content = binding.Value;
        _shortcutCaptureTarget = null;
        StopShortcutCaptureHook();
    }
}
