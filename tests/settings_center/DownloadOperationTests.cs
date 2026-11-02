using OwO_Settings;

var checks = 0;
void Check(bool condition, string message) { if (!condition) throw new Exception(message); checks++; }
using (var task = new DownloadOperation("test.bin", CancellationToken.None)) {
    task.Report(64, 256); task.TogglePause();
    var waiting = task.WaitIfPausedAsync(task.Token);
    Check(!waiting.IsCompleted && task.Snapshot.State == DownloadState.Paused, "Pause gate must block actual IO");
    task.TogglePause(); await waiting.WaitAsync(TimeSpan.FromSeconds(2));
    Check(task.Snapshot.State == DownloadState.Downloading && task.Snapshot.Percent == 25, "Resume retains bytes");
    task.Processing(); Check(!task.Snapshot.CanPause && !task.Snapshot.Terminal, "Verification is cancellable, not pausable");
    Check(await task.RunAsync(_ => Task.FromResult(42)) == 42 && task.Snapshot.State == DownloadState.Completed, "Completion");
    task.TogglePause(); task.Cancel(); task.Report(100, 100);
    Check(task.Snapshot.State == DownloadState.Completed && task.Snapshot.Bytes == 64, "Late events cannot revive a terminal task");
}
using (var task = new DownloadOperation("cancel.bin", CancellationToken.None)) {
    task.TogglePause();
    var running = task.RunAsync(async ct => { await task.WaitIfPausedAsync(ct); return 1; });
    task.Cancel();
    try { await running.WaitAsync(TimeSpan.FromSeconds(2)); throw new Exception("Cancellation ignored"); } catch (OperationCanceledException) { checks++; }
    Check(task.Snapshot.State == DownloadState.Cancelled, "Cancel while paused terminates without resume");
}
using (var cancelled = new CancellationTokenSource()) using (var task = new DownloadOperation("external.bin", cancelled.Token)) {
    task.TogglePause(); var running = task.RunAsync(async ct => { await task.WaitIfPausedAsync(ct); return 1; }); cancelled.Cancel();
    try { await running.WaitAsync(TimeSpan.FromSeconds(2)); throw new Exception("External cancellation ignored"); } catch (OperationCanceledException) { checks++; }
}
using (var task = new DownloadOperation("timeout.bin", CancellationToken.None, TimeSpan.FromMilliseconds(500))) {
    task.TogglePause(); await Task.Delay(650);
    Check(!task.Token.IsCancellationRequested, "Pause does not spend the active transfer timeout");
    task.TogglePause();
    try { await task.RunAsync(async ct => { await Task.Delay(2000, ct); return 1; }); throw new Exception("Active timeout ignored"); }
    catch (OperationCanceledException) { checks++; }
}
using (var task = new DownloadOperation("failure.bin", CancellationToken.None)) {
    try { await task.RunAsync<int>(_ => throw new InvalidDataException("hash mismatch")); throw new Exception("Failure ignored"); }
    catch (InvalidDataException) { checks++; }
    Check(task.Snapshot.State == DownloadState.Failed, "Verification errors cannot be success");
}
var clock = new TestClock(); var deadline = new SettingsNoticeDeadline(clock);
deadline.Show(SettingsNoticeKind.Success); clock.Advance(3); Check(!deadline.Expired, "Success retained before deadline");
deadline.Show(SettingsNoticeKind.Error); clock.Advance(2); Check(!deadline.Expired, "Old success tick cannot dismiss replacement error");
clock.Advance(13); Check(deadline.Expired, "Errors expire after 15 seconds");
foreach (var kind in Enum.GetValues<SettingsNoticeKind>()) {
    deadline.Show(kind); clock.Advance(SettingsNotificationPolicy.Lifetime(kind).TotalSeconds);
    Check(deadline.Expired, "Every notice kind has a bounded lifetime");
}
Check(SettingsNotificationPolicy.Retention(DownloadState.Failed) == TimeSpan.FromSeconds(15), "Failed downloads are retained briefly");
Console.WriteLine($"PASS: {checks} transfer state, pause/cancel/timeout and notification deadline checks.");

sealed class TestClock : TimeProvider {
    private long _now;
    public override long TimestampFrequency => TimeSpan.TicksPerSecond;
    public override long GetTimestamp() => _now;
    internal void Advance(double seconds) => _now += TimeSpan.FromSeconds(seconds).Ticks;
}
