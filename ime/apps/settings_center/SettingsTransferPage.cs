using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Storage.Pickers;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private bool _configurationTransferInProgress;
    internal bool ConfigurationTransferInProgress => _configurationTransferInProgress;

    private void InitializeTransferPicker(object picker) =>
        WinRT.Interop.InitializeWithWindow.Initialize(picker,
            WinRT.Interop.WindowNative.GetWindowHandle(((App)Application.Current).MainWindow));

    private void ShowTransferStatus(string message, InfoBarSeverity severity)
    {
        ConfigurationTransferStatus.Message = message;
        ConfigurationTransferStatus.Severity = severity;
        ConfigurationTransferStatus.IsOpen = true;
        ShowStatus(message, severity);
    }

    private async void ExportConfiguration_Click(object sender, RoutedEventArgs e)
    {
        if (_settingsBusy || !_settingsLoaded || _configurationTransferInProgress) return;
        _configurationTransferInProgress = true;
        SetBusy(true);
        try {
            var snapshot = await _client.ValidateAsync(CaptureSettings());
            var document = SettingsTransfer.Create(snapshot, AutoSaveSettings.IsOn);
            var picker = new FileSavePicker {
                SuggestedFileName = "OwO-设置-" + DateTime.Now.ToString("yyyyMMdd")
            };
            picker.FileTypeChoices.Add("OwO 设置文件", [".json"]);
            InitializeTransferPicker(picker);
            var file = await picker.PickSaveFileAsync();
            if (file is null) return;
            await SettingsTransfer.WriteAsync(file.Path, document);
            ShowTransferStatus($"配置已导出：{file.Path}（包含当前界面尚未保存的调整）", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowTransferStatus($"导出失败：{error.Message}", InfoBarSeverity.Error);
        } finally {
            _configurationTransferInProgress = false;
            SetBusy(false);
        }
    }

    private async void ImportConfiguration_Click(object sender, RoutedEventArgs e)
    {
        if (_settingsBusy || !_settingsLoaded || _configurationTransferInProgress) return;
        _configurationTransferInProgress = true;
        SetBusy(true);
        var gateHeld = false;
        var wasLoaded = _settingsLoaded;
        try {
            var picker = new FileOpenPicker();
            picker.FileTypeFilter.Add(".json");
            InitializeTransferPicker(picker);
            var file = await picker.PickSingleFileAsync();
            if (file is null) return;
            var document = await SettingsTransfer.ReadAsync(file.Path);
            var snapshot = await _client.ValidateAsync(document.Settings);
            var dialog = new ContentDialog {
                XamlRoot = XamlRoot, Title = "导入配置并立即应用？",
                Content = $"输入方案：{snapshot.InputScheme}\n每页候选：{snapshot.CandidatePageSize}\n" +
                    $"自动保存：{(document.UiPreferences.AutoSaveEnabled ? "开启" : "关闭")}\n" +
                    $"语料学习：{(snapshot.CorpusLearningEnabled ? "开启" : "关闭")}\n" +
                    $"语气云端模型：{(snapshot.ToneCloudModelEnabled ? "开启" : "关闭")}\n\n" +
                    "将覆盖输入、候选、学习与排序、快捷键、标点、语料学习、风格、语气和自动保存偏好，以及这些页面尚未保存的修改。\n" +
                    "原配置会先备份。不会导入用户词库、插件配置或权限；插件输入方案仍需在本机安装并授权。\n" +
                    "即使自动保存关闭，此次导入也会立即保存。",
                PrimaryButtonText = "导入并应用", CloseButtonText = "取消",
                DefaultButton = ContentDialogButton.Close
            };
            if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
            await _settingsSaveGate.WaitAsync();
            gateHeld = true;
            _autoSaveTimer.Stop();
            // 插件不属于导入范围；先保存其已排队修改，避免导入关闭自动保存后丢失。
            foreach (var entry in _pendingPluginSaves.ToArray()) {
                entry.Key.Save(entry.Value);
                _pendingPluginSaves.Remove(entry.Key);
                NotifyCandidateThemeChanged();
            }
            var backup = await SettingsTransfer.ApplyAsync(_client, UiPreferencesPath, document);
            _settingsLoaded = false;
            ApplySettings(snapshot);
            _loadingUiPreferences = true;
            try { AutoSaveSettings.IsOn = document.UiPreferences.AutoSaveEnabled; }
            finally { _loadingUiPreferences = false; }
            _pendingSettingsSave = false;
            ShowTransferStatus($"配置已导入并保存，Core Service 将自动应用。原配置备份：{backup}",
                               InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowTransferStatus($"导入失败：{error.Message}", InfoBarSeverity.Error);
        } finally {
            _settingsLoaded = wasLoaded;
            if (gateHeld) _settingsSaveGate.Release();
            _configurationTransferInProgress = false;
            SetBusy(false);
            if (AutoSaveSettings.IsOn && (_pendingSettingsSave || _pendingPluginSaves.Count != 0))
                _autoSaveTimer.Start();
        }
    }
}
