using System.Diagnostics;
using System.Globalization;
using System.Text;

namespace OwO_Settings;

internal sealed record DictionaryReading(string Code, uint Frequency);
internal sealed record DictionaryEncodingSuggestion(DictionaryWord Word, IReadOnlyList<DictionaryReading> Readings, bool WholeWord);
internal sealed record DictionaryEncodingResult(ManagedDictionary Dictionary, int Updated, int Duplicates);

// Offline suggestions, separate from candidate generation and persistence.
// No arbitrary network converters, personal learning records or enabled flags.
internal static class DictionaryEncoding
{
    internal const int MaximumBatch = 1000;
    internal const int MaximumQueries = 32768;
    internal const int MaximumReadings = 8;
    internal const string Header = "OWO_DICTIONARY_READINGS_V1\n";
    internal static string Normalize(string code) => string.Join("'", code.Trim().ToLowerInvariant().Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));
    private static string[] Characters(string text) => text.EnumerateRunes().Select(value => value.ToString()).ToArray();
    private static bool Chinese(string text) => text.Length > 0 && text.EnumerateRunes().All(value =>
        value.Value is >= 0x3400 and <= 0x9fff or >= 0xf900 and <= 0xfaff or >= 0x20000 and <= 0x323af);
    private static bool ReadingFits(string text, string code) {
        var syllables = code.Split('\'');
        return syllables.Length == text.EnumerateRunes().Count() && syllables.All(s => s.Length is > 0 and <= 6 && s.All(c => c is >= 'a' and <= 'z'));
    }
    internal static IReadOnlyList<string> Queries(IReadOnlyList<DictionaryWord> words) {
        if (words.Count is < 1 or > MaximumBatch || words.Any(word => DictionaryStore.Usable(word)))
            throw new InvalidDataException("仅支持每批 1–1000 条待补编码词语，不覆盖已有编码。");
        var queries = new HashSet<string>(StringComparer.Ordinal);
        foreach (var word in words) {
            DictionaryStore.ValidateWord(word);
            if (!Chinese(word.Text)) continue;
            queries.Add(word.Text);
            var characters = Characters(word.Text);
            // Very long phrases can use whole-word evidence or manual entry;
            // do not expand them into a quadratic substring request.
            if (characters.Length > 64) continue;
            for (var offset = 0; offset < characters.Length; ++offset)
                for (var count = 1; count <= Math.Min(8, characters.Length - offset); ++count)
                    queries.Add(string.Concat(characters.Skip(offset).Take(count)));
            if (queries.Count > MaximumQueries) throw new InvalidDataException("分词查询超过上限，请减小本批数量；尚未修改词库。");
        }
        return queries.Order(StringComparer.Ordinal).ToArray();
    }
    internal static IReadOnlyDictionary<string, IReadOnlyList<DictionaryReading>> Parse(string output, IReadOnlyList<string> queries) {
        output = output.Replace("\r\n", "\n", StringComparison.Ordinal);
        if (!output.StartsWith(Header, StringComparison.Ordinal)) throw new InvalidDataException("注音后端版本或响应格式不匹配。");
        var wanted = queries.ToHashSet(StringComparer.Ordinal);
        var result = new Dictionary<string, List<DictionaryReading>>(StringComparer.Ordinal);
        foreach (var line in output[Header.Length..].Split('\n', StringSplitOptions.RemoveEmptyEntries)) {
            var fields = line.Split('\t');
            if (fields.Length != 3 || !wanted.Contains(fields[0]) || !ReadingFits(fields[0], fields[1]) ||
                !uint.TryParse(fields[2], NumberStyles.None, CultureInfo.InvariantCulture, out var frequency) || frequency == 0)
                throw new InvalidDataException("注音响应包含无效词语或编码，未应用任何改动。");
            if (!result.TryGetValue(fields[0], out var readings)) result[fields[0]] = readings = [];
            if (readings.Any(value => value.Code == fields[1]) || readings.Count >= MaximumReadings)
                throw new InvalidDataException("注音响应重复或超过读音上限。");
            readings.Add(new(fields[1], frequency));
        }
        return result.ToDictionary(pair => pair.Key, pair => (IReadOnlyList<DictionaryReading>)pair.Value, StringComparer.Ordinal);
    }
    internal static IReadOnlyList<DictionaryEncodingSuggestion> Suggest(IReadOnlyList<DictionaryWord> words,
        IReadOnlyDictionary<string, IReadOnlyList<DictionaryReading>> source, CancellationToken token = default) {
        var result = new List<DictionaryEncodingSuggestion>();
        foreach (var word in words) {
            token.ThrowIfCancellationRequested();
            if (source.TryGetValue(word.Text, out var whole) && whole.Count > 0) {
                result.Add(new(word, whole.OrderByDescending(value => value.Frequency).ThenBy(value => value.Code, StringComparer.Ordinal).ToArray(), true)); continue;
            }
            var characters = Characters(word.Text);
            if (!Chinese(word.Text) || characters.Length > 64) { result.Add(new(word, [], false)); continue; }
            var chart = new List<(string Code, double Score)>[characters.Length + 1];
            for (var i = 0; i < chart.Length; ++i) chart[i] = [];
            chart[0].Add(("", 0));
            for (var offset = 0; offset < characters.Length; ++offset) {
                token.ThrowIfCancellationRequested();
                // A bounded beam, not a Cartesian product of every character's
                // readings. Longer dictionary phrases preserve word evidence.
                chart[offset] = Best(chart[offset]);
                foreach (var prefix in chart[offset])
                    for (var count = 1; count <= Math.Min(8, characters.Length - offset); ++count) {
                        var part = string.Concat(characters.Skip(offset).Take(count));
                        if (!source.TryGetValue(part, out var readings)) continue;
                        foreach (var reading in readings.Take(MaximumReadings)) {
                            var code = prefix.Code.Length == 0 ? reading.Code : prefix.Code + "'" + reading.Code;
                            if (code.Length > 256) continue;
                            chart[offset + count].Add((code, prefix.Score + count * count * 100000 + Math.Log(1.0 + reading.Frequency) * 1000 - 10000));
                        }
                        // Limit intermediate storage even for repetitive text.
                        if (chart[offset + count].Count > 128) chart[offset + count] = Best(chart[offset + count]);
                    }
            }
            result.Add(new(word, Best(chart[^1]).Select(value => new DictionaryReading(value.Code, 0)).ToArray(), false));
        }
        return result;
    }
    private static List<(string Code, double Score)> Best(List<(string Code, double Score)> values) => values
        .GroupBy(value => value.Code, StringComparer.Ordinal).Select(group => group.MaxBy(value => value.Score))
        .OrderByDescending(value => value.Score).ThenBy(value => value.Code, StringComparer.Ordinal).Take(MaximumReadings).ToList();
    internal static DictionaryEncodingResult Apply(ManagedDictionary dictionary, IReadOnlyDictionary<string, string> selected) {
        var ids = dictionary.Entries.Select(word => word.Id).ToHashSet(StringComparer.Ordinal);
        if (selected.Keys.Any(id => !ids.Contains(id))) throw new InvalidDataException("词条已变化，请重新生成建议。");
        var identities = dictionary.Entries.Where(DictionaryStore.Usable).Select(DictionaryStore.WordIdentity).ToHashSet();
        var updated = 0; var duplicates = 0;
        var entries = dictionary.Entries.Select(word => {
            if (!selected.TryGetValue(word.Id, out var code)) return word;
            if (DictionaryStore.Usable(word)) throw new InvalidDataException("不能通过编码补充覆盖已有编码。");
            var next = word with { Input = Normalize(code) }; DictionaryStore.ValidateWord(next);
            if (!DictionaryStore.Usable(next)) throw new InvalidDataException("选中的词条必须填写编码。");
            if (!identities.Add(DictionaryStore.WordIdentity(next))) { duplicates++; return word; }
            updated++; return next;
        }).ToArray();
        return new(dictionary with { Entries = entries }, updated, duplicates);
    }
}

