using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System.Runtime.InteropServices;
using System.Text.Json;
using Windows.System;
using Windows.Storage.Pickers;

namespace OwO_Settings;

public sealed partial class MainPage : Page
{
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
    private NavigationViewItemHeader? _pluginPagesHeader;
    private PluginSettingsModel? _pluginSettings;
    private string _currentPage = "schemes";
    private sealed record InputSchemeOption(string Id, string Name, string Description,
                                            string Capabilities);
    private static readonly InputSchemeOption[] InputSchemes = [
        new("full_pinyin", "全拼", "使用汉语拼音连续输入，适合日常中文输入。",
            "支持自动分割、不完整拼音、简拼、混拼与可切换纠错。"),
        new("exact_code", "精确编码（实验性）", "按显式撇号分段并精确查询词典，用于验证表码方案接入。",
            "不启用拼音纠错、自动补全或缩写展开；当前仍使用拼音词典数据。"),
    ];
    private readonly List<InputSchemeOption> _availableSchemes = [.. InputSchemes];
    private sealed record SearchItem(string Title, string Detail, string Page,
                                     Microsoft.UI.Xaml.FrameworkElement? Target = null,
                                     string? PluginField = null)
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
        InputScheme.ItemsSource = _availableSchemes;
        InputScheme.SelectedIndex = 0;
        SetShortcutBindings("correction", ["Ctrl+Q"]);
        SetShortcutBindings("language", ["Ctrl+Space"]);
        SetShortcutBindings("raw", ["Enter"]);
        SetShortcutBindings("cursor-left", ["Left"]);
        SetShortcutBindings("cursor-right", ["Right"]);
        SetShortcutBindings("previous-page", ["[", "Up"]);
        SetShortcutBindings("next-page", ["]", "Down"]);
        SetShortcutBindings("text-navigation", ["Shift+Left", "Shift+Right", "Shift+Up", "Shift+Down"]);
        SetShortcutBindings("punctuation", []);
        UpdateShortcutButtons();
        InitializePunctuationSettings();
        InitializeAutoSave();
        InitializeSearchIndex();
        ShowPage("schemes");
        ConfigPath.Text = _client.ConfigPath;
        PluginPath.Text = _pluginClient.StorePath;
        Loaded += async (_, _) => {
            await LoadConfigAsync();
            await LoadPluginsAsync();
        };
        Unloaded += (_, _) => StopShortcutCaptureHook();
    }

    private async Task LoadConfigAsync()
    {
        await _settingsSaveGate.WaitAsync();
        _settingsLoaded = false;
        _pendingSettingsSave = false;
        SetBusy(true);
        try {
            var value = await _client.LoadAsync();
            ApplySchemeSettings(value);
            ApplyCandidateSettings(value);
            ApplyIntelligenceSettings(value);
            ApplyShortcutSettings(value);
            _settingsLoaded = true;
            ShowStatus("配置已加载", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
            _settingsSaveGate.Release();
        }
    }

    private void ApplySchemeSettings(SettingsSnapshot value)
    {
        if (value.InputScheme.StartsWith("plugin:", StringComparison.Ordinal) &&
            !_availableSchemes.Any(scheme => scheme.Id == value.InputScheme)) {
            _availableSchemes.Add(new(value.InputScheme, value.InputScheme[7..],
                "已保存的插件方案；点击发现按钮核对当前授权和可用性。",
                "实验性插件输入；不可用时保留编码，不自动回退拼音。"));
            InputScheme.ItemsSource = _availableSchemes.ToArray();
        }
        InputScheme.SelectedItem = _availableSchemes.FirstOrDefault(
            scheme => scheme.Id == value.InputScheme) ?? _availableSchemes[0];
        UpdateInputSchemePresentation();
    }

    private async void DiscoverSchemes_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        DiscoverSchemesButton.IsEnabled = false;
        try {
            var selected = (InputScheme.SelectedItem as InputSchemeOption)?.Id;
            var discovered = await _pluginClient.DiscoverInputSchemesAsync();
            var previous = _availableSchemes.FirstOrDefault(s => s.Id == selected);
            _availableSchemes.Clear(); _availableSchemes.AddRange(InputSchemes);
            _availableSchemes.AddRange(discovered.Select(s => new InputSchemeOption(s.Id,
                s.Name + "（插件·实验性）", $"{s.PluginId} · {s.Language}",
                "独立编码、候选选择和分页；仅发送本次编码，无周边文本。拼音智能功能不参与。")));
            if (previous is not null && !_availableSchemes.Any(s => s.Id == selected))
                _availableSchemes.Add(previous with { Description = "方案当前不可用或未获授权；请检查插件页面。" });
            InputScheme.ItemsSource = _availableSchemes.ToArray();
            InputScheme.SelectedItem = _availableSchemes.FirstOrDefault(s => s.Id == selected) ?? _availableSchemes[0];
            ShowStatus($"发现 {discovered.Count} 个已授权方案；选择后点击保存。", InfoBarSeverity.Informational);
        } catch (Exception error) { ShowStatus(error.Message, InfoBarSeverity.Error); }
        finally { DiscoverSchemesButton.IsEnabled = true; }
    }

    private void ApplyCandidateSettings(SettingsSnapshot value)
    {
        CandidatePageSize.Value = value.CandidatePageSize;
        CandidateWrapLength.Value = value.CandidateWrapLength;
        CandidatePartition.IsOn = value.CandidatePartitionEnabled;
        CandidateImmediateCommit.IsOn = value.CandidateImmediateCommitEnabled;
        FocusLossCommit.IsOn = value.FocusLossCommitEnabled;
        ChinesePunctuation.IsOn = value.ChinesePunctuationEnabled;
        ApplyPunctuationMask(value.ChinesePunctuationMask);
        AltCodePreview.IsOn = value.AltCodePreviewEnabled;
        AltCodePreviewPageSize.Value = value.AltCodePreviewPageSize;
        AltCodePreviewWheel.IsOn = value.AltCodePreviewWheelEnabled;
        UpdateDependentSettings();
    }

    private void ApplyIntelligenceSettings(SettingsSnapshot value)
    {
        UserLearningSensitivity.Value = value.UserLearningSensitivity;
        UserLearning.IsOn = value.UserLearningEnabled;
        UserCompletion.IsOn = value.UserCompletionEnabled;
        UserCompletionRange.Value = value.UserCompletionMaxSyllables;
        ModelRanking.IsOn = value.ModelRankingEnabled;
        ModelTimeout.Value = value.ModelTimeoutMs;
        UpdateDependentSettings();
    }

    private void InputScheme_SelectionChanged(
        object sender, Microsoft.UI.Xaml.Controls.SelectionChangedEventArgs e)
    {
        UpdateInputSchemePresentation();
    }

    private void UpdateInputSchemePresentation()
    {
        if (InputScheme.SelectedItem is not InputSchemeOption scheme ||
            InputSchemeDescription is null || InputSchemeCapabilities is null) return;
        InputSchemeDescription.Text = scheme.Description;
        InputSchemeCapabilities.Message = scheme.Capabilities;
        if (CorrectionShortcutCard is null) return;
        var enabled = scheme.Id == "full_pinyin";
        CorrectionShortcutCard.IsHitTestVisible = enabled;
        CorrectionShortcutCard.Opacity = enabled ? 1.0 : 0.55;
        UpdateDependentSettings();
    }

    private void DependentSetting_Toggled(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e) =>
        UpdateDependentSettings();

    private void UpdateDependentSettings()
    {
        if (LearningSensitivityCard is null || UserCompletionCard is null ||
            UserCompletionRangeCard is null ||
            ModelTimeoutCard is null ||
            AltCodePreviewPageSizeCard is null || AltCodePreviewWheelCard is null) return;
        var pluginScheme = (InputScheme.SelectedItem as InputSchemeOption)?.Id
            .StartsWith("plugin:", StringComparison.Ordinal) == true;
        SetCardEnabled(UserLearningCard, !pluginScheme);
        SetCardEnabled(LearningSensitivityCard, !pluginScheme && UserLearning.IsOn);
        SetCardEnabled(ModelRankingCard, !pluginScheme);
        SetCardEnabled(CandidatePartitionCard, !pluginScheme);
        SetCardEnabled(CandidateImmediateCommitCard, !pluginScheme);
        SetCardEnabled(FocusLossCommitCard, !pluginScheme);
        SetCardEnabled(TextNavigationShortcutCard, !pluginScheme);
        SetCardEnabled(ChinesePunctuationCard, !pluginScheme);
        foreach (var toggle in _punctuationSwitches.Values) toggle.IsEnabled = !pluginScheme;
        SetCardEnabled(PunctuationShortcutCard, !pluginScheme);
        var pinyinScheme = (InputScheme.SelectedItem as InputSchemeOption)?.Id ==
            "full_pinyin";
        SetCardEnabled(UserCompletionCard, pinyinScheme);
        SetCardEnabled(UserCompletionRangeCard, pinyinScheme && UserCompletion.IsOn);
        SetCardEnabled(ModelTimeoutCard, !pluginScheme && ModelRanking.IsOn);
        SetCardEnabled(AltCodePreviewPageSizeCard, AltCodePreview.IsOn);
        SetCardEnabled(AltCodePreviewWheelCard, AltCodePreview.IsOn);
    }

    private static void SetCardEnabled(Microsoft.UI.Xaml.FrameworkElement card, bool enabled)
    {
        card.IsHitTestVisible = enabled;
        card.Opacity = enabled ? 1.0 : 0.55;
    }

    private void ApplyShortcutSettings(SettingsSnapshot value)
    {
        CorrectionShortcutEnabled.IsOn = value.CorrectionShortcutEnabled;
        LanguageShortcutEnabled.IsOn = value.LanguageShortcutEnabled;
        RawInputShortcutEnabled.IsOn = value.RawInputShortcutEnabled;
        CursorLeftShortcutEnabled.IsOn = value.CursorLeftShortcutEnabled;
        CursorRightShortcutEnabled.IsOn = value.CursorRightShortcutEnabled;
        PreviousPageShortcutEnabled.IsOn = value.PreviousPageShortcutEnabled;
        NextPageShortcutEnabled.IsOn = value.NextPageShortcutEnabled;
        PunctuationShortcutEnabled.IsOn = value.PunctuationShortcutEnabled;
        TextNavigationShortcutEnabled.IsOn = value.TextNavigationShortcutEnabled;
        TextNavigationSelectEnabled.IsOn = value.TextNavigationSelectEnabled;
        SetShortcutBindings("correction", value.CorrectionShortcuts);
        SetShortcutBindings("language", value.LanguageShortcuts);
        SetShortcutBindings("raw", value.RawInputShortcuts);
        SetShortcutBindings("cursor-left", value.CursorLeftShortcuts);
        SetShortcutBindings("cursor-right", value.CursorRightShortcuts);
        SetShortcutBindings("previous-page", value.PreviousPageShortcuts);
        SetShortcutBindings("next-page", value.NextPageShortcuts);
        SetShortcutBindings("punctuation", value.PunctuationShortcuts);
        SetShortcutBindings("text-navigation", value.TextNavigationShortcuts);
        UpdateShortcutButtons();
    }

    private async void SaveButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
        => await SaveSettingsManuallyAsync();

    private SettingsSnapshot CaptureSettings()
    {
            NumberBox[] numbers = [CandidatePageSize, CandidateWrapLength,
                AltCodePreviewPageSize, UserCompletionRange, ModelTimeout];
            if (numbers.Any(number => !double.IsFinite(number.Value)))
                throw new InvalidOperationException("数值设置尚未填写完整，请输入有效数字后保存。");
            ValidateShortcutConflicts();
            return new SettingsSnapshot(
                (InputScheme.SelectedItem as InputSchemeOption)?.Id ?? "full_pinyin",
                (uint)CandidatePageSize.Value,
                (uint)CandidateWrapLength.Value,
                CandidatePartition.IsOn,
                CandidateImmediateCommit.IsOn,
                FocusLossCommit.IsOn,
                ChinesePunctuation.IsOn,
                AltCodePreview.IsOn,
                (uint)AltCodePreviewPageSize.Value,
                AltCodePreviewWheel.IsOn,
                (uint)UserLearningSensitivity.Value,
                UserLearning.IsOn, UserCompletion.IsOn,
                (uint)UserCompletionRange.Value,
                ModelRanking.IsOn, (uint)ModelTimeout.Value,
                CorrectionShortcutEnabled.IsOn, ShortcutValues("correction"),
                LanguageShortcutEnabled.IsOn, ShortcutValues("language"),
                RawInputShortcutEnabled.IsOn, ShortcutValues("raw"),
                CursorLeftShortcutEnabled.IsOn, ShortcutValues("cursor-left"),
                CursorRightShortcutEnabled.IsOn, ShortcutValues("cursor-right"),
                PreviousPageShortcutEnabled.IsOn, ShortcutValues("previous-page"),
                NextPageShortcutEnabled.IsOn, ShortcutValues("next-page"),
                PunctuationShortcutEnabled.IsOn, ShortcutValues("punctuation"),
                TextNavigationShortcutEnabled.IsOn, ShortcutValues("text-navigation"),
                TextNavigationSelectEnabled.IsOn,
                PunctuationMask());
    }

    private async void ReloadButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e) =>
        await LoadConfigAsync();

    private async void ResetPageButton_Click(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        SetBusy(true);
        try {
            if (_currentPage == "general") {
                AutoSaveSettings.IsOn = true;
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
            switch (_currentPage) {
                case "schemes": ApplySchemeSettings(defaults); break;
                case "candidates": ApplyCandidateSettings(defaults); break;
                case "punctuation": ApplyPunctuationMask(defaults.ChinesePunctuationMask); break;
                case "intelligence": ApplyIntelligenceSettings(defaults); break;
                case "shortcuts": ApplyShortcutSettings(defaults); break;
                default: return;
            }
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
                      InputSchemeCapabilities, DiscoverSchemesButton);
        SetVisibility(page == "candidates", CandidateSectionTitle,
                      CandidatePageSizeCard, CandidateWrapLengthCard,
                      CandidatePartitionCard, CandidateImmediateCommitCard,
                      FocusLossCommitCard, ChinesePunctuationCard,
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
            "general" => ("通用设置", "设置中心的保存行为与通用偏好"),
            "schemes" => ("输入方案", "选择编码方案并查看当前方案能力"),
            "candidates" => ("候选与输入", "调整候选显示、组合上屏、标点和符号预览"),
            "punctuation" => ("中文标点", "逐项选择中文格式；总开关和中英文模式仍决定是否启用"),
            "intelligence" => ("学习与排序", "管理本地词频学习、上下文排序和模型预算"),
            "shortcuts" => ("快捷键", "管理输入、翻页和模式切换的多组按键绑定"),
            "plugins" => ("插件", "安装、授权、配置和维护 OwO 插件"),
            _ => ("输入方案", "选择编码方案并查看当前方案能力"),
        };
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
        Add("输入方案", "全拼 拼音 精确编码 表码", "schemes", InputSchemeCard);
        Add("每页候选数", "候选 显示 数量", "candidates", CandidatePageSizeCard);
        Add("候选词换行长度", "长词 换行", "candidates", CandidateWrapLengthCard);
        Add("候选分区", "字典区 单字区", "candidates", CandidatePartitionCard);
        Add("候选词即时上屏", "暂存 组合 上屏", "candidates", CandidateImmediateCommitCard);
        Add("失焦保护", "候选框 关闭 上屏", "candidates", FocusLossCommitCard);
        Add("中文标点", "全角 标点", "candidates", ChinesePunctuationCard);
        foreach (var option in PunctuationOptions)
            Add($"中文标点 · {option.Name}", $"标点 {option.Source} {option.Output}",
                "punctuation", _punctuationCards[option.Source]);
        Add("用户词频学习", "学习 词频", "intelligence", UserLearningCard);
        Add("词频学习灵敏度", "灵敏度 1 10", "intelligence", LearningSensitivityCard);
        Add("用户词拼音补全", "学习 词库 前缀 补全", "intelligence", UserCompletionCard);
        Add("最多补全后续音节", "用户词 补全 范围 长短", "intelligence", UserCompletionRangeCard);
        Add("智能排序", "模型 上下文 本地", "intelligence", ModelRankingCard);
        Add("外部模型预算", "模型 超时 毫秒", "intelligence", ModelTimeoutCard);
        Add("Alt 数字符号预览", "小键盘 符号", "candidates", AltCodePreviewCard);
        Add("符号预览每页项数", "Alt 数量", "candidates", AltCodePreviewPageSizeCard);
        Add("符号预览鼠标滚轮翻页", "Alt 滚轮", "candidates", AltCodePreviewWheelCard);
        Add("切换中文标点", "快捷键", "shortcuts", PunctuationShortcutCard);
        Add("切换输入纠错", "快捷键 全拼 严格输入", "shortcuts", CorrectionShortcutCard);
        Add("切换中英文输入", "快捷键 语言", "shortcuts", LanguageShortcutCard);
        Add("输入编码原样上屏", "快捷键 Enter", "shortcuts", RawInputShortcutCard);
        Add("预编辑光标左移", "快捷键 编码 光标", "shortcuts", CursorLeftShortcutCard);
        Add("预编辑光标右移", "快捷键 编码 光标", "shortcuts", CursorRightShortcutCard);
        Add("候选词上一页", "快捷键 翻页", "shortcuts", PreviousPageShortcutCard);
        Add("候选词下一页", "快捷键 翻页", "shortcuts", NextPageShortcutCard);
        Add("插件管理", "安装 权限 卸载 恢复", "plugins", PluginSection);
    }

    private void SettingsSearchBox_TextChanged(AutoSuggestBox sender,
                                                AutoSuggestBoxTextChangedEventArgs args)
    {
        if (args.Reason != AutoSuggestionBoxTextChangeReason.UserInput) return;
        var query = sender.Text.Trim();
        if (query.Length == 0) { sender.ItemsSource = null; return; }
        sender.ItemsSource = _baseSearchItems.Concat(_pluginSearchItems)
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
        selected ??= _baseSearchItems.Concat(_pluginSearchItems).FirstOrDefault(item =>
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
            var navigation = SettingsNavigation.MenuItems.OfType<NavigationViewItem>()
                .FirstOrDefault(value => Equals(value.Tag, item.Page));
            if (navigation is not null) SettingsNavigation.SelectedItem = navigation;
            await ShowPluginSettingsAsync(plugin, item.PluginField);
            return;
        }
        SettingsNavigation.SelectedItem = item.Page switch {
            "general" => GeneralNavigationItem,
            "schemes" => SchemeNavigationItem,
            "candidates" => CandidateNavigationItem,
            "punctuation" => CandidateNavigationItem,
            "intelligence" => IntelligenceNavigationItem,
            "shortcuts" => ShortcutNavigationItem,
            "plugins" => PluginNavigationItem,
            _ => SchemeNavigationItem,
        };
        ShowPage(item.Page);
        if (item.Target is not null) DispatcherQueue.TryEnqueue(() => {
            item.Target.StartBringIntoView();
            item.Target.Focus(Microsoft.UI.Xaml.FocusState.Programmatic);
        });
    }

    private void ShortcutButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button button) return;
        _shortcutCaptureTarget = button;
        StartShortcutCaptureHook();
        button.Content = "请按新的快捷键…";
        button.Focus(Microsoft.UI.Xaml.FocusState.Programmatic);
    }

    private void Page_KeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (_shortcutCaptureTarget is null) return;
        if (IsModifierKey(e.Key)) {
            var modifiers = ShortcutForKeyEvent(e.Key);
            _shortcutCaptureTarget.Content = string.IsNullOrEmpty(modifiers)
                ? "请按新的快捷键…" : $"{modifiers}+…";
            e.Handled = true;
            return;
        }
        var shortcut = ShortcutForKeyEvent(e.Key);
        if (shortcut is null) {
            ShowStatus("该按键暂不支持，请使用字母、数字、功能键、导航键或常用符号键。",
                       InfoBarSeverity.Warning);
            e.Handled = true;
            return;
        }
        CommitCapturedShortcut(shortcut);
        e.Handled = true;
    }

    private void Page_KeyUp(object sender, KeyRoutedEventArgs e)
    {
        if (_shortcutCaptureTarget is null || !IsModifierKey(e.Key)) return;
        var shortcut = ShortcutForKeyEvent(e.Key);
        if (shortcut is not null &&
            (shortcut == "Alt" || shortcut.Count(character => character == '+') >= 1)) {
            CommitCapturedShortcut(shortcut);
        } else {
            _shortcutCaptureTarget.Content = "请继续按主键…";
            ShowStatus("Ctrl、Shift 或多个修饰键不能单独作为快捷键，请继续按一个主键。",
                       InfoBarSeverity.Warning);
        }
        e.Handled = true;
    }

    private static bool IsModifierKey(VirtualKey key) =>
        (int)key is 0x10 or 0x11 or 0x12 or 0xA0 or 0xA1 or 0xA2 or 0xA3 or 0xA4 or 0xA5;

    private void CommitCapturedShortcut(string shortcut)
    {
        if (_shortcutCaptureTarget is null) return;
        if (_shortcutCaptureTarget.Tag is ShortcutBinding binding) binding.Value = shortcut;
        _shortcutCaptureTarget = null;
        StopShortcutCaptureHook();
        UpdateShortcutButtons();
        ShowStatus($"快捷键已改为 {shortcut}，点击“保存”后生效。", InfoBarSeverity.Informational);
    }

    private static bool IsDown(int key) => (GetKeyState(key) & 0x8000) != 0;

    private static string? ShortcutForKeyEvent(VirtualKey key)
    {
        const int shiftKey = 0x10;
        const int controlKey = 0x11;
        const int altKey = 0x12;
        var code = (int)key;
        var control = code is 0x11 or 0xA2 or 0xA3 || IsDown(controlKey);
        var alt = code is 0x12 or 0xA4 or 0xA5 || IsDown(altKey);
        var shift = code is 0x10 or 0xA0 or 0xA1 || IsDown(shiftKey);
        return ShortcutForKeyCode(code, control, alt, shift);
    }

    private static string? ShortcutForKeyCode(int code, bool control, bool alt, bool shift)
    {
        var parts = new List<string>();
        if (control) parts.Add("Ctrl");
        if (alt) parts.Add("Alt");
        if (shift) parts.Add("Shift");
        if (code is not (0x10 or 0x11 or 0x12 or 0xA0 or 0xA1 or 0xA2 or 0xA3 or 0xA4 or 0xA5)) {
            var primary = PrimaryKeyName(code);
            if (primary is null) return null;
            parts.Add(primary);
        }
        return parts.Count == 0 ? null : string.Join('+', parts);
    }

    private void StartShortcutCaptureHook()
    {
        StopShortcutCaptureHook();
        // A managed callback is represented by a runtime thunk rather than code in the
        // WinUI executable image. Passing the EXE module handle makes Windows reject the
        // low-level hook on some systems; a null module is correct for this in-process hook.
        _shortcutCaptureHook = SetWindowsHookEx(13, _shortcutCaptureHookProc,
                                                IntPtr.Zero, 0);
        if (_shortcutCaptureHook == IntPtr.Zero)
            ShowStatus($"无法启动快捷键捕获（Win32 {Marshal.GetLastWin32Error()}）。",
                       InfoBarSeverity.Error);
    }

    private void StopShortcutCaptureHook()
    {
        if (_shortcutCaptureHook == IntPtr.Zero) return;
        UnhookWindowsHookEx(_shortcutCaptureHook);
        _shortcutCaptureHook = IntPtr.Zero;
    }

    private IntPtr ShortcutCaptureHook(int code, IntPtr message, IntPtr data)
    {
        const int space = 0x20;
        const int keyDown = 0x0100;
        const int systemKeyDown = 0x0104;
        if (code >= 0 && _shortcutCaptureTarget is not null &&
            ((int)message == keyDown || (int)message == systemKeyDown) &&
            Marshal.ReadInt32(data) == space) {
            var shortcut = ShortcutForKeyCode(space,
                (GetAsyncKeyState(0x11) & 0x8000) != 0,
                (GetAsyncKeyState(0x12) & 0x8000) != 0,
                (GetAsyncKeyState(0x10) & 0x8000) != 0);
            if (shortcut is not null)
                DispatcherQueue.TryEnqueue(() => CommitCapturedShortcut(shortcut));
            return new IntPtr(1);
        }
        return CallNextHookEx(_shortcutCaptureHook, code, message, data);
    }

    private static string? PrimaryKeyName(int key)
    {
        if (key is >= 0x41 and <= 0x5A || key is >= 0x30 and <= 0x39)
            return ((char)key).ToString();
        if (key is >= 0x70 and <= 0x87) return $"F{key - 0x6F}";
        return key switch {
            0x20 => "Space", 0x0D => "Enter", 0x09 => "Tab", 0x1B => "Escape",
            0x08 => "Backspace", 0x2E => "Delete", 0x2D => "Insert",
            0x24 => "Home", 0x23 => "End", 0x21 => "PageUp", 0x22 => "PageDown",
            0x25 => "Left", 0x27 => "Right", 0x26 => "Up", 0x28 => "Down",
            0xDB => "[", 0xDD => "]", 0xBD => "Minus", 0xBB => "Plus",
            0xBC => "Comma", 0xBE => "Period", 0xBF => "Slash",
            0xBA => "Semicolon", 0xDE => "Quote", 0xC0 => "Backtick", _ => null,
        };
    }

    private void SetShortcutBindings(string group, IEnumerable<string> values)
    {
        var items = values.Where(value => !string.IsNullOrWhiteSpace(value))
            .Select(value => new ShortcutBinding(group, value)).ToList();
        _shortcutBindings[group] = items;
    }

    private IReadOnlyList<string> ShortcutValues(string group) =>
        _shortcutBindings[group].Select(binding => binding.Value).ToArray();

    private StackPanel ShortcutPanel(string group) => group switch {
        "correction" => CorrectionShortcutBindings,
        "language" => LanguageShortcutBindings,
        "raw" => RawInputShortcutBindings,
        "cursor-left" => CursorLeftShortcutBindings,
        "cursor-right" => CursorRightShortcutBindings,
        "previous-page" => PreviousPageShortcutBindings,
        "next-page" => NextPageShortcutBindings,
        "text-navigation" => TextNavigationShortcutBindings,
        "punctuation" => PunctuationShortcutBindings,
        _ => throw new ArgumentOutOfRangeException(nameof(group)),
    };

    private void UpdateShortcutButtons()
    {
        QueueSettingsAutoSave();
        foreach (var (group, bindings) in _shortcutBindings) {
            var panel = ShortcutPanel(group);
            panel.Children.Clear();
            foreach (var binding in bindings) {
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                var capture = new Button {
                    Content = binding.Value, Tag = binding,
                    HorizontalAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Left,
                };
                capture.Click += ShortcutButton_Click;
                var remove = new Button { Content = "−", Tag = binding };
                remove.Click += RemoveShortcutButton_Click;
                row.Children.Add(capture);
                row.Children.Add(remove);
                panel.Children.Add(row);
            }
        }
    }

    private void AddShortcutButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: string group }) return;
        var bindings = _shortcutBindings[group];
        if (bindings.Count >= 16) {
            ShowStatus("每项快捷键最多可添加 16 种方案。", InfoBarSeverity.Warning);
            return;
        }
        var binding = new ShortcutBinding(group,
            bindings.Count == 0 ? "Ctrl+Period" : bindings[^1].Value);
        bindings.Add(binding);
        UpdateShortcutButtons();
        var panel = ShortcutPanel(group);
        var row = (StackPanel)panel.Children[panel.Children.Count - 1];
        ShortcutButton_Click(row.Children[0], e);
    }

    private void RemoveShortcutButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: ShortcutBinding binding }) return;
        var bindings = _shortcutBindings[binding.Group];
        if (bindings.Count == 1 && binding.Group != "punctuation") {
            ShowStatus("每项快捷键至少保留一种方案；如不需要可关闭该快捷键项。",
                       InfoBarSeverity.Warning);
            return;
        }
        if (_shortcutCaptureTarget?.Tag == binding) {
            _shortcutCaptureTarget = null;
            StopShortcutCaptureHook();
        }
        bindings.Remove(binding);
        UpdateShortcutButtons();
    }

    private void ValidateShortcutConflicts()
    {
        var shortcuts = new List<string>();
        if (CorrectionShortcutEnabled.IsOn) shortcuts.AddRange(ShortcutValues("correction"));
        if (LanguageShortcutEnabled.IsOn) shortcuts.AddRange(ShortcutValues("language"));
        if (RawInputShortcutEnabled.IsOn) shortcuts.AddRange(ShortcutValues("raw"));
        if (CursorLeftShortcutEnabled.IsOn) shortcuts.AddRange(ShortcutValues("cursor-left"));
        if (CursorRightShortcutEnabled.IsOn) shortcuts.AddRange(ShortcutValues("cursor-right"));
        if (PreviousPageShortcutEnabled.IsOn) shortcuts.AddRange(ShortcutValues("previous-page"));
        if (NextPageShortcutEnabled.IsOn) shortcuts.AddRange(ShortcutValues("next-page"));
        if (TextNavigationShortcutEnabled.IsOn)
            shortcuts.AddRange(ShortcutValues("text-navigation"));
        if (PunctuationShortcutEnabled.IsOn) {
            var punctuation = ShortcutValues("punctuation");
            if (punctuation.Count == 0)
                throw new InvalidOperationException("启用中文标点快捷键前，请先添加至少一种绑定方案。");
            shortcuts.AddRange(punctuation);
        }
        if (shortcuts.Count != shortcuts.Distinct(StringComparer.Ordinal).Count())
            throw new InvalidOperationException("启用的快捷键不能重复。");
    }

    private async Task LoadPluginsAsync()
    {
        SetBusy(true);
        try {
            var value = await _pluginClient.LoadAsync();
            PluginVersions.ItemsSource = value.Plugins;
            RecoveryItems.ItemsSource = value.Recovery;
            RebuildPluginNavigation(value.Plugins);
            NoPlugins.Visibility = value.Plugins.Count == 0
                ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
            NoRecovery.Visibility = value.Recovery.Count == 0
                ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
            ShowStatus($"插件状态已刷新：{value.Plugins.Count} 个版本，{value.Recovery.Count} 个恢复项。",
                       InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private void RebuildPluginNavigation(IReadOnlyList<PluginVersionSnapshot> plugins)
    {
        foreach (var item in SettingsNavigation.MenuItems.OfType<NavigationViewItem>()
                     .Where(item => item.Tag is string tag &&
                                    tag.StartsWith("plugin:", StringComparison.Ordinal)).ToArray())
            SettingsNavigation.MenuItems.Remove(item);
        if (_pluginPagesHeader is not null) SettingsNavigation.MenuItems.Remove(_pluginPagesHeader);
        _pluginPages.Clear();
        _pluginSearchItems.Clear();
        if (plugins.Count == 0) return;
        _pluginPagesHeader = new NavigationViewItemHeader { Content = "插件设置" };
        SettingsNavigation.MenuItems.Add(_pluginPagesHeader);
        foreach (var plugin in plugins.OrderByDescending(value => value.Active)
                                      .ThenBy(value => value.Name, StringComparer.Ordinal)) {
            var page = $"plugin:{plugin.Id}:{plugin.Version}";
            _pluginPages[page] = plugin;
            SettingsNavigation.MenuItems.Add(new NavigationViewItem {
                Content = plugin.Name, Tag = page,
                Icon = new FontIcon { Glyph = "\uE74C" },
            });
            _pluginSearchItems.Add(new SearchItem(plugin.Name,
                $"插件 {plugin.Id} {plugin.Version}", page));
            try {
                var model = PluginSettingsModel.Load(_pluginClient.StorePath, plugin);
                foreach (var field in model.Fields)
                    _pluginSearchItems.Add(new SearchItem(field.Title,
                        $"{plugin.Name} {field.Description}", page, null, field.Key));
            } catch {
                // A damaged or legacy schema remains manageable from the main plugin page.
            }
        }
    }

    private async void PluginVersionSettings_Click(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginVersionSnapshot plugin }) return;
        var page = $"plugin:{plugin.Id}:{plugin.Version}";
        var navigation = SettingsNavigation.MenuItems.OfType<NavigationViewItem>()
            .FirstOrDefault(value => Equals(value.Tag, page));
        if (navigation is not null) SettingsNavigation.SelectedItem = navigation;
        await ShowPluginSettingsAsync(plugin);
    }

    private Task ShowPluginSettingsAsync(PluginVersionSnapshot plugin,
                                          string? focusField = null)
    {
        try {
            _pluginSettings = PluginSettingsModel.Load(_pluginClient.StorePath, plugin);
            _currentPage = $"plugin:{plugin.Id}:{plugin.Version}";
            PluginConfigFields.Children.Clear();
            _pluginConfigControls.Clear();
            _pluginConfigExpanders.Clear();
            _buildingPluginSettings = true;
            try { AddPluginSettingFields(); }
            finally { _buildingPluginSettings = false; }
            GeneralSettingsPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            ShortcutSettingsPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            PluginSettingsPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            ChinesePunctuationPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            GeneralPreferencesPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            PluginConfigPage.Visibility = Microsoft.UI.Xaml.Visibility.Visible;
            ResetPageButton.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            SaveButton.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            ReloadButton.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            CurrentPageTitle.Text = _pluginSettings.Title;
            CurrentPageDescription.Text = $"{plugin.Name} {plugin.Version} · {plugin.Id}";
            PluginConfigNotice.Message = _pluginSettings.Description;
            PluginConfigNotice.Severity = InfoBarSeverity.Informational;
            PluginConfigSaveButton.IsEnabled = _pluginSettings.Fields.Count != 0;
            if (_pluginSettings.Fields.Count == 0) {
                PluginConfigNotice.Message = "该插件没有声明可配置字段。";
            }
            if (focusField is not null && _pluginConfigControls.TryGetValue(focusField, out var target))
                DispatcherQueue.TryEnqueue(() => {
                    if (_pluginConfigExpanders.TryGetValue(focusField, out var expander))
                        expander.IsExpanded = true;
                    target.StartBringIntoView();
                    target.Focus(Microsoft.UI.Xaml.FocusState.Programmatic);
                });
        } catch (Exception error) {
            ShowPage("plugins");
            SettingsNavigation.SelectedItem = PluginNavigationItem;
            ShowStatus($"无法打开插件设置：{error.Message}", InfoBarSeverity.Error);
        }
        return Task.CompletedTask;
    }

    private void AddPluginSettingFields()
    {
        var grouped = new HashSet<string>(StringComparer.Ordinal);
        foreach (var field in _pluginSettings!.Fields) {
            if (grouped.Contains(field.Key)) continue;
            PluginSettingField? opacity = null;
            if (field.Format == "color" && field.Key.EndsWith("_color", StringComparison.Ordinal)) {
                var opacityKey = field.Key[..^"_color".Length] + "_opacity";
                opacity = _pluginSettings.Fields.FirstOrDefault(value =>
                    value.Key == opacityKey && value.Type is "integer" or "number");
                if (opacity is not null) grouped.Add(opacity.Key);
            }
            AddPluginSettingField(field, opacity);
        }
    }

    private static StackPanel PluginSettingText(PluginSettingField field)
    {
        var title = new TextBlock { Text = field.Title, FontWeight =
            Microsoft.UI.Text.FontWeights.SemiBold };
        var text = new StackPanel { Spacing = 3 };
        text.Children.Add(title);
        if (!string.IsNullOrWhiteSpace(field.Description)) text.Children.Add(new TextBlock {
            Text = field.Description, TextWrapping = Microsoft.UI.Xaml.TextWrapping.Wrap,
            Foreground = (Microsoft.UI.Xaml.Media.Brush)Microsoft.UI.Xaml.Application.Current.Resources[
                "TextFillColorSecondaryBrush"], FontSize = 12,
        });
        return text;
    }

    private Microsoft.UI.Xaml.FrameworkElement CreatePluginSettingControl(
        PluginSettingField field)
    {
        var value = _pluginSettings!.Values[field.Key];
        Microsoft.UI.Xaml.FrameworkElement control;
        if (field.Options.Count != 0) {
            var combo = new ComboBox { ItemsSource = field.Options,
                DisplayMemberPath = nameof(PluginSettingOption.Label) };
            combo.SelectedItem = field.Options.FirstOrDefault(option =>
                option.Value.GetRawText() == value.GetRawText()) ?? field.Options[0];
            control = combo;
        } else if (field.Format == "color") {
            var color = value.ValueKind == JsonValueKind.String
                ? ParsePluginColor(value.GetString()) : Windows.UI.Color.FromArgb(255, 0, 0, 0);
            control = new ColorPicker {
                Color = color,
                IsAlphaEnabled = false,
                IsAlphaSliderVisible = false,
                IsAlphaTextInputVisible = false,
                IsColorChannelTextInputVisible = true,
                IsHexInputVisible = true,
                IsColorSpectrumVisible = true,
                IsColorSliderVisible = true,
                MinWidth = 300,
            };
        } else if (field.Type == "boolean") {
            control = new ToggleSwitch { IsOn = value.ValueKind == JsonValueKind.True,
                OnContent = "开", OffContent = "关" };
        } else if (field.Type is "integer" or "number") {
            control = new NumberBox { SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline,
                Minimum = field.Minimum ?? double.MinValue,
                Maximum = field.Maximum ?? double.MaxValue,
                SmallChange = field.Step ?? 1.0,
                Value = value.TryGetDouble(out var number) ? number : 0 };
        } else {
            control = new TextBox { Text = value.ValueKind == JsonValueKind.String
                ? value.GetString() ?? "" : "" };
        }
        control.VerticalAlignment = Microsoft.UI.Xaml.VerticalAlignment.Center;
        ObserveAutoSave(control, plugin: true);
        _pluginConfigControls[field.Key] = control;
        return control;
    }

    private void AddPluginSettingField(PluginSettingField field,
                                       PluginSettingField? opacityField = null)
    {
        var firstColorField = field.Format == "color" &&
            !_pluginConfigControls.Values.Any(value => value is ColorPicker);
        var text = PluginSettingText(field);
        var grid = new Grid { ColumnSpacing = 24 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new Microsoft.UI.Xaml.GridLength(1,
            Microsoft.UI.Xaml.GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = Microsoft.UI.Xaml.GridLength.Auto });
        var control = CreatePluginSettingControl(field);
        Microsoft.UI.Xaml.FrameworkElement cardContent;
        var cardPadding = new Microsoft.UI.Xaml.Thickness(16, 12, 8, 12);
        if (field.Format == "color") {
            var colorControls = new StackPanel { Spacing = 12 };
            colorControls.Children.Add(control);
            if (opacityField is not null) {
                colorControls.Children.Add(new Border {
                    Height = 1,
                    Background = (Microsoft.UI.Xaml.Media.Brush)
                        Microsoft.UI.Xaml.Application.Current.Resources[
                            "DividerStrokeColorDefaultBrush"],
                });
                var opacityGrid = new Grid { ColumnSpacing = 24 };
                opacityGrid.ColumnDefinitions.Add(new ColumnDefinition {
                    Width = new Microsoft.UI.Xaml.GridLength(
                        1, Microsoft.UI.Xaml.GridUnitType.Star),
                });
                opacityGrid.ColumnDefinitions.Add(new ColumnDefinition {
                    Width = Microsoft.UI.Xaml.GridLength.Auto,
                });
                opacityGrid.Children.Add(PluginSettingText(opacityField));
                var opacityControl = CreatePluginSettingControl(opacityField);
                Grid.SetColumn(opacityControl, 1);
                opacityGrid.Children.Add(opacityControl);
                colorControls.Children.Add(opacityGrid);
            }
            var colorBody = new Border {
                Child = colorControls,
                Padding = new Microsoft.UI.Xaml.Thickness(16, 0, 16, 16),
                HorizontalAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Stretch,
            };
            var expander = new Expander {
                Header = text,
                Content = colorBody,
                IsExpanded = firstColorField,
                HorizontalAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Stretch,
                HorizontalContentAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Stretch,
            };
            cardContent = expander;
            cardPadding = new Microsoft.UI.Xaml.Thickness(0);
            _pluginConfigExpanders[field.Key] = expander;
            if (opacityField is not null)
                _pluginConfigExpanders[opacityField.Key] = expander;
        } else {
            grid.Children.Add(text);
            Grid.SetColumn(control, 1);
            grid.Children.Add(control);
            cardContent = grid;
        }
        var card = new Border { Child = cardContent, Padding = cardPadding,
            CornerRadius = new Microsoft.UI.Xaml.CornerRadius(8), BorderThickness = new Microsoft.UI.Xaml.Thickness(1),
            Background = (Microsoft.UI.Xaml.Media.Brush)Microsoft.UI.Xaml.Application.Current.Resources[
                "CardBackgroundFillColorDefaultBrush"],
            BorderBrush = (Microsoft.UI.Xaml.Media.Brush)Microsoft.UI.Xaml.Application.Current.Resources[
                "CardStrokeColorDefaultBrush"] };
        PluginConfigFields.Children.Add(card);
    }

    private Dictionary<string, object?> CapturePluginValues()
    {
            var values = new Dictionary<string, object?>(StringComparer.Ordinal);
            foreach (var field in _pluginSettings!.Fields) {
                var control = _pluginConfigControls[field.Key];
                values[field.Key] = control switch {
                    ToggleSwitch toggle => toggle.IsOn,
                    NumberBox number when field.Type == "integer" => checked((long)Math.Round(number.Value)),
                    NumberBox number => number.Value,
                    ComboBox { SelectedItem: PluginSettingOption option } =>
                        JsonSerializer.Deserialize<object>(option.Value.GetRawText()),
                    ColorPicker picker =>
                        $"#{picker.Color.R:X2}{picker.Color.G:X2}{picker.Color.B:X2}",
                    TextBox input => input.Text,
                    _ => null,
                };
            }
            return values;
    }

    private void PluginConfigSaveButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (_pluginSettings is null) return;
        try {
            var values = CapturePluginValues();
            _pluginSettings.Save(values);
            _pendingPluginSaves.Remove(_pluginSettings);
            NotifyCandidateThemeChanged();
            ShowStatus($"{_pluginSettings.Plugin.Name} 的设置已保存。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus($"插件设置保存失败：{error.Message}", InfoBarSeverity.Error);
        }
    }

    private void PluginConfigResetButton_Click(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (_pluginSettings is null) return;
        _pluginSettings.Values.Clear();
        foreach (var value in _pluginSettings.DefaultValues())
            _pluginSettings.Values[value.Key] = value.Value;
        PluginConfigFields.Children.Clear();
        _pluginConfigControls.Clear();
        _pluginConfigExpanders.Clear();
        _buildingPluginSettings = true;
        try { AddPluginSettingFields(); }
        finally { _buildingPluginSettings = false; }
        QueuePluginAutoSave();
        ShowStatus(AutoSaveSettings.IsOn ? $"{_pluginSettings.Plugin.Name} 已恢复默认值，正在自动保存。" : $"{_pluginSettings.Plugin.Name} 已恢复默认值；点击“保存插件设置”后应用。",
                   InfoBarSeverity.Success);
    }

    private static Windows.UI.Color ParsePluginColor(string? text)
    {
        if (text is null || text.Length != 7 || text[0] != '#' ||
            !byte.TryParse(text.AsSpan(1, 2), System.Globalization.NumberStyles.HexNumber,
                           null, out var red) ||
            !byte.TryParse(text.AsSpan(3, 2), System.Globalization.NumberStyles.HexNumber,
                           null, out var green) ||
            !byte.TryParse(text.AsSpan(5, 2), System.Globalization.NumberStyles.HexNumber,
                           null, out var blue))
            return Windows.UI.Color.FromArgb(255, 0, 0, 0);
        return Windows.UI.Color.FromArgb(255, red, green, blue);
    }

    private async void PluginReloadButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e) =>
        await LoadPluginsAsync();

    private async void PluginInstallButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        var picker = new FileOpenPicker {
            SuggestedStartLocation = PickerLocationId.Downloads,
            ViewMode = PickerViewMode.List,
        };
        picker.FileTypeFilter.Add(".owopkg");
        picker.FileTypeFilter.Add(".zip");
        var window = ((App)Microsoft.UI.Xaml.Application.Current).MainWindow;
        WinRT.Interop.InitializeWithWindow.Initialize(
            picker, WinRT.Interop.WindowNative.GetWindowHandle(window));
        var package = await picker.PickSingleFileAsync();
        if (package is null) return;
        await InstallPluginSourceAsync(package.Name, package.Path);
    }

    private async void PluginInstallFolderButton_Click(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        var picker = new FolderPicker {
            SuggestedStartLocation = PickerLocationId.Downloads,
            ViewMode = PickerViewMode.List,
        };
        picker.FileTypeFilter.Add("*");
        var window = ((App)Microsoft.UI.Xaml.Application.Current).MainWindow;
        WinRT.Interop.InitializeWithWindow.Initialize(
            picker, WinRT.Interop.WindowNative.GetWindowHandle(window));
        var folder = await picker.PickSingleFolderAsync();
        if (folder is null) return;
        await InstallPluginSourceAsync(folder.Name, folder.Path);
    }

    private async Task InstallPluginSourceAsync(string sourceName, string sourcePath)
    {
        SetBusy(true);
        try {
            var preview = await _pluginClient.InspectInstallAsync(sourcePath);
            PluginInstallSnapshot result;
            if (preview.RequiresRiskConsent) {
                SetBusy(false);
                if (!await ConfirmRiskInstallAsync(sourceName, sourcePath, preview)) return;
                SetBusy(true);
                result = await _pluginClient.InstallRiskAsync(sourcePath, preview.InventorySha256);
            } else {
                SetBusy(false);
                var message = $"{sourceName}\n{sourcePath}\n\n"
                    + $"插件：{preview.Name} {preview.Version}（{preview.PluginId}）\n"
                    + $"发布者：{PluginUiText.Trust(preview.TrustTier)}\n"
                    + "系统将重新核对精确包摘要，随后原子安装并立即启用。";
                if (!await ConfirmAsync("安装插件包", message, "验证并安装")) return;
                SetBusy(true);
                result = await _pluginClient.InstallAsync(sourcePath);
            }
            await LoadPluginsAsync();
            if (result.Activated) NotifyCandidateThemeChanged();
            var publisher = string.IsNullOrWhiteSpace(result.PublisherDisplayName)
                ? PluginUiText.Trust(result.TrustTier) : result.PublisherDisplayName;
            var state = result.Activated ? "已安装并启用" : "已安装但保持停用";
            ShowStatus($"{state} {result.Name} {result.Version}（{result.PluginId}）；发布者：{publisher}。",
                       InfoBarSeverity.Success);
        } catch (Exception error) {
            await LoadPluginsAsync();
            ShowStatus($"插件安装失败：{error.Message}", InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private async Task<bool> ConfirmRiskInstallAsync(
        string packageName, string packagePath, PluginInstallPreviewSnapshot preview)
    {
        var permissions = preview.Permissions.Count == 0 ? "（未申请具名权限）"
            : string.Join("\n", preview.Permissions.Select(
                permission => $"• {PluginUiText.Permission(permission)}（{permission}）"));
        var trustDetail = string.IsNullOrWhiteSpace(preview.TrustDiagnostic)
            ? "未能建立 Windows 发布者信任。" : preview.TrustDiagnostic;
        var acknowledgement = new CheckBox {
            Content = "我已阅读并理解：该插件可能造成隐私泄露、文件或账户损失，风险由我承担。",
        };
        var content = new StackPanel { Spacing = 12 };
        content.Children.Add(new TextBlock {
            Text = $"{packageName}\n{packagePath}\n\n"
                + $"插件：{preview.Name} {preview.Version}（{preview.PluginId}）\n"
                + $"风险：{PluginUiText.Risk(preview.RiskLevel)}\n"
                + $"信任：{PluginUiText.Trust(preview.TrustTier)}\n"
                + $"包摘要：{preview.InventorySha256}\n\n"
                + $"申请的能力：\n{permissions}\n\n"
                + $"验证说明：{trustDetail}\n\n"
                + "风险与免责：第三方插件可能读取、修改、删除或上传你的数据，捕获屏幕/音频、"
                + "覆盖界面、启动程序或以当前 Windows 用户权限执行操作。OwO 项目不代表已审计"
                + "第三方代码，也不保证其安全性、可用性或数据可恢复性。继续安装表示你理解并自行承担后果。"
                + "本授权只绑定上方精确版本与包摘要，更新或新增权限必须重新授权。",
            TextWrapping = Microsoft.UI.Xaml.TextWrapping.Wrap,
        });
        content.Children.Add(acknowledgement);
        var disclosure = new ContentDialog {
            XamlRoot = XamlRoot,
            Title = "高风险插件安全告知",
            Content = new ScrollViewer {
                Content = content,
                MaxHeight = 520,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            },
            PrimaryButtonText = "我已理解风险",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
            IsPrimaryButtonEnabled = false,
        };
        acknowledgement.Checked += (_, _) => disclosure.IsPrimaryButtonEnabled = true;
        acknowledgement.Unchecked += (_, _) => disclosure.IsPrimaryButtonEnabled = false;
        if (await disclosure.ShowAsync() != ContentDialogResult.Primary) return false;
        return await ConfirmAsync("单独授权安装",
            $"只授权安装此包：\n{preview.Name} {preview.Version}\n{preview.PluginId}\n"
                + $"SHA-256 清单摘要：{preview.InventorySha256}\n\n"
                + "安装后将保持停用；启用前仍可撤销权限或卸载。",
            "授权并安装");
    }

    private async Task<bool> ConfirmAsync(string title, string message, string action)
    {
        var dialog = new ContentDialog {
            XamlRoot = XamlRoot,
            Title = title,
            Content = message,
            PrimaryButtonText = action,
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
        };
        return await dialog.ShowAsync() == ContentDialogResult.Primary;
    }

    private async void PluginVersionAction_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginVersionSnapshot plugin }) return;
        var action = plugin.Active ? "停用" : "启用";
        var warning = plugin.Active ? "" : $"\n\n{PluginUiText.Risk(plugin.RiskLevel)} · "
            + $"{PluginUiText.Trust(plugin.TrustTier)}\n"
            + (plugin.Permissions.Count == 0 ? "未申请具名权限" :
               $"已授权：{string.Join("、", plugin.Permissions.Select(PluginUiText.Permission))}");
        var agentPrivacyConsent = false;
        if (!plugin.Active && plugin.Permissions.Contains("agent.ipc", StringComparer.Ordinal)) {
            if (!await ConfirmAgentActivationAsync(plugin)) return;
            agentPrivacyConsent = true;
        } else if (!await ConfirmAsync($"{action}插件",
                       $"{plugin.Name} {plugin.Version}\n{plugin.Id}{warning}", action)) return;
        SetBusy(true);
        try {
            if (plugin.Active) await _pluginClient.DeactivateAsync(plugin.Id, plugin.Version);
            else await _pluginClient.ActivateAsync(
                plugin.Id, plugin.Version, agentPrivacyConsent);
            await LoadPluginsAsync();
            NotifyCandidateThemeChanged();
            ShowStatus($"插件已{action}。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private async Task<bool> ConfirmAgentActivationAsync(PluginVersionSnapshot plugin)
    {
        var acknowledgement = new CheckBox {
            Content = "我明确同意启用，并理解第三方 Agent 可能执行系统操作。",
        };
        var content = new StackPanel { Spacing = 12 };
        content.Children.Add(new TextBlock {
            Text = $"{plugin.Name} {plugin.Version}\n{plugin.Id}\n\n"
                + "启用后，在你以 v 开头输入并按 Enter 或空格确认时，插件可能读取：去除 v 后的输入、"
                + "焦点应用的进程 ID/程序名/窗口类，以及必要的短局部上下文。它会先过滤常见凭据、令牌"
                + "和支付卡号，再把最少数据发送给你配置的本机第三方 Agent。\n\n"
                + "密码框和安全输入框不会采集上下文；不会发送窗口标题、完整文档或剪贴板。第三方 Agent "
                + "仍可能保存请求、关联应用使用习惯，或根据你选择的指令执行文件、程序、系统设置、账户等操作。"
                + "OwO 不审计第三方 Agent，也不保证其操作可恢复。高风险指令不能通过候选框直接确认，必须回到"
                + "第三方 Agent 自身的高风险确认界面。\n\n"
                + "此同意只绑定当前精确插件版本与已授权权限；更新或新增权限需要重新确认。",
            TextWrapping = Microsoft.UI.Xaml.TextWrapping.Wrap,
        });
        content.Children.Add(acknowledgement);
        var dialog = new ContentDialog {
            XamlRoot = XamlRoot,
            Title = "启用 Agent IPC 连接器",
            Content = new ScrollViewer {
                Content = content,
                MaxHeight = 520,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            },
            PrimaryButtonText = "明确同意并启用",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
            IsPrimaryButtonEnabled = false,
        };
        acknowledgement.Checked += (_, _) => dialog.IsPrimaryButtonEnabled = true;
        acknowledgement.Unchecked += (_, _) => dialog.IsPrimaryButtonEnabled = false;
        return await dialog.ShowAsync() == ContentDialogResult.Primary;
    }

    private async void PluginVersionRevoke_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginVersionSnapshot plugin } || !plugin.CanRevoke) return;
        var message = $"{plugin.Name} {plugin.Version}\n{plugin.Id}\n\n"
            + "将撤销此精确版本的全部运行权限。插件文件与数据保留；已运行的插件应先停用，"
            + "再次授权需要重新安装该精确包。";
        if (!await ConfirmAsync("撤销插件权限", message, "撤销权限")) return;
        SetBusy(true);
        try {
            if (plugin.Active) await _pluginClient.DeactivateAsync(plugin.Id, plugin.Version);
            await _pluginClient.RevokeAsync(plugin.Id, plugin.Version);
            await LoadPluginsAsync();
            NotifyCandidateThemeChanged();
            ShowStatus("插件权限已撤销，插件已停用。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private async void PluginVersionUninstall_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginVersionSnapshot plugin } || plugin.Active) return;
        var message = $"{plugin.Name} {plugin.Version}\n{plugin.Id}\n\n"
            + "此操作会删除该版本及其精确授权，无法撤销。插件用户数据会保留。";
        if (!await ConfirmAsync("卸载插件版本", message, "卸载")) return;
        SetBusy(true);
        try {
            await _pluginClient.UninstallAsync(plugin.Id, plugin.Version);
            await LoadPluginsAsync();
            ShowStatus("插件版本已卸载；用户数据已保留。", InfoBarSeverity.Success);
        } catch (Exception error) {
            await LoadPluginsAsync();
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private async void RecoveryAction_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginRecoverySnapshot item } || !item.CanApply) return;
        var activating = item.Action == "activate";
        var action = activating ? "切换版本" : "清理";
        if (!await ConfirmAsync(action, $"{item.Title}\n{item.Detail}", action)) return;
        SetBusy(true);
        try {
            if (activating) await _pluginClient.ActivateAsync(item.PluginId, item.Version);
            else await _pluginClient.CleanupAsync(item);
            await LoadPluginsAsync();
            if (activating) NotifyCandidateThemeChanged();
            ShowStatus($"恢复操作“{action}”已完成。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
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
        Status.IsOpen = true;
    }

    private void ShowStatus(string message, InfoBarSeverity severity)
    {
        Status.Title = message;
        Status.Severity = severity;
        Status.IsOpen = true;
    }
}
