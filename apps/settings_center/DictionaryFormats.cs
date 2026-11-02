using System.Globalization;
using System.IO.Compression;
using System.Text;
using System.Text.Json;

namespace OwO_Settings;

internal static class DictionaryFormats
{
    internal static readonly string[] Extensions = [".json", ".tsv", ".csv", ".txt", ".yaml", ".yml", ".zip"];
    internal static async Task<DictionaryImport> ReadAsync(string path) {
        if (new FileInfo(path).Length > DictionaryStore.MaximumFileBytes) throw new InvalidDataException("导入文件不能超过 128 MiB。" );
        var bytes = await ReadBoundedAsync(path);
        if (Path.GetExtension(path).Equals(".zip", StringComparison.OrdinalIgnoreCase)) {
            return await Task.Run(async () => {
            using var archive = new ZipArchive(new MemoryStream(bytes), ZipArchiveMode.Read);
            if (archive.Entries.Count > 16) throw new InvalidDataException("Gboard ZIP 文件项过多。" );
            foreach (var entry in archive.Entries) {
                var name = entry.FullName.Replace('\\', '/');
                if (name.StartsWith('/') || name.Contains(':') || name.Split('/').Contains("..") ||
                    ((entry.ExternalAttributes >> 16) & 0xf000) == 0xa000)
                    throw new InvalidDataException("压缩包包含不安全路径或链接。" );
            }
            var files = archive.Entries.Where(entry => !entry.FullName.EndsWith('/')).ToArray();
            if (files.Length != 1 || !Path.GetFileName(files[0].FullName.Replace('\\', '/')).Equals("dictionary.txt", StringComparison.OrdinalIgnoreCase))
                throw new InvalidDataException("只支持包含唯一 dictionary.txt 的 Gboard 个人词典 ZIP，不支持二进制学习模型。" );
            if (files[0].Length > DictionaryStore.MaximumFileBytes) throw new InvalidDataException("压缩包展开后超过 128 MiB。" );
            await using var source = files[0].Open(); using var result = new MemoryStream();
            var buffer = new byte[8192]; int read;
            while ((read = await source.ReadAsync(buffer)) > 0) {
                if (result.Length + read > DictionaryStore.MaximumFileBytes) throw new InvalidDataException("压缩数据超过展开上限。" );
                result.Write(buffer, 0, read);
            }
            var text = Decode(result.ToArray());
            if (!text.TrimStart().StartsWith("# Gboard Dictionary", StringComparison.Ordinal)) throw new InvalidDataException("未识别到 Gboard 个人词典标头。" );
            return Parse(text, "dictionary.txt");
            });
        }
        return await Task.Run(() => Parse(Decode(bytes), Path.GetFileName(path)));
    }
    internal static async Task<byte[]> ReadBoundedAsync(string path, int maximum = DictionaryStore.MaximumFileBytes) {
        await using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read, 8192, FileOptions.Asynchronous);
        using var output = new MemoryStream(); var buffer = new byte[8192]; int count;
        while ((count = await file.ReadAsync(buffer)) > 0) {
            if (output.Length + count > maximum) throw new InvalidDataException("文件超过允许的大小上限。" );
            output.Write(buffer, 0, count);
        }
        return output.ToArray();
    }
    private static string Decode(byte[] bytes) {
        if (bytes.Length >= 2 && bytes[0] == 0xff && bytes[1] == 0xfe) return new UnicodeEncoding(false, true, true).GetString(bytes, 2, bytes.Length - 2);
        if (bytes.Length >= 2 && bytes[0] == 0xfe && bytes[1] == 0xff) return new UnicodeEncoding(true, true, true).GetString(bytes, 2, bytes.Length - 2);
        var offset = bytes.Length >= 3 && bytes[0] == 0xef && bytes[1] == 0xbb && bytes[2] == 0xbf ? 3 : 0;
        return new UTF8Encoding(false, true).GetString(bytes, offset, bytes.Length - offset);
    }
    internal static DictionaryImport Parse(string text, string name) {
        var entries = new List<DictionaryWord>();
        var duplicates = 0; var pending = 0; var adjusted = 0;
        var seen = new Dictionary<(string, string, string, string, string), int>();
        void Add(string input, string word, string? weight = null, string locale = "", string? originalInput = null, string? originalWeight = null, string tag = "") {
            input = input.Trim(); locale = locale.Trim();
            var normalized = string.Join("'", input.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries)).ToLowerInvariant();
            if (normalized.Length > 256 || normalized.Any(c => c != '\'' && !char.IsAsciiLetter(c))) normalized = "";
            if (UserLexiconClient.CanonicalInput(normalized).Length == 0) normalized = "";
            var count = 1u;
            if (!string.IsNullOrWhiteSpace(weight)) {
                var raw = weight.TrimEnd('%');
                if (!decimal.TryParse(raw, NumberStyles.AllowDecimalPoint, CultureInfo.InvariantCulture, out var number) || number < 0)
                    throw new InvalidDataException("词频须为非负数字。" );
                count = (uint)Math.Clamp(decimal.Truncate(number), 1, 128);
                if (number != count) adjusted++;
            }
            var entry = new DictionaryWord(Guid.NewGuid().ToString("N"), normalized, word, count, locale,
                originalInput ?? (normalized != input ? input : ""), originalWeight ?? weight ?? "", tag);
            DictionaryStore.ValidateWord(entry);
            var key = DictionaryStore.WordIdentity(entry);
            if (seen.TryGetValue(key, out var index)) {
                duplicates++; if (count > entries[index].Count) entries[index] = entry; return;
            }
            if (entries.Count >= DictionaryStore.MaximumEntries) throw new InvalidDataException("单次导入最多支持 500000 条。" );
            seen[key] = entries.Count; entries.Add(entry);
        }
        var trimmed = text.TrimStart(); var format = "TSV / 文本";
        if (trimmed.StartsWith('{')) {
            using var json = JsonDocument.Parse(text, new JsonDocumentOptions { MaxDepth = 12 });
            var root = json.RootElement;
            if (root.ValueKind != JsonValueKind.Object || root.EnumerateObject().Select(value => value.Name).Distinct().Count() != root.EnumerateObject().Count())
                throw new InvalidDataException("JSON 顶层字段重复或结构无效。" );
            if (root.GetProperty("schema_version").GetInt32() != 1 ||
                root.GetProperty("kind").GetString() is not ("OwO.UserLexicon" or "OwO.Dictionary"))
                throw new InvalidDataException("不支持的 OwO 字典 JSON 版本。" );
            format = "OwO JSON";
            foreach (var row in root.GetProperty("entries").EnumerateArray()) {
                if (row.EnumerateObject().Select(value => value.Name).Distinct().Count() != row.EnumerateObject().Count())
                    throw new InvalidDataException("JSON 词条字段重复。" );
                string Value(string key) => row.TryGetProperty(key, out var field) && field.ValueKind != JsonValueKind.Null
                    ? field.ValueKind == JsonValueKind.String ? field.GetString()! : field.GetRawText() : "";
                Add(Value("input"), Value("text"), Value("count"), Value("locale"),
                    row.TryGetProperty("original_input", out _) ? Value("original_input") : null,
                    row.TryGetProperty("original_weight", out _) ? Value("original_weight") : null, Value("tag"));
            }
        } else {
            var gboard = trimmed.StartsWith("# Gboard Dictionary", StringComparison.Ordinal);
            var rime = !gboard && (name.EndsWith(".yaml", StringComparison.OrdinalIgnoreCase) || name.EndsWith(".yml", StringComparison.OrdinalIgnoreCase) || trimmed.StartsWith("---"));
            var csv = !gboard && !rime && name.EndsWith(".csv", StringComparison.OrdinalIgnoreCase);
            format = gboard ? "Gboard 个人词典" : rime ? "Rime 文本词典" : csv ? "CSV" : format;
            string[]? columns = null;
            var body = !rime; var columnBlock = false; var columnList = new List<string>();
            var lineNumber = 0;
            foreach (var raw in text.Split('\n')) {
                lineNumber++; var line = raw.TrimEnd('\r');
                if (string.IsNullOrWhiteSpace(line)) continue;
                try {
                    if (line.StartsWith('#')) {
                        if (gboard && line.StartsWith("# Gboard Dictionary version:") && line.Split(':').Last().Trim() is not ("1" or "2"))
                            throw new InvalidDataException("只支持 Gboard 个人词典版本 1/2。" );
                        if (gboard && line.StartsWith("# Gboard Dictionary format:")) columns = line[(line.IndexOf(':') + 1)..].Split('\t').Select(value => value.Trim()).ToArray();
                        continue;
                    }
                    if (rime && !body) {
                        if (line.Trim() == "...") { body = true; if (columnList.Count != 0) columns = columnList.ToArray(); continue; }
                        if (line.TrimStart().StartsWith("columns:")) {
                            var declaration = line[(line.IndexOf(':') + 1)..].Trim(); columnBlock = declaration.Length == 0;
                            if (!columnBlock) columns = declaration.Trim('[', ']').Split(',').Select(value => value.Trim(' ', '\'', '"')).ToArray();
                        } else if (columnBlock && line.TrimStart().StartsWith('-')) columnList.Add(line.Trim().TrimStart('-').Trim());
                        else if (!string.IsNullOrWhiteSpace(line) && !line.StartsWith(' ')) columnBlock = false;
                        continue;
                    }
                    var cells = csv ? Csv(line) : line.Split('\t');
                    if (!gboard && !rime && columns is null &&
                        cells.Any(value => value.Trim().ToLowerInvariant() is "input" or "code" or "pinyin" or "reading") &&
                        cells.Any(value => value.Trim().ToLowerInvariant() is "text" or "word")) {
                        columns = cells.Select(value => value.Trim().ToLowerInvariant()).ToArray(); continue;
                    }
                    columns ??= gboard ? ["shortcut", "word", "language_tag", "pos_tag"] : rime ? ["text", "code", "weight"] : ["input", "text", "count", "locale"];
                    if (cells.Length == 1 && !gboard && !rime && !csv) { Add("", cells[0]); continue; }
                    if (cells.Length < 2 || gboard && cells.Length < 3) throw new InvalidDataException("词条列数不足。" );
                    string Value(params string[] keys) {
                        for (var index = 0; index < columns.Length && index < cells.Length; ++index)
                            if (keys.Contains(columns[index], StringComparer.OrdinalIgnoreCase)) return cells[index];
                        return "";
                    }
                    Add(Value("input", "code", "pinyin", "reading", "shortcut"), Value("text", "word"), Value("count", "weight", "frequency"), Value("locale", "language", "language_tag"),
                        columns.Contains("original_input") ? Value("original_input") : null,
                        columns.Contains("original_weight") ? Value("original_weight") : null, Value("pos_tag", "tag"));
                } catch (Exception error) when (error is InvalidDataException or OverflowException) {
                    throw new InvalidDataException($"第 {lineNumber} 行：{error.Message}", error);
                }
            }
            if (rime && !body) throw new InvalidDataException("Rime 词典缺少 ... 数据区分隔符。" );
        }
        if (entries.Count == 0) throw new InvalidDataException("文件没有可导入的词条。" );
        pending = entries.Count(word => !DictionaryStore.Usable(word));
        return new(format, entries, duplicates, pending, adjusted);
    }
    private static string[] Csv(string line) {
        var cells = new List<string>(); var value = new StringBuilder(); var quoted = false;
        for (var index = 0; index < line.Length; ++index) {
            var c = line[index];
            if (c == '"') {
                if (quoted && index + 1 < line.Length && line[index + 1] == '"') { value.Append('"'); index++; }
                else quoted = !quoted;
            } else if (c == ',' && !quoted) { cells.Add(value.ToString()); value.Clear(); } else value.Append(c);
        }
        if (quoted) throw new InvalidDataException("CSV 引号未闭合；不支持跨行词条。" );
        cells.Add(value.ToString()); return cells.ToArray();
    }
    internal static string Export(ManagedDictionary dictionary, bool tsv) => tsv
        ? "input\ttext\tcount\tlocale\toriginal_input\toriginal_weight\ttag\n" + string.Concat(dictionary.Entries.Select(word =>
            $"{word.Input}\t{word.Text}\t{word.Count}\t{word.Locale}\t{word.OriginalInput}\t{word.OriginalWeight}\t{word.Tag}\n"))
        : JsonSerializer.Serialize(new { schema_version = 1, kind = "OwO.Dictionary", name = dictionary.Name,
            entries = dictionary.Entries.Select(word => new { input = word.Input, text = word.Text, count = word.Count,
                locale = word.Locale, original_input = word.OriginalInput, original_weight = word.OriginalWeight, tag = word.Tag }) }, new JsonSerializerOptions { WriteIndented = true });
}
