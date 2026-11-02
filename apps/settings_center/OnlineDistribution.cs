using System.Net;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace OwO_Settings;

internal sealed record OnlineAsset(string Name, Uri Url, long Size, string Sha256);
internal sealed record OnlineResource(string Id, string Name, string Kind, string Version, string License, OnlineAsset Asset);
internal sealed record OnlineRelease(string Tag, Version Version, Uri Page, IReadOnlyList<OnlineAsset> Assets);

// No credentials, input text, contexts or user dictionaries leave this client.
internal sealed class OnlineDistribution : IDisposable
{
    internal const string Repository = "4919545484946757/OwO-release";
    private readonly HttpClient _http;
    internal OnlineDistribution(HttpMessageHandler? handler = null) {
        _http = new(handler ?? new HttpClientHandler { AllowAutoRedirect = false });
        _http.Timeout = Timeout.InfiniteTimeSpan;
        _http.DefaultRequestHeaders.UserAgent.ParseAdd("OwO-Settings/0.2");
        _http.DefaultRequestHeaders.Accept.ParseAdd("application/vnd.github+json");
    }
    internal static Version StableVersion(string tag) {
        if (!Regex.IsMatch(tag, @"^v?\d+\.\d+\.\d+$")) throw new InvalidDataException("不是受支持的正式版本号。");
        return Version.Parse(tag.TrimStart('v'));
    }
    internal static void ValidateUrl(Uri url, bool initial = false) {
        var hosts = new[] { "github.com", "api.github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com" };
        if (!url.IsAbsoluteUri || url.Scheme != "https" || url.Port != 443 || url.UserInfo.Length != 0 ||
            url.Fragment.Length != 0 || !hosts.Contains(url.IdnHost, StringComparer.OrdinalIgnoreCase))
            throw new InvalidDataException("拒绝非官方 HTTPS 下载地址。");
        if (initial && (url.Host != "github.com" || !url.AbsolutePath.StartsWith($"/{Repository}/releases/download/", StringComparison.Ordinal)))
            throw new InvalidDataException("资产不属于官方发行仓库。");
    }
    private async Task<HttpResponseMessage> GetAsync(Uri url, CancellationToken token) {
        for (var redirects = 0; redirects < 6; redirects++) {
            ValidateUrl(url);
            var response = await _http.GetAsync(url, HttpCompletionOption.ResponseHeadersRead, token);
            if ((int)response.StatusCode is >= 300 and <= 399) {
                var location = response.Headers.Location; response.Dispose();
                if (location is null) throw new InvalidDataException("下载重定向缺少目标。");
                url = location.IsAbsoluteUri ? location : new Uri(url, location); continue;
            }
            if (response.StatusCode == HttpStatusCode.NotFound) { response.Dispose(); throw new InvalidOperationException("官方尚未发布此资源或正式版本。"); }
            if (!response.IsSuccessStatusCode) { var code = (int)response.StatusCode; response.Dispose(); throw new HttpRequestException($"GitHub 请求失败（HTTP {code}），请稍后重试。"); }
            return response;
        }
        throw new InvalidDataException("下载重定向次数过多。");
    }
    private async Task<byte[]> ReadAsync(Uri url, int maximum, CancellationToken token) {
        using var response = await GetAsync(url, token);
        if (response.Content.Headers.ContentLength > maximum) throw new InvalidDataException("响应超过大小限制。");
        await using var input = await response.Content.ReadAsStreamAsync(token);
        using var output = new MemoryStream(); var buffer = new byte[16384]; int count;
        while ((count = await input.ReadAsync(buffer, token)) != 0) {
            if (output.Length + count > maximum) throw new InvalidDataException("响应超过大小限制。");
            output.Write(buffer, 0, count);
        }
        return output.ToArray();
    }
    internal async Task<OnlineRelease> LatestAsync(CancellationToken token) {
        var bytes = await ReadAsync(new($"https://api.github.com/repos/{Repository}/releases/latest"), 2 * 1024 * 1024, token);
        using var json = JsonDocument.Parse(bytes); var root = json.RootElement;
        if (root.GetProperty("draft").GetBoolean() || root.GetProperty("prerelease").GetBoolean()) throw new InvalidDataException("拒绝草稿或预发行版本。");
        var tag = root.GetProperty("tag_name").GetString()!; var version = StableVersion(tag);
        var assets = new List<OnlineAsset>();
        foreach (var item in root.GetProperty("assets").EnumerateArray()) {
            if (assets.Count >= 100) throw new InvalidDataException("发行资产过多。");
            var name = item.GetProperty("name").GetString()!;
            if (!SafeName(name)) throw new InvalidDataException("发行资产名称无效。");
            var url = new Uri(item.GetProperty("browser_download_url").GetString()!); ValidateUrl(url, true);
            if (!url.AbsolutePath.StartsWith($"/{Repository}/releases/download/{tag}/", StringComparison.Ordinal)) throw new InvalidDataException("资产版本不匹配。");
            var digest = item.TryGetProperty("digest", out var value) ? value.GetString() ?? "" : "";
            assets.Add(new(name, url, item.GetProperty("size").GetInt64(), digest.StartsWith("sha256:") ? digest[7..] : ""));
        }
        if (assets.Select(a => a.Name).Distinct(StringComparer.OrdinalIgnoreCase).Count() != assets.Count) throw new InvalidDataException("发行资产重名。");
        return new(tag, version, new Uri($"https://github.com/{Repository}/releases/tag/{tag}"), assets);
    }
    internal static bool SafeName(string name) => name.Length is > 0 and <= 180 && Regex.IsMatch(name, @"^[A-Za-z0-9][A-Za-z0-9._-]+$") && !name.Contains("..");
    internal static void ValidateAsset(OnlineAsset asset) {
        ValidateUrl(asset.Url, true);
        if (!SafeName(asset.Name) || asset.Size <= 0 || asset.Size > 2L * 1024 * 1024 * 1024 || !Regex.IsMatch(asset.Sha256, "^[a-fA-F0-9]{64}$"))
            throw new InvalidDataException("下载缺少有效大小或 SHA-256；不会降低校验要求。");
    }
    internal async Task<IReadOnlyList<OnlineResource>> ResourcesAsync(OnlineRelease release, CancellationToken token) {
        var resources = new List<OnlineResource>();
        var manifest = release.Assets.SingleOrDefault(a => a.Name == "owo-resources.json");
        if (manifest is null) {
            foreach (var asset in release.Assets.Where(a => a.Name.EndsWith(".owopkg", StringComparison.OrdinalIgnoreCase)))
                if (Regex.IsMatch(asset.Sha256, "^[a-fA-F0-9]{64}$")) {
                    var identity = Regex.Match(asset.Name, @"^(?<id>[a-zA-Z0-9][a-zA-Z0-9._-]+)-(?<version>\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?)\.owopkg$");
                    resources.Add(new(identity.Success ? identity.Groups["id"].Value : asset.Name, asset.Name, "plugin",
                        identity.Success ? identity.Groups["version"].Value : release.Tag, "以包内许可证和安装告知为准", asset));
                }
            return resources;
        }
        ValidateAsset(manifest);
        if (manifest.Size > 1024 * 1024) throw new InvalidDataException("资源目录过大。");
        var data = await ReadAsync(manifest.Url, 1024 * 1024, token);
        if (data.LongLength != manifest.Size || !Convert.ToHexString(SHA256.HashData(data)).Equals(manifest.Sha256, StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("资源目录校验失败。");
        using var json = JsonDocument.Parse(data); var root = json.RootElement;
        if (root.GetProperty("schema_version").GetInt32() != 1 || root.GetProperty("release_tag").GetString() != release.Tag) throw new InvalidDataException("资源目录版本不匹配。");
        var ids = new HashSet<string>(StringComparer.Ordinal);
        foreach (var item in root.GetProperty("resources").EnumerateArray()) {
            var id = item.GetProperty("id").GetString()!; var kind = item.GetProperty("kind").GetString()!;
            var name = item.GetProperty("name").GetString()!; var license = item.GetProperty("license").GetString()!;
            var version = item.GetProperty("version").GetString()!;
            if (!ids.Add(id) || resources.Count >= 100 || !SafeName(id) || name.Length is < 1 or > 128 || license.Length is < 1 or > 256 || version.Length is < 1 or > 64 ||
                kind is not ("plugin" or "dictionary" or "model")) throw new InvalidDataException("资源条目无效。");
            var asset = release.Assets.Single(a => a.Name == item.GetProperty("asset").GetString());
            var hash = item.GetProperty("sha256").GetString()!;
            if (asset.Sha256.Length != 0 && !asset.Sha256.Equals(hash, StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("目录与 GitHub 摘要不一致。");
            asset = asset with { Sha256 = hash }; ValidateAsset(asset);
            if (kind == "dictionary" && asset.Size > 128 * 1024 * 1024) throw new InvalidDataException("词库超过导入器的 128 MiB 上限。");
            if (kind == "plugin" && !asset.Name.EndsWith(".owopkg", StringComparison.OrdinalIgnoreCase) ||
                kind == "dictionary" && !new[] { ".json", ".txt", ".tsv", ".csv", ".yaml", ".yml", ".zip" }.Any(e => asset.Name.EndsWith(e, StringComparison.OrdinalIgnoreCase)) ||
                kind == "model" && !asset.Name.EndsWith(".lm", StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("资源格式不支持。");
            resources.Add(new(id, name, kind, version, license, asset));
        }
        return resources;
    }
    internal async Task<string> DownloadAsync(OnlineAsset asset, string directory, IProgress<double>? progress, CancellationToken token) {
        ValidateAsset(asset); Directory.CreateDirectory(directory);
        var destination = Path.Combine(directory, asset.Sha256.ToLowerInvariant()); Directory.CreateDirectory(destination);
        destination = Path.Combine(destination, asset.Name);
        var temporary = destination + "." + Guid.NewGuid().ToString("N") + ".partial";
        try {
            using var response = await GetAsync(asset.Url, token);
            if (response.Content.Headers.ContentLength is long length && length != asset.Size) throw new InvalidDataException("下载长度与发布信息不符。");
            await using var input = await response.Content.ReadAsStreamAsync(token);
            using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
            await using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None, 65536, true)) {
                var buffer = new byte[65536]; long total = 0; int count;
                while ((count = await input.ReadAsync(buffer, token)) != 0) {
                    total += count; if (total > asset.Size) throw new InvalidDataException("下载超出声明大小。");
                    hash.AppendData(buffer, 0, count); await output.WriteAsync(buffer.AsMemory(0, count), token);
                    progress?.Report((double)total / asset.Size * 100);
                }
                if (total != asset.Size || !Convert.ToHexString(hash.GetHashAndReset()).Equals(asset.Sha256, StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("下载不完整或 SHA-256 校验失败。");
                await output.FlushAsync(token);
            }
            token.ThrowIfCancellationRequested(); File.Move(temporary, destination, true); return destination;
        } finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
    public void Dispose() => _http.Dispose();
}
