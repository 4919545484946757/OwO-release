using OwO_Settings;
using System.Net;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.IO.Pipes;
using System.Diagnostics;

var checks = 0;
void Check(bool value, string label) { if (!value) throw new Exception(label); ++checks; }
void Reject(Action action, string label) { try { action(); } catch { ++checks; return; } throw new Exception(label); }
async Task RejectAsync(Func<Task> action, string label) { try { await action(); } catch { ++checks; return; } throw new Exception(label); }
var root = Path.Combine(Path.GetTempPath(), "owo-model-manager-" + Guid.NewGuid().ToString("N"));
var store = new ModelManagerStore(Path.Combine(root, "models"));
var key = "unit-test-not-a-real-api-key";
var profile = new UserModelProfile(Guid.NewGuid().ToString("N"), "Fixture", "https://example.invalid/v1", "test-model", ModelKeyProtection.Protect(key));
var settings = new TextAssistantSettings { Profiles = [profile], ActiveProfile = profile.Id, Enabled = true, PrivacyConsent = true };
var input = new TextAssistantInput("polish", "测试文字", "附近测试文字", "Fixture.exe");
if (args.Length == 2 && args[0] == "--seed-ui") {
    var fixture = Path.GetFullPath(args[1]);
    if (!Path.GetFileName(fixture).StartsWith("model-manager-ui-", StringComparison.Ordinal)) throw new ArgumentException("Use an isolated UI fixture directory.");
    new ModelManagerStore(Path.Combine(fixture, "models")).Save(settings with { Enabled = false, PrivacyConsent = false });
    Console.WriteLine("Seeded synthetic model configuration; no API request.");
    return;
}
try {
    Check(!store.Load().Enabled, "feature must default off");
    Check(ModelKeyProtection.Unprotect(profile.ProtectedKey) == key && !profile.ProtectedKey.Contains(key), "DPAPI round trip");
    store.Save(settings);
    Check(File.Exists(Path.Combine(store.Root, "selection-assistant.enabled")), "enable native gate");
    Check(File.Exists(Path.Combine(store.Root, "context.enabled")), "context gate");
    Check(!File.ReadAllText(Path.Combine(store.Root, "profiles.json")).Contains(key), "no plaintext key persistence");
    Check(store.Load().Profiles.Single().ProtectedKey == profile.ProtectedKey, "protected profile round trip");
    store.Save(settings with { Enabled = false });
    Check(!File.Exists(Path.Combine(store.Root, "selection-assistant.enabled")) && !File.Exists(Path.Combine(store.Root, "context.enabled")), "disable both gates");
    foreach (var endpoint in new[] { "http://example.invalid", "file:///secret", "https://user:pass@example.invalid/v1", "https://example.invalid/?key=secret", "https://example.invalid/#secret" })
        Reject(() => ModelManagerStore.Endpoint(endpoint), "unsafe endpoint accepted");
    Check(ModelManagerStore.Endpoint(profile.Endpoint).AbsolutePath == "/v1/chat/completions", "base endpoint append");
    Check(ModelManagerStore.Endpoint(profile.Endpoint + "/chat/completions").AbsolutePath == "/v1/chat/completions", "full endpoint preserved");
    Reject(() => store.Save(settings with { PrivacyConsent = false }), "enable without consent");
    Reject(() => store.Save(settings with { SecondaryLanguage = settings.PreferredLanguage }), "same languages");
    Reject(() => store.Save(settings with { PolishLevel = 4 }), "invalid polish level");
    Reject(() => store.Save(settings with { CodeLevel = 0 }), "invalid code level");
    Reject(() => store.Save(settings with { Profiles = [profile, profile] }), "duplicate profile");
    foreach (var operation in new[] { "translate", "polish", "code", "generate" }) for (var level = 1; level <= 3; ++level) {
        var instructions = TextAssistantApi.Instructions(settings with { PolishLevel = level, CodeLevel = level }, operation);
        Check(instructions.Contains("不执行") && instructions.Length > 100, "prompt operation and non-execution policy");
    }
    Check(TextAssistantApi.Instructions(settings, "translate").Contains(settings.SecondaryLanguage), "bidirectional language routing prompt");
    Reject(() => TextAssistantApi.Instructions(settings, "execute"), "unknown operation");
    var wire = "OWOTEXT1\npolish\n" + TextAssistantHost.Encode(input.Selection) + "\n" + TextAssistantHost.Encode(input.Context) + "\n" + TextAssistantHost.Encode(input.Application) + "\n";
    Check(TextAssistantHost.Parse(wire) == input, "strict IPC UTF8 round trip");
    Reject(() => TextAssistantHost.Parse(wire + "extra"), "extra wire field");
    Reject(() => TextAssistantHost.Parse(wire.Replace("polish", "execute")), "wire operation cannot be arbitrary text");
    Reject(() => TextAssistantHost.Decode("GG"), "invalid hex");
    Reject(() => TextAssistantHost.Decode("FF"), "invalid UTF8");
    Reject(() => TextAssistantHost.CheckSensitive(input with { Selection = "api_key='abcdefghijklmnop123456'" }), "credential filter");
    TextAssistantHost.CheckSensitive(input);
    var handler = new FakeHandler(async request => {
        Check(request.RequestUri!.AbsoluteUri == "https://example.invalid/v1/chat/completions", "only configured destination");
        Check(request.Headers.Authorization?.Parameter == key, "key only in authorization");
        using var body = JsonDocument.Parse(await request.Content!.ReadAsStringAsync());
        Check(body.RootElement.GetProperty("model").GetString() == profile.Model, "configured model");
        Check(body.RootElement.GetProperty("max_tokens").GetInt32() == settings.MaxOutputTokens, "legacy token field");
        using var user = JsonDocument.Parse(body.RootElement.GetProperty("messages")[1].GetProperty("content").GetString()!);
        Check(user.RootElement.GetProperty("context").GetString() == "", "disabled context is not transmitted");
        return FakeHandler.Json("{\"choices\":[{\"message\":{\"content\":\"生成的结果\"},\"finish_reason\":\"stop\"}]}");
    });
    Check(await TextAssistantApi.ExecuteAsync(settings with { IncludeContext = false }, input, default, handler) == "生成的结果", "model text returned without executing");
    foreach (var response in new[] {
        "{\"choices\":[{\"message\":{\"content\":\"截断结果\"},\"finish_reason\":\"length\"}]}",
        "{\"choices\":[{\"message\":{\"content\":\"\",\"tool_calls\":[{}]},\"finish_reason\":\"stop\"}]}",
        "{\"choices\":[]}", "not json",
        "{\"choices\":[{\"message\":{\"content\":\" \"},\"finish_reason\":\"stop\"}]}" }) {
        await RejectAsync(async () => { await TextAssistantApi.ExecuteAsync(settings, input, default, new FakeHandler(_ => Task.FromResult(FakeHandler.Json(response)))); }, "malformed/incomplete response accepted");
    }
    await RejectAsync(async () => { await TextAssistantApi.ExecuteAsync(settings, input, default, new FakeHandler(_ => Task.FromResult(new HttpResponseMessage(HttpStatusCode.Redirect) { Content = new StringContent("redirect") }))); }, "redirect accepted");
    await RejectAsync(async () => { await TextAssistantApi.ExecuteAsync(settings with { Enabled = false }, input, default, new FakeHandler(_ => throw new Exception("must not call"))); }, "disabled feature connected");
    await RejectAsync(async () => { await TextAssistantApi.ExecuteAsync(settings, input with { Application = "pwsh.exe" }, default, new FakeHandler(_ => throw new Exception("must not call"))); }, "command-execution environment connected");
    await RejectAsync(async () => { await TextAssistantApi.ExecuteAsync(settings, input with { Selection = new string('a', 8193) }, default, new FakeHandler(_ => throw new Exception("must not call"))); }, "oversize selection connected");
    using (var cancellation = new CancellationTokenSource()) {
        cancellation.Cancel();
        await RejectAsync(async () => { await TextAssistantApi.ExecuteAsync(settings, input, cancellation.Token, new FakeHandler(_ => Task.FromResult(FakeHandler.Json("{}")))); }, "cancelled call accepted");
    }
    if (args.Length == 1) {
        // Real worker IPC, disabled model: no network, no personal profile writes.
        store.Save(settings with { Enabled = false });
        var name = "OwO.TextAssistant." + Guid.NewGuid().ToString("N");
        var start = new ProcessStartInfo(Path.GetFullPath(args[0])) { UseShellExecute = false, CreateNoWindow = true };
        start.ArgumentList.Add("--text-assistant-host"); start.ArgumentList.Add(name);
        start.ArgumentList.Add("--parent-pid"); start.ArgumentList.Add(Environment.ProcessId.ToString());
        start.Environment["OWO_CONFIG_PATH"] = Path.Combine(root, "config", "owo.conf");
        using var worker = Process.Start(start)!;
        try {
            using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(20));
            await using var pipe = new NamedPipeClientStream(".", name, PipeDirection.InOut, PipeOptions.Asynchronous);
            await pipe.ConnectAsync(timeout.Token);
            var bytes = Encoding.UTF8.GetBytes(wire); var length = BitConverter.GetBytes(bytes.Length);
            await pipe.WriteAsync(length, timeout.Token); await pipe.WriteAsync(bytes, timeout.Token);
            await pipe.ReadExactlyAsync(length, timeout.Token);
            var size = BitConverter.ToInt32(length); Check(size is > 0 and < 128 * 1024, "worker response size");
            var reply = new byte[size]; await pipe.ReadExactlyAsync(reply, timeout.Token);
            Check(Encoding.UTF8.GetString(reply).StartsWith("OWOTEXT1\nERROR\n"), "disabled real worker rejects request");
        } finally { if (!worker.WaitForExit(5000)) { worker.Kill(); worker.WaitForExit(); } }
    }
    Console.WriteLine($"PASS: {checks} model manager/privacy/protocol/API checks; mock HTTP only, no paid model requests.");
} finally { Directory.Delete(root, true); }

sealed class FakeHandler(Func<HttpRequestMessage, Task<HttpResponseMessage>> action) : HttpMessageHandler {
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) {
        cancellationToken.ThrowIfCancellationRequested(); return action(request);
    }
    internal static HttpResponseMessage Json(string body) => new(HttpStatusCode.OK) { Content = new StringContent(body, Encoding.UTF8, "application/json") };
}
