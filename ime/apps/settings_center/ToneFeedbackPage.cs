using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

// "语气反馈" page (learning layer 5): what the IME learned from earlier tone
// drafts, and the rollback controls. Deleting one record or clearing the file
// only changes later ranking; drafts already sent to a model cannot be undone.
public sealed partial class MainPage
{
    private void InitializeToneFeedbackPage()
    {
        if (ToneFeedbackList is not null)
            ToneFeedbackList.ItemsSource = Array.Empty<ToneFeedbackRow>();
    }

    private async void OpenToneFeedback_Click(object sender, RoutedEventArgs e)
    {
        ShowPage("feedback");
        await RefreshToneFeedbackAsync();
    }

    private async void RefreshToneFeedback_Click(object sender, RoutedEventArgs e) =>
        await RefreshToneFeedbackAsync();

    private void ToneFeedbackBack_Click(object sender, RoutedEventArgs e) =>
        ShowPage("intelligence");

    private void ToneFeedbackCard_Tapped(object sender,
        Microsoft.UI.Xaml.Input.TappedRoutedEventArgs e)
    {
        // A tap on the card body opens the page; the button does the same.
        for (var node = e.OriginalSource as Microsoft.UI.Xaml.DependencyObject; node is not null;
             node = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetParent(node)) {
            if (node is Button) return;
            if (node == ToneFeedbackCard) break;
        }
        OpenToneFeedback_Click(sender, new RoutedEventArgs());
    }

    // Reads the summary and the record list in one pass. A failed read leaves
    // the last list untouched and says why, so an empty page never looks like
    // an empty history.
    private async Task<bool> RefreshToneFeedbackAsync()
    {
        if (ToneFeedbackList is null || ToneFeedbackSummary is null) return false;
        ToneFeedbackRefreshButton.IsEnabled = false;
        ToneFeedbackSummary.Text = "正在读取本机反馈记录…";
        try {
            var summary = await _corpusClient.DescribeToneFeedbackAsync();
            var listed = await _corpusClient.ListToneFeedbackAsync();
            ToneFeedbackSummary.Text = ToneFeedbackText.DescribeSummary(summary);
            ToneFeedbackList.ItemsSource = ToneFeedbackText.ParseRows(listed);
            ToneFeedbackEmpty.Visibility = ToneFeedbackList.Items.Count == 0
                ? Visibility.Visible : Visibility.Collapsed;
            ToneFeedbackClearButton.IsEnabled = ToneFeedbackList.Items.Count != 0;
            ShowFeedbackNotice(
                "记录只在本机，删除后不会恢复；已经发送过草稿的情况不会被撤回。", false);
            return true;
        } catch (Exception error) {
            ToneFeedbackSummary.Text = "暂时无法读取反馈记录（" + error.Message + "）。";
            ToneFeedbackEmpty.Visibility = ToneFeedbackList.Items.Count == 0
                ? Visibility.Visible : Visibility.Collapsed;
            ShowFeedbackNotice("反馈记录读取失败：" + error.Message, true);
            return false;
        } finally {
            ToneFeedbackRefreshButton.IsEnabled = true;
        }
    }

    private void ShowFeedbackNotice(string message, bool warning)
    {
        if (ToneFeedbackNotice is null) return;
        ToneFeedbackNotice.Message = message;
        ToneFeedbackNotice.Severity = warning ? InfoBarSeverity.Warning
                                              : InfoBarSeverity.Informational;
    }

    private async void ToneFeedbackDelete_Click(object sender, RoutedEventArgs e)
    {
        if (sender is not Button { Tag: ToneFeedbackRow row }) return;
        if (!await ConfirmAsync("删除这条反馈",
                $"{row.Title} · {row.Detail}\n\n删除后这条记录不再影响排序；其他记录保持不变。", "删除"))
            return;
        try {
            await _corpusClient.DeleteToneFeedbackAsync(row.Id);
            if (await RefreshToneFeedbackAsync())
                ShowFeedbackNotice("反馈删除操作已完成。", false);
        } catch (Exception error) {
            ShowFeedbackNotice("删除失败：" + error.Message, true);
        }
    }

    private async void ToneFeedbackClear_Click(object sender, RoutedEventArgs e)
    {
        if (!await ConfirmAsync("清空反馈记录",
                "将删除全部语气反馈记录，排序回到模型给出的原始顺序。此操作不可撤销；"
                    + "语料和其他设置不受影响。", "全部清空"))
            return;
        try {
            await _corpusClient.ClearToneFeedbackAsync();
            if (await RefreshToneFeedbackAsync())
                ShowFeedbackNotice("已清空反馈记录。", false);
        } catch (Exception error) {
            ShowFeedbackNotice("清空失败：" + error.Message, true);
        }
    }
}
