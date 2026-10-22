using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace OwO_Settings;

// "我的风格": the explainable style profile derived from the authorized corpus,
// the user's own written preferences, and per-scene switches. The profile is
// read-only; everything the user can change is a normal configuration field,
// so it participates in auto-save and the save/reload buttons like any card.
public sealed partial class MainPage
{
    // Mirrors engine::StyleScene order and config::kStyleSceneAll.
    private static readonly (uint Bit, string Name, string Detail)[] StyleScenes = [
        (1, "聊天", "微信、QQ、Telegram 等即时聊天应用"),
        (2, "工作", "Word、Outlook、飞书、钉钉等办公应用"),
        (4, "其他", "未归入以上两类的应用"),
    ];
    private readonly List<ToggleSwitch> _styleSceneToggles = [];
    // The profile viewer is read-only, so its scene toggles and preference box
    // are the only controls that feed CaptureSettings.
    private void InitializeStyleProfilePage()
    {
        foreach (var scene in StyleScenes) {
            var grid = new Grid { ColumnSpacing = 16 };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var label = new StackPanel { Spacing = 3 };
            label.Children.Add(new TextBlock {
                Text = scene.Name, Style = (Style)Resources["SettingTitleStyle"]
            });
            label.Children.Add(new TextBlock {
                Text = scene.Detail, Style = (Style)Resources["SettingDescriptionStyle"]
            });
            var toggle = new ToggleSwitch { IsOn = true, Tag = scene.Bit };
            toggle.Toggled += (_, _) => {
                UpdateStyleNotice();
                UpdateToneSendingScope();
            };
            Grid.SetColumn(toggle, 1);
            grid.Children.Add(label);
            grid.Children.Add(toggle);
            StyleSceneFields.Children.Add(new Border {
                Style = (Style)Resources["SettingsCardStyle"],
                Child = grid,
            });
            _styleSceneToggles.Add(toggle);
        }
        UpdateStyleNotice();
    }

    private uint StyleSceneMask() => _styleSceneToggles.Aggregate(0u,
        (mask, toggle) => toggle.IsOn ? mask | (uint)toggle.Tag : mask);

    private void ApplyStyleSceneMask(uint mask)
    {
        foreach (var toggle in _styleSceneToggles) toggle.IsOn = (mask & (uint)toggle.Tag) != 0;
        UpdateStyleNotice();
    }

    private string StylePreferences() =>
        ConfigShellClient.NormalizeStylePreferences(StylePreferencesBox.Text).Trim();

    private void UpdateStyleNotice()
    {
        if (StyleProfileNotice is null || StyleProfileStatus is null) return;
        var enabled = _styleSceneToggles.Where(toggle => toggle.IsOn)
            .Select(toggle => StyleScenes.First(scene => scene.Bit == (uint)toggle.Tag).Name)
            .ToArray();
        StyleProfileNotice.Message = enabled.Length == 0
            ? "所有场景都已关闭：档案不会统计任何句子，按语气起草也不会取用例句。"
            : $"档案只统计已选场景（{string.Join("、", enabled)}）里的句子；所有数字都在本机计算，不上传。";
        StyleProfileStatus.Message = AutoSaveSettings?.IsOn == true
            ? "场景与偏好修改后会自动保存；档案按最新语料统计，随时可刷新。"
            : "场景与偏好修改后点击保存；档案按最新语料统计，随时可刷新。";
    }

    // The corpus shell prints the report; this only surfaces it. A slow or
    // failed run must never leave the page looking like an empty profile.
    private async Task RefreshStyleProfileAsync()
    {
        if (StyleProfileText is null) return;
        StyleProfileText.Text = "正在统计本地语料…";
        try {
            StyleProfileText.Text = await _corpusClient.DescribeStyleAsync();
        } catch (Exception error) {
            StyleProfileText.Text = "暂时无法读取风格档案（" + error.Message + "）。";
        }
    }

    private async void OpenStyleProfile_Click(object sender, RoutedEventArgs e)
    {
        ShowPage("style");
        await RefreshStyleProfileAsync();
    }

    private async void RefreshStyleProfile_Click(object sender, RoutedEventArgs e) =>
        await RefreshStyleProfileAsync();

    // Shows the exact local request so the user can decide whether the cloud
    // switch is acceptable. The button stays disabled when drafting is off,
    // because then there is no request to preview.
    private async void TonePreview_Click(object sender, RoutedEventArgs e)
    {
        if (TonePreviewText is null || TonePreviewMessage is null) return;
        if (!ToneReply.IsOn && !ToneContinuation.IsOn) {
            TonePreviewText.Text = "请先在上面打开“按我的语气回复”或“整句续写”。";
            return;
        }
        var continuation = TonePreviewContinuation?.IsOn == true;
        if (continuation && !ToneContinuation.IsOn) {
            TonePreviewText.Text = "“整句续写”在设置中没有打开。";
            return;
        }
        if (!continuation && !ToneReply.IsOn) {
            TonePreviewText.Text = "“按我的语气回复”在设置中没有打开。";
            return;
        }
        TonePreviewButton.IsEnabled = false;
        TonePreviewText.Text = "正在读取本地语料…";
        try {
            TonePreviewText.Text = await _corpusClient.DescribeTonePromptAsync(
                TonePreviewMessage.Text, continuation);
        } catch (Exception error) {
            TonePreviewText.Text = "暂时无法组装请求（" + error.Message + "）。";
        } finally {
            TonePreviewButton.IsEnabled = true;
        }
    }

    private void StyleProfileBack_Click(object sender, RoutedEventArgs e) =>
        ShowPage("intelligence");

    private void StylePreferences_TextChanged(object sender, TextChangedEventArgs e)
    {
        if (StylePreferencesCount is not null)
            StylePreferencesCount.Text = $"{StylePreferencesBox.Text.Length}/512";
    }

    // Drafting sends exactly these items when cloud sending is allowed; the
    // list is shown next to the switch so the scope is never implicit.
    private void UpdateToneSendingScope()
    {
        if (ToneSendingScope is null) return;
        var scenes = _styleSceneToggles.Where(toggle => toggle.IsOn)
            .Select(toggle => StyleScenes.First(scene => scene.Bit == (uint)toggle.Tag).Name)
            .ToArray();
        ToneSendingScope.Text =
            "发送范围：当前选中的消息或输入的开头、风格摘要（句长、标点与口头语统计）、" +
            "最多 4 句你自己写过的同类场景句子、最多 3 条反馈说法、手写偏好和固定指令。" +
            (scenes.Length == 0
                 ? "所有场景已关闭，因此没有例句可发。"
                 : $"只会取用这些场景的句子：{string.Join("、", scenes)}。") +
            "只取同场景且符合当前应用允许/排除列表的例句；密码框和敏感内容不发送。";
    }

    private void StyleProfileCard_Tapped(object sender, Microsoft.UI.Xaml.Input.TappedRoutedEventArgs e)
    {
        // Only a tap on the card body opens the page; the explicit button does
        // the same through its own Click handler.
        for (var node = e.OriginalSource as DependencyObject; node is not null;
             node = VisualTreeHelper.GetParent(node)) {
            if (node is Button) return;
            if (node == StyleProfileCard) break;
        }
        OpenStyleProfile_Click(sender, new RoutedEventArgs());
    }
}
