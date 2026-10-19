using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private readonly DispatcherTimer _autoSaveTimer = new() { Interval = TimeSpan.FromMilliseconds(450) };
    private readonly SemaphoreSlim _settingsSaveGate = new(1, 1);
    private readonly Dictionary<PluginSettingsModel, Dictionary<string, object?>> _pendingPluginSaves = [];
    private bool _settingsLoaded, _settingsBusy, _pendingSettingsSave, _loadingUiPreferences, _buildingPluginSettings;
    private string UiPreferencesPath => Path.Combine(Path.GetDirectoryName(_client.ConfigPath)!, "settings-ui.json");

    private void InitializeAutoSave()
    {
        _loadingUiPreferences = true;
        try { AutoSaveSettings.IsOn = SettingsUiPreferences.Load(UiPreferencesPath).AutoSaveEnabled; }
        catch (Exception error) {
            AutoSaveSettings.IsOn = false; // Never overwrite configuration after a preference read failure.
            ShowStatus($"自动保存偏好读取失败：{error.Message}", InfoBarSeverity.Error);
        } finally { _loadingUiPreferences = false; }
        _autoSaveTimer.Tick += async (_, _) => {
            if (_settingsBusy) return;
            _autoSaveTimer.Stop();
            await FlushAutoSaveAsync();
        };
        Control[] settings = [InputScheme, CandidatePageSize, CandidateWrapLength,
            CandidatePartition, CandidateImmediateCommit, FocusLossCommit, ChinesePunctuation,
            AltCodePreview, AltCodePreviewPageSize, AltCodePreviewWheel, UserLearning,
            UserLearningSensitivity, UserCompletion, UserCompletionRange, ModelRanking, ModelTimeout,
            CorrectionShortcutEnabled, LanguageShortcutEnabled, RawInputShortcutEnabled,
            CursorLeftShortcutEnabled, CursorRightShortcutEnabled, PreviousPageShortcutEnabled,
            NextPageShortcutEnabled, PunctuationShortcutEnabled, TextNavigationShortcutEnabled,
            TextNavigationSelectEnabled];
        foreach (var control in settings) ObserveAutoSave(control);
        foreach (var control in _punctuationSwitches.Values) ObserveAutoSave(control);
        Unloaded += (_, _) => _autoSaveTimer.Stop();
    }

    private void ObserveAutoSave(FrameworkElement control, bool plugin = false)
    {
        object? Value() => control switch {
            ToggleSwitch toggle => toggle.IsOn,
            NumberBox number => number.Value,
            Slider slider => slider.Value,
            ComboBox combo => combo.SelectedItem,
            TextBox text => text.Text,
            ColorPicker picker => picker.Color,
            _ => null
        };
        var previous = Value();
        void Changed() {
            var current = Value();
            if (Equals(previous, current)) return; // Template initialization is not a user edit.
            previous = current;
            if (plugin) QueuePluginAutoSave(); else QueueSettingsAutoSave();
        }
        switch (control) {
            case ToggleSwitch toggle: toggle.Toggled += (_, _) => Changed(); break;
            case NumberBox number: number.ValueChanged += (_, _) => Changed(); break;
            case Slider slider: slider.ValueChanged += (_, _) => Changed(); break;
            case ComboBox combo: combo.SelectionChanged += (_, _) => Changed(); break;
            case TextBox text: text.TextChanged += (_, _) => Changed(); break;
            case ColorPicker picker: picker.ColorChanged += (_, _) => Changed(); break;
        }
    }

    private void GeneralSettings_Click(object sender, RoutedEventArgs e)
    {
        SettingsNavigation.SelectedItem = GeneralNavigationItem;
        ShowPage("general");
    }

    private void AutoSaveSettings_Toggled(object sender, RoutedEventArgs e)
    {
        if (_loadingUiPreferences) return;
        try { new SettingsUiPreferences(AutoSaveSettings.IsOn).Save(UiPreferencesPath); }
        catch (Exception error) {
            _loadingUiPreferences = true;
            AutoSaveSettings.IsOn = !AutoSaveSettings.IsOn;
            _loadingUiPreferences = false;
            ShowStatus($"自动保存开关保存失败：{error.Message}", InfoBarSeverity.Error);
            return;
        }
        if (!AutoSaveSettings.IsOn) {
            _autoSaveTimer.Stop();
            _pendingSettingsSave = false;
            _pendingPluginSaves.Clear();
        } else {
            QueueSettingsAutoSave();
            QueuePluginAutoSave();
        }
        ShowStatus(AutoSaveSettings.IsOn ? "自动保存已开启。" : "自动保存已关闭，请手动保存配置。", InfoBarSeverity.Success);
    }

    private void QueueSettingsAutoSave()
    {
        if (!_settingsLoaded || !AutoSaveSettings.IsOn) return;
        _pendingSettingsSave = true;
        _autoSaveTimer.Stop();
        _autoSaveTimer.Start();
    }

    private void QueuePluginAutoSave()
    {
        if (!_settingsLoaded || !AutoSaveSettings.IsOn || _pluginSettings is null || _buildingPluginSettings) return;
        try { _pendingPluginSaves[_pluginSettings] = CapturePluginValues(); }
        catch (Exception error) {
            _pendingPluginSaves.Remove(_pluginSettings);
            ShowStatus($"插件配置尚未保存：{error.Message}", InfoBarSeverity.Warning);
            return;
        }
        _autoSaveTimer.Stop();
        _autoSaveTimer.Start();
    }

    internal async Task<bool> FlushAutoSaveAsync()
    {
        await _settingsSaveGate.WaitAsync();
        var success = true;
        try {
            if (!AutoSaveSettings.IsOn) return true;
            if (_pendingSettingsSave) {
                _pendingSettingsSave = false;
                try {
                    await _client.SaveAsync(CaptureSettings());
                    ShowStatus("配置已自动保存。", InfoBarSeverity.Success);
                } catch (Exception error) {
                    _pendingSettingsSave = true;
                    success = false;
                    ShowStatus($"自动保存失败：{error.Message}；请修正配置或手动保存。", InfoBarSeverity.Error);
                }
            }
            foreach (var entry in _pendingPluginSaves.ToArray()) {
                try {
                    entry.Key.Save(entry.Value);
                    _pendingPluginSaves.Remove(entry.Key);
                    NotifyCandidateThemeChanged();
                    if (success) ShowStatus($"{entry.Key.Plugin.Name} 的设置已自动保存。", InfoBarSeverity.Success);
                } catch (Exception error) {
                    success = false;
                    ShowStatus($"插件自动保存失败：{error.Message}", InfoBarSeverity.Error);
                }
            }
            return success;
        } finally { _settingsSaveGate.Release(); }
    }

    private async Task SaveSettingsManuallyAsync()
    {
        SetBusy(true);
        await _settingsSaveGate.WaitAsync();
        try {
            _pendingSettingsSave = false;
            await _client.SaveAsync(CaptureSettings());
            ShowStatus("配置已保存，Core Service 将自动应用。", InfoBarSeverity.Success);
        } catch (Exception error) {
            _pendingSettingsSave = AutoSaveSettings.IsOn;
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            _settingsSaveGate.Release();
            SetBusy(false);
        }
    }
}
