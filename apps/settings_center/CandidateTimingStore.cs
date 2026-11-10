using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.Json;

namespace OwO_Settings;

// Internal diagnostic protocol, deliberately independent of settings snapshots,
// candidate IPC and WinUI. Only fixed event names and numeric metrics are read.
internal sealed record CandidateTimingEvent(string Session, uint Pid, ulong RequestId,
    long TimestampUs, string Component, string Event, Dictionary<string, long> Values) {
    internal long Get(string name) => Values.GetValueOrDefault(name);
}

internal sealed record CandidateTimingRow(ulong RequestId, uint Pid, long StartUs, long Page,
    long InputUnits, string Status, double? FirstMs, double? SettledMs, double? ModelMs,
    Dictionary<string, double> Stages, int CacheHits, int Rounds);

internal sealed record CandidateTimingSnapshot(IReadOnlyList<CandidateTimingEvent> Events,
    IReadOnlyList<CandidateTimingRow> Rows, string Warning, long Dropped,
    IReadOnlyList<string> Components);

internal sealed class CandidateTimingStore {
    internal const int MaximumEvents = 50000;
    private const long MaximumReadPerScan = 2 * 1024 * 1024;
    internal static readonly string DefaultRoot = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OwO", "InputMethod", "diagnostics");
    internal string Root { get; }
    internal string? Session { get; private set; }
    internal long Deadline { get; private set; }
    internal bool Running => Session is not null && Environment.TickCount64 < Deadline;
    private readonly List<CandidateTimingEvent> _events = [];
    private readonly Dictionary<string, long> _offsets = [];
    private bool _limited;
    private bool _fileLimit;
    private int _malformed;
    private string ControlPath => Path.Combine(Root, "candidate-timing.control");
    internal CandidateTimingStore(string? root = null) { Root = root ?? DefaultRoot; }

    internal void Start(int minutes) {
        if (minutes is < 1 or > 30) throw new ArgumentOutOfRangeException(nameof(minutes));
        Directory.CreateDirectory(Root);
        Stop();
        // Never silently take over another settings window's active capture.
        if (File.Exists(ControlPath)) {
            var current = File.ReadAllLines(ControlPath);
            if (current.Length >= 4 && long.TryParse(current[1], out var expiry) && expiry > Environment.TickCount64 && ControllerIsAlive(current))
                throw new InvalidOperationException("另一个设置窗口正在计时，请先在那里停止，或等待计时到期。");
            File.Delete(ControlPath);
        }
        var session = Guid.NewGuid().ToString("N");
        var deadline = Environment.TickCount64 + minutes * 60000L;
        var temporary = Path.Combine(Root, session + ".control.tmp");
        try {
            using var controller = Process.GetCurrentProcess();
            File.WriteAllText(temporary, session + "\n" + deadline.ToString(CultureInfo.InvariantCulture) + "\n" +
                Environment.ProcessId.ToString(CultureInfo.InvariantCulture) + "\n" +
                controller.StartTime.ToUniversalTime().ToFileTimeUtc().ToString(CultureInfo.InvariantCulture) + "\n", new UTF8Encoding(false));
            File.Move(temporary, ControlPath, false);
        } finally { if (File.Exists(temporary)) File.Delete(temporary); }
        Session = session; Deadline = deadline;
        _events.Clear(); _offsets.Clear(); _limited = _fileLimit = false; _malformed = 0;
    }

    private static bool ControllerIsAlive(string[] marker) {
        if (!int.TryParse(marker[2], out var pid) || !long.TryParse(marker[3], out var created)) return false;
        try {
            using var process = Process.GetProcessById(pid);
            return !process.HasExited && process.StartTime.ToUniversalTime().ToFileTimeUtc() == created;
        } catch (ArgumentException) { return false; }
        catch (InvalidOperationException) { return false; }
        catch (System.ComponentModel.Win32Exception) { return true; } // Unknown owner: do not take over.
    }

    internal void Stop() {
        if (Session is null) return;
        // Do not stop a different settings instance's newer session.
        try {
            if (File.Exists(ControlPath) && File.ReadLines(ControlPath).FirstOrDefault() == Session)
                File.Delete(ControlPath);
        } finally { Deadline = 0; }
    }

    internal void Clear() {
        if (Running) throw new InvalidOperationException("请先停止计时。");
        if (Session is not null)
            foreach (var file in Directory.EnumerateFiles(Root, Session + "-*.jsonl")) File.Delete(file);
        _events.Clear(); _offsets.Clear(); Session = null; _limited = _fileLimit = false; _malformed = 0;
    }

