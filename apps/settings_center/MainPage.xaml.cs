using Microsoft.UI.Xaml.Controls;
using System.Runtime.InteropServices;
using System.Text.Json;

namespace OwO_Settings;

public sealed partial class MainPage : Page
{
    private readonly Microsoft.UI.Xaml.DispatcherTimer _statusExpiry = new() { Interval = TimeSpan.FromMilliseconds(250) };
    private readonly SettingsNoticeDeadline _statusDeadline = new();
    private readonly WindowDownloads _windowDownloads;
    private bool _showingTransientStatus;
    private long _statusVersion;
    private readonly ConfigShellClient _client = new();
    private readonly PluginShellClient _pluginClient = new();
    private Button? _shortcutCaptureTarget;
    private IntPtr _shortcutCaptureHook;
    private readonly LowLevelKeyboardProc _shortcutCaptureHookProc;
    private readonly List<SearchItem> _baseSearchItems = [];
    private readonly List<SearchItem> _pluginSearchItems = [];
    private readonly Dictionary<string, PluginVersionSnapshot> _pluginPages = [];
    private readonly Dictionary<string, Microsoft.UI.Xaml.FrameworkElement> _pluginConfigControls = [];
    private readonly Dictionary<string, Expander> _pluginConfigExpanders = [];
    private ComboBox? _pluginConfigPageSelector;
    private NavigationViewItem? _pluginPagesGroup;
    private NavigationViewItemSeparator? _pluginPagesSeparator;
    private PluginSettingsModel? _pluginSettings;
    private string _currentPage = "schemes";
    private sealed record SearchItem(string Title, string Detail, string Page,
                                     Microsoft.UI.Xaml.FrameworkElement? Target = null,
                                     string? PluginField = null,
                                     SettingsCapability Capability = SettingsCapability.None)
    {
        public override string ToString() => string.IsNullOrWhiteSpace(Detail)
            ? Title : $"{Title}  ·  {Detail}";
    }
    private sealed class ShortcutBinding(string group, string value)
    {
        internal string Group { get; } = group;
        internal string Value { get; set; } = value;
    }

    private readonly Dictionary<string, List<ShortcutBinding>> _shortcutBindings = new();

    [DllImport("user32.dll")]
    private static extern short GetKeyState(int virtualKey);

    [DllImport("user32.dll")]
    private static extern short GetAsyncKeyState(int virtualKey);

    private delegate IntPtr LowLevelKeyboardProc(int code, IntPtr message, IntPtr data);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr SetWindowsHookEx(int hook, LowLevelKeyboardProc callback,
                                                  IntPtr module, uint threadId);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool UnhookWindowsHookEx(IntPtr hook);

    [DllImport("user32.dll")]
    private static extern IntPtr CallNextHookEx(IntPtr hook, int code, IntPtr message, IntPtr data);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern uint RegisterWindowMessage(string message);

