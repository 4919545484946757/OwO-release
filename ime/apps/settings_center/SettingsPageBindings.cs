using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private readonly SettingsModules _settingsModules = new();
    private readonly SettingsUiEditors _settingsEditors = new();
    private readonly List<SettingsCardBinding> _settingsCards = [];
    private SettingsSnapshot? _settingsBaseline;
    private sealed record SettingsCardBinding(string Key, FrameworkElement Card,
        string Title, string Keywords, string? SearchPage, Func<bool>? Dependency, string? DependencyHint);

    private void InitializeSettingsModules()
    {
        void Toggle(string key, ToggleSwitch control) =>
            _settingsEditors.Bind(key, () => control.IsOn, value => control.IsOn = value, control);
        void Number(string key, NumberBox control, uint min, uint max, string title) =>
            _settingsEditors.Bind(key, () => SettingsModules.UnsignedNumber(control.Value, min, max, title),
                value => control.Value = value, control);
        void Card(string key, FrameworkElement card, string title, string keywords,
                  Func<bool>? dependency = null, string? hint = null, string? page = null) =>
            _settingsCards.Add(new(key, card, title, keywords, page, dependency, hint));

        _settingsEditors.Bind(nameof(SettingsSnapshot.InputScheme),
            () => CurrentScheme.Id, SelectInputScheme, InputScheme);
        Card(nameof(SettingsSnapshot.InputScheme), InputSchemeCard, "输入方案", "全拼 拼音 精确编码 表码");

        Number(nameof(SettingsSnapshot.CandidatePageSize), CandidatePageSize, 1, 9, "每页候选数");
        Number(nameof(SettingsSnapshot.CandidateWrapLength), CandidateWrapLength, 4, 64, "候选词换行长度");
        Toggle(nameof(SettingsSnapshot.CandidatePartitionEnabled), CandidatePartition);
        Toggle(nameof(SettingsSnapshot.CandidateSingleCharacterFirstEnabled), CandidateSingleCharacterFirst);
        Toggle(nameof(SettingsSnapshot.CandidateImmediateCommitEnabled), CandidateImmediateCommit);
        Toggle(nameof(SettingsSnapshot.FocusLossCommitEnabled), FocusLossCommit);
        Card(nameof(SettingsSnapshot.CandidatePageSize), CandidatePageSizeCard, "每页候选数", "候选 显示 数量");
        Card(nameof(SettingsSnapshot.CandidateWrapLength), CandidateWrapLengthCard, "候选词换行长度", "长词 换行");
        Card(nameof(SettingsSnapshot.CandidatePartitionEnabled), CandidatePartitionCard, "候选分区", "字典区 单字区");
        Card(nameof(SettingsSnapshot.CandidateSingleCharacterFirstEnabled), CandidateSingleCharacterFirstCard,
            "完整单字拼音时单字区优先", "候选 分区 单字 多字 顺序 每页", () => CandidatePartition.IsOn, "请先开启候选分区。");
        Card(nameof(SettingsSnapshot.CandidateImmediateCommitEnabled), CandidateImmediateCommitCard, "候选词即时上屏", "暂存 组合 上屏");
        Card(nameof(SettingsSnapshot.FocusLossCommitEnabled), FocusLossCommitCard, "失焦保护", "候选框 关闭 上屏");

        Toggle(nameof(SettingsSnapshot.ChinesePunctuationEnabled), ChinesePunctuation);
        _settingsEditors.Bind(nameof(SettingsSnapshot.ChinesePunctuationMask), PunctuationMask,
            ApplyPunctuationMask, _punctuationSwitches.Values.Cast<Control>().ToArray());
        Card(nameof(SettingsSnapshot.ChinesePunctuationEnabled), ChinesePunctuationCard, "中文标点", "全角 标点", page: "candidates");
        Card(nameof(SettingsSnapshot.ChinesePunctuationMask), ChinesePunctuationFields, "中文标点逐项设置", "标点 格式 开关");

        Toggle(nameof(SettingsSnapshot.AltCodePreviewEnabled), AltCodePreview);
        Number(nameof(SettingsSnapshot.AltCodePreviewPageSize), AltCodePreviewPageSize, 2, 10, "符号预览每页项数");
        Toggle(nameof(SettingsSnapshot.AltCodePreviewWheelEnabled), AltCodePreviewWheel);
        Card(nameof(SettingsSnapshot.AltCodePreviewEnabled), AltCodePreviewCard, "Alt 数字符号预览", "小键盘 符号");
        Card(nameof(SettingsSnapshot.AltCodePreviewPageSize), AltCodePreviewPageSizeCard, "符号预览每页项数", "Alt 数量", () => AltCodePreview.IsOn, "请先开启 Alt 数字符号预览。");
        Card(nameof(SettingsSnapshot.AltCodePreviewWheelEnabled), AltCodePreviewWheelCard, "符号预览鼠标滚轮翻页", "Alt 滚轮", () => AltCodePreview.IsOn, "请先开启 Alt 数字符号预览。");

        Toggle(nameof(SettingsSnapshot.UserLearningEnabled), UserLearning);
        _settingsEditors.Bind(nameof(SettingsSnapshot.UserLearningSensitivity),
            () => SettingsModules.UnsignedNumber(UserLearningSensitivity.Value, 1, 10, "词频学习灵敏度"),
            value => UserLearningSensitivity.Value = value, UserLearningSensitivity);
        Toggle(nameof(SettingsSnapshot.UserCompletionEnabled), UserCompletion);
        Number(nameof(SettingsSnapshot.UserCompletionMaxSyllables), UserCompletionRange, 0, 8, "最多补全后续音节");
        Toggle(nameof(SettingsSnapshot.ModelRankingEnabled), ModelRanking);
        Number(nameof(SettingsSnapshot.ModelTimeoutMs), ModelTimeout, 5, 500, "外部模型预算");
        Card(nameof(SettingsSnapshot.UserLearningEnabled), UserLearningCard, "用户词频学习", "学习 词频");
        Card(nameof(SettingsSnapshot.UserLearningSensitivity), LearningSensitivityCard, "词频学习灵敏度", "灵敏度 1 10", () => UserLearning.IsOn, "请先开启用户词频学习。");
        Card(nameof(SettingsSnapshot.UserCompletionEnabled), UserCompletionCard, "用户词拼音补全", "学习 词库 前缀 补全");
        Card(nameof(SettingsSnapshot.UserCompletionMaxSyllables), UserCompletionRangeCard, "最多补全后续音节", "用户词 补全 范围 长短", () => UserCompletion.IsOn, "请先开启用户词拼音补全。");
        Card(nameof(SettingsSnapshot.ModelRankingEnabled), ModelRankingCard, "智能排序", "模型 上下文 本地");
        Card(nameof(SettingsSnapshot.ModelTimeoutMs), ModelTimeoutCard, "外部模型预算", "模型 超时 毫秒", () => ModelRanking.IsOn, "请先开启智能排序。");

        Toggle(nameof(SettingsSnapshot.CorpusLearningEnabled), CorpusLearning);
        Toggle(nameof(SettingsSnapshot.CorpusLearningPaused), CorpusPaused);
        Toggle(nameof(SettingsSnapshot.CorpusPredictionEnabled), CorpusPrediction);
        _settingsEditors.Bind<IReadOnlyList<string>>(nameof(SettingsSnapshot.CorpusAllowedApps),
            () => CorpusAllowedApps.Text.Split(';', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries),
            values => CorpusAllowedApps.Text = string.Join(';', values), CorpusAllowedApps);
        _settingsEditors.Bind<IReadOnlyList<string>>(nameof(SettingsSnapshot.CorpusExcludedApps),
            () => CorpusExcludedApps.Text.Split(';', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries),
            values => CorpusExcludedApps.Text = string.Join(';', values), CorpusExcludedApps);
        Number(nameof(SettingsSnapshot.CorpusRetentionDays), CorpusRetention, 0, 3650, "语料保留天数");
        _settingsEditors.Bind(nameof(SettingsSnapshot.StyleSceneMask), StyleSceneMask, ApplyStyleSceneMask,
            _styleSceneToggles.Cast<Control>().ToArray());
        _settingsEditors.Bind(nameof(SettingsSnapshot.StylePreferences), StylePreferences,
            value => StylePreferencesBox.Text = value, StylePreferencesBox);
        Toggle(nameof(SettingsSnapshot.ToneReplyEnabled), ToneReply);
        Toggle(nameof(SettingsSnapshot.ToneContinuationEnabled), ToneContinuation);
        Toggle(nameof(SettingsSnapshot.ToneCloudModelEnabled), ToneCloudModel);
        Card(nameof(SettingsSnapshot.CorpusLearningEnabled), CorpusLearningCard, "语料学习", "语料学习");
        Card(nameof(SettingsSnapshot.CorpusLearningPaused), CorpusPauseCard, "暂停语料学习", "暂停语料学习", () => CorpusLearning.IsOn, "请先开启语料学习和所需的草稿功能。");
        Card(nameof(SettingsSnapshot.CorpusAllowedApps), CorpusAllowedCard, "允许学习的应用", "允许学习的应用", () => CorpusLearning.IsOn, "请先开启语料学习和所需的草稿功能。");
        Card(nameof(SettingsSnapshot.CorpusExcludedApps), CorpusExcludedCard, "排除的应用", "排除的应用", () => CorpusLearning.IsOn, "请先开启语料学习和所需的草稿功能。");
        Card(nameof(SettingsSnapshot.CorpusRetentionDays), CorpusRetentionCard, "语料保留天数", "语料保留天数", () => CorpusLearning.IsOn, "请先开启语料学习和所需的草稿功能。");
        Card(nameof(SettingsSnapshot.CorpusPredictionEnabled), CorpusPredictionCard, "下句预测", "下句预测", () => CorpusLearning.IsOn, "请先开启语料学习和所需的草稿功能。");
        Card(nameof(SettingsSnapshot.StyleSceneMask), StyleSceneFields, "我的风格场景", "我的风格场景", () => CorpusLearning.IsOn, "请先开启语料学习和所需的草稿功能。");
        Card(nameof(SettingsSnapshot.StylePreferences), StylePreferencesBox, "我的写作偏好", "我的写作偏好");
        Card(nameof(SettingsSnapshot.ToneReplyEnabled), ToneReplyCard, "语气回复草稿", "语气回复草稿", () => CorpusLearning.IsOn, "请先开启语料学习和所需的草稿功能。");
        Card(nameof(SettingsSnapshot.ToneContinuationEnabled), ToneContinuationCard, "语气续写草稿", "语气续写草稿", () => CorpusLearning.IsOn, "请先开启语料学习和所需的草稿功能。");
        Card(nameof(SettingsSnapshot.ToneCloudModelEnabled), ToneCloudModelCard, "语气草稿云端模型", "语气草稿云端模型", () => CorpusLearning.IsOn && (ToneReply.IsOn || ToneContinuation.IsOn), "请先开启语料学习和所需的草稿功能。");

        void Shortcut(string enabledKey, string bindingsKey, ToggleSwitch enabled, string group,
                      FrameworkElement card, string title, string keywords) {
            Toggle(enabledKey, enabled);
            _settingsEditors.Bind<IReadOnlyList<string>>(bindingsKey, () => ShortcutValues(group),
                values => SetShortcutBindings(group, values));
            Card(enabledKey, card, title, keywords);
        }
        Shortcut(nameof(SettingsSnapshot.CorrectionShortcutEnabled), nameof(SettingsSnapshot.CorrectionShortcuts), CorrectionShortcutEnabled, "correction", CorrectionShortcutCard, "切换输入纠错", "快捷键 全拼 严格输入");
        Shortcut(nameof(SettingsSnapshot.LanguageShortcutEnabled), nameof(SettingsSnapshot.LanguageShortcuts), LanguageShortcutEnabled, "language", LanguageShortcutCard, "切换中英文输入", "快捷键 语言");
        Shortcut(nameof(SettingsSnapshot.RawInputShortcutEnabled), nameof(SettingsSnapshot.RawInputShortcuts), RawInputShortcutEnabled, "raw", RawInputShortcutCard, "输入编码原样上屏", "快捷键 Enter");
        Shortcut(nameof(SettingsSnapshot.CursorLeftShortcutEnabled), nameof(SettingsSnapshot.CursorLeftShortcuts), CursorLeftShortcutEnabled, "cursor-left", CursorLeftShortcutCard, "预编辑光标左移", "快捷键 编码 光标");
        Shortcut(nameof(SettingsSnapshot.CursorRightShortcutEnabled), nameof(SettingsSnapshot.CursorRightShortcuts), CursorRightShortcutEnabled, "cursor-right", CursorRightShortcutCard, "预编辑光标右移", "快捷键 编码 光标");
        Shortcut(nameof(SettingsSnapshot.PreviousPageShortcutEnabled), nameof(SettingsSnapshot.PreviousPageShortcuts), PreviousPageShortcutEnabled, "previous-page", PreviousPageShortcutCard, "候选词上一页", "快捷键 翻页");
        Shortcut(nameof(SettingsSnapshot.NextPageShortcutEnabled), nameof(SettingsSnapshot.NextPageShortcuts), NextPageShortcutEnabled, "next-page", NextPageShortcutCard, "候选词下一页", "快捷键 翻页");
        Shortcut(nameof(SettingsSnapshot.ExpandedPreviousPageShortcutEnabled), nameof(SettingsSnapshot.ExpandedPreviousPageShortcuts), ExpandedPreviousPageShortcutEnabled, "expanded-previous-page", ExpandedPreviousPageShortcutCard, "展开并选中上一候选页", "快捷键 上方向键 展开 高亮 选中 页");
        Shortcut(nameof(SettingsSnapshot.ExpandedNextPageShortcutEnabled), nameof(SettingsSnapshot.ExpandedNextPageShortcuts), ExpandedNextPageShortcutEnabled, "expanded-next-page", ExpandedNextPageShortcutCard, "展开并选中下一候选页", "快捷键 下方向键 展开 高亮 选中 页");
        Shortcut(nameof(SettingsSnapshot.PunctuationShortcutEnabled), nameof(SettingsSnapshot.PunctuationShortcuts), PunctuationShortcutEnabled, "punctuation", PunctuationShortcutCard, "切换中文标点", "快捷键");
        Shortcut(nameof(SettingsSnapshot.TextNavigationShortcutEnabled), nameof(SettingsSnapshot.TextNavigationShortcuts), TextNavigationShortcutEnabled, "text-navigation", TextNavigationShortcutCard, "移动应用文本光标", "快捷键 Shift 方向键 选中");
        Toggle(nameof(SettingsSnapshot.TextNavigationSelectEnabled), TextNavigationSelectEnabled);
        Card(nameof(SettingsSnapshot.TextNavigationSelectEnabled), TextNavigationSelectEnabled,
            "移动文本光标时扩展选区", "快捷键 Shift 方向键 选中文字", () => TextNavigationShortcutEnabled.IsOn,
            "请先开启移动应用文本光标快捷键。");
        _settingsModules.ValidateEditors(_settingsEditors);
    }

    private void ApplySettings(SettingsSnapshot value)
    {
        var loaded = _settingsLoaded;
        _settingsLoaded = false;
        try {
            CancelShortcutCapture();
            _settingsModules.Load(value, _settingsEditors);
            _settingsBaseline = value;
            UpdateShortcutButtons();
            UpdateInputSchemePresentation();
        } finally { _settingsLoaded = loaded; }
    }

    private SettingsSnapshot CaptureSettings()
    {
        var snapshot = _settingsModules.Capture(_settingsBaseline ??
            throw new InvalidOperationException("配置尚未加载完成，请重新加载后再保存。"), _settingsEditors);
        ShortcutSettingsValidation.Validate(snapshot);
        return snapshot;
    }

    private void ApplyPageDefaults(SettingsSnapshot defaults)
    {
        var loaded = _settingsLoaded;
        _settingsLoaded = false;
        try {
            CancelShortcutCapture();
            _settingsModules.ResetPage(_currentPage, defaults, _settingsEditors, CurrentScheme);
            UpdateShortcutButtons();
            UpdateInputSchemePresentation();
        } finally { _settingsLoaded = loaded; }
    }

    private void UpdateDependentSettings()
    {
        if (_settingsCards.Count == 0) return;
        foreach (var binding in _settingsCards) {
            var supported = CurrentScheme.Supports(_settingsModules.Field(binding.Key).Capability);
            var dependencyMet = binding.Dependency?.Invoke() ?? true;
            var enabled = supported && dependencyMet;
            SetDescendantControlsEnabled(binding.Card, enabled);
            binding.Card.Opacity = enabled ? 1 : 0.55;
            ToolTipService.SetToolTip(binding.Card, !supported
                ? $"当前方案“{CurrentScheme.Name}”不支持此宿主功能；原配置已保留。"
                : !dependencyMet ? binding.DependencyHint : null);
        }
        var corpusLearning = CurrentScheme.Supports(SettingsCapability.UserLearning) && CorpusLearning.IsOn;
        StyleProfileCard.IsHitTestVisible = corpusLearning;
        StyleProfileCard.Opacity = corpusLearning ? 1 : 0.55;
        UpdateStyleNotice();
        UpdateToneSendingScope();
        UpdatePunctuationNotice();
    }

    private static void SetDescendantControlsEnabled(DependencyObject node, bool enabled)
    {
        if (node is Control control) { control.IsEnabled = enabled; return; }
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(node); ++index)
            SetDescendantControlsEnabled(VisualTreeHelper.GetChild(node, index), enabled);
    }

    private IEnumerable<SearchItem> SettingsSearchItems() => _baseSearchItems.Concat(_pluginSearchItems)
        .Select(item => CurrentScheme.Supports(item.Capability) ? item
            : item with { Detail = item.Detail + " · 当前方案不支持，原配置保留" });
}