    internal CandidateTimingSnapshot Scan() {
        if (Session is null) return Build([], "尚未开始计时。", 0);
        long budget = MaximumReadPerScan;
        foreach (var file in Directory.EnumerateFiles(Root, Session + "-*.jsonl").Order(StringComparer.Ordinal)) {
            if (_events.Count >= MaximumEvents) { _limited = true; break; }
            if (budget <= 0) break;
            using var stream = new FileStream(file, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            if (stream.Length >= 8 * 1024 * 1024 - 65536) _fileLimit = true;
            var offset = _offsets.GetValueOrDefault(file);
            if (offset > stream.Length) offset = 0;
            stream.Position = offset;
            var wanted = (int)Math.Min(budget, stream.Length - offset);
            if (wanted <= 0) continue;
            var buffer = new byte[wanted];
            var count = stream.Read(buffer, 0, wanted);
            if (count == 0) continue;
            budget -= count;
            var lastNewline = Array.LastIndexOf(buffer, (byte)'\n', count - 1, count);
            if (lastNewline < 0) {
                if (count == MaximumReadPerScan) { _malformed++; _offsets[file] = offset + count; }
                continue; // An incomplete final line is retried, not counted as malformed.
            }
            var consumed = lastNewline + 1;
            _offsets[file] = offset + consumed;
            foreach (var line in Encoding.UTF8.GetString(buffer, 0, consumed).Split('\n', StringSplitOptions.RemoveEmptyEntries)) {
                if (_events.Count >= MaximumEvents) { _limited = true; break; }
                if (TryParse(line, Session, out var record)) _events.Add(record!);
                else _malformed++;
            }
        }
        var warning = _limited ? "已达到 50,000 条事件容量；统计不完整，请停止后重新开始短时采样。" : "";
        if (_fileLimit) warning += " 有进程的记录文件已接近 8 MiB 上限，可能丢失后续事件，请缩短采样时长。";
        if (_malformed > 0) warning += $" 已跳过 {_malformed} 条无效记录。";
        return Build(_events.ToArray(), warning, _events.GroupBy(e => e.Pid).Sum(g => g.Max(e => e.Get("dropped_total"))));
    }

    private static readonly HashSet<string> AllowedEvents = new(StringComparer.Ordinal) {
        "queued", "preedit_update", "exchange_begin", "exchange_end", "superseded", "response_posted", "ui_received",
        "stale_discarded", "ui_applied", "model_poll_begin", "model_poll_end", "received", "prepare",
        "generation", "model_queued", "model_begin", "model_end", "model_wait_cancelled", "response_ready", "encoded", "written", "begin", "ranked"
    };
    private static readonly HashSet<string> AllowedFields = new(StringComparer.Ordinal) {
        "generation", "page", "expanded", "input_started_us", "input_units", "correction", "queue_us",
        "defer_expensive", "duration_us", "ok", "error", "conversion_us", "candidate_count", "refinement_pending",
        "model_pending", "model_update", "failed", "visible", "update", "auxiliary", "full_pinyin", "page_size",
        "partition", "model_enabled", "model_timeout_ms", "base_dictionary", "imported_dictionary", "parse_us",
        "normalization_us", "segmentation_us", "correction_us", "assistance_us", "generation_us", "lookup_us", "sort_us",
        "lookup_count", "beam_transitions", "beam_materialized", "search_us", "layout_us", "cache_hit", "cancelled",
        "correction_fallback", "parse_paths", "status", "dropped_total", "render_us", "model_prepare_us", "model_prepared"
    };
    internal static bool TryParse(string line, string session, out CandidateTimingEvent? record) {
        record = null;
        if (line.Length > 8192) return false;
        try {
            using var document = JsonDocument.Parse(line, new JsonDocumentOptions { MaxDepth = 4 });
            var root = document.RootElement;
            if (root.GetProperty("schema").GetInt32() != 1 || root.GetProperty("session").GetString() != session) return false;
            var component = root.GetProperty("component").GetString();
            var name = root.GetProperty("event").GetString();
            if (component is not ("tsf" or "core" or "model") || name is null || !AllowedEvents.Contains(name)) return false;
            var values = new Dictionary<string, long>(StringComparer.Ordinal);
            foreach (var property in root.EnumerateObject())
                if (property.Value.ValueKind == JsonValueKind.Number && AllowedFields.Contains(property.Name) &&
                    property.Value.TryGetInt64(out var value)) values[property.Name] = value;
            var timestamp = root.GetProperty("timestamp_us").GetInt64();
            if (timestamp < 0) return false;
            record = new(session, root.GetProperty("pid").GetUInt32(), root.GetProperty("request_id").GetUInt64(),
                timestamp, component, name, values);
            return true;
        } catch (Exception e) when (e is JsonException or InvalidOperationException or KeyNotFoundException or FormatException or OverflowException) { return false; }
    }

    internal static CandidateTimingSnapshot Build(IReadOnlyList<CandidateTimingEvent> events, string warning = "", long dropped = 0) {
        var rows = new List<CandidateTimingRow>();
        foreach (var group in events.GroupBy(e => e.RequestId)) {
            var sorted = group.OrderBy(e => e.TimestampUs).ToArray();
            var queued = sorted.FirstOrDefault(e => e.Component == "tsf" && e.Event == "queued");
            if (queued is null) continue; // Core-only fragments are preserved in export, not fabricated as end-to-end samples.
            var start = queued.Get("input_started_us");
            if (start <= 0 || start > queued.TimestampUs) start = queued.TimestampUs;
            var applied = sorted.Where(e => e.Component == "tsf" && e.Event == "ui_applied" && e.Get("failed") == 0 && e.Get("candidate_count") > 0).ToArray();
            var first = applied.FirstOrDefault();
            var settled = applied.LastOrDefault(e => e.Get("refinement_pending") == 0 && e.Get("model_update") == 0);
            var model = applied.LastOrDefault(e => e.Get("model_update") != 0);
            double? Since(CandidateTimingEvent? e) => e is null ? null : Math.Max(0, e.TimestampUs - start) / 1000.0;
            var stages = new Dictionary<string, double>();
            void Sum(string key, string component, string name, string field, bool mainOnly = false) {
                var matches = sorted.Where(e => e.Component == component && e.Event == name && e.Values.ContainsKey(field) &&
                    (!mainOnly || e.Get("auxiliary") == 0 && e.Get("update") == 0)).ToArray();
                if (matches.Length > 0) stages[key] = matches.Sum(e => Math.Max(0, e.Get(field))) / 1000.0;
            }
            stages["input"] = (queued.TimestampUs - start) / 1000.0;
            Sum("preedit", "tsf", "preedit_update", "duration_us");
            Sum("queue", "tsf", "exchange_begin", "queue_us");
            Sum("ipc", "tsf", "exchange_end", "duration_us");
            var mainResponses = sorted.Where(e => e.Component == "core" && e.Event == "response_ready" && e.Get("update") == 0).ToArray();
            if (mainResponses.Length > 0) stages["core"] = mainResponses.Sum(e => e.Get("duration_us")) / 1000.0;
            Sum("prepare", "core", "prepare", "duration_us", true);
            foreach (var (key, field) in new[] { ("parse", "parse_us"), ("normalize", "normalization_us"), ("segment", "segmentation_us"),
                ("correction", "correction_us"), ("assistance", "assistance_us"),
                ("generation", "generation_us"), ("lookup", "lookup_us"), ("sort", "sort_us"), ("layout", "layout_us"), ("model_prepare", "model_prepare_us") })
                Sum(key, "core", "generation", field, true);
            Sum("encode", "core", "encoded", "duration_us", true);
            Sum("write", "core", "written", "duration_us", true);
            Sum("convert", "tsf", "response_posted", "conversion_us");
            Sum("render", "tsf", "ui_applied", "render_us");
            Sum("model", "model", "ranked", "duration_us");
            Sum("model_ipc", "core", "model_end", "duration_us");
            Sum("model_poll", "tsf", "model_poll_end", "duration_us");
            // Nested metrics must not be summed into an invented end-to-end time.
            var exchanges = sorted.Where(e => e.Component == "tsf" && e.Event == "exchange_end").ToArray();
            var residuals = new List<double>();
            var arrivalWait = new List<double>();
            var returnWait = new List<double>();
            foreach (var end in exchanges) {
                var begin = sorted.LastOrDefault(e => e.Component == "tsf" && e.Event == "exchange_begin" && e.TimestampUs <= end.TimestampUs);
                if (begin is null || end.Get("ok") == 0) continue;
                CandidateTimingEvent? Within(string name) => sorted.FirstOrDefault(e => e.Component == "core" && e.Event == name &&
                    e.Get("update") == 0 && e.TimestampUs >= begin.TimestampUs && e.TimestampUs <= end.TimestampUs);
                var response = Within("response_ready"); var encode = Within("encoded"); var write = Within("written");
                var received = Within("received");
                if (received is not null) arrivalWait.Add((received.TimestampUs - begin.TimestampUs) / 1000.0);
                if (response is not null) returnWait.Add((end.TimestampUs - response.TimestampUs) / 1000.0);
                if (response is null || encode is null || write is null) continue;
                residuals.Add(Math.Max(0, end.Get("duration_us") - response.Get("duration_us") - encode.Get("duration_us") - write.Get("duration_us")) / 1000.0);
            }
            if (exchanges.Length > 0 && residuals.Count == exchanges.Length) stages["transport_wait"] = residuals.Sum();
            if (exchanges.Length > 0 && arrivalWait.Count == exchanges.Length) stages["service_wait"] = arrivalWait.Sum();
            if (exchanges.Length > 0 && returnWait.Count == exchanges.Length) stages["return_wait"] = returnWait.Sum();
            var modelQueue = sorted.Where(e => e.Component == "core" && e.Event == "model_queued").ToArray();
            var queueDelays = new List<double>();
            foreach (var queuedModel in modelQueue) {
                var started = sorted.FirstOrDefault(e => e.Component == "core" && e.Event == "model_begin" && e.TimestampUs >= queuedModel.TimestampUs);
                if (started is not null) queueDelays.Add((started.TimestampUs - queuedModel.TimestampUs) / 1000.0);
            }
            if (modelQueue.Length > 0 && queueDelays.Count == modelQueue.Length) stages["model_queue"] = queueDelays.Sum();
            var posted = sorted.Where(e => e.Component == "tsf" && e.Event == "response_posted").ToArray();
            var dispatch = 0L; var dispatchCount = 0;
            foreach (var received in sorted.Where(e => e.Component == "tsf" && e.Event == "ui_received" && e.Get("model_update") == 0)) {
                var previous = posted.LastOrDefault(e => e.TimestampUs <= received.TimestampUs);
                if (previous is not null) { dispatch += received.TimestampUs - previous.TimestampUs; dispatchCount++; }
            }
            if (dispatchCount > 0) stages["dispatch"] = dispatch / 1000.0;
            var failed = sorted.Any(e => e.Component == "tsf" && (e.Event == "exchange_end" && e.Get("ok") == 0 || e.Event == "ui_received" && e.Get("failed") != 0));
            var superseded = sorted.Any(e => e.Event is "superseded" or "stale_discarded");
            var modelRequested = sorted.Any(e => e.Component == "core" && e.Event == "model_queued");
            var status = superseded && first is null ? "已淘汰" : failed ? "失败/超时" : first is null ? "未完成/被替换" :
                model is not null ? "模型已应用" : modelRequested ? "基础就绪·模型未应用" : settled is not null ? "已就绪" : "首批就绪·待精化";
            rows.Add(new(group.Key, queued.Pid, start, queued.Get("page"), queued.Get("input_units"), status,
                Since(first), Since(settled), Since(model), stages,
                sorted.Count(e => e.Component == "core" && e.Event == "generation" && e.Get("cache_hit") != 0),
                sorted.Count(e => e.Component == "tsf" && e.Event == "exchange_begin")));
        }
        return new(events, rows.OrderByDescending(e => e.StartUs).ToArray(), warning, dropped,
            events.Select(e => e.Component).Distinct().Order(StringComparer.Ordinal).ToArray());
    }

    internal static double Percentile(IEnumerable<double> source, double fraction) {
        var values = source.Order().ToArray();
        return values.Length == 0 ? 0 : values[Math.Clamp((int)Math.Ceiling(values.Length * fraction) - 1, 0, values.Length - 1)];
    }

    internal object Export(CandidateTimingSnapshot snapshot) => new {
        schema = 1, session = Session, created_utc = DateTime.UtcNow,
        environment = new {
            app_version = typeof(CandidateTimingStore).Assembly.GetName().Version?.ToString(),
            os = System.Runtime.InteropServices.RuntimeInformation.OSDescription,
            architecture = System.Runtime.InteropServices.RuntimeInformation.OSArchitecture.ToString(),
            process_architecture = System.Runtime.InteropServices.RuntimeInformation.ProcessArchitecture.ToString(),
            logical_processors = Environment.ProcessorCount,
            available_memory_bytes = GC.GetGCMemoryInfo().TotalAvailableMemoryBytes,
            stopwatch_frequency = Stopwatch.Frequency
        },
        privacy = "No input text, candidate text, application titles, context, paths or credentials are collected. PID, input length and numeric timings are included. Local only; no upload.",
        measurement = "TSF OnKeyDown (or request scheduling) to usable UI snapshot submission, not physical key latency or GPU scanout. Nested stage metrics overlap. Missing stages are unknown, not zero. Superseded/failed requests are not latency-success samples.",
        warning = snapshot.Warning, dropped = snapshot.Dropped, components = snapshot.Components,
        rows = snapshot.Rows.Select(r => new {
            request_id = r.RequestId.ToString(CultureInfo.InvariantCulture), pid = r.Pid,
            start_us = r.StartUs, page = r.Page, input_units = r.InputUnits, status = r.Status,
            first_ms = r.FirstMs, settled_ms = r.SettledMs, model_applied_ms = r.ModelMs,
            stages_ms = r.Stages, cache_hits = r.CacheHits, rounds = r.Rounds
        }),
        events = snapshot.Events.Select(e => new {
            session = e.Session, pid = e.Pid, request_id = e.RequestId.ToString(CultureInfo.InvariantCulture),
            timestamp_us = e.TimestampUs, component = e.Component, @event = e.Event, values = e.Values
        })
    };
}
