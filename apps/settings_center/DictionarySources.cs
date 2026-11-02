using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using System.IO.Compression;

namespace OwO_Settings;

internal sealed record DictionarySource(string Id, string Name, string Repository, string Folder, string License, string Description);
internal sealed record DictionaryDownload(DictionarySource Source, string Name, Uri Url, long Size, string Hash, bool GitBlob)
{
    internal string Title => Source.Id == "thu" ? Path.GetFileNameWithoutExtension(Name) switch {
        "THUOCL_IT" => "计算机与 IT", "THUOCL_animal" => "动物", "THUOCL_caijing" => "财经",
        "THUOCL_car" => "汽车", "THUOCL_chengyu" => "成语", "THUOCL_diming" => "地名",
        "THUOCL_food" => "饮食", "THUOCL_law" => "法律", "THUOCL_lishimingren" => "历史人物",
        "THUOCL_medical" => "医学", "THUOCL_poem" => "诗词", _ => Name
    } : Name;
}
internal sealed record PreparedDictionary(DictionaryImport Data, int Available, string Provenance);

internal sealed class DictionarySources : IDisposable
{
    internal const int DownloadLimit = 256 * 1024 * 1024;
    internal static readonly DictionarySource[] Sources = [
        new("ice", "雾凇拼音", "iDvel/rime-ice", "cn_dicts", "GPL-3.0-only", "通用中文。部分词表无拼音；与内置词库可能重复。"),
        new("wiki", "维基百科词库", "felixonmars/fcitx5-pinyin-zhwiki", "", "数据遵循 Wikimedia 许可，详见上游声明", "百科与专名；使用带拼音的 Rime 发行数据，不运行构建脚本。"),
        new("moegirl", "萌娘百科词库", "outloudvi/mw2fcitx", "", "数据许可见上游，不能用生成器 MIT 代替", "动漫、游戏和角色。安装前请核对上游数据许可。"),
        new("thu", "THUOCL 专业词库", "thunlp/THUOCL", "data", "MIT", "行业词语和原始词频，不含拼音；导入后需补编码才能参与候选。"),
        new("cedict", "CC-CEDICT", "", "", "CC BY-SA 4.0", "简繁词语与带声调拼音；转换为简体、无调全拼。不导入英文释义。")
    ];
    private readonly HttpClient _http;
    internal DictionarySources(HttpMessageHandler? handler = null) {
        _http = new(handler ?? new HttpClientHandler { AllowAutoRedirect = false });
        _http.Timeout = Timeout.InfiniteTimeSpan;
        _http.DefaultRequestHeaders.UserAgent.ParseAdd("OwO-DictionaryManager/0.2");
    }
    internal static void ValidateUrl(Uri uri) {
        if (!uri.IsAbsoluteUri || uri.Scheme != "https" || uri.Port != 443 || uri.UserInfo != "" || uri.Fragment != "") throw new InvalidDataException("拒绝非 HTTPS 词库地址。");
        var host = uri.IdnHost;
        if (host is "release-assets.githubusercontent.com" or "objects.githubusercontent.com") return;
        if (host == "www.mdbg.net" && uri.AbsolutePath == "/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.txt.gz") return;
        var prefix = host switch { "api.github.com" => "/repos/", "raw.githubusercontent.com" => "/", "github.com" => "/", _ => null };
        if (prefix is not null && Sources.Where(s => s.Repository != "").Any(s => uri.AbsolutePath.StartsWith(prefix + s.Repository + "/", StringComparison.Ordinal))) return;
        throw new InvalidDataException("地址不属于精选上游白名单。");
    }
    private async Task<byte[]> ReadAsync(Uri uri, int limit, IProgress<double>? progress, CancellationToken token, long expected = 0, DownloadOperation? operation = null) {
        for (var redirects = 0; redirects < 6; redirects++) {
            if (operation is not null) await operation.WaitIfPausedAsync(token);
            ValidateUrl(uri);
            using var response = await _http.GetAsync(uri, HttpCompletionOption.ResponseHeadersRead, token);
            if ((int)response.StatusCode is >= 300 and < 400) {
                var next = response.Headers.Location ?? throw new InvalidDataException("缺少重定向目标。");
                uri = next.IsAbsoluteUri ? next : new Uri(uri, next); continue;
            }
            if (!response.IsSuccessStatusCode) throw new HttpRequestException($"上游 HTTP {(int)response.StatusCode}；可能限流或网络不可用，可稍后重试。");
            var length = response.Content.Headers.ContentLength;
            operation?.Report(0, expected > 0 ? expected : length.GetValueOrDefault());
            if (length > limit || expected > 0 && length.HasValue && length != expected) throw new InvalidDataException("词库大小不符或超过限制。");
            using var output = new MemoryStream(); await using var stream = await response.Content.ReadAsStreamAsync(token);
            var buffer = new byte[65536]; int count;
            while (true) {
                if (operation is not null) await operation.WaitIfPausedAsync(token);
                count = await stream.ReadAsync(buffer, token);
                if (operation is not null) await operation.WaitIfPausedAsync(token);
                if (count == 0) break;
                if (output.Length + count > limit) throw new InvalidDataException("下载超限。");
                output.Write(buffer, 0, count);
                operation?.Report(output.Length, expected > 0 ? expected : length.GetValueOrDefault());
                if ((expected > 0 ? expected : length.GetValueOrDefault()) is var total && total > 0) progress?.Report(output.Length * 100.0 / total);
            }
            if (expected > 0 && output.Length != expected) throw new InvalidDataException("词库下载不完整。");
            return output.ToArray();
        }
        throw new InvalidDataException("重定向过多。");
    }
    internal async Task<IReadOnlyList<DictionaryDownload>> ListAsync(DictionarySource source, CancellationToken token) {
        if (!Sources.Contains(source)) throw new InvalidDataException("未知词库源。");
        if (source.Id == "cedict") return [new(source, "cedict.txt.gz", new("https://www.mdbg.net/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.txt.gz"), 0, "", false)];
        var path = source.Folder == "" ? "releases/latest" : "contents/" + source.Folder;
        using var json = JsonDocument.Parse(await ReadAsync(new($"https://api.github.com/repos/{source.Repository}/{path}"), 4 * 1024 * 1024, null, token));
        var result = new List<DictionaryDownload>();
        foreach (var row in (source.Folder == "" ? json.RootElement.GetProperty("assets") : json.RootElement).EnumerateArray()) {
            var name = row.GetProperty("name").GetString()!;
            if (!(name.EndsWith(".dict.yaml", StringComparison.OrdinalIgnoreCase) || source.Id == "thu" && name.EndsWith(".txt"))) continue;
            if (!OnlineDistribution.SafeName(name)) continue;
            var url = new Uri(row.GetProperty(source.Folder == "" ? "browser_download_url" : "download_url").GetString()!); ValidateUrl(url);
            var hash = source.Folder == "" ? row.GetProperty("digest").GetString() ?? "" : row.GetProperty("sha").GetString() ?? "";
            if (source.Folder == "" && !hash.StartsWith("sha256:")) continue;
            hash = hash.Replace("sha256:", "");
            if (!Regex.IsMatch(hash, source.Folder == "" ? "^[a-fA-F0-9]{64}$" : "^[a-fA-F0-9]{40}$")) continue;
            result.Add(new(source, name, url, row.GetProperty("size").GetInt64(), hash, source.Folder != ""));
            if (result.Count > 100) throw new InvalidDataException("源目录条目过多。");
        }
        return result;
    }
    internal async Task<PreparedDictionary> FetchAsync(DictionaryDownload item, int maximum, IProgress<double>? progress, CancellationToken token, DownloadOperation? operation = null) {
        using var linked = operation is null ? null : CancellationTokenSource.CreateLinkedTokenSource(token, operation.Token);
        token = linked?.Token ?? token;
        if (maximum < 1 || maximum > DictionaryStore.MaximumEntries || !Sources.Contains(item.Source) || item.Size < 0 || item.Size > DownloadLimit) throw new InvalidDataException("下载或选取数量超限。");
        var data = await ReadAsync(item.Url, DownloadLimit, progress, token, item.Size, operation);
        token.ThrowIfCancellationRequested(); operation?.Processing("正在校验并转换词库");
        var digest = Convert.ToHexString(SHA256.HashData(data)).ToLowerInvariant();
        if (item.Source.Id != "cedict") {
            var actual = digest;
            if (item.GitBlob) {
                using var blob = IncrementalHash.CreateHash(HashAlgorithmName.SHA1);
                blob.AppendData(Encoding.ASCII.GetBytes($"blob {data.Length}\0")); blob.AppendData(data);
                actual = Convert.ToHexString(blob.GetHashAndReset());
            }
            if (!actual.Equals(item.Hash, StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("词库内容与上游摘要不一致，请刷新目录重试。");
        }
        // CEDICT has HTTPS transport + local SHA-256 only, no upstream checksum.
        var result = await Task.Run(() => Prepare(item, data, maximum, digest, token), token);
        token.ThrowIfCancellationRequested(); return result;
    }
    internal static PreparedDictionary Prepare(DictionaryDownload item, byte[] data, int maximum, string digest, CancellationToken token) {
        if (maximum < 1 || maximum > DictionaryStore.MaximumEntries) throw new InvalidDataException("选取数量无效。");
        if (item.Source.Id == "cedict") {
            using var gzip = new GZipStream(new MemoryStream(data), CompressionMode.Decompress); using var expanded = new MemoryStream();
            var buffer = new byte[65536]; int count;
            while ((count = gzip.Read(buffer)) != 0) { token.ThrowIfCancellationRequested(); if (expanded.Length + count > DownloadLimit) throw new InvalidDataException("词库解压超限。"); expanded.Write(buffer, 0, count); }
            data = expanded.ToArray();
        }
        var text = new UTF8Encoding(false, true).GetString(data).TrimStart('\uFEFF');
        var output = new StringBuilder("input\ttext\tcount\n"); var total = 0; var body = item.Source.Id == "cedict" || !item.Name.EndsWith(".yaml");
        using var reader = new StringReader(text); string? line;
        while ((line = reader.ReadLine()) is not null) {
            token.ThrowIfCancellationRequested(); if (item.Source.Id != "thu") line = line.Trim();
            if (line == "...") { body = true; continue; }
            if (!body || string.IsNullOrWhiteSpace(line) || line.TrimStart().StartsWith('#')) continue;
            string input, word;
            if (item.Source.Id == "cedict") {
                var match = Regex.Match(line, @"^\S+ (\S+) \[([^\]]+)\] /", RegexOptions.None, TimeSpan.FromSeconds(1));
                if (!match.Success) continue;
                word = match.Groups[1].Value;
                input = Regex.Replace(match.Groups[2].Value.ToLowerInvariant().Replace("u:", "v").Replace('ü', 'v'), "[1-5]", "");
            } else {
                var cells = line.Split('\t'); word = cells[0]; input = item.Source.Id == "thu" || cells.Length < 2 ? "" : cells[1];
                // THUOCL uses padded term columns (e.g. "字符串 \t...").
                // The padding is not part of the dictionary word. Do not
                // apply this rule to general/Gboard text expansions.
                if (item.Source.Id == "thu") word = word.Trim();
            }
            if (word.Length == 0 || word.Any(char.IsControl)) continue;
            total++;
            if (total <= maximum) output.Append(input).Append('\t').Append(word).Append("\t1\n");
        }
        var parsed = DictionaryFormats.Parse(output.ToString(), "online.tsv");
        return new(parsed, total, $"{item.Source.Name}/{item.Name} sha256:{digest}");
    }
    public void Dispose() => _http.Dispose();
}
