using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using OwO_Settings;

internal static class OnlineDistributionTests
{
    private sealed class Handler(Func<HttpRequestMessage, HttpResponseMessage> send) : HttpMessageHandler {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) {
            cancellationToken.ThrowIfCancellationRequested(); return Task.FromResult(send(request));
        }
    }
    private sealed class ImmediateProgress(Action<double> action) : IProgress<double> { public void Report(double value) => action(value); }
    private static void Check(bool value) { if (!value) throw new Exception("distribution assertion failed"); }
    private static void Reject(Action action) { try { action(); } catch (InvalidDataException) { return; } throw new Exception("unsafe metadata accepted"); }
    private static async Task RejectAsync(Func<Task> action) { try { await action(); } catch (InvalidDataException) { return; } throw new Exception("unsafe download accepted"); }
    public static async Task Main(string[] args) {
        if (args.SequenceEqual(new[] { "--live-read-only" })) {
            using var live = new OnlineDistribution(); using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var latest = await live.LatestAsync(deadline.Token);
            var items = await live.ResourcesAsync(latest, deadline.Token);
            Console.WriteLine($"Official latest={latest.Tag}; assets={latest.Assets.Count}; verifiable resources={items.Count}; no downloads or installation performed.");
            return;
        }
        Check(OnlineDistribution.StableVersion("v0.2.7") > OnlineDistribution.StableVersion("0.2.6"));
        Check(OnlineDistribution.StableVersion("0.2.3") < OnlineDistribution.StableVersion("0.2.6"));
        Reject(() => OnlineDistribution.StableVersion("v1.0.0-rc.1"));
        foreach (var url in new[] { "http://github.com/file", "https://github.com.evil.invalid/file", "https://localhost/file", "https://github.com:444/file", "https://user@github.com/file", "file:///C:/test" })
            Reject(() => OnlineDistribution.ValidateUrl(new(url)));
        Reject(() => OnlineDistribution.ValidateUrl(new("https://github.com/another/repo/releases/download/v1/file"), true));
        var bytes = Encoding.UTF8.GetBytes("verified bytes"); var sha = Convert.ToHexString(SHA256.HashData(bytes));
        var urlGood = new Uri($"https://github.com/{OnlineDistribution.Repository}/releases/download/0.2.7/test.lm");
        var asset = new OnlineAsset("test.lm", urlGood, bytes.Length, sha);
        Reject(() => OnlineDistribution.ValidateAsset(asset with { Name = "../test.lm" }));
        Reject(() => OnlineDistribution.ValidateAsset(asset with { Sha256 = "" }));
        var directory = Path.Combine(Path.GetTempPath(), "owo-download-test-" + Guid.NewGuid().ToString("N"));
        try {
            using var client = new OnlineDistribution(new Handler(_ => new(HttpStatusCode.OK) { Content = new ByteArrayContent(bytes) }));
            var path = await client.DownloadAsync(asset, directory, null, CancellationToken.None);
            Check(File.ReadAllBytes(path).SequenceEqual(bytes));
            await RejectAsync(() => client.DownloadAsync(asset with { Sha256 = new string('0', 64) }, directory, null, CancellationToken.None));
            await RejectAsync(() => client.DownloadAsync(asset with { Size = bytes.Length + 1 }, directory, null, CancellationToken.None));
            using var redirects = new OnlineDistribution(new Handler(_ => {
                var response = new HttpResponseMessage(HttpStatusCode.Redirect); response.Headers.Location = new("https://evil.invalid/file"); return response;
            }));
            await RejectAsync(() => redirects.DownloadAsync(asset, directory, null, CancellationToken.None));
            using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
            try { await client.DownloadAsync(asset, directory, null, cancelled.Token); throw new Exception("cancellation ignored"); } catch (OperationCanceledException) { }
            Check(!Directory.EnumerateFiles(directory, "*.partial", SearchOption.AllDirectories).Any());
            var large = Enumerable.Range(0, 262144).Select(i => (byte)(i % 251)).ToArray();
            var largeAsset = asset with { Name = "pause.lm", Size = large.Length, Sha256 = Convert.ToHexString(SHA256.HashData(large)) };
            using var transfers = new OnlineDistribution(new Handler(_ => new(HttpStatusCode.OK) { Content = new ByteArrayContent(large) }));
            string verified;
            using (var operation = new DownloadOperation(largeAsset.Name, CancellationToken.None)) {
                var paused = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously); var once = false;
                var progress = new ImmediateProgress(value => { if (!once && value > 0) { once = true; operation.TogglePause(); paused.SetResult(); } });
                var transfer = operation.RunAsync(ct => transfers.DownloadAsync(largeAsset, directory, progress, ct, operation));
                await paused.Task.WaitAsync(TimeSpan.FromSeconds(3)); var stoppedAt = operation.Snapshot.Bytes;
                await Task.Delay(80); Check(!transfer.IsCompleted && operation.Snapshot.Bytes == stoppedAt && stoppedAt < large.Length);
                operation.TogglePause(); verified = await transfer.WaitAsync(TimeSpan.FromSeconds(3));
                Check(File.ReadAllBytes(verified).SequenceEqual(large) && operation.Snapshot.State == DownloadState.Completed);
            }
            using (var operation = new DownloadOperation(largeAsset.Name, CancellationToken.None)) {
                var paused = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously); var once = false;
                var progress = new ImmediateProgress(value => { if (!once && value > 0) { once = true; operation.TogglePause(); paused.SetResult(); } });
                var transfer = operation.RunAsync(ct => transfers.DownloadAsync(largeAsset, directory, progress, ct, operation));
                await paused.Task.WaitAsync(TimeSpan.FromSeconds(3)); operation.Cancel();
                try { await transfer.WaitAsync(TimeSpan.FromSeconds(3)); throw new Exception("Paused cancellation ignored"); } catch (OperationCanceledException) { }
                Check(File.ReadAllBytes(verified).SequenceEqual(large) && !Directory.EnumerateFiles(directory, "*.partial", SearchOption.AllDirectories).Any());
            }
            var catalog = JsonSerializer.SerializeToUtf8Bytes(new { schema_version = 1, release_tag = "0.2.7", resources = new[] {
                new { id = "sample.model", name = "测试模型", kind = "model", version = "1.0.0", license = "test-only", asset = "test.lm", sha256 = sha }
            }});
            using var catalogs = new OnlineDistribution(new Handler(_ => new(HttpStatusCode.OK) { Content = new ByteArrayContent(catalog) }));
            var manifest = new OnlineAsset("owo-resources.json", new($"https://github.com/{OnlineDistribution.Repository}/releases/download/0.2.7/owo-resources.json"), catalog.Length, Convert.ToHexString(SHA256.HashData(catalog)));
            var release = new OnlineRelease("0.2.7", new(0,2,7), new($"https://github.com/{OnlineDistribution.Repository}/releases/tag/0.2.7"), [asset, manifest]);
            var resources = await catalogs.ResourcesAsync(release, CancellationToken.None);
            Check(resources.Count == 1 && resources[0].Kind == "model");
            var oldPlugin = asset with { Name = "org.owo.candidate-glass-1.4.1.owopkg" };
            var oldResources = await catalogs.ResourcesAsync(release with { Assets = [oldPlugin] }, CancellationToken.None);
            Check(oldResources.Single().Id == "org.owo.candidate-glass" && oldResources.Single().Version == "1.4.1");
            await RejectAsync(() => catalogs.ResourcesAsync(release with { Assets = [asset with { Sha256 = new string('0',64) }, manifest] }, CancellationToken.None));
            var json = JsonSerializer.SerializeToUtf8Bytes(new { tag_name = "0.2.7", draft = false, prerelease = false, assets = new[] {
                new { name = "test.lm", browser_download_url = urlGood.AbsoluteUri, size = bytes.Length, digest = "sha256:" + sha }
            }});
            using var releases = new OnlineDistribution(new Handler(_ => new(HttpStatusCode.OK) { Content = new ByteArrayContent(json) }));
            Check((await releases.LatestAsync(CancellationToken.None)).Version == new Version(0,2,7));
        } finally { if (Directory.Exists(directory)) Directory.Delete(directory, true); }
        Console.WriteLine("Online distribution: versions, origins, redirects, hashes, lengths, cancellation, catalog and cleanup passed.");
    }
}
