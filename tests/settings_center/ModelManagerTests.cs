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
        using var download = new DownloadOperation(selected.Name, default, TimeSpan.FromMinutes(6));
        await download.RunAsync(token => localTestStore.InstallAsync(selected, download, token));
        var chat = await LocalModelChat.SendAsync(localTestStore, selected, "", [], "请用中文说一句问候。", default);
        Check(chat.Text.Any(c => c is >= '\u3400' and <= '\u9fff'), "real local chat returns Chinese text");
        Console.WriteLine($"PASS: verified download and CPU debug chat for {selected.Name}");
    } finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
    return;
}
const string targetPinyin = "tihuandangqianbanbenyigongceshi";
const string targetText = "替换当前版本以供测试";
if (args.Length == 1 && args[0] == "--verify-live-zero-model") {
    var installed = new ModelManagerStore(ModelManagerStore.DefaultRoot);
    var clock = Stopwatch.StartNew();
    var result = await ZeroModelReranker.RankAsync(installed, installed.Load(),
        new("eco", targetPinyin, "",
            ["替换当前版本一共测试", "替换当前版本一公测试", targetText, "替换当前版本义工测试"]), default);
    Check(result == targetText, $"Qwen3-4B did not choose the expected phrase: {result}");
    Console.WriteLine($"PASS: Qwen3-4B selected {result} ({clock.Elapsed.TotalSeconds:F1}s)");
    clock.Restart();
    var live = await ZeroCandidateDebug.ProbeAsync(targetPinyin, false, installed, installed.Load(), default);
    Check(live.Paths.Contains(targetText) && live.QwenApplied && live.Final == targetText,
        $"Live lattice did not produce the expected Qwen3-4B winner: {live.Final}");
    Console.WriteLine($"PASS: {live.Paths.Length} live lattice paths → Qwen3-4B → {live.Final} ({clock.Elapsed.TotalSeconds:F1}s)");
    return;
}
try {
    var rankInput = new ZeroRankInput("eco", targetPinyin, "", ["替换当前版本一共测试", targetText]);
    var rankWire = "OWORANK1\neco\n" + TextAssistantHost.Encode(targetPinyin) + "\n\n" +
        string.Join("\n", rankInput.Candidates.Select(TextAssistantHost.Encode)) + "\n";
    Check(ZeroModelReranker.Parse(rankWire).Candidates.SequenceEqual(rankInput.Candidates), "rank wire preserves a bounded lattice");
    Reject(() => ZeroModelReranker.Parse(rankWire.Replace("eco", "cloud")), "rank must not accept a cloud mode");
    Reject(() => ZeroModelReranker.Parse(rankWire.Replace(TextAssistantHost.Encode(rankInput.Candidates[0]),
        TextAssistantHost.Encode(targetText))), "rank must reject duplicate rows");
    Check(await ZeroModelReranker.RankAsync(store, settings, rankInput, default, new FakeHandler(async request => {
        using var body = JsonDocument.Parse(await request.Content!.ReadAsStringAsync());
        var data = body.RootElement;
        Check(request.RequestUri!.IsLoopback && data.GetProperty("model").GetString() == "qwen3-4b-q4km", "only local Qwen3-4B");
        Check(data.GetProperty("grammar").GetString() == "root ::= \"A\" | \"B\"" &&
            !data.GetProperty("chat_template_kwargs").GetProperty("enable_thinking").GetBoolean(), "only shortlist IDs, no reasoning");
        return FakeHandler.Json("{\"choices\":[{\"message\":{\"content\":\"B\"},\"finish_reason\":\"stop\"}]}");
    })) == targetText, "selection must use the ranked member, not the initial winner");
    foreach (var invalid in new[] { "在在", "C", "AB", "B: " + targetText }) {
        await RejectAsync(() => ZeroModelReranker.RankAsync(store, settings, rankInput, default,
            new FakeHandler(_ => Task.FromResult(FakeHandler.Json(JsonSerializer.Serialize(new {
                choices = new[] { new { message = new { content = invalid }, finish_reason = "stop" } }
            }))))), "rank must reject free text and invalid IDs");
    }
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
    var zeroGate = Path.Combine(store.Root, "zero-candidate.mode");
    Check(ModelManagerStore.ZeroCandidateMode(zero) == "pinyin", "models without context are pinned to pinyin");
    File.WriteAllText(Path.Combine(store.Root, "profiles.json"), JsonSerializer.Serialize(zero));
    var zeroWire = "OWOTEXT1\nzero-pinyin\n" + TextAssistantHost.Encode("nihao") + "\n\n\n" + zero.ZeroCandidateApproval + "\n";
    Reject(() => TextAssistantHost.Parse(zeroWire), "retired wire operation cannot call the generation helper");
    var migrated = ModelManagerStore.UseCoreRanking(store.Load());
    Check(migrated.ZeroCandidateEnabled && migrated.ZeroCandidateNoContext && migrated.ZeroCandidateApproval.Length == 0,
        "old pinyin-only opt-in migrates without widening context");
    store.Save(migrated);
    Check(File.ReadAllText(zeroGate) == "lattice-pinyin" &&
        !File.Exists(Path.Combine(store.Root, "selection-assistant.enabled")),
        "old TSF cannot consume the new lattice marker, and selection assistance stays disabled");
    var energy = migrated with { Profiles = [], ActiveProfile = "", ZeroCandidateNoContext = false, ZeroCandidatePerformance = false };
    store.Save(energy);
    Check(File.ReadAllText(zeroGate) == "lattice-eco", "local ranking does not require any API profile");
    store.Save(energy with { ZeroCandidatePerformance = true });
    Check(File.ReadAllText(zeroGate) == "lattice-performance", "performance mode persists independently of API");
    var localModel = LocalModelStore.Catalog[0];
    Check(LocalModelStore.Catalog.Count == 1 && localModel.Id == LocalModelStore.PreferredId &&
        localModel.Weights.Bytes == 2497280256L, "Qwen3-4B is the sole download and size supports over 2 GiB");
    var localOnly = zero with {
        Profiles = [], ActiveProfile = "", ZeroCandidateLocalModel = localModel.Id,
        ZeroCandidateConsentProfile = ModelManagerStore.LocalConsent(localModel.Id), ZeroCandidatePerformance = false
    };
    store.Save(localOnly);
    Check(store.Load().ZeroCandidateLocalModel == localModel.Id && File.ReadAllText(zeroGate) == "lattice-eco",
        "local settings don't require cloud profile or key");
    if (OperatingSystem.IsWindows()) {
        var parent = Directory.CreateDirectory(Path.Combine(root, "another-drive-fixture")).FullName;
        store.Save(localOnly with { LocalModelStorageParent = parent });
        var alternate = LocalModelStore.ForModel(store.Root, parent, localModel);
        Check(store.Load().LocalModelStorageParent == parent && LocalModelStore.CanChooseStorage(parent) &&
            alternate.Root == Path.Combine(parent, "OwO-LocalModels") && alternate.Executable == new LocalModelStore(store.Root).Executable &&
            LocalModelStore.ForModel(store.Root, parent, LocalModelStore.Legacy).Root == Path.Combine(store.Root, "local"),
            "4B alternate storage only relocates the new weight, not native runtime or legacy files");
        Directory.CreateDirectory(alternate.Root);
        var unverified = localModel with { Weights = localModel.Weights with { Bytes = 4 } };
        File.WriteAllBytes(alternate.ModelPath(unverified), new byte[4]);
        Check(!alternate.HasWeights(unverified), "preexisting same-size weights in user-selected folders are not trusted");
        if (LocalModelStore.Supported) {
            var runtimeDir = Directory.CreateDirectory(Path.GetDirectoryName(alternate.Executable)!).FullName;
            File.WriteAllBytes(alternate.Executable, [0]);
            File.WriteAllText(Path.Combine(runtimeDir, "verified.sha256"), LocalModelStore.Runtime.Sha256);
            var bytes = "GGUF"u8.ToArray();
            var fixtureWeight = unverified with { Weights = localModel.Weights with {
                Bytes = bytes.Length, Sha256 = Convert.ToHexString(SHA256.HashData(bytes))
            } };
            using var progress = new DownloadOperation("local model placement fixture", default);
            await alternate.InstallAsync(fixtureWeight, progress, default,
                new FakeHandler(_ => Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) { Content = new ByteArrayContent(bytes) })));
            Check(alternate.HasWeights(fixtureWeight) && File.ReadAllBytes(alternate.ModelPath(fixtureWeight)).SequenceEqual(bytes),
                "hash-verified weights acquire a receipt in shared app storage");
            File.SetLastWriteTimeUtc(alternate.ModelPath(fixtureWeight), DateTime.UtcNow.AddDays(-1));
            Check(!alternate.HasWeights(fixtureWeight), "changed file metadata invalidates the existing receipt");
        }
        Reject(() => store.Save(localOnly with { LocalModelStorageParent = @"\\server\models" }), "UNC model storage accepted");
        Reject(() => store.Save(localOnly with { LocalModelStorageParent = "relative" }), "relative model storage accepted");
    }
    var oldId = LocalModelStore.Legacy.Id;
    store.Save(localOnly with { ZeroCandidateLocalModel = oldId, ZeroCandidateConsentProfile = ModelManagerStore.LocalConsent(oldId) });
    Check(store.Load().ZeroCandidateEnabled && store.Load().ZeroCandidateLocalModel == oldId,
        "migration keeps 0 enabled and retains its old model ID for local debugging");
    Reject(() => store.Save(localOnly with { ZeroCandidateLocalModel = "https://example.invalid" }), "unlisted local model accepted");
    Check(ModelManagerStore.UseCoreRanking(store.Load()).ZeroCandidateEnabled &&
        !ModelManagerStore.UseCoreRanking(store.Load()).ZeroCandidateNoContext, "authorized offline setting migrates with its prior context scope");
    var chatHistory = new[] { new LocalChatTurn("先前的问题", "先前的回答") };
    var localReply = await LocalModelChat.SendAsync(new LocalModelStore(store.Root), localModel, "只用中文", chatHistory, "继续", default,
        new FakeHandler(async request => {
            Check(request.RequestUri!.IsLoopback, "debug chat only talks to loopback");
            using var body = JsonDocument.Parse(await request.Content!.ReadAsStringAsync());
            var messages = body.RootElement.GetProperty("messages");
            Check(messages.GetArrayLength() == 4 && messages[0].GetProperty("content").GetString() == "只用中文" &&
                messages[2].GetProperty("role").GetString() == "assistant", "handwritten prompt and prior turns preserved");
            Check(!body.RootElement.TryGetProperty("tools", out _), "debug chat advertises no tools");
            return FakeHandler.Json("{\"choices\":[{\"message\":{\"content\":\"本地回复\"},\"finish_reason\":\"length\"}]}");
        }));
    Check(localReply == new LocalChatReply("本地回复", true), "debug chat labels truncated text");
    var longHistory = Enumerable.Repeat(new LocalChatTurn(new string('a', 1200), new string('b', 1200)), 10).ToArray();
    Check(LocalModelChat.Messages("", longHistory, new string('c', 1000)).Count == 1, "drop whole turns outside debug context budget");
    using (var aborted = new CancellationTokenSource()) {
        aborted.Cancel();
        await RejectAsync(() => LocalModelChat.SendAsync(new LocalModelStore(store.Root), localModel, "", [], "你好", aborted.Token,
            new FakeHandler(_ => throw new Exception("unexpected send"))), "canceled debug chat must not generate");
    }

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
    store.Save(energy with { ZeroCandidateEnabled = false });
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
    if (args.Length == 1) foreach (var workerRequest in new[] { wire, zeroWire, rankWire }) {
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
                Check(workerReply == "OWOTEXT1\nERROR\n" + TextAssistantHost.Encode("文字助手请求格式错误。") + "\n",
                    "real worker rejects retired transport before executing any model request");
            if (workerRequest == rankWire)
                Check(workerReply == "OWOTEXT1\nERROR\n" + TextAssistantHost.Encode("0 号候选未启用或档位已变更。") + "\n",
                    "real worker requires a live zero-candidate opt-in");
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
