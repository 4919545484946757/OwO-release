using System.Diagnostics;

namespace OwO_Settings;

internal enum DownloadState { Downloading, Paused, Processing, Completed, Cancelled, Failed }
internal sealed record DownloadSnapshot(string FileName, DownloadState State, long Bytes, long Total, string Detail)
{
    internal bool Terminal => State is DownloadState.Completed or DownloadState.Cancelled or DownloadState.Failed;
    internal bool CanPause => State is DownloadState.Downloading or DownloadState.Paused;
    internal double Percent => Total > 0 ? Math.Clamp(Bytes * 100.0 / Total, 0, 100) : 0;
}

// One task, shared by the page and footer. Pausing stops subsequent stream
// reads/writes; it does not bypass verification or promise cross-process resume.
internal sealed class DownloadOperation : IDisposable
{
    private readonly object _gate = new();
    private readonly CancellationTokenSource _cancel;
    private readonly CancellationToken _token;
    private readonly Stopwatch _active = Stopwatch.StartNew();
    private TimeSpan _remaining;
    private TaskCompletionSource? _resume;
    private DownloadSnapshot _snapshot;
    private long _lastReport;
    internal event Action? Changed;
    internal CancellationToken Token => _token;
    internal DownloadSnapshot Snapshot { get { lock (_gate) return _snapshot; } }
    internal DownloadOperation(string fileName, CancellationToken token, TimeSpan? timeout = null) {
        _snapshot = new(fileName, DownloadState.Downloading, 0, 0, "正在连接");
        _cancel = CancellationTokenSource.CreateLinkedTokenSource(token); _token = _cancel.Token;
        _remaining = timeout ?? TimeSpan.FromMinutes(15); _cancel.CancelAfter(_remaining);
    }
    internal void TogglePause() {
        lock (_gate) {
            if (!_snapshot.CanPause || _token.IsCancellationRequested) return;
            if (_snapshot.State == DownloadState.Paused) {
                _cancel.CancelAfter(_remaining > TimeSpan.Zero ? _remaining : TimeSpan.Zero);
                _active.Restart(); _snapshot = _snapshot with { State = DownloadState.Downloading, Detail = "正在下载" };
                _resume?.TrySetResult(); _resume = null;
            } else {
                _remaining -= _active.Elapsed; _active.Reset(); _cancel.CancelAfter(Timeout.InfiniteTimeSpan);
                _resume = new(TaskCreationOptions.RunContinuationsAsynchronously);
                _snapshot = _snapshot with { State = DownloadState.Paused, Detail = "已暂停" };
            }
        }
        Changed?.Invoke();
    }
    internal async Task WaitIfPausedAsync(CancellationToken token) {
        while (true) {
            token.ThrowIfCancellationRequested(); _token.ThrowIfCancellationRequested();
            Task? wait; lock (_gate) wait = _resume?.Task;
            if (wait is null) return;
            await wait.WaitAsync(token).ConfigureAwait(false);
        }
    }
    internal void Report(long bytes, long total) {
        bool notify;
        lock (_gate) {
            if (_snapshot.Terminal) return;
            _snapshot = _snapshot with { Bytes = bytes, Total = total,
                Detail = _snapshot.State == DownloadState.Paused ? "已暂停" : "正在下载" };
            var now = Stopwatch.GetTimestamp();
            notify = total > 0 && bytes >= total || Stopwatch.GetElapsedTime(_lastReport, now) >= TimeSpan.FromMilliseconds(100);
            if (notify) _lastReport = now;
        }
        if (notify) Changed?.Invoke();
    }
    internal void Processing(string detail = "正在校验") {
        lock (_gate) {
            if (_snapshot.Terminal) return;
            _snapshot = _snapshot with { State = DownloadState.Processing, Detail = detail,
                Total = _snapshot.Total > 0 ? _snapshot.Total : _snapshot.Bytes };
        }
        Changed?.Invoke();
    }
    private void Finish(DownloadState state, string detail) {
        lock (_gate) {
            if (_snapshot.Terminal) return;
            _snapshot = _snapshot with { State = state, Detail = detail }; _cancel.CancelAfter(Timeout.InfiniteTimeSpan);
            _resume?.TrySetResult(); _resume = null;
        }
        Changed?.Invoke();
    }
    internal void Cancel() {
        if (Snapshot.Terminal) return;
        try { _cancel.Cancel(); } catch (ObjectDisposedException) { return; }
        Finish(DownloadState.Cancelled, "已取消");
    }
    internal async Task<T> RunAsync<T>(Func<CancellationToken, Task<T>> action) {
        try {
            _token.ThrowIfCancellationRequested();
            var result = await action(_token); _token.ThrowIfCancellationRequested();
            Finish(DownloadState.Completed, "下载及校验完成"); return result;
        } catch (OperationCanceledException) { Finish(DownloadState.Cancelled, "已取消或超时"); throw; }
        catch { Finish(DownloadState.Failed, "下载失败，可重试"); throw; }
    }
    public void Dispose() {
        if (!Snapshot.Terminal) Cancel();
        _cancel.Dispose();
    }
}
