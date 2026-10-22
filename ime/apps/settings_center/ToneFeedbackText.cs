using System.Globalization;

namespace OwO_Settings;

// One row of the feedback list. Only the kind, application and time are shown:
// the user needs enough to recognise a record, not its full draft text.
internal sealed record ToneFeedbackRow(ulong Id, string Kind, string Application,
                                       string Created)
{
    public string Title => $"{Kind} · #{Id}";

    public string Detail => string.IsNullOrEmpty(Application)
        ? $"未指定应用（归入“其他”场景）· {Created}"
        : $"{Application} · {Created}";
}

// Formats the output of `owo_corpus_shell feedback summary|metadata`. Kept free of
// any UI dependency so the settings tests can exercise it directly.
internal static class ToneFeedbackText
{
    internal const string Empty = "还没有反馈记录。输入法草稿确认上屏后记录采纳或修改；Esc 放弃记录拒绝，即时 Ctrl+Z 记录撤销。";

    // Turns the shell's key=value summary into one readable sentence. Unknown
    // or missing lines are skipped; the record list is rendered separately.
    internal static string DescribeSummary(string summary)
    {
        uint total = 0, adopted = 0, edited = 0, rejected = 0, undone = 0;
        var scenes = new List<string>();
        foreach (var raw in summary.Split('\n')) {
            var line = raw.TrimEnd('\r');
            if (line.Length == 0) continue;
            var fields = line.Split('\t');
            if (fields.Length >= 4 && fields[0].StartsWith("scene=", StringComparison.Ordinal)) {
                var name = fields[0]["scene=".Length..] switch {
                    "chat" => "聊天", "work" => "工作", _ => "其他",
                };
                scenes.Add(fields[3] == "on" ? $"{name} {fields[2]} 项"
                                             : $"{name}（已关闭）");
                continue;
            }
            var pair = line.Split('=', 2);
            if (pair.Length != 2 || !uint.TryParse(pair[1], NumberStyles.None,
                    CultureInfo.InvariantCulture, out var count)) continue;
            switch (pair[0]) {
                case "records": total = count; break;
                case "adopted": adopted = count; break;
                case "edited": edited = count; break;
                case "rejected": rejected = count; break;
                case "undone": undone = count; break;
            }
        }
        if (total == 0) return Empty;
        var detail = $"共 {total} 条：采纳 {adopted}、修改 {edited}、拒绝 {rejected}、撤销 {undone}。";
        return scenes.Count == 0 ? detail
            : detail + "按场景学到的说法：" + string.Join("、", scenes) + "。";
    }

    // Metadata includes id, created_at_ms, kind and application; no draft or
    // final text leaves the shell process when the settings center lists it.
    internal static IReadOnlyList<ToneFeedbackRow> ParseRows(string listed)
    {
        var rows = new List<ToneFeedbackRow>();
        foreach (var raw in listed.Split('\n')) {
            var line = raw.TrimEnd('\r');
            var fields = line.Split('\t');
            if (fields.Length < 4 || !ulong.TryParse(fields[0], NumberStyles.None,
                    CultureInfo.InvariantCulture, out var id)) continue;
            var kind = fields[2] switch {
                "adopted" => "采纳", "edited" => "修改",
                "rejected" => "拒绝", "undone" => "撤销", _ => fields[2],
            };
            var created = long.TryParse(fields[1], NumberStyles.None,
                CultureInfo.InvariantCulture, out var milliseconds) &&
                milliseconds >= 0 && milliseconds <= 253402300799999L
                ? DateTimeOffset.FromUnixTimeMilliseconds(milliseconds).ToLocalTime()
                    .ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture)
                : "时间未知";
            rows.Add(new ToneFeedbackRow(id, kind, Unescape(fields[3]), created));
        }
        return rows;
    }

    // The shell escapes backslash, tab, carriage return and newline; only the
    // application name is shown, so undo exactly those.
    internal static string Unescape(string value)
    {
        var result = new System.Text.StringBuilder(value.Length);
        for (var index = 0; index < value.Length; ++index) {
            if (value[index] != '\\' || index + 1 >= value.Length) {
                result.Append(value[index]);
                continue;
            }
            result.Append(value[++index] switch {
                't' => '\t', 'n' => '\n', 'r' => '\r', '\\' => '\\', var other => other,
            });
        }
        return result.ToString();
    }
}