    [DllImport("user32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SendNotifyMessage(IntPtr window, uint message,
                                                  IntPtr wparam, IntPtr lparam);

    private static readonly uint CandidateThemeChangedMessage =
        RegisterWindowMessage("OwO.CandidateTheme.Refresh.v1");

    private static void NotifyCandidateThemeChanged()
    {
        const int HwndBroadcast = 0xffff;
        if (CandidateThemeChangedMessage != 0)
            SendNotifyMessage(new IntPtr(HwndBroadcast), CandidateThemeChangedMessage,
                              IntPtr.Zero, IntPtr.Zero);
    }

    public MainPage()
    {
        _shortcutCaptureHookProc = ShortcutCaptureHook;
        InitializeComponent();
        InformationCardStyle.Apply(Status);
        _windowDownloads = new(WindowDownloadHost, ShowStatus);
        _statusExpiry.Tick += (_, _) => { if (_statusDeadline.Expired) ShowPageStatus(); };
        Status.Closed += (_, _) => {
            _statusExpiry.Stop(); var version = _statusVersion;
            DispatcherQueue.TryEnqueue(() => { if (version == _statusVersion) ShowPageStatus(); });
        };
        InputScheme.ItemsSource = _availableSchemes;
        InputScheme.SelectedIndex = 0;
        SetShortcutBindings("correction", ["Ctrl+Q"]);
        SetShortcutBindings("language", ["Ctrl+Space"]);
        SetShortcutBindings("raw", ["Enter"]);
        SetShortcutBindings("separator", ["Quote"]);
        SetShortcutBindings("cursor-left", ["Left"]);
        SetShortcutBindings("cursor-right", ["Right"]);
        SetShortcutBindings("previous-page", ["["]);
        SetShortcutBindings("next-page", ["]"]);
        SetShortcutBindings("expanded-previous-page", ["Up"]);
        SetShortcutBindings("expanded-next-page", ["Down"]);
        SetShortcutBindings("text-navigation", ["Shift+Left", "Shift+Right", "Shift+Up", "Shift+Down"]);
        SetShortcutBindings("punctuation", []);
        UpdateShortcutButtons();
        InitializePunctuationSettings();
        InitializeSettingsModules();
        UserLexiconNavigationItem.Content = "词库管理器";
        UserLexiconPageHost.Content = new DictionaryManager(ShowStatus, _windowDownloads.Start);
        PluginDiscoveryHost.Content = new PluginDiscoveryView(InstallPluginSourceAsync, () => PluginTabs.SelectedItem = InstalledPluginsTab, ShowStatus, _windowDownloads.Start);
        OnlineDistributionHost.Content = new OnlineDistributionView(
            async (name, path) => {
                if (UserLexiconPageHost.Content is DictionaryManager manager) {
                    await manager.ImportDownloadedAsync(name, path, XamlRoot);
                    SettingsNavigation.SelectedItem = UserLexiconNavigationItem;
                    ShowPage("user-lexicon");
                }
            }, ShowStatus, _windowDownloads.Start);
        InitializeAutoSave();
        InitializeLogoSettings();
        InitializeSearchIndex();
        ShowPage("schemes");
        ConfigPath.Text = _client.ConfigPath;
        PluginPath.Text = _pluginClient.StorePath;
        Loaded += async (_, _) => {
            await LoadConfigAsync();
            await LoadPluginsAsync();
        };
        Unloaded += (_, _) => { StopShortcutCaptureHook(); _statusExpiry.Stop(); _windowDownloads.Close(); };
    }

    private async Task LoadConfigAsync()
    {
        await _settingsSaveGate.WaitAsync();
        _settingsLoaded = false;
        _pendingSettingsSave = false;
        SetBusy(true);
        try {
            var value = await _client.LoadAsync();
            ApplySettings(value);
            _settingsLoaded = true;
            ShowStatus("配置已加载", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
            _settingsSaveGate.Release();
        }
    }

    private async void SaveButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
        => await SaveSettingsManuallyAsync();

    private async void ReloadButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e) =>
        await LoadConfigAsync();

    private async void ResetPageButton_Click(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        SetBusy(true);
        try {
            if (_currentPage == "general") {
                AutoSaveSettings.IsOn = true;
                _logoThemeSelector.SelectedIndex = 0;
                return;
            }
            if (_currentPage == "plugins") {
                SetBusy(false);
                if (!await ConfirmAsync("重置插件配置",
                    "将所有已安装插件的声明式配置恢复为各自 schema 中的默认值。"
                        + "安装、授权和启停状态不会改变。", "恢复默认")) return;
                SetBusy(true);
                var resetCount = 0;
                foreach (var group in _pluginPages.Values.GroupBy(plugin => plugin.Id,
                             StringComparer.Ordinal)) {
                    var plugin = group.OrderByDescending(value => value.Active)
                                      .ThenByDescending(value => value.Version,
                                                        StringComparer.Ordinal).First();
                    try {
                        var model = PluginSettingsModel.Load(_pluginClient.StorePath, plugin);
                        var pluginDefaults = model.DefaultValues().ToDictionary(
                            value => value.Key,
                            value => (object?)JsonSerializer.Deserialize<object>(
                                value.Value.GetRawText()), StringComparer.Ordinal);
                        model.Save(pluginDefaults);
                        ++resetCount;
                    } catch (InvalidOperationException) {
                        // Plugins without a supported declarative schema have no resettable page.
                    }
                }
                NotifyCandidateThemeChanged();
                ShowStatus($"已恢复 {resetCount} 个插件的默认配置。", InfoBarSeverity.Success);
                return;
            }
            var defaults = await _client.LoadDefaultsAsync();
            ApplyPageDefaults(defaults);
            ShowStatus(AutoSaveSettings.IsOn ? "本页已恢复默认值，正在自动保存。" : "本页已恢复默认值；点击“保存”后应用。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus($"恢复默认值失败：{error.Message}", InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
            QueueSettingsAutoSave();
        }
    }

    private void UserLearningSensitivity_ValueChanged(
        object sender,
        Microsoft.UI.Xaml.Controls.Primitives.RangeBaseValueChangedEventArgs e)
    {
        if (UserLearningSensitivityValue is not null)
            UserLearningSensitivityValue.Text = ((int)Math.Round(e.NewValue)).ToString();
    }

    private async void SettingsNavigation_SelectionChanged(
        NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (args.SelectedItemContainer?.Tag is not string page) return;
        if (page.StartsWith("plugin:", StringComparison.Ordinal)) {
            if (_pluginPages.TryGetValue(page, out var plugin))
                await ShowPluginSettingsAsync(plugin);
            return;
        }
        ShowPage(page);
    }

    private void ShowPage(string page)
    {
        _currentPage = page;
        var generalPage = page is "schemes" or "candidates" or "intelligence";
        GeneralSettingsPage.Visibility = generalPage
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        UserLexiconPageHost.Visibility = page == "user-lexicon"
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        if (page == "user-lexicon" && UserLexiconPageHost.Content is DictionaryManager manager)
            _ = manager.RefreshAsync();
        ShortcutSettingsPage.Visibility = page == "shortcuts"
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        PluginSettingsPage.Visibility = page == "plugins"
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        PluginConfigPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
        GeneralPreferencesPage.Visibility = page == "general"
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        ChinesePunctuationPage.Visibility = page == "punctuation"
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        if (page == "punctuation") {
            ChinesePunctuationPage.ChangeView(null, 0, null, true);
            UpdatePunctuationNotice();
        }
        SetVisibility(page == "schemes", InputSchemeSectionTitle, InputSchemeCard,
                      DiscoverSchemesButton);
        SetVisibility(page == "candidates", CandidateSectionTitle,
                      CandidatePageSizeCard, CandidateWrapLengthCard,
                      CandidatePartitionCard, CandidateSingleCharacterFirstCard, CandidateImmediateCommitCard,
                      FocusLossCommitCard, ChinesePunctuationCard, PinyinSymbolCompatibilityCard,
                      AltCodeSectionTitle, AltCodePreviewCard,
                      AltCodePreviewPageSizeCard, AltCodePreviewWheelCard);
        SetVisibility(page == "intelligence", LearningSectionTitle,
                      UserLearningCard, LearningSensitivityCard,
                      UserCompletionCard, UserCompletionRangeCard,
                      ModelRankingCard, ModelTimeoutCard);
        if (generalPage) GeneralSettingsPage.ChangeView(null, 0, null, true);
        var configurationPage = generalPage || page is "shortcuts" or "punctuation";
        SaveButton.Visibility = configurationPage
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        ReloadButton.Visibility = configurationPage
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        ResetPageButton.Visibility = page is "schemes" or "candidates" or
            "intelligence" or "shortcuts" or "plugins" or "punctuation" or "general"
            ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
        (CurrentPageTitle.Text, CurrentPageDescription.Text) = page switch {
            "general" => ("通用设置", "管理自动保存、配置备份、更新与资源下载"),
            "schemes" => ("输入方案", "选择编码方案并查看当前方案能力"),
            "candidates" => ("候选与输入", "调整候选显示、组合上屏、标点和符号预览"),
            "punctuation" => ("中文标点", "逐项选择中文格式；总开关和中英文模式仍决定是否启用"),
            "intelligence" => ("学习与排序", "管理本地词频学习、上下文排序和模型预算"),
            "user-lexicon" => ("词库管理器", "搜索、导入和管理字典，或进入用户词库管理学习数据"),
            "shortcuts" => ("快捷键", "管理输入、翻页和模式切换的多组按键绑定"),
            "plugins" => ("插件管理器", "管理已安装插件、浏览官方目录并独立审核权限"),
            _ => ("输入方案", "选择编码方案并查看当前方案能力"),
        };
        UpdateSchemeHintVisibility();
        if (!_showingTransientStatus) ShowPageStatus();
    }

    private static void SetVisibility(
        bool visible, params Microsoft.UI.Xaml.FrameworkElement[] elements)
    {
        var value = visible ? Microsoft.UI.Xaml.Visibility.Visible
                            : Microsoft.UI.Xaml.Visibility.Collapsed;
        foreach (var element in elements) element.Visibility = value;
    }

    private void InitializeSearchIndex()
    {
        void Add(string title, string detail, string page,
                 Microsoft.UI.Xaml.FrameworkElement target) =>
            _baseSearchItems.Add(new SearchItem(title, detail, page, target));
        Add("自动保存设置", "通用 自动 保存 配置", "general", AutoSaveCard);
        Add("Logo 外观", "通用 图标 品牌 logo 深色 浅色 跟随系统", "general", _logoThemeCard!);
        Add("配置导入与导出", "通用 备份 恢复 迁移 JSON import export", "general", ConfigurationTransferCard);
        Add("更新与资源下载", "GitHub 版本 更新 下载 插件 词库 字典 模型 网络", "general", OnlineDistributionHost);
        foreach (var binding in _settingsCards)
            _baseSearchItems.Add(new SearchItem(binding.Title, binding.Keywords,
                binding.SearchPage ?? _settingsModules.PageFor(binding.Key), binding.Card,
                Capability: _settingsModules.Field(binding.Key).Capability));
        foreach (var option in PunctuationOptions)
            _baseSearchItems.Add(new SearchItem($"中文标点 · {option.Name}",
                $"标点 {option.Source} {option.Output}", "punctuation", _punctuationCards[option.Source],
                Capability: SettingsCapability.ChinesePunctuation));
        Add("词库管理器", "用户词库 字典 搜索 编辑 新增 删除 清除 导入 导出 Gboard Rime CSV TSV 词频", "user-lexicon", UserLexiconPageHost);
        Add("插件管理器", "下载 在线 发现 搜索 插件 更新 安装 权限 卸载 恢复", "plugins", PluginSection);
    }

    private void SettingsSearchBox_TextChanged(AutoSuggestBox sender,
                                                AutoSuggestBoxTextChangedEventArgs args)
    {
        if (args.Reason != AutoSuggestionBoxTextChangeReason.UserInput) return;
        var query = sender.Text.Trim();
        if (query.Length == 0) { sender.ItemsSource = null; return; }
        sender.ItemsSource = SettingsSearchItems()
            .Where(item => ($"{item.Title} {item.Detail}").Contains(
                query, StringComparison.OrdinalIgnoreCase)).Take(12).ToArray();
    }

    private void SettingsSearchBox_SuggestionChosen(
        AutoSuggestBox sender, AutoSuggestBoxSuggestionChosenEventArgs args)
    {
        if (args.SelectedItem is SearchItem item) sender.Text = item.Title;
    }

    private async void SettingsSearchBox_QuerySubmitted(
        AutoSuggestBox sender, AutoSuggestBoxQuerySubmittedEventArgs args)
    {
        var selected = args.ChosenSuggestion as SearchItem;
        selected ??= SettingsSearchItems().FirstOrDefault(item =>
            ($"{item.Title} {item.Detail}").Contains(args.QueryText.Trim(),
                StringComparison.OrdinalIgnoreCase));
        if (selected is null) {
            ShowStatus("未找到匹配的设置。", InfoBarSeverity.Warning);
            return;
        }
        await NavigateToSearchItemAsync(selected);
    }

    private async Task NavigateToSearchItemAsync(SearchItem item)
    {
        if (item.Page.StartsWith("plugin:", StringComparison.Ordinal) &&
            _pluginPages.TryGetValue(item.Page, out var plugin)) {
            SelectPluginNavigation(item.Page);
            await ShowPluginSettingsAsync(plugin, item.PluginField);
            return;
        }
        SettingsNavigation.SelectedItem = item.Page switch {
            "general" => GeneralNavigationItem,
            "schemes" => SchemeNavigationItem,
            "candidates" => CandidateNavigationItem,
            "punctuation" => CandidateNavigationItem,
            "intelligence" => IntelligenceNavigationItem,
            "user-lexicon" => UserLexiconNavigationItem,
            "shortcuts" => ShortcutNavigationItem,
            "plugins" => PluginNavigationItem,
            _ => SchemeNavigationItem,
        };
        ShowPage(item.Page);
        if (!CurrentScheme.Supports(item.Capability))
            ShowStatus("当前输入方案不支持此功能，原配置已保留；切换受支持的方案后可编辑。", InfoBarSeverity.Informational);
        if (item.Target is not null) DispatcherQueue.TryEnqueue(() => {
            item.Target.StartBringIntoView();
            item.Target.Focus(Microsoft.UI.Xaml.FocusState.Programmatic);
        });
    }

    private void SetBusy(bool busy)
    {
        _settingsBusy = busy;
        SaveButton.IsEnabled = !busy;
        ReloadButton.IsEnabled = !busy;
        ResetPageButton.IsEnabled = !busy;
        PluginConfigResetButton.IsEnabled = !busy && _pluginSettings?.Fields.Count != 0;
        ShortcutSection.IsEnabled = !busy;
        PluginSection.IsEnabled = !busy;
        GeneralPreferencesPage.IsEnabled = !busy;
        GeneralSettingsPage.IsEnabled = !busy;
        ChinesePunctuationPage.IsEnabled = !busy;
        SettingsNavigation.IsEnabled = !busy;
    }

    private void ShowStatus(string message, InfoBarSeverity severity)
    {
        _showingTransientStatus = true; ++_statusVersion;
        InformationCardStyle.Apply(Status, severity == InfoBarSeverity.Informational);
        Status.Title = "";
        Status.Message = message;
        Status.Severity = severity;
        Status.IconSource = null;
        Status.IsClosable = true;
        ToolTipService.SetToolTip(Status, message);
        _statusDeadline.Show(severity switch {
            InfoBarSeverity.Success => SettingsNoticeKind.Success, InfoBarSeverity.Warning => SettingsNoticeKind.Warning,
            InfoBarSeverity.Error => SettingsNoticeKind.Error, _ => SettingsNoticeKind.Information
        });
        _statusExpiry.Start();
        Status.IsOpen = true;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(Status, message);
    }
    private void ShowPageStatus() {
        _statusExpiry.Stop(); _showingTransientStatus = false; ++_statusVersion;
        InformationCardStyle.Apply(Status);
        var message = string.IsNullOrWhiteSpace(CurrentPageDescription.Text) ? CurrentPageTitle.Text :
            CurrentPageTitle.Text + "：" + CurrentPageDescription.Text;
        Status.Title = ""; Status.Message = message; Status.Severity = InfoBarSeverity.Informational;
        Status.IconSource = new FontIconSource { Glyph = "ⓘ", FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Segoe UI Symbol") };
        Status.IsClosable = false; Status.IsOpen = true;
        ToolTipService.SetToolTip(Status, message);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(Status, "ⓘ " + message);
    }
}
