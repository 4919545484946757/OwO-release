using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Text.RegularExpressions;

namespace OwO_Settings;

internal sealed record SchemeDictionarySource(string Id, string Title, string License, string Url, int Bytes, string Sha256);
internal sealed record SchemePackageDocument(int Version, string Scheme, string[] CandidateFields, string[] LearningFields,
    string? LexiconExtension, SchemeDictionarySource[] Sources);

// Declarative package assets only. No plugin XAML/code is loaded into Settings.
internal sealed class SchemePackageProfile
{
    internal const int MaximumLexiconBytes = 16 * 1024 * 1024;
    internal const string ShortcutHeader = "OWO_SCHEME_SHORTCUTS_V1";
    internal static readonly string[] Actions = ["raw", "cursor_left", "cursor_right", "previous_page", "next_page", "expanded_previous_page", "expanded_next_page"];
    private static readonly Regex Id = new(@"^[A-Za-z0-9][A-Za-z0-9_-]{0,95}(\.[A-Za-z0-9_-]+)*$", RegexOptions.CultureInvariant);
    private static readonly Regex Extension = new(@"^\.[a-z0-9]{1,10}$", RegexOptions.CultureInvariant);
    private static readonly Regex RawPath = new(@"^/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/[a-fA-F0-9]{40}/[A-Za-z0-9_./-]+$", RegexOptions.CultureInvariant);
    private static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow, RespectRequiredConstructorParameters = true, MaxDepth = 8 };
    internal SchemePackageDocument Document { get; }
    internal string DataDirectory { get; }
    internal string DefaultsPath { get; }
    internal bool HasShortcuts => File.Exists(DefaultsPath);
    internal string ShortcutPath => Path.Combine(DataDirectory, "shortcuts.conf");
    internal string UserLexiconPath => Path.Combine(DataDirectory, "user-lexicon" + Document.LexiconExtension);
    private SchemePackageProfile(SchemePackageDocument document, string data, string defaults) {
        Document = document; DataDirectory = data; DefaultsPath = defaults;
    }

    internal static SchemePackageProfile? Load(string store, string pluginId, string version, string schemeId) {
        var prefix = "plugin:" + pluginId + "/";
        if (!schemeId.StartsWith(prefix, StringComparison.Ordinal) || !SafeId(pluginId) ||
            !SafeId(schemeId[prefix.Length..]) || version.Length is < 1 or > 96 || !Regex.IsMatch(version, @"^[a-zA-Z0-9][a-zA-Z0-9.+-]*$") || version.EndsWith('.'))
            throw new InvalidDataException("输入方案或插件版本标识无效。");
        var local = schemeId[prefix.Length..];
        var directory = Path.Combine(store, "versions", pluginId, version, "input-schemes", local);
        var metadata = Path.Combine(directory, "settings.json");
        CheckPath(metadata);
        if (!File.Exists(metadata)) return null;
        using var file = File.OpenRead(metadata);
        if (file.Length is <= 0 or > 65536) throw new InvalidDataException("方案设置声明超过容量限制。");
        var bytes = new byte[(int)file.Length]; file.ReadExactly(bytes);
        using var parsed = JsonDocument.Parse(bytes, new JsonDocumentOptions { MaxDepth = 8 });
        RejectDuplicates(parsed.RootElement);
        var document = JsonSerializer.Deserialize<SchemePackageDocument>(bytes, Json) ?? throw new InvalidDataException("输入方案配置声明无效。");
        if (document.Version != 1 || document.Scheme != local || document.CandidateFields is null || document.LearningFields is null || document.Sources is null ||
            document.Sources.Length > 32 || document.LexiconExtension is not null && !Extension.IsMatch(document.LexiconExtension) ||
            document.Sources.Length != 0 && document.LexiconExtension is null)
            throw new InvalidDataException("输入方案配置声明与方案身份不匹配。");
        var keys = document.CandidateFields.Concat(document.LearningFields).ToArray();
        if (keys.Length > 128 || keys.Any(key => !SafeId(key)) || keys.Distinct(StringComparer.Ordinal).Count() != keys.Length ||
            document.Sources.Any(source => source is null) || document.Sources.Select(source => source.Id).Distinct(StringComparer.OrdinalIgnoreCase).Count() != document.Sources.Length)
            throw new InvalidDataException("方案字段或词库来源重复、无效或超过限制。");
        foreach (var source in document.Sources) ValidateSource(source);
        return new(document, Path.Combine(store, "data", pluginId, "input-schemes", local), Path.Combine(directory, "shortcuts.conf"));
    }

    internal static bool SafeId(string? id) => id is { Length: > 0 and <= 96 } && Id.IsMatch(id);
    private static void RejectDuplicates(JsonElement element) {
        if (element.ValueKind == JsonValueKind.Object) {
            var keys = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in element.EnumerateObject()) {
                if (!keys.Add(property.Name)) throw new InvalidDataException("方案声明含重复字段。");
                RejectDuplicates(property.Value);
            }
        } else if (element.ValueKind == JsonValueKind.Array) {
            foreach (var item in element.EnumerateArray()) RejectDuplicates(item);
        }
    }
    internal static void CheckPath(string path) {
        for (string? current = Path.GetFullPath(path); current is not null; current = Path.GetDirectoryName(current)) {
            if ((File.Exists(current) || Directory.Exists(current)) && (File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                throw new IOException("方案配置不允许使用链接或重解析路径。");
        }
    }
    private static Uri ValidateSource(SchemeDictionarySource source) {
        if (!SafeId(source.Id) || string.IsNullOrWhiteSpace(source.Title) || source.Title.Length > 96 ||
            string.IsNullOrWhiteSpace(source.License) || source.License.Length > 128 || source.Bytes is < 1 or > MaximumLexiconBytes ||
            source.Sha256 is null || !Regex.IsMatch(source.Sha256, @"^[a-fA-F0-9]{64}$") ||
            !Uri.TryCreate(source.Url, UriKind.Absolute, out var uri) || uri.Scheme != "https" || uri.Port != 443 ||
            uri.IdnHost != "raw.githubusercontent.com" || uri.UserInfo != "" || uri.Query != "" || uri.Fragment != "" ||
            !RawPath.IsMatch(uri.AbsolutePath) || uri.AbsolutePath.Contains("..", StringComparison.Ordinal))
            throw new InvalidDataException("方案词库来源必须为固定 GitHub 提交的 HTTPS 文件，并提供大小、许可和 SHA-256。");
        return uri;
    }

    internal static async Task<byte[]> ReadLexiconAsync(string path, CancellationToken token = default) {
        CheckPath(path);
        await using var file = File.OpenRead(path);
        if (file.Length is <= 0 or > MaximumLexiconBytes) throw new InvalidDataException("方案词库文件必须为 1 字节至 16 MiB。");
        var bytes = new byte[(int)file.Length]; await file.ReadExactlyAsync(bytes, token);
        if (file.ReadByte() != -1) throw new InvalidDataException("读取期间词库大小发生变化。");
        return bytes;
    }
    internal string DownloadPath(SchemeDictionarySource source) => Path.Combine(DataDirectory, "dictionaries", source.Id + Document.LexiconExtension);
    internal static async Task<byte[]> DownloadAsync(SchemeDictionarySource source, HttpClient http, CancellationToken token) {
        using var response = await http.GetAsync(ValidateSource(source), HttpCompletionOption.ResponseHeadersRead, token);
        response.EnsureSuccessStatusCode();
        if (response.Content.Headers.ContentLength is long length && length != source.Bytes) throw new InvalidDataException("下载词库大小不符。");
        await using var stream = await response.Content.ReadAsStreamAsync(token);
        var bytes = new byte[source.Bytes]; await stream.ReadExactlyAsync(bytes, token);
        if (await stream.ReadAsync(new byte[1], token) != 0 || !Convert.ToHexString(SHA256.HashData(bytes)).Equals(source.Sha256, StringComparison.OrdinalIgnoreCase))
            throw new InvalidDataException("下载词库大小或 SHA-256 不符。");
        return bytes;
    }
    internal static void WriteFile(string path, byte[] bytes) {
        CheckPath(path);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var temp = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try { File.WriteAllBytes(temp, bytes); File.Move(temp, path, overwrite: true); }
        finally { if (File.Exists(temp)) File.Delete(temp); }
    }
}
