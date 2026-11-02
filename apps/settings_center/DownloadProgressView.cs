using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;

namespace OwO_Settings;

// The same native WinUI template is used in every page and in the footer.
internal sealed class DownloadProgressView : UserControl
{
    private readonly TextBlock _name = new() { TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly TextBlock _detail = new() { FontSize = 12, Opacity = 0.75, TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly ProgressBar _progress = new() { Minimum = 0, Maximum = 100, VerticalAlignment = VerticalAlignment.Center };
    private readonly Button _pause = new() { Content = "暂停", MinWidth = 60 }, _cancel = new() { Content = "取消", MinWidth = 60 };
    private DownloadOperation? _task;
    private int _queued;
    internal DownloadProgressView(string id) {
        var grid = new Grid { ColumnSpacing = 8, RowSpacing = 4, Padding = new Thickness(4), MinWidth = 0 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        grid.Children.Add(_name); Grid.SetColumnSpan(_name, 3);
        Grid.SetRow(_progress, 1); grid.Children.Add(_progress);
        Grid.SetRow(_pause, 1); Grid.SetColumn(_pause, 1); grid.Children.Add(_pause);
        Grid.SetRow(_cancel, 1); Grid.SetColumn(_cancel, 2); grid.Children.Add(_cancel);
        Grid.SetRow(_detail, 2); Grid.SetColumnSpan(_detail, 3); grid.Children.Add(_detail);
        Content = grid; HorizontalContentAlignment = HorizontalAlignment.Stretch; Visibility = Visibility.Collapsed;
        AutomationProperties.SetAutomationId(this, id);
        AutomationProperties.SetAutomationId(_pause, id + "PauseResume"); AutomationProperties.SetAutomationId(_cancel, id + "Cancel");
        _pause.Click += (_, _) => _task?.TogglePause(); _cancel.Click += (_, _) => _task?.Cancel();
    }
    internal void Bind(DownloadOperation? task) {
        if (_task is not null) _task.Changed -= Schedule;
        _task = task; if (_task is not null) _task.Changed += Schedule;
        Refresh();
    }
    private void Schedule() {
        if (Interlocked.Exchange(ref _queued, 1) != 0) return;
        if (!DispatcherQueue.TryEnqueue(() => { Interlocked.Exchange(ref _queued, 0); Refresh(); })) Interlocked.Exchange(ref _queued, 0);
    }
    private void Refresh() {
        Visibility = _task is null ? Visibility.Collapsed : Visibility.Visible;
        if (_task is null) return;
        var value = _task.Snapshot; _name.Text = value.FileName;
        ToolTipService.SetToolTip(_name, value.FileName);
        _progress.IsIndeterminate = value.Total <= 0 && !value.Terminal; _progress.Value = value.Percent;
        static string Size(long bytes) => bytes >= 1048576 ? $"{bytes / 1048576.0:F1} MiB" : $"{bytes / 1024.0:F1} KiB";
        _detail.Text = value.Detail + " · " + Size(value.Bytes) + (value.Total > 0 ? $" / {Size(value.Total)} · {value.Percent:F0}%" : "");
        _pause.Content = value.State == DownloadState.Paused ? "继续" : "暂停";
        _pause.IsEnabled = value.CanPause; _cancel.IsEnabled = !value.Terminal;
        AutomationProperties.SetName(_pause, $"{_pause.Content}下载 {value.FileName}");
        AutomationProperties.SetName(_cancel, $"取消下载 {value.FileName}");
    }
}
