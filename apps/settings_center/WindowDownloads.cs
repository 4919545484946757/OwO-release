using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

internal sealed class WindowDownloads
{
    private readonly ContentControl _host;
    private readonly StackPanel _rows = new() { Spacing = 4 };
    private readonly Dictionary<DownloadOperation, (DownloadProgressView Row, Action Changed)> _tasks = [];
    private readonly List<DispatcherTimer> _timers = [];
    private readonly Action<string, InfoBarSeverity> _notify;
    private bool _closed;
    internal WindowDownloads(ContentControl host, Action<string, InfoBarSeverity> notify) {
        _host = host; _notify = notify;
        host.Content = new ScrollViewer { Content = _rows, MaxHeight = 180, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    }
    internal DownloadOperation Start(string fileName, CancellationToken token) {
        if (_closed) throw new OperationCanceledException();
        var task = new DownloadOperation(fileName, token);
        var row = new DownloadProgressView("FooterDownload" + Guid.NewGuid().ToString("N")); row.Bind(task);
        var ended = false;
        void Changed() => _host.DispatcherQueue.TryEnqueue(() => {
            if (_closed || ended || !task.Snapshot.Terminal) return;
            ended = true; var snapshot = task.Snapshot;
            _notify(snapshot.FileName + "：" + snapshot.Detail, snapshot.State == DownloadState.Failed ? InfoBarSeverity.Error :
                snapshot.State == DownloadState.Completed ? InfoBarSeverity.Success : InfoBarSeverity.Informational);
            var timer = new DispatcherTimer { Interval = SettingsNotificationPolicy.Retention(snapshot.State) };
            timer.Tick += (_, _) => { timer.Stop(); _timers.Remove(timer); task.Changed -= Changed; row.Bind(null);
                _rows.Children.Remove(row); _tasks.Remove(task); _host.Visibility = _rows.Children.Count == 0 ? Visibility.Collapsed : Visibility.Visible; };
            _timers.Add(timer); timer.Start();
        });
        _tasks.Add(task, (row, Changed)); task.Changed += Changed;
        _rows.Children.Add(row); _host.Visibility = Visibility.Visible; return task;
    }
    internal void Close() {
        _closed = true;
        foreach (var timer in _timers) timer.Stop(); _timers.Clear();
        foreach (var (task, binding) in _tasks) { task.Changed -= binding.Changed; binding.Row.Bind(null); task.Cancel(); }
        _tasks.Clear(); _rows.Children.Clear(); _host.Visibility = Visibility.Collapsed;
    }
}
