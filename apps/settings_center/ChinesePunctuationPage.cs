using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;

namespace OwO_Settings;

public sealed partial class MainPage
{
    // Keep this append-only order aligned with config::kChinesePunctuationSources.
    private const string PunctuationSources = ",.<>?;:[]{}\\|-_+=`~'\"()!$^";
    private readonly Dictionary<char, ToggleSwitch> _punctuationSwitches = [];
    private readonly Dictionary<char, Border> _punctuationCards = [];
    private static readonly (char Source, string Name, string Output)[] PunctuationOptions = [
        (',', "逗号", "，"), ('.', "句号", "。"),
        ('<', "左书名号", "《"), ('>', "右书名号", "》"),
        ('?', "问号", "？"), (';', "分号", "；"), (':', "冒号", "："),
        ('[', "左方括号", "【"), (']', "右方括号", "】"),
        ('{', "左花括号", "【"), ('}', "右花括号", "】"),
        ('\\', "顿号", "、"), ('|', "竖线", "｜"),
        ('-', "连字符", "－"), ('_', "破折号", "——"),
        ('+', "加号", "＋"), ('=', "等号", "＝"),
        ('`', "间隔号", "·"), ('~', "波浪号", "～"),
        ('\'', "单引号", "‘ ’"), ('"', "双引号", "“ ”"),
        ('(', "左圆括号", "（"), (')', "右圆括号", "）"),
        ('!', "感叹号", "！"), ('$', "货币符号", "￥"), ('^', "省略号", "……")
    ];

    private void InitializePunctuationSettings()
    {
        foreach (var option in PunctuationOptions) {
            var grid = new Grid { ColumnSpacing = 16 };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var label = new StackPanel { Spacing = 3 };
            label.Children.Add(new TextBlock {
                Text = option.Name, Style = (Style)Resources["SettingTitleStyle"]
            });
            label.Children.Add(new TextBlock {
                Text = $"{option.Source} → {option.Output}；关闭后保留英文格式。",
                Style = (Style)Resources["SettingDescriptionStyle"]
            });
            var toggle = new ToggleSwitch { IsOn = true, OnContent = "开", OffContent = "关" };
            AutomationProperties.SetName(toggle, $"{option.Name}使用中文格式");
            Grid.SetColumn(toggle, 1);
            grid.Children.Add(label);
            grid.Children.Add(toggle);
            var card = new Border { Style = (Style)Resources["SettingsCardStyle"], Child = grid };
            ChinesePunctuationFields.Children.Add(card);
            _punctuationSwitches.Add(option.Source, toggle);
            _punctuationCards.Add(option.Source, card);
        }
        UpdatePunctuationNotice();
    }

    private uint PunctuationMask() => _punctuationSwitches.Aggregate(0u,
        (mask, entry) => entry.Value.IsOn ? mask | (1u << PunctuationSources.IndexOf(entry.Key)) : mask);

    private void ApplyPunctuationMask(uint mask)
    {
        foreach (var entry in _punctuationSwitches)
            entry.Value.IsOn = (mask & (1u << PunctuationSources.IndexOf(entry.Key))) != 0;
    }

    private void UpdatePunctuationNotice()
    {
        if (ChinesePunctuationNotice is null || ChinesePunctuation is null) return;
        if (!CurrentScheme.Supports(SettingsCapability.ChinesePunctuation)) {
            ChinesePunctuationNotice.Message = "当前方案不使用宿主中文标点转换。以下偏好保持不变；方案自有标点请在对应插件中配置。";
            return;
        }
        var saveHint = AutoSaveSettings?.IsOn == true ? "修改后会自动保存。" : "修改后点击保存。";
        ChinesePunctuationNotice.Message = ChinesePunctuation.IsOn
            ? $"总开关已开启：中文模式使用下方已开启的中文标点。{saveHint}英文模式不受影响。"
            : "总开关已关闭：目前保留英文标点。下方偏好仍会保存，重新打开总开关后生效。";
    }

    private void ChinesePunctuation_Toggled(object sender, RoutedEventArgs e) => UpdatePunctuationNotice();
    private void OpenChinesePunctuation_Click(object sender, RoutedEventArgs e) => ShowPage("punctuation");
    private void ChinesePunctuationBack_Click(object sender, RoutedEventArgs e) => ShowPage("candidates");

    private void ChinesePunctuationCard_Tapped(object sender, TappedRoutedEventArgs e)
    {
        // Switch clicks (including its text/knob) change only the master switch.
        for (var node = e.OriginalSource as DependencyObject; node is not null;
             node = VisualTreeHelper.GetParent(node)) {
            if (node is ToggleSwitch or Button) return;
            if (node == ChinesePunctuationCard) break;
        }
        ShowPage("punctuation");
    }
}
