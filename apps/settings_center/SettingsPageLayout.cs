using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private FrameworkElement _candidateSettingsLayout = null!;
    private FrameworkElement _learningSettingsLayout = null!;
    private FrameworkElement _nativeShortcutLayout = null!;

    private void InitializePageLayouts()
    {
        // Rearrange the existing fields once. Names, search targets, focus,
        // persisted values and event subscriptions stay attached to the same controls.
        NativeSettingsContent.Children.Clear();
        InputSchemeCard.Children.Remove(InputSchemeActions);
        InputSchemeCard.Children.Remove(InputSchemeRows);
        InputSchemeCard.Children.Add(SettingsSection("方案顺序",
            "切换快捷键按此顺序轮换已启用方案；第一个已启用方案用于新窗口。", [InputSchemeActions, InputSchemeRows]));
        InputSchemeActions.SizeChanged += (_, e) => {
            var orientation = e.NewSize.Width >= 280 ? Orientation.Horizontal : Orientation.Vertical;
            if (InputSchemeActions.Orientation != orientation) InputSchemeActions.Orientation = orientation;
        };

        _candidateSettingsLayout = SettingsStack(12,
            SettingsSection("候选栏", "先设置拼音显示位置，再调整候选数量与长词换行。",
                [PinyinPreviewLocationCard, CandidatePageSizeCard, CandidateWrapLengthCard], "CandidateDisplayLayout"),
            SettingsColumns(
                SettingsSection("列表顺序", "控制词语与单字在一页中的位置。", [CandidatePartitionCard, CandidateSingleCharacterFirstCard], "CandidateOrderLayout"),
                SettingsSection("确认与上屏", "控制分段选词和离开输入位置时的处理。", [CandidateImmediateCommitCard, FocusLossCommitCard], "CandidateCommitLayout")),
            SettingsColumns(
                SettingsSection("拼音与标点", "选择如何将符号带入当前输入。", [PinyinSymbolCompatibilityCard, ChinesePunctuationCard]),
                SettingsSection("Alt 数字符号", "调整代码结果预览和浏览行为。", [AltCodePreviewCard, AltCodePreviewPageSizeCard, AltCodePreviewWheelCard])));
        _learningSettingsLayout = SettingsStack(12,
            SettingsColumns(
                SettingsSection("词频学习", "调整已上屏词语对后续候选的影响。", [UserLearningCard, LearningSensitivityCard], "LearningFrequencyLayout"),
                SettingsSection("用户词补全", "控制只输入词语的一部分时，是否以及何时出现已学词语。", [UserCompletionCard, UserCompletionRangeCard], "LearningCompletionLayout")),
            SettingsSection("上下文排序", "本地排序与选区大模型功能独立。", [ModelRankingCard, ModelTimeoutCard], "LearningRankingLayout"));
        NativeSettingsContent.Children.Add(InputSchemeCard);
        NativeSettingsContent.Children.Add(_candidateSettingsLayout);
        NativeSettingsContent.Children.Add(_learningSettingsLayout);

        // The global binding remains visible for plugin and pure keyboard
        // scopes. Everything else in the host groups is scoped by the scheme.
        foreach (var element in new UIElement[] { NativeShortcutHint, LanguageShortcutCard, SchemeShortcutHost,
            CorrectionShortcutCard, PunctuationShortcutCard, RawInputShortcutCard, SeparatorShortcutCard,
            CursorLeftShortcutCard, CursorRightShortcutCard, PreviousPageShortcutCard, NextPageShortcutCard,
            ExpandedPreviousPageShortcutCard, ExpandedNextPageShortcutCard }) ShortcutContent.Children.Remove(element);
        ShortcutContent.Children.Insert(0, SettingsSection("全局快捷键", "对所有输入方案生效。", [LanguageShortcutCard], "GlobalShortcutLayout"));
        ShortcutContent.Children.Insert(1, SchemeShortcutHost);
        _nativeShortcutLayout = SettingsStack(12, NativeShortcutHint,
            SettingsColumns(
                SettingsSection("输入与模式", null, [CorrectionShortcutCard, PunctuationShortcutCard, RawInputShortcutCard, SeparatorShortcutCard], "CompositionShortcutLayout"),
                SettingsSection("预编辑光标", null, [CursorLeftShortcutCard, CursorRightShortcutCard], "CursorShortcutLayout")),
            SettingsColumns(
                SettingsSection("候选翻页", null, [PreviousPageShortcutCard, NextPageShortcutCard], "PagingShortcutLayout"),
                SettingsSection("展开并选择候选页", null, [ExpandedPreviousPageShortcutCard, ExpandedNextPageShortcutCard], "ExpandedPagingShortcutLayout")));
        ShortcutContent.Children.Insert(2, _nativeShortcutLayout);

        GeneralPreferencesContent.Children.Clear();
        GeneralPreferencesContent.Children.Add(SettingsColumns(
            SettingsSection("使用偏好", null, [AutoSaveCard, _logoThemeCard!], "GeneralPreferenceLayout"),
            SettingsSection("备份与诊断", null, [ConfigurationTransferCard, DiagnosticsEntryCard], "GeneralMaintenanceLayout")));
        GeneralPreferencesContent.Children.Add(OnlineDistributionHost);
    }

    private static StackPanel SettingsStack(double spacing, params UIElement[] children)
    {
        var stack = new StackPanel { Spacing = spacing };
        foreach (var child in children) stack.Children.Add(child);
        return stack;
    }

    private Border SettingsSection(string title, string? description, IReadOnlyList<FrameworkElement> rows, string? id = null)
    {
        var heading = new TextBlock { Text = title, FontSize = 18, TextWrapping = TextWrapping.Wrap };
        AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level2);
        var header = SettingsStack(4, heading);
        header.Padding = new Thickness(16, 14, 16, 13);
        if (description is not null) header.Children.Add(new TextBlock {
            Text = description, Style = (Style)Resources["SettingDescriptionStyle"]
        });
        var body = SettingsStack(0, header);
        foreach (var row in rows) {
            if (row is Border border) {
                border.Style = (Style)Resources["SettingsGroupedRowStyle"];
                body.Children.Add(border);
            } else {
                var wrapper = new Border { Child = row, Style = (Style)Resources["SettingsGroupedRowStyle"] };
                if (ReferenceEquals(row, InputSchemeRows)) wrapper.Padding = wrapper.BorderThickness = new Thickness(0);
                body.Children.Add(wrapper);
            }
        }
        var section = new Border { Child = body, Style = (Style)Resources["SettingsCardStyle"], Padding = new Thickness(0), VerticalAlignment = VerticalAlignment.Top };
        if (id is not null) AutomationProperties.SetAutomationId(heading, id);
        return section;
    }

    // CSS-grid-like breakpoints depend on the actual content width, not the
    // outer window: navigation width and display scaling are already accounted for.
    private static Grid SettingsColumns(FrameworkElement left, FrameworkElement right, double minimumWidth = 690)
    {
        var grid = new Grid { RowSpacing = 12 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(0) });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        grid.Children.Add(left);
        grid.Children.Add(right);
        Grid.SetRow(right, 1);
        var wide = false;
        grid.SizeChanged += (_, e) => {
            var next = e.NewSize.Width >= minimumWidth;
            if (next == wide) return;
            wide = next;
            grid.ColumnDefinitions[1].Width = wide ? new GridLength(1, GridUnitType.Star) : new GridLength(0);
            grid.ColumnSpacing = wide ? 12 : 0;
            grid.RowSpacing = wide ? 0 : 12;
            Grid.SetColumn(right, wide ? 1 : 0);
            Grid.SetRow(right, wide ? 0 : 1);
        };
        return grid;
    }
}
