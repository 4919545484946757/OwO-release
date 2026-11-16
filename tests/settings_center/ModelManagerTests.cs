using OwO_Settings;
using System.Net;
using System.Net.Http;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.IO.Compression;
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
if (args.Length == 2 && args[0] == "--verify-local") {
    var selected = LocalModelStore.Find(args[1]) ?? throw new ArgumentException("Unknown catalog model.");
    var localTestStore = new LocalModelStore(store.Root);
    try {
        using var download = new DownloadOperation(selected.Name, default, TimeSpan.FromSeconds(115));
        await download.RunAsync(token => localTestStore.InstallAsync(selected, download, token));
        var approval = Guid.NewGuid().ToString("N");
        var fixture = new TextAssistantSettings { ZeroCandidateEnabled = true, ZeroCandidateLocalModel = selected.Id,
            ZeroCandidateConsentProfile = ModelManagerStore.LocalConsent(selected.Id), ZeroCandidateApproval = approval };
        var result = await TextAssistantApi.ExecuteAsync(fixture, new("zero-eco", "nihao", "", "", approval), default,
            modelRoot: store.Root);
        Check(result == "你好", "local model must consume all of nihao, not just the last syllable");
        var second = await TextAssistantApi.ExecuteAsync(fixture, new("zero-eco", "beijing", "", "", approval), default,
            modelRoot: store.Root);
        Check(second == "北京", "local model must generate a second independently chosen spelling");
        var reported = await TextAssistantApi.ExecuteAsync(fixture, new("zero-eco", "xianzai", "", "", approval, "现在"), default,
            modelRoot: store.Root);
        Check(reported == "现在", "wrong reading for xianzai must be replaced by a verified full candidate");
        Console.WriteLine($"PASS: verified download and CPU inference {selected.Name} → {result} / {second} / {reported}");
    } finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
    return;
}
try {
    Check(!store.Load().Enabled, "feature must default off");
    Check(ModelManagerStore.ZeroCandidateMode(store.Load()) is null, "automatic candidate must default off");
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
    var zero = settings with {
        Enabled = false, PrivacyConsent = false, ZeroCandidateEnabled = true,
        ZeroCandidateConsentProfile = profile.Id, ZeroCandidateApproval = Guid.NewGuid().ToString("N"),
        ZeroCandidatePerformance = true
    };
    var zeroInput = new TextAssistantInput("zero-pinyin", "nihao", "", "", zero.ZeroCandidateApproval);
    const string zeroOk = "{\"choices\":[{\"message\":{\"content\":\"你好\"},\"finish_reason\":\"stop\"}]}";
    var zeroGate = Path.Combine(store.Root, "zero-candidate.mode");
    Check(ModelManagerStore.ZeroCandidateMode(zero) == "pinyin", "models without context are pinned to pinyin");
    store.Save(zero);
    Check(File.ReadAllText(zeroGate) == "pinyin:" + zero.ZeroCandidateApproval &&
        !File.Exists(Path.Combine(store.Root, "selection-assistant.enabled")), "separate opt-in and runtime policy");
    var zeroWire = "OWOTEXT1\nzero-pinyin\n" + TextAssistantHost.Encode("nihao") + "\n\n\n" + zero.ZeroCandidateApproval + "\n";
    Check(TextAssistantHost.Parse(zeroWire) == zeroInput, "zero candidate wire has separate authorization");
    Check(TextAssistantHost.Parse(zeroWire + TextAssistantHost.Encode("你好\n拟好") + "\n") ==
        zeroInput with { LocalReferences = "你好\n拟好" }, "zero candidate wire accepts an optional local-only shortlist");
    Reject(() => TextAssistantHost.Parse(zeroWire.Replace(zero.ZeroCandidateApproval, "not-an-approval")), "reject malformed runtime approval");
    var withContext = zero with { Profiles = [profile with { SupportsCandidateContext = true }] };
    var eco = withContext with { ZeroCandidatePerformance = false };
    Check(ModelManagerStore.ZeroCandidateMode(eco) == "eco" && ModelManagerStore.ZeroCandidateMode(withContext) == "performance", "context-capable model offers both modes");
    FakeHandler CheckCandidateBody(string expectedContext, string expectedApp, int expectedTokens) => new(async request => {
        using var payload = JsonDocument.Parse(await request.Content!.ReadAsStringAsync());
        var item = payload.RootElement;
        using var user = JsonDocument.Parse(item.GetProperty("messages")[1].GetProperty("content").GetString()!);
        Check(item.GetProperty("max_tokens").GetInt32() == expectedTokens && user.RootElement.GetProperty("pinyin").GetString() == "nihao" &&
            user.RootElement.GetProperty("context").GetString() == expectedContext && user.RootElement.GetProperty("application").GetString() == expectedApp &&
            !item.GetRawText().Contains(zero.ZeroCandidateApproval), "only bounded payload reaches the model");
        return FakeHandler.Json(zeroOk);
    });
    Check(await TextAssistantApi.ExecuteAsync(zero, zeroInput, default, CheckCandidateBody("", "", 256)) == "你好", "pinyin-only result");
    var ecoInput = zeroInput with { Operation = "zero-eco", Context = "单号１２3⑨" };
    Check(await TextAssistantApi.ExecuteAsync(eco, ecoInput, default, CheckCandidateBody("单号0", "", 256)) == "你好", "energy mode redact numbers");
    var perfInput = zeroInput with { Operation = "zero-performance", Context = "前文\n<光标>\n后文", Application = "notepad.exe" };
    Check(await TextAssistantApi.ExecuteAsync(withContext, perfInput, default, CheckCandidateBody(perfInput.Context, "notepad.exe", 512)) == "你好", "performance mode transmits bounded environment and context");
    Check(await TextAssistantApi.ExecuteAsync(zero with { MaxOutputTokens = 128 }, zeroInput, default,
        CheckCandidateBody("", "", 128)) == "你好", "zero candidate respects lower user-defined output limit");
    var zeroCalls = 0;
    var noCall = new FakeHandler(_ => { ++zeroCalls; return Task.FromResult(FakeHandler.Json(zeroOk)); });
    foreach (var pair in new (TextAssistantSettings Config, TextAssistantInput Request)[] {
        (zero with { ZeroCandidateEnabled = false }, zeroInput),
        (zero, zeroInput with { Approval = Guid.NewGuid().ToString("N") }),
        (zero, zeroInput with { Context = "不该发送" }),
        (zero, perfInput), (eco, ecoInput with { Context = new string('字', 17) }),
        (eco, ecoInput with { Application = "notepad.exe" }),
        (withContext, perfInput with { Application = @"C:\Windows\notepad.exe" }),
        (withContext, perfInput with { Context = "api_key='abcdefghijklmnop123456'" }),
        (withContext, perfInput with { Selection = "ni，hao" })
    }) await RejectAsync(() => TextAssistantApi.ExecuteAsync(pair.Config, pair.Request, default, noCall), "invalid zero request accepted");
    Check(zeroCalls == 0, "invalid candidates do not reach HTTP");
    var localModel = LocalModelStore.Catalog[0];
    var localOnly = zero with {
        Profiles = [], ActiveProfile = "", ZeroCandidateLocalModel = localModel.Id,
        ZeroCandidateConsentProfile = ModelManagerStore.LocalConsent(localModel.Id), ZeroCandidatePerformance = false
    };
    store.Save(localOnly);
    Check(store.Load().ZeroCandidateLocalModel == localModel.Id && File.ReadAllText(zeroGate) == "eco:" + zero.ZeroCandidateApproval,
        "local settings don't require cloud profile or key");
    Reject(() => store.Save(localOnly with { ZeroCandidateLocalModel = "https://example.invalid" }), "unlisted local model accepted");
    Reject(() => store.Save(localOnly with { ZeroCandidateConsentProfile = profile.Id }), "cloud consent reused for offline backend");
    Check(await TextAssistantApi.ExecuteAsync(localOnly, zeroInput with { Operation = "zero-eco" }, default, new FakeHandler(async request => {
        Check(request.RequestUri!.AbsoluteUri == "http://127.0.0.1/v1/chat/completions", "local-only route uses a loopback endpoint");
        using var body = JsonDocument.Parse(await request.Content!.ReadAsStringAsync());
        Check(body.RootElement.GetProperty("model").GetString() == localModel.Id &&
            !body.RootElement.GetProperty("chat_template_kwargs").GetProperty("enable_thinking").GetBoolean() &&
            body.RootElement.GetProperty("grammar").GetString()!.EndsWith("{2}"), "local generation disables reasoning and constrains output");
        return FakeHandler.Json(zeroOk);
    })) == "你好", "local-only route uses common candidate validation");
    Check(LocalPinyin.Describe("nihao") == ("ni hao", 2) && LocalPinyin.Describe("xi'an") == ("xi an", 2) &&
        LocalPinyin.Describe("wanan").Count == 0 && LocalPinyin.Describe("wsm").Count == 0 &&
        LocalPinyin.Describe("xian").Count == 0, "standard syllables guide clear full pinyin without constraining ambiguous or incomplete input");
    Check(LocalPinyin.CanVerify("XIANZAI") && LocalPinyin.Matches("XIANZAI", "现在") &&
        !LocalPinyin.Matches("xianzai", "在在") && !LocalPinyin.Matches("xianzai", "现"), "verify every character and the entire input");
    Check(LocalPinyin.Matches("xian", "先") && LocalPinyin.Matches("xian", "西安") &&
        LocalPinyin.Matches("xi'an", "西安") && !LocalPinyin.Matches("xi'an", "先") &&
        LocalPinyin.Matches("chongqing", "重庆") && LocalPinyin.Matches("zhong", "重") &&
        LocalPinyin.Matches("culue", "粗略") && !LocalPinyin.CanVerify("wsm"), "preserve ambiguous splits, manual boundaries, polyphones and partial input");
    var wrongReading = new TextAssistantInput("zero-pinyin", "xianzai", "", "", zero.ZeroCandidateApproval, "在在\n现在");
    FakeHandler DuplicateResponse() => new(async request => {
        Check(!(await request.Content!.ReadAsStringAsync()).Contains("\\u73B0", StringComparison.OrdinalIgnoreCase),
            "fallback candidates are never included in outgoing requests");
        return FakeHandler.Json("{\"choices\":[{\"message\":{\"content\":\"在在\"},\"finish_reason\":\"stop\"}]}");
    });
    Check(await TextAssistantApi.ExecuteAsync(zero, wrongReading, default, DuplicateResponse()) == "现在",
        "reject similarly sized off-pronunciation text and use the first verified reference");
    try {
        await TextAssistantApi.ExecuteAsync(zero, wrongReading with { LocalReferences = "再在" }, default, DuplicateResponse());
        throw new Exception("unverified fallback shown");
    } catch (ZeroCandidateFailure failure) { Check(failure.Code == "pronunciation", "hide zero result when no reference matches"); }
    try {
        await TextAssistantApi.ExecuteAsync(localOnly, zeroInput with { Operation = "zero-eco" }, default, modelRoot: store.Root);
        throw new Exception("missing local model fell through instead of failing locally");
    } catch (ZeroCandidateFailure failure) { Check(failure.Code == "local-not-installed", "missing local model has no cloud fallback"); }

    var sample = "test model bytes"u8.ToArray();
    var fixtureAsset = new ModelAsset("https://huggingface.co/fixture", sample.Length, Convert.ToHexString(SHA256.HashData(sample)));
    var disallowedRedirect = new HttpResponseMessage(HttpStatusCode.Redirect);
    disallowedRedirect.Headers.Location = new Uri("https://evil.invalid/model");
    using var fakeRedirect = new HttpClient(new FakeHandler(_ => Task.FromResult(disallowedRedirect)));
    using var rejectedDownload = new DownloadOperation("redirect", default);
    await RejectAsync(() => LocalModelStore.FetchAsync(fakeRedirect, fixtureAsset, Path.Combine(root, "redirect.part"),
        rejectedDownload, 0, fixtureAsset.Bytes, default), "unsafe download redirect followed");
    Check(!File.Exists(Path.Combine(root, "redirect.part")), "redirect didn't open any target");
    using var verifiedDownload = new DownloadOperation("verified", default);
    using var fakeModel = new HttpClient(new FakeHandler(_ => Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) { Content = new ByteArrayContent(sample) })));
    var verifiedFile = Path.Combine(root, "verified.part");
    await LocalModelStore.FetchAsync(fakeModel, fixtureAsset, verifiedFile, verifiedDownload, 0, fixtureAsset.Bytes, default);
    Check(File.ReadAllBytes(verifiedFile).SequenceEqual(sample) && verifiedDownload.Snapshot.Percent == 100, "verified download reports real progress");
    await RejectAsync(() => LocalModelStore.FetchAsync(fakeModel, fixtureAsset with { Sha256 = new string('0', 64) },
        Path.Combine(root, "failed.part"), verifiedDownload, 0, fixtureAsset.Bytes, default), "wrong digest treated as valid");
    using (var canceled = new CancellationTokenSource()) {
        canceled.Cancel();
        await RejectAsync(() => LocalModelStore.FetchAsync(fakeModel, fixtureAsset, Path.Combine(root, "cancelled.part"),
            verifiedDownload, 0, fixtureAsset.Bytes, canceled.Token), "cancelled download accepted");
    }
    var archive = Path.Combine(root, "fixture.zip");
    using (var zip = ZipFile.Open(archive, ZipArchiveMode.Create))
        foreach (var name in new[] { "llama-server.exe", "llama.dll", "ggml.dll", "../escape.txt", "unused.exe" }) zip.CreateEntry(name);
    var unpacked = Directory.CreateDirectory(Path.Combine(root, "unpacked")).FullName;
    LocalModelStore.UnpackRuntime(archive, unpacked, default);
    Check(Directory.GetFiles(unpacked).Length == 3 && !File.Exists(Path.Combine(root, "escape.txt")), "runtime extraction keeps only allowlisted file names");
    if (OperatingSystem.IsWindows()) {
        var listen = new TcpListener(IPAddress.Loopback, 0);
        listen.Start();
        try {
            var port = ((IPEndPoint)listen.LocalEndpoint).Port;
            using var outbound = new TcpClient();
            await outbound.ConnectAsync(IPAddress.Loopback, port);
            using var inbound = await listen.AcceptTcpClientAsync();
            int clientPort = ((IPEndPoint)outbound.Client.LocalEndPoint!).Port;
            Check(LocalModelInference.OwnsConnection(Environment.ProcessId, port, clientPort), "verify the server side of an actual loopback connection");
            Check(!LocalModelInference.OwnsConnection(int.MaxValue, port, clientPort), "reject a different owner of the loopback connection");
        } finally { listen.Stop(); }
    }
    foreach (var output in new[] { "你好。", "“你好”", "「你好。」", "\"你好\"." }) {
        var reply = JsonSerializer.Serialize(new { choices = new[] { new { message = new { content = output }, finish_reason = "stop" } } });
        Check(await TextAssistantApi.ExecuteAsync(zero, zeroInput, default, new FakeHandler(_ => Task.FromResult(FakeHandler.Json(reply)))) == "你好",
            "strip known single-answer decoration without changing the candidate");
    }
    foreach (var invalidOutput in new[] { "hello", "你好。解释", "你\n好", "0. 你好", "`你好`", "超过当前拼音长度" }) {
        var reply = JsonSerializer.Serialize(new { choices = new[] { new { message = new { content = invalidOutput }, finish_reason = "stop" } } });
        await RejectAsync(() => TextAssistantApi.ExecuteAsync(zero, zeroInput, default, new FakeHandler(_ => Task.FromResult(FakeHandler.Json(reply)))), "invalid candidate output");
    }
    try {
        const string truncated = "{\"choices\":[{\"message\":{\"content\":\"\"},\"finish_reason\":\"length\"}]}";
        await TextAssistantApi.ExecuteAsync(zero, zeroInput, default, new FakeHandler(_ => Task.FromResult(FakeHandler.Json(truncated))));
        throw new Exception("truncated zero response accepted");
    } catch (ZeroCandidateFailure failure) { Check(failure.Code == "length", "diagnostics distinguish incomplete API output"); }
    store.Save(zero with { ZeroCandidateEnabled = false, ZeroCandidateConsentProfile = "", ZeroCandidateApproval = "" });
    Check(!File.Exists(zeroGate), "revoking authorization removes native gate");
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
    if (args.Length == 1) foreach (var workerRequest in new[] { wire, zeroWire }) {
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
            var bytes = Encoding.UTF8.GetBytes(workerRequest); var length = BitConverter.GetBytes(bytes.Length);
            await pipe.WriteAsync(length, timeout.Token); await pipe.WriteAsync(bytes, timeout.Token);
            await pipe.ReadExactlyAsync(length, timeout.Token);
            var size = BitConverter.ToInt32(length); Check(size is > 0 and < 128 * 1024, "worker response size");
            var reply = new byte[size]; await pipe.ReadExactlyAsync(reply, timeout.Token);
            var workerReply = Encoding.UTF8.GetString(reply);
            Check(workerReply.StartsWith("OWOTEXT1\nERROR\n"), "disabled real worker rejects request");
            if (workerRequest == zeroWire)
                Check(workerReply == "OWOTEXT1\nERROR\n" + TextAssistantHost.Encode("zero-error:consent") + "\n",
                    "real worker reports only the machine-readable zero-candidate failure");
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
