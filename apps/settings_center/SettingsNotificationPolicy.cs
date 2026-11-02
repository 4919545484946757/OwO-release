namespace OwO_Settings;

internal enum SettingsNoticeKind { Information, Success, Warning, Error }
internal static class SettingsNotificationPolicy
{
    internal static TimeSpan Lifetime(SettingsNoticeKind kind) => TimeSpan.FromSeconds(kind switch {
        SettingsNoticeKind.Success => 4, SettingsNoticeKind.Warning => 10, SettingsNoticeKind.Error => 15, _ => 6
    });
    internal static TimeSpan Retention(DownloadState state) => TimeSpan.FromSeconds(state switch {
        DownloadState.Failed => 15, DownloadState.Cancelled => 6, _ => 8
    });
}

// Monotonic and replaceable: a queued tick for an old message cannot dismiss
// its replacement, and a wall-clock adjustment cannot pin a notification.
internal sealed class SettingsNoticeDeadline(TimeProvider? clock = null)
{
    private readonly TimeProvider _clock = clock ?? TimeProvider.System;
    private long _start;
    private TimeSpan _duration;
    private bool _active;
    internal void Show(SettingsNoticeKind kind) { _start = _clock.GetTimestamp(); _duration = SettingsNotificationPolicy.Lifetime(kind); _active = true; }
    internal bool Expired => _active && _clock.GetElapsedTime(_start) >= _duration;
}
