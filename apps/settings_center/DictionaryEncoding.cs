using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace OwO_Settings;

internal sealed record DictionaryReading(string Code, uint Frequency, bool Uncertain = false);
internal sealed record DictionaryEncodingSuggestion(DictionaryWord Word, IReadOnlyList<DictionaryReading> Readings, bool WholeWord, string? AutomaticCode = null);
internal sealed record DictionaryEncodingResult(ManagedDictionary Dictionary, int Updated, int Duplicates, int CleanedText = 0);
internal sealed record DictionaryEncodingProgress(int Completed, int Total, string Sources);
internal sealed class DictionaryEncodingCapacityException(string message) : IOException(message);

// Offline suggestions, separate from candidate generation and persistence.
// No arbitrary network converters, personal learning records or enabled flags.
internal static class DictionaryEncoding
{
    internal const int MaximumBatch = 1000;
    internal const int MaximumQueries = 32768;
    internal const int MaximumReadings = 8;
    internal const string Header = "OWO_DICTIONARY_READINGS_V1\n";
    internal static DictionaryWord PrepareWord(ManagedDictionary dictionary, DictionaryWord word) {
        // Repair the old online THUOCL import's column padding by provenance,
        // not display name. A renamed dictionary remains recognizable; a
        // Gboard expansion or arbitrary local file must retain its spaces.
        var thu = Regex.IsMatch(dictionary.SourceName,
            @"^THUOCL 专业词库/THUOCL_[A-Za-z0-9_]+\.txt sha256:[a-fA-F0-9]{64}$",
            RegexOptions.CultureInvariant, TimeSpan.FromMilliseconds(100));
        if (!thu || DictionaryStore.Usable(word)) return word;
        var text = word.Text.Trim();
        return text.Length == 0 || text == word.Text ? word : word with { Text = text };
    }
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
            if (queries.Count > MaximumQueries) throw new DictionaryEncodingCapacityException("分词查询超过单批上限。");
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
            if (fields.Length is not (3 or 4) || !wanted.Contains(fields[0]) || !ReadingFits(fields[0], fields[1]) ||
                fields.Length == 4 && fields[3] is not ("0" or "1") ||
                !uint.TryParse(fields[2], NumberStyles.None, CultureInfo.InvariantCulture, out var frequency) || frequency == 0)
                throw new InvalidDataException("注音响应包含无效词语或编码，未应用任何改动。");
            if (!result.TryGetValue(fields[0], out var readings)) result[fields[0]] = readings = [];
            if (readings.Any(value => value.Code == fields[1]) || readings.Count >= MaximumReadings)
                throw new InvalidDataException("注音响应重复或超过读音上限。");
            readings.Add(new(fields[1], frequency, fields.Length == 4 && fields[3] == "1"));
        }
        return result.ToDictionary(pair => pair.Key, pair => (IReadOnlyList<DictionaryReading>)pair.Value, StringComparer.Ordinal);
    }
    internal static IReadOnlyList<DictionaryEncodingSuggestion> Suggest(IReadOnlyList<DictionaryWord> words,
        IReadOnlyDictionary<string, IReadOnlyList<DictionaryReading>> source, CancellationToken token = default) {
        var result = new List<DictionaryEncodingSuggestion>();
        foreach (var word in words) {
            token.ThrowIfCancellationRequested();
            if (source.TryGetValue(word.Text, out var whole) && whole.Count > 0) {
                result.Add(new(word, whole.OrderByDescending(value => value.Frequency).ThenBy(value => value.Code, StringComparer.Ordinal).ToArray(), true,
                    whole.Count == 1 && !whole[0].Uncertain ? whole[0].Code : null)); continue;
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
            var automatic = UniquePhraseReading(characters, source, token);
            var alternatives = Best(chart[^1]).Select(value => new DictionaryReading(value.Code, 0)).ToList();
            if (automatic is not null) {
                alternatives.RemoveAll(reading => reading.Code == automatic);
                alternatives.Insert(0, new(automatic, 0));
                if (alternatives.Count > MaximumReadings) alternatives.RemoveAt(alternatives.Count - 1);
            }
            result.Add(new(word, alternatives, false, automatic));
        }
        return result;
    }
    private static string? UniquePhraseReading(string[] characters, IReadOnlyDictionary<string, IReadOnlyList<DictionaryReading>> source, CancellationToken token) {
        // Certainty is independent of frequency/beam pruning. Prefer coverage
        // by longer dictionary words. All equally supported decompositions
        // must agree; a high frequency alone must not resolve a polyphone.
        var coverage = Enumerable.Repeat(-1, characters.Length + 1).ToArray(); coverage[0] = 0;
        var codes = Enumerable.Range(0, characters.Length + 1).Select(_ => new HashSet<string>(StringComparer.Ordinal)).ToArray(); codes[0].Add("");
        var uncertain = new bool[characters.Length + 1];
        for (var offset = 0; offset < characters.Length; ++offset) {
            token.ThrowIfCancellationRequested();
            if (coverage[offset] < 0) continue;
            for (var count = 1; count <= Math.Min(8, characters.Length - offset); ++count) {
                var part = string.Concat(characters.Skip(offset).Take(count));
                if (!source.TryGetValue(part, out var readings) || readings.Count == 0) continue;
                var end = offset + count; var score = coverage[offset] + count * count;
                if (score < coverage[end]) continue;
                var proposals = new HashSet<string>(StringComparer.Ordinal);
                foreach (var prefix in codes[offset]) foreach (var reading in readings) {
                    var code = prefix.Length == 0 ? reading.Code : prefix + "'" + reading.Code;
                    if (code.Length <= 256 && proposals.Count < 2) proposals.Add(code);
                }
                if (proposals.Count == 0) continue;
                if (score > coverage[end]) { coverage[end] = score; codes[end].Clear(); uncertain[end] = false; }
                foreach (var proposal in proposals) if (codes[end].Count < 2) codes[end].Add(proposal);
                uncertain[end] |= uncertain[offset] || readings.Any(reading => reading.Uncertain);
            }
        }
        return codes[^1].Count == 1 && !uncertain[^1] ? codes[^1].Single() : null;
    }
    internal static IReadOnlyDictionary<string, IReadOnlyList<DictionaryWord>> InstalledReferences(IReadOnlyList<ManagedDictionary> dictionaries) => dictionaries
        .Where(dictionary => dictionary.Scheme == "full_pinyin")
        .SelectMany(dictionary => dictionary.Entries).Where(word => DictionaryStore.Usable(word) && Chinese(word.Text))
        .GroupBy(word => word.Text, StringComparer.Ordinal)
        .ToDictionary(group => group.Key, group => (IReadOnlyList<DictionaryWord>)group
            .GroupBy(word => Normalize(word.Input), StringComparer.Ordinal).Select(codes => codes.MaxBy(word => word.Count)!).ToArray(), StringComparer.Ordinal);
    internal static string Payload(IReadOnlyList<string> queries, IReadOnlyDictionary<string, IReadOnlyList<DictionaryWord>> references) {
        var text = new StringBuilder(Header); var bytes = Encoding.UTF8.GetByteCount(Header); var rows = 0;
        void Append(string row) { bytes += Encoding.UTF8.GetByteCount(row) + 1; if (bytes > 2 * 1024 * 1024) throw new DictionaryEncodingCapacityException("请求超过单批容量。"); text.Append(row).Append('\n'); }
        foreach (var query in queries) Append(query);
        foreach (var query in queries) if (references.TryGetValue(query, out var words)) foreach (var word in words) {
            if (++rows > 32768) throw new DictionaryEncodingCapacityException("注音参考项超过单批容量。");
            Append(word.Text + "\t" + Normalize(word.Input) + "\t" + word.Count.ToString(CultureInfo.InvariantCulture));
        }
        return text.ToString();
    }
    private static List<(string Code, double Score)> Best(List<(string Code, double Score)> values) => values
        .GroupBy(value => value.Code, StringComparer.Ordinal).Select(group => group.MaxBy(value => value.Score))
        .OrderByDescending(value => value.Score).ThenBy(value => value.Code, StringComparer.Ordinal).Take(MaximumReadings).ToList();
    internal static DictionaryEncodingResult Apply(ManagedDictionary dictionary, IReadOnlyDictionary<string, string> selected) {
        var ids = dictionary.Entries.Select(word => word.Id).ToHashSet(StringComparer.Ordinal);
        if (selected.Keys.Any(id => !ids.Contains(id))) throw new InvalidDataException("词条已变化，请重新生成建议。");
        var identities = dictionary.Entries.Where(DictionaryStore.Usable).Select(DictionaryStore.WordIdentity).ToHashSet();
        var updated = 0; var duplicates = 0; var cleaned = 0;
        var entries = dictionary.Entries.Select(word => {
            if (!selected.TryGetValue(word.Id, out var code)) return word;
            if (DictionaryStore.Usable(word)) throw new InvalidDataException("不能通过编码补充覆盖已有编码。");
            var next = PrepareWord(dictionary, word) with { Input = Normalize(code) }; DictionaryStore.ValidateWord(next);
            if (!DictionaryStore.Usable(next)) throw new InvalidDataException("选中的词条必须填写编码。");
            if (!identities.Add(DictionaryStore.WordIdentity(next))) { duplicates++; return word; }
            updated++; if (next.Text != word.Text) cleaned++; return next;
        }).ToArray();
        return new(dictionary with { Entries = entries }, updated, duplicates, cleaned);
    }
}