internal sealed class DictionaryEncodingClient
{
    internal async Task<IReadOnlyList<DictionaryEncodingSuggestion>> SuggestAsync(IReadOnlyList<DictionaryWord> words,
        string? referencePath, CancellationToken token) {
        var queries = await Task.Run(() => DictionaryEncoding.Queries(words), token);
        if (queries.Count == 0) return words.Select(word => new DictionaryEncodingSuggestion(word, [], false)).ToArray();
        referencePath ??= await new UserLexiconClient().BaseFileAsync().WaitAsync(token);
        if (!Path.IsPathFullyQualified(referencePath) || !Path.GetExtension(referencePath).Equals(".owolx", StringComparison.OrdinalIgnoreCase))
            throw new InvalidDataException("请选择本机 .owolx 参考词典。");
        var shell = Environment.GetEnvironmentVariable("OWO_CONFIG_SHELL_PATH") ?? Path.Combine(AppContext.BaseDirectory, "owo_config_shell.exe");
        if (!File.Exists(shell)) throw new FileNotFoundException("找不到编码补充后端，请使用新版设置中心。", shell);
        var payload = DictionaryEncoding.Header + string.Join('\n', queries) + "\n";
        if (Encoding.UTF8.GetByteCount(payload) > 2 * 1024 * 1024) throw new InvalidDataException("查询内容超过 2 MiB，请减小批量。");
        var start = new ProcessStartInfo(shell) {
            UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
            StandardInputEncoding = new UTF8Encoding(false), StandardOutputEncoding = new UTF8Encoding(false, true), StandardErrorEncoding = Encoding.UTF8
        };
        start.ArgumentList.Add("--dictionary-readings"); start.ArgumentList.Add(referencePath);
        using var process = Process.Start(start) ?? throw new IOException("无法启动编码补充后端。");
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(token); timeout.CancelAfter(TimeSpan.FromMinutes(2));
        using var stop = timeout.Token.Register(() => Stop(process));
        try {
            var output = ReadBoundedAsync(process.StandardOutput, 16 * 1024 * 1024, timeout.Token);
            var error = ReadBoundedAsync(process.StandardError, 65536, timeout.Token);
            // A bounded reader failure must also stop the writer; otherwise a
            // full stdout pipe could make WaitForExit wait until the timeout.
            _ = output.ContinueWith(_ => timeout.Cancel(), CancellationToken.None, TaskContinuationOptions.OnlyOnFaulted | TaskContinuationOptions.ExecuteSynchronously, TaskScheduler.Default);
            _ = error.ContinueWith(_ => timeout.Cancel(), CancellationToken.None, TaskContinuationOptions.OnlyOnFaulted | TaskContinuationOptions.ExecuteSynchronously, TaskScheduler.Default);
            await process.StandardInput.WriteAsync(payload.AsMemory(), timeout.Token); process.StandardInput.Close();
            await process.WaitForExitAsync(timeout.Token);
            var stdout = await output; var stderr = await error;
            if (process.ExitCode != 0) throw new IOException(string.IsNullOrWhiteSpace(stderr) ? "注音参考查询失败，请检查词典与后端版本。" : stderr.Trim());
            return await Task.Run(() => DictionaryEncoding.Suggest(words, DictionaryEncoding.Parse(stdout, queries), token), token);
        } finally { Stop(process); }
    }
    private static void Stop(Process process) {
        try { if (!process.HasExited) process.Kill(true); }
        catch (InvalidOperationException) { }
        catch (System.ComponentModel.Win32Exception) { }
    }
    private static async Task<string> ReadBoundedAsync(StreamReader reader, int maximum, CancellationToken token) {
        var text = new StringBuilder(); var buffer = new char[8192]; int count;
        while ((count = await reader.ReadAsync(buffer.AsMemory(), token)) > 0) {
            if (text.Length + count > maximum) throw new InvalidDataException("注音响应过大，请减小批量。");
            text.Append(buffer, 0, count);
        }
        return text.ToString();
    }
}
