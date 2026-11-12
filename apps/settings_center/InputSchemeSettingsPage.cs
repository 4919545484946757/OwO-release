using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private readonly Dictionary<string, InputSchemeOption> _knownSchemes = InputSchemeSettingsPolicy.BuiltIns.ToDictionary(s => s.Id);
    private HashSet<string> _availableSchemeIds = InputSchemeSettingsPolicy.BuiltIns.Select(s => s.Id).ToHashSet(StringComparer.Ordinal);
    private List<string> _schemeOrder = ["full_pinyin", "english", "!exact_code"];
    private string _loadedPreferredScheme = "full_pinyin";
    private string PreferredInputScheme => _schemeOrder.First(InputSchemeSettingsPolicy.IsEnabled);
    private InputSchemeOption CurrentScheme {
        get {
            var options = _schemeOrder.Where(id => InputSchemeSettingsPolicy.IsEnabled(id) && _availableSchemeIds.Contains(id))
                .Select(id => _knownSchemes[id]).ToArray();
            var preferred = options.FirstOrDefault() ?? InputSchemeSettingsPolicy.Resolve("english");
            return preferred with { SettingsCapabilities = options.Aggregate(preferred.SettingsCapabilities,
                (capabilities, option) => capabilities | option.SettingsCapabilities) };
        }
    }

    private void SelectInputScheme(string id)
    {
        _loadedPreferredScheme = id;
        LoadInputSchemeOrder("");
    }

    private string CaptureInputSchemeOrder() {
        // Keep every enabled item; newly discovered disabled schemes beyond the
        // persisted list limit remain visible until the user chooses them.
        var free = InputSchemeSettingsPolicy.MaximumOrderedSchemes - _schemeOrder.Count(InputSchemeSettingsPolicy.IsEnabled);
        return string.Join(';', _schemeOrder.Where(entry => InputSchemeSettingsPolicy.IsEnabled(entry) || free-- > 0));
    }
    private void LoadInputSchemeOrder(string value) {
        _schemeOrder = InputSchemeSettingsPolicy.BuildOrder(value, _loadedPreferredScheme, _availableSchemeIds);
    }

    private async void DiscoverSchemes_Click(object sender, RoutedEventArgs e)
    {
        DiscoverSchemesButton.IsEnabled = false;
        try {
            var count = await RefreshInputSchemesAsync();
            ShowStatus($"发现 {count} 个已授权插件方案。打开对应开关后才会加入快捷键轮换。", InfoBarSeverity.Informational);
        } catch (Exception error) { ShowStatus(error.Message, InfoBarSeverity.Error); }
        finally { DiscoverSchemesButton.IsEnabled = true; }
    }

    private void InstallSchemes_Click(object sender, RoutedEventArgs e) {
        PluginTabs.SelectedItem = InstalledPluginsTab;
        SettingsNavigation.SelectedItem = PluginNavigationItem;
        ShowPage("plugins");
    }

    private async Task<int> RefreshInputSchemesAsync()
    {
        var discovered = await _pluginClient.DiscoverInputSchemesAsync();
        _availableSchemeIds = InputSchemeSettingsPolicy.BuiltIns.Select(s => s.Id).ToHashSet(StringComparer.Ordinal);
        foreach (var scheme in discovered) {
            _knownSchemes[scheme.Id] = InputSchemeSettingsPolicy.Plugin(scheme.Id, scheme.Name, $"{scheme.PluginId} · {scheme.Language}");
            _availableSchemeIds.Add(scheme.Id);
        }
        _schemeOrder = InputSchemeSettingsPolicy.BuildOrder(string.Join(';', _schemeOrder), PreferredInputScheme, _availableSchemeIds);
        UpdateInputSchemePresentation();
        RefreshConfigurationSchemes();
        return discovered.Select(s => s.Id).Distinct(StringComparer.Ordinal).Count();
    }

    private async Task DisablePluginInputSchemesAsync(string pluginId)
    {
        await _settingsSaveGate.WaitAsync();
        try {
            var saved = await _client.LoadAsync();
            var entries = InputSchemeSettingsPolicy.BuildOrder(saved.InputSchemeOrder, saved.InputScheme, []);
            if (entries.Any(entry => InputSchemeSettingsPolicy.IsEnabled(entry) && InputSchemeSettingsPolicy.NeedsFallback(entry, pluginId))) {
                var next = InputSchemeSettingsPolicy.DisablePlugin(entries, pluginId);
                var primary = next.First(InputSchemeSettingsPolicy.IsEnabled);
                var order = string.Join(';', next);
                await _client.SaveAsync(saved with { InputScheme = primary, InputSchemeOrder = order });
                if (_settingsBaseline is not null)
                    _settingsBaseline = _settingsBaseline with { InputScheme = primary, InputSchemeOrder = order };
            }
            _schemeOrder = [.. InputSchemeSettingsPolicy.DisablePlugin(_schemeOrder, pluginId)];
            UpdateInputSchemePresentation();
        } catch (Exception error) {
            throw new InvalidOperationException($"插件已停用，但更新输入方案失败：{error.Message}", error);
        } finally { _settingsSaveGate.Release(); }
    }

    private void UpdateInputSchemePresentation()
    {
        if (InputSchemeRows is null) return;
        RenderInputSchemeRows();
        UpdateDependentSettings();
        UpdateSchemeHintVisibility();
    }

    private void RenderInputSchemeRows() {
        InputSchemeRows.Children.Clear();
        var preferred = PreferredInputScheme;
        for (var index = 0; index < _schemeOrder.Count; index++) {
            var entry = _schemeOrder[index];
            var id = InputSchemeSettingsPolicy.SchemeId(entry);
            var enabled = InputSchemeSettingsPolicy.IsEnabled(entry);
            var available = _availableSchemeIds.Contains(id);
            var scheme = _knownSchemes.GetValueOrDefault(id) ?? InputSchemeSettingsPolicy.Resolve(id);
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var labels = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
            labels.Children.Add(new TextBlock { Text = scheme.Name, TextWrapping = TextWrapping.Wrap, Style = (Style)Resources["SettingTitleStyle"] });
            var detail = scheme.OwnerPluginId is null ? "内置方案" : "插件方案";
            if (!available) detail += " · 不可用（请在插件管理器核对启用与授权）";
            else if (id == preferred) detail += " · 默认";
            labels.Children.Add(new TextBlock { Text = $"{detail} · {scheme.Description}", Style = (Style)Resources["SettingDescriptionStyle"] });
            row.Children.Add(labels);
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
            var toggle = new ToggleSwitch { IsOn = enabled, OnContent = null, OffContent = null, IsEnabled = available || enabled, MinWidth = 0 };
            AutomationProperties.SetName(toggle, $"启用{scheme.Name}");
            AutomationProperties.SetAutomationId(toggle, "InputSchemeToggle:" + id);
            toggle.Toggled += (_, _) => SetSchemeEnabled(id, toggle);
            actions.Children.Add(toggle);
            foreach (var direction in new[] { -1, 1 }) {
                var move = new Button { Content = direction < 0 ? "↑" : "↓", IsEnabled = index + direction >= 0 && index + direction < _schemeOrder.Count };
                AutomationProperties.SetName(move, $"{(direction < 0 ? "上移" : "下移")}{scheme.Name}");
                AutomationProperties.SetAutomationId(move, $"InputSchemeMove{(direction < 0 ? "Up" : "Down")}:{id}");
                move.Click += (_, _) => {
                    var from = _schemeOrder.FindIndex(value => InputSchemeSettingsPolicy.SchemeId(value) == id);
                    var to = from + direction;
                    if (from < 0 || to < 0 || to >= _schemeOrder.Count) return;
                    (_schemeOrder[from], _schemeOrder[to]) = (_schemeOrder[to], _schemeOrder[from]);
                    OnSchemeOrderEdited();
                };
                actions.Children.Add(move);
            }
            Grid.SetColumn(actions, 1); row.Children.Add(actions);
            InputSchemeRows.Children.Add(new Border { Style = (Style)Resources["SettingsCardStyle"], Child = row });
        }
    }

    private void SetSchemeEnabled(string id, ToggleSwitch toggle) {
        var index = _schemeOrder.FindIndex(value => InputSchemeSettingsPolicy.SchemeId(value) == id);
        if (index < 0 || InputSchemeSettingsPolicy.IsEnabled(_schemeOrder[index]) == toggle.IsOn) return;
        var enabled = _schemeOrder.Count(InputSchemeSettingsPolicy.IsEnabled);
        if ((!toggle.IsOn && enabled <= 1) || (toggle.IsOn && (enabled >= InputSchemeSettingsPolicy.MaximumOrderedSchemes || !_availableSchemeIds.Contains(id)))) {
            toggle.IsOn = !toggle.IsOn;
            ShowStatus(enabled <= 1 ? "请至少保留一个启用的输入方案。" : "此方案不可用或已达到启用上限。", InfoBarSeverity.Warning);
            return;
        }
        _schemeOrder[index] = toggle.IsOn ? id : "!" + id;
        OnSchemeOrderEdited();
    }
    private void OnSchemeOrderEdited() {
        CancelShortcutCapture();
        UpdateInputSchemePresentation();
        QueueSettingsAutoSave();
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
