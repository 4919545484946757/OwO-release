using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace OwO_Settings;

internal sealed record LexiconWord(
    [property: JsonPropertyName("input")] string Input,
    [property: JsonPropertyName("text")] string Text,
    [property: JsonPropertyName("count")] uint Count);
internal sealed record LexiconPage(ulong Revision, int Total, int Added, int Updated,
                                   IReadOnlyList<LexiconWord> Words);
internal sealed record LexiconFile(
    [property: JsonPropertyName("schema_version")] int Version,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("entries")] IReadOnlyList<LexiconWord> Entries);

internal sealed class UserLexiconClient
{
    private readonly string _shell = Environment.GetEnvironmentVariable("OWO_CONFIG_SHELL_PATH")
        ?? Path.Combine(AppContext.BaseDirectory, "owo_config_shell.exe");
    internal static string CanonicalInput(string value) => value.Replace("'", "").ToLowerInvariant();
    internal static void Validate(LexiconWord word)
    {
        if (string.IsNullOrEmpty(word.Input) || word.Input.Length > 256 || CanonicalInput(word.Input).Length == 0 ||
            word.Input.Any(c => c != '\'' && !char.IsAsciiLetter(c)))
            throw new InvalidDataException("编码只支持 1–256 位英文字母及拼音分隔符。");
        if (string.IsNullOrEmpty(word.Text) || word.Text.Any(char.IsControl) ||
            new UTF8Encoding(false, true).GetByteCount(word.Text) > 1024 || word.Count is < 1 or > 128)
            throw new InvalidDataException("词语不能包含控制字符，长度不超过 1024 UTF-8 字节；权重须为 1–128。");
    }
    internal async Task<LexiconPage> RequestAsync(string action, ulong? revision = null,
        string query = "", int offset = 0, int limit = 50, string mode = "merge",
        LexiconWord? original = null, IReadOnlyList<LexiconWord>? words = null)
    {
        if (query.Any(char.IsControl) || Encoding.UTF8.GetByteCount(query) > 256)
            throw new InvalidDataException("搜索内容过长或含控制字符。");
        var payload = new StringBuilder($"{action}\t{revision?.ToString(CultureInfo.InvariantCulture) ?? "-"}\t{offset}\t{limit}\t{mode}\n{query}\n{original?.Input ?? ""}\t{original?.Text ?? ""}\n");
        if (original is not null) Validate(original);
        foreach (var word in words ?? []) {
            Validate(word);
            payload.Append($"{word.Input}\t{word.Text}\t{word.Count}\n");
        }
        if (Encoding.UTF8.GetByteCount(payload.ToString()) > 512 * 1024)
            throw new InvalidDataException("本批数据超过 512 KiB，请拆分后导入。");
        if (!File.Exists(_shell)) throw new FileNotFoundException("找不到词库管理后端，请使用最新版设置中心。", _shell);
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(15));
        var start = new ProcessStartInfo(_shell) {
            UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
            StandardInputEncoding = new UTF8Encoding(false), StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8
        };
        start.ArgumentList.Add("--user-lexicon");
        using var process = Process.Start(start) ?? throw new IOException("无法启动词库管理后端。");
        try {
            var output = process.StandardOutput.ReadToEndAsync(timeout.Token);
            var error = process.StandardError.ReadToEndAsync(timeout.Token);
            await process.StandardInput.WriteAsync(payload.ToString().AsMemory(), timeout.Token);
            process.StandardInput.Close();
            await process.WaitForExitAsync(timeout.Token);
            if (process.ExitCode != 0) {
                var detail = (await error).Trim();
                throw new IOException(detail.Length == 0 ? $"词库后端退出码：{process.ExitCode}" : detail);
            }
            var lines = (await output).Split('\n', StringSplitOptions.RemoveEmptyEntries);
            if (lines.Length == 0) throw new InvalidDataException("词库响应为空，请检查 Core 和设置中心版本。");
            var header = lines[0].TrimEnd('\r').Split('\t');
            if (header.Length != 4) throw new InvalidDataException("词库响应格式错误。");
            var rows = new List<LexiconWord>();
            foreach (var line in lines.Skip(1)) {
                var fields = line.TrimEnd('\r').Split('\t');
                if (fields.Length != 4) throw new InvalidDataException("词条响应无效。");
                rows.Add(new(fields[0], fields[1], uint.Parse(fields[2], CultureInfo.InvariantCulture)));
            }
            return new(ulong.Parse(header[0], CultureInfo.InvariantCulture), int.Parse(header[1], CultureInfo.InvariantCulture),
                int.Parse(header[2], CultureInfo.InvariantCulture), int.Parse(header[3], CultureInfo.InvariantCulture), rows);
        } catch (OperationCanceledException) {
            if (!process.HasExited) process.Kill(true);
            throw new IOException("词库操作超时，请刷新确认结果后重试；不会自动重复提交。");
        } finally {
            if (!process.HasExited) process.Kill(true);
        }
    }
    internal static async Task<IReadOnlyList<LexiconWord>> ReadImportAsync(string path)
    {
        if (new FileInfo(path).Length > 4 * 1024 * 1024) throw new InvalidDataException("文件不能超过 4 MiB。");
        var text = await File.ReadAllTextAsync(path, new UTF8Encoding(false, true));
        if (Encoding.UTF8.GetByteCount(text) > 4 * 1024 * 1024) throw new InvalidDataException("文件大小发生变化，请重新选择。");
        var result = new List<LexiconWord>();
        if (Path.GetExtension(path).Equals(".tsv", StringComparison.OrdinalIgnoreCase)) {
            foreach (var line in text.Split('\n')) {
                var value = line.TrimEnd('\r');
                if (value.Length == 0 || value == "input\ttext\tcount") continue;
                var fields = value.Split('\t');
                if (fields.Length != 3 || !uint.TryParse(fields[2], NumberStyles.None, CultureInfo.InvariantCulture, out var count))
                    throw new InvalidDataException("TSV 必须为三列：input、text、count。");
                result.Add(new(fields[0], fields[1], count));
            }
        } else {
            using var document = JsonDocument.Parse(text, new JsonDocumentOptions { MaxDepth = 16 });
            static void Exact(JsonElement element, params string[] fields) {
                if (element.ValueKind != JsonValueKind.Object || element.EnumerateObject().Count() != fields.Length ||
                    !element.EnumerateObject().Select(p => p.Name).ToHashSet().SetEquals(fields))
                    throw new InvalidDataException("JSON 包含重复、缺失或未知字段。");
            }
            var root = document.RootElement;
            Exact(root, "schema_version", "kind", "entries");
            if (root.GetProperty("schema_version").GetInt32() != 1 || root.GetProperty("kind").GetString() != "OwO.UserLexicon")
                throw new InvalidDataException("不支持的词库文件类型或版本。");
            foreach (var entry in root.GetProperty("entries").EnumerateArray()) {
                Exact(entry, "input", "text", "count");
                result.Add(new(entry.GetProperty("input").GetString()!, entry.GetProperty("text").GetString()!, entry.GetProperty("count").GetUInt32()));
            }
        }
        if (result.Count is < 1 or > 5000) throw new InvalidDataException("每次导入须为 1–5000 条词语。");
        var keys = new HashSet<(string, string)>();
        foreach (var word in result) {
            Validate(word);
            if (!keys.Add((CanonicalInput(word.Input), word.Text))) throw new InvalidDataException("文件包含重复编码和词语，请合并后导入。");
        }
        return result;
    }
}
