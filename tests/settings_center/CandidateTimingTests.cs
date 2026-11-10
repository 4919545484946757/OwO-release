using OwO_Settings;
using System.Text.Json;

var checks = 0;
void Check(bool condition, string title) { if (!condition) throw new Exception(title); checks++; }
var root = Path.Combine(Path.GetTempPath(), "OwO-candidate-timing-" + Guid.NewGuid().ToString("N"));
var store = new CandidateTimingStore(root);
Check(!store.Running && store.Scan().Rows.Count == 0, "disabled by default");
store.Start(1);
var session = store.Session!;
Check(store.Running && File.ReadAllLines(Path.Combine(root, "candidate-timing.control"))[0] == session, "session marker");
Check(!File.ReadAllBytes(Path.Combine(root, "candidate-timing.control")).Take(3).SequenceEqual(new byte[] { 239, 187, 191 }), "native UTF-8 without BOM");
var competing = new CandidateTimingStore(root);
try { competing.Start(1); throw new Exception("must reject competing capture"); } catch (InvalidOperationException) { checks++; }
CandidateTimingEvent Event(string component, string name, long time, params (string, long)[] values) =>
    new(session, component == "tsf" ? 12U : component == "core" ? 20U : 30U, 123, time, component, name,
        values.ToDictionary(v => v.Item1, v => v.Item2));
var events = new[] {
    Event("tsf", "queued", 1100, ("input_started_us", 1000), ("input_units", 5)),
    Event("tsf", "exchange_begin", 1300, ("queue_us", 200)),
    Event("core", "received", 1500), Event("core", "prepare", 1700, ("duration_us", 200)),
    Event("core", "generation", 4000, ("parse_us", 300), ("generation_us", 1400), ("cache_hit", 0)),
    Event("core", "response_ready", 4500, ("duration_us", 3000)),
    Event("core", "encoded", 4510, ("duration_us", 0)), Event("core", "written", 4520, ("duration_us", 0)),
    Event("tsf", "exchange_end", 5000, ("duration_us", 3700), ("ok", 1)),
    Event("tsf", "response_posted", 5100, ("candidate_count", 7), ("model_pending", 1)),
    Event("tsf", "ui_received", 5300),
    Event("tsf", "ui_applied", 6000, ("candidate_count", 7), ("render_us", 700)),
    Event("core", "model_queued", 4400), Event("model", "ranked", 8000, ("duration_us", 2000)),
    Event("tsf", "ui_applied", 9000, ("candidate_count", 7), ("model_update", 1), ("render_us", 300)) };
var snapshot = CandidateTimingStore.Build(events);
var row = snapshot.Rows.Single();
Check(row.FirstMs == 5 && row.SettledMs == 5 && row.ModelMs == 8, "distinct first/refined/model timing");
Check(row.Stages["core"] == 3 && Math.Abs(row.Stages["transport_wait"] - .7) < .000001 && row.Stages["dispatch"] == .2, "nested metrics and transport residual");
Check(!row.Stages.ContainsKey("lookup"), "missing metric must not be fabricated");
Check(!CandidateTimingStore.Build(events.Where(e => e.Event != "written").ToArray()).Rows.Single().Stages.ContainsKey("transport_wait"), "incomplete IPC trace has unknown residual");
Check(row.Status == "模型已应用", "model application status");
var fragment = CandidateTimingStore.Build(events.Where(e => e.Component != "tsf").ToArray());
Check(fragment.Rows.Count == 0 && fragment.Events.Count > 0, "Core-only fragments not latency samples");
Check(CandidateTimingStore.Percentile(new double[] { 1, 2, 3, 4, 5 }, .95) == 5, "nearest rank percentile");
string Json(CandidateTimingEvent e) => JsonSerializer.Serialize(new {
    schema = 1, session, pid = e.Pid, request_id = e.RequestId, timestamp_us = e.TimestampUs,
    component = e.Component, @event = e.Event, candidate_count = e.Get("candidate_count"), ok = e.Get("ok") });
var file = Path.Combine(root, session + "-12.jsonl");
var validLine = Json(events[0]);
File.WriteAllText(file, validLine[..^1]);
Check(store.Scan().Events.Count == 0, "partial final line retried");
File.AppendAllText(file, "}\n");
Check(store.Scan().Events.Count == 1 && store.Scan().Events.Count == 1, "incremental reader does not duplicate");
Check(!CandidateTimingStore.TryParse(validLine, "different", out _), "reject wrong session");
Check(!CandidateTimingStore.TryParse("{}", session, out _), "reject malformed event");
var unsafeLine = validLine[..^1] + ",\"input\":\"secret-input\",\"api_key\":\"secret-key\",\"numeric_secret\":987654321}";
Check(CandidateTimingStore.TryParse(unsafeLine, session, out var sanitized), "valid envelope accepted");
Check(!JsonSerializer.Serialize(sanitized).Contains("secret"), "unknown string payload not retained/exported");
Check(!sanitized!.Values.ContainsKey("numeric_secret"), "unknown numeric fields not retained/exported");
Check(!JsonSerializer.Serialize(store.Export(snapshot)).Contains("secret"), "report contains only sanitized data");
var largeId = events.Select(e => e with { RequestId = ulong.MaxValue }).ToArray();
Check(JsonSerializer.Serialize(store.Export(CandidateTimingStore.Build(largeId))).Contains("\"request_id\":\"18446744073709551615\""), "export request IDs are lossless strings");
store.Stop();
Check(!store.Running && !File.Exists(Path.Combine(root, "candidate-timing.control")), "stop removes only owned control");
store.Start(1);
var otherSession = new string('a', 32);
File.WriteAllText(Path.Combine(root, "candidate-timing.control"), otherSession + "\n1\n");
store.Stop();
Check(File.Exists(Path.Combine(root, "candidate-timing.control")), "stop preserves a different owner's marker");
File.Delete(Path.Combine(root, "candidate-timing.control"));
store.Clear();
Check(store.Session is null && store.Scan().Events.Count == 0, "clear resets capture state");
Check(!Directory.EnumerateFiles(root, "*.control.tmp").Any(), "no temporary marker left");
File.WriteAllText(Path.Combine(root, "candidate-timing.control"), new string('b', 32) + "\n" + (Environment.TickCount64 + 60000) + "\n" + Environment.ProcessId + "\n1\n");
store.Start(1);
Check(store.Running, "PID reuse or old boot marker cannot prevent a new capture");
store.Stop();
Console.WriteLine($"Candidate timing store: {checks} checks passed.");