internal sealed class DictionaryEncodingClient
{
    internal async Task<IReadOnlyList<DictionaryEncodingSuggestion>> SuggestAsync(IReadOnlyList<DictionaryWord> words,
        string? referencePath, CancellationToken token, IReadOnlyList<ManagedDictionary>? installed = null,
        IProgress<DictionaryEncodingProgress>? progress = null) {
        if (words.Count is < 1 or > DictionaryStore.MaximumEntries || words.Any(DictionaryStore.Usable))
            throw new InvalidDataException("只能自动补充待编码词条，不覆盖已有编码。");
        installed ??= (await new DictionaryStore().LoadAsync()).Dictionaries;
        var references = await Task.Run(() => DictionaryEncoding.InstalledReferences(installed), token);
        var sourceCount = installed.Count(dictionary => dictionary.Scheme == "full_pinyin" && dictionary.Entries.Any(DictionaryStore.Usable));
        string? unavailable = null;
        if (referencePath is null) {
            try { referencePath = await new UserLexiconClient().BaseFileAsync().WaitAsync(token); }
            catch (IOException) { unavailable = "基础词典暂不可用；"; }
        }
        if (referencePath is not null && (!Path.IsPathFullyQualified(referencePath) || !Path.GetExtension(referencePath).Equals(".owolx", StringComparison.OrdinalIgnoreCase)))
            throw new InvalidDataException("请选择本机 .owolx 参考词典。");
        var sources = referencePath is null ? $"{unavailable}仅参考 {sourceCount} 本已安装注音词库" : $"基础／自选只读词典 + {sourceCount} 本已安装注音词库";
        var results = new List<DictionaryEncodingSuggestion>();
        progress?.Report(new(0, words.Count, sources));
        async Task BatchAsync(IReadOnlyList<DictionaryWord> batch) {
            token.ThrowIfCancellationRequested();
            try {
                var queries = await Task.Run(() => DictionaryEncoding.Queries(batch), token);
                if (queries.Count == 0) results.AddRange(batch.Select(word => new DictionaryEncodingSuggestion(word, [], false)));
                else {
                    var payload = await Task.Run(() => DictionaryEncoding.Payload(queries, references), token);
                    var readings = await QueryAsync(payload, referencePath, token);
                    results.AddRange(await Task.Run(() => DictionaryEncoding.Suggest(batch, DictionaryEncoding.Parse(readings, queries), token), token));
                }
                progress?.Report(new(results.Count, words.Count, sources));
            } catch (DictionaryEncodingCapacityException) {
                if (batch.Count == 1) {
                    // An overlarge source row is a skipped manual item, never
                    // a reason to truncate evidence and call it unambiguous.
                    results.Add(new(batch[0], [], false)); progress?.Report(new(results.Count, words.Count, sources)); return;
                }
                var middle = batch.Count / 2;
                await BatchAsync(batch.Take(middle).ToArray()); await BatchAsync(batch.Skip(middle).ToArray());
            }
        }
        for (var offset = 0; offset < words.Count; offset += 500)
            await BatchAsync(words.Skip(offset).Take(Math.Min(500, words.Count - offset)).ToArray());
        return results;
    }
    internal static async Task<string> QueryAsync(string payload, string? referencePath, CancellationToken token) {
        var shell = Environment.GetEnvironmentVariable("OWO_CONFIG_SHELL_PATH") ?? Path.Combine(AppContext.BaseDirectory, "owo_config_shell.exe");
        if (!File.Exists(shell)) throw new FileNotFoundException("找不到编码补充后端，请使用新版设置中心。", shell);
        var start = new ProcessStartInfo(shell) {
            UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
            StandardInputEncoding = new UTF8Encoding(false), StandardOutputEncoding = new UTF8Encoding(false, true), StandardErrorEncoding = Encoding.UTF8
        };
        start.ArgumentList.Add("--dictionary-readings"); start.ArgumentList.Add(referencePath ?? "--installed-only");
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
            return stdout;
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
