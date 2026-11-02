using System.Text;
using System.Text.Json;

namespace OwO_Settings;

internal sealed record DictionaryWord(string Id, string Input, string Text, uint Count,
    string Locale = "", string OriginalInput = "", string OriginalWeight = "", string Tag = "");
internal sealed record ManagedDictionary(string Id, string Name, string Format, string SourceName,
    string Scheme, bool Enabled, IReadOnlyList<DictionaryWord> Entries);
internal sealed record BuiltinDictionary(string Id, string Name, bool Enabled = true, bool Deleted = false);
internal sealed record DictionaryCatalog(int SchemaVersion, long Revision, IReadOnlyList<ManagedDictionary> Dictionaries)
{
    public IReadOnlyList<BuiltinDictionary> Builtins { get; init; } = [new("user", "用户词库"), new("base", "基础中文词典")];
}
internal sealed record DictionaryImport(string Format, IReadOnlyList<DictionaryWord> Entries,
    int Duplicates, int Pending, int AdjustedWeights);

internal sealed class DictionaryStore
{
    internal const int MaximumEntries = 500000;
    internal const int MaximumFileBytes = 128 * 1024 * 1024;
    internal const int MaximumCatalogBytes = 256 * 1024 * 1024;
    internal const int MaximumActiveBytes = 256 * 1024 * 1024;
    // Source metadata is not an active input key. Preserve long upstream
    // readings as pending entries without raising the 256-character runtime
    // encoding limit or silently truncating a word's original annotation.
    internal const int MaximumOriginalInputLength = 4096;
    private static readonly JsonSerializerOptions JsonOptions = new() {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase, WriteIndented = false, MaxDepth = 12
    };
    internal string Root { get; }
    internal static string DefaultRoot {
        get {
            if (Environment.GetEnvironmentVariable("OWO_DICTIONARY_STORE_PATH") is { Length: > 0 } custom) return custom;
            if (Environment.GetEnvironmentVariable("OWO_CONFIG_PATH") is { Length: > 0 } config) {
                var parent = Path.GetDirectoryName(Path.GetFullPath(config))!;
                return Path.GetFileName(parent).Equals("config", StringComparison.OrdinalIgnoreCase)
                    ? Path.Combine(Path.GetDirectoryName(parent)!, "data", "dictionaries") : Path.Combine(parent, "dictionaries");
            }
            return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "OwO", "InputMethod", "data", "dictionaries");
        }
    }
    internal DictionaryStore(string? root = null) => Root = Path.GetFullPath(root ?? DefaultRoot);
    private static bool Id(string value) => Guid.TryParseExact(value, "N", out _) && value == value.ToLowerInvariant();
    private static void RejectLink(string path) {
        if ((File.Exists(path) || Directory.Exists(path)) && (File.GetAttributes(path) & FileAttributes.ReparsePoint) != 0)
            throw new IOException("词典目录或文件不能是链接。" );
    }
    private string? Current() {
        RejectLink(Root);
        var path = Path.Combine(Root, "current"); RejectLink(path);
        if (!File.Exists(path)) return null;
        if (new FileInfo(path).Length != 32) throw new InvalidDataException("词典快照指针无效。" );
        var id = File.ReadAllText(path, Encoding.ASCII);
        if (!Id(id)) throw new InvalidDataException("词典快照标识无效。" );
        var snapshots = Path.Combine(Root, "snapshots"); RejectLink(snapshots);
        var directory = Path.Combine(snapshots, id); RejectLink(directory);
        return directory;
    }
    internal async Task<DictionaryCatalog> LoadAsync() {
        var directory = Current();
        if (directory is null) return new(1, 0, []);
        var path = Path.Combine(directory, "catalog.json"); RejectLink(path);
        if (new FileInfo(path).Length > MaximumCatalogBytes) throw new InvalidDataException("词典目录数据超过上限。" );
        var bytes = await DictionaryFormats.ReadBoundedAsync(path, MaximumCatalogBytes);
        if (bytes.Length > MaximumCatalogBytes) throw new InvalidDataException("词典数据大小发生变化。" );
        var catalog = await Task.Run(() => JsonSerializer.Deserialize<DictionaryCatalog>(bytes, JsonOptions)
            ?? throw new InvalidDataException("词典目录数据无效。"));
        await Task.Run(() => Validate(catalog));
        return catalog;
    }
    internal static bool Usable(DictionaryWord word) => word.Input.Length > 0;
    internal static (string, string, string, string, string) WordIdentity(DictionaryWord word) =>
        (word.Input.ToLowerInvariant(), word.Text, word.Locale, word.Tag,
         Usable(word) ? "" : word.OriginalInput);
    internal static void ValidateWord(DictionaryWord word) {
        if (word is null || word.Input is null || word.Locale is null || word.OriginalInput is null || word.OriginalWeight is null || word.Tag is null ||
            !Id(word.Id) || string.IsNullOrWhiteSpace(word.Text) || word.Text.Any(char.IsControl) ||
            new UTF8Encoding(false, true).GetByteCount(word.Text) > 1024 || word.Count is < 1 or > 128 ||
            word.Locale.Length > 128 || word.Locale.Any(char.IsControl) ||
            word.OriginalWeight.Length > 64 || word.OriginalWeight.Any(char.IsControl))
            throw new InvalidDataException("词条正文、语言、原编码或权重无效。" );
        if (word.OriginalInput.Length > MaximumOriginalInputLength || word.OriginalInput.Any(char.IsControl))
            throw new InvalidDataException($"原编码不能含控制字符，长度不超过 {MaximumOriginalInputLength} 字符；超出运行时编码上限的原编码仅作待补编码信息保留。");
        if (word.Tag.Length > 128 || word.Tag.Any(char.IsControl)) throw new InvalidDataException("词条标签无效。" );
        if (word.Input.Length != 0) UserLexiconClient.Validate(new(word.Input, word.Text, word.Count));
    }
    private static void Validate(DictionaryCatalog catalog) {
        if (catalog.SchemaVersion != 1 || catalog.Revision < 0 || catalog.Dictionaries is null || catalog.Dictionaries.Count > 128)
            throw new InvalidDataException("不支持的词典目录版本或数量。" );
        var ids = new HashSet<string>(StringComparer.Ordinal);
        if (catalog.Builtins is null || catalog.Builtins.Count != 2 ||
            !catalog.Builtins.Select(value => value?.Id).ToHashSet().SetEquals(new[] { "user", "base" }) ||
            catalog.Builtins.Any(value => value is null || string.IsNullOrWhiteSpace(value.Name) ||
                value.Name.Length > 128 || value.Name.Any(char.IsControl)))
            throw new InvalidDataException("内置词库名称或标识无效。");
        var total = 0;
        foreach (var dictionary in catalog.Dictionaries) {
            if (dictionary is null || dictionary.SourceName is null || dictionary.Format is null || !Id(dictionary.Id) || !ids.Add(dictionary.Id) || string.IsNullOrWhiteSpace(dictionary.Name) ||
                dictionary.Name.Length > 128 || dictionary.Name.Any(char.IsControl) || dictionary.Entries is null ||
                dictionary.SourceName.Length > 256 || dictionary.SourceName.Any(char.IsControl) ||
                dictionary.Format.Length > 64 || dictionary.Scheme is not ("full_pinyin" or "archive"))
                throw new InvalidDataException("词典名称、方案或标识无效。" );
            var words = new HashSet<string>(StringComparer.Ordinal);
            foreach (var word in dictionary.Entries) { ValidateWord(word); if (!words.Add(word.Id)) throw new InvalidDataException("词条标识重复。" ); }
            total = checked(total + dictionary.Entries.Count);
            if (total > MaximumEntries) throw new InvalidDataException("独立词典合计最多支持 500000 条，未修改已有词典。" );
        }
    }
    // One atomic pointer publishes catalog and active candidates together. A
    // failed/cancelled operation never exposes a half-written dictionary.
    internal async Task<DictionaryCatalog> SaveAsync(DictionaryCatalog previous, IReadOnlyList<ManagedDictionary> dictionaries,
        IReadOnlyList<BuiltinDictionary>? builtins = null) {
        RejectLink(Root); Directory.CreateDirectory(Root);
        var lockPath = Path.Combine(Root, "store.lock"); RejectLink(lockPath);
        using var gate = new FileStream(lockPath, FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);
        var latest = await LoadAsync();
        if (latest.Revision != previous.Revision) throw new IOException("词典已被其他窗口修改，请刷新后重试。" );
        var next = new DictionaryCatalog(1, checked(latest.Revision + 1), dictionaries) { Builtins = builtins ?? previous.Builtins };
        var bytes = await Task.Run(() => { Validate(next); return JsonSerializer.SerializeToUtf8Bytes(next, JsonOptions); });
        if (bytes.Length > MaximumCatalogBytes) throw new InvalidDataException("词典目录超过 256 MiB，未修改已有数据。" );
        var activeBytes = await Task.Run(() => {
            var active = new Dictionary<(string, string), uint>();
            foreach (var dictionary in dictionaries.Where(value => value.Enabled && value.Scheme == "full_pinyin"))
                foreach (var word in dictionary.Entries.Where(Usable)) {
                    // Runtime phonetic indexing needs upstream syllable
                    // boundaries; compacting here loses xi'an vs xian.
                    var key = (word.Input.ToLowerInvariant(), word.Text);
                    active[key] = Math.Max(active.GetValueOrDefault(key), word.Count);
                }
            var text = new StringBuilder("OWO_DICTIONARIES_V1\n");
            foreach (var word in active.OrderBy(value => value.Key.Item1, StringComparer.Ordinal).ThenBy(value => value.Key.Item2, StringComparer.Ordinal))
                text.Append(word.Key.Item1).Append('\t').Append(word.Key.Item2).Append('\t').Append(word.Value).Append('\n');
            var payload = new UTF8Encoding(false, true).GetBytes(text.ToString());
            if (payload.Length > MaximumActiveBytes) throw new InvalidDataException("启用的词条超过运行时上限。" );
            return payload;
        });
        var id = Guid.NewGuid().ToString("N");
        var snapshots = Path.Combine(Root, "snapshots"); RejectLink(snapshots);
        var destination = Path.Combine(snapshots, id);
        Directory.CreateDirectory(destination);
        await DurableWrite(Path.Combine(destination, "catalog.json"), bytes);
        await DurableWrite(Path.Combine(destination, "active.tsv"), activeBytes);
        var sources = "OWO_DICTIONARY_SOURCES_V1\n" + string.Concat(new[] { "base", "user" }.Select(id => {
            var source = next.Builtins.Single(value => value.Id == id);
            return $"{id}\t{(source.Enabled && !source.Deleted ? 1 : 0)}\n";
        }));
        await DurableWrite(Path.Combine(destination, "sources.tsv"), Encoding.ASCII.GetBytes(sources));
        var temporary = Path.Combine(Root, id + ".tmp");
        var current = Path.Combine(Root, "current"); RejectLink(current);
        try {
            await DurableWrite(temporary, Encoding.ASCII.GetBytes(id));
            for (var attempt = 0;; ++attempt) {
                try { File.Move(temporary, current, true); break; }
                catch (IOException error) when (attempt < 9 && (error.HResult & 0xffff) is 32 or 33) { await Task.Delay(20); }
            }
        }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
        return next;
    }
    private static async Task DurableWrite(string path, byte[] bytes) {
        await using var stream = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.None, 4096, FileOptions.WriteThrough);
        await stream.WriteAsync(bytes); await stream.FlushAsync(); stream.Flush(true);
    }
}
