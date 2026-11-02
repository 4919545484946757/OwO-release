using OwO_Settings;

var checks = 0;
void Check(bool value, string message) { if (!value) throw new Exception(message); checks++; }
void Reject(Action action, string message) { try { action(); } catch (InvalidDataException) { checks++; return; } throw new Exception("Expected rejection: " + message); }
DictionaryWord Word(string text, string input = "") => new(Guid.NewGuid().ToString("N"), input, text, 12, "zh-CN", "原编码", "12345", "noun");
var exact = Word("测试"); var poly = Word("重"); var composed = Word("测试词库"); var missing = Word("𰻞"); var latin = Word("hello");
var words = new[] { exact, poly, composed, missing, latin };
if (args.Length == 2 && args[0] == "--seed-ui") {
    var path = Path.GetFullPath(args[1]);
    if (!Path.GetFileName(path).StartsWith("dictionary-encoding-ui-", StringComparison.Ordinal)) throw new ArgumentException("Use a dedicated dictionary-encoding-ui-* fixture.");
    var store = new DictionaryStore(path); var empty = await store.LoadAsync();
    if (empty.Revision != 0) throw new InvalidOperationException("Do not overwrite an existing UI fixture.");
    await store.SaveAsync(empty, [new ManagedDictionary(Guid.NewGuid().ToString("N"), "编码补充测试", "text", "synthetic", "full_pinyin", false, words)]);
    Console.WriteLine("Created isolated dictionary encoding UI fixture."); return;
}
var queries = DictionaryEncoding.Queries(words);
Check(queries.Contains("测试词库") && queries.Contains("词库") && queries.Contains("测") && !queries.Contains("hello"), "Subphrase queries and non-Chinese isolation");
Reject(() => DictionaryEncoding.Queries([Word("已有", "yiyou")]), "Must not replace existing code");
Reject(() => DictionaryEncoding.Queries(Enumerable.Range(0, 1001).Select(_ => exact).ToArray()), "Batch cap");
var output = DictionaryEncoding.Header + "测试\tce'shi\t1000\n重\tzhong\t10\n重\tchong\t9\n词库\tci'ku\t10\n";
var source = DictionaryEncoding.Parse(output, queries);
Check(DictionaryEncoding.Parse(output.Replace("\n", "\r\n"), queries).Count == source.Count, "Windows helper line endings");
var suggestions = DictionaryEncoding.Suggest(words, source);
Check(suggestions[0].WholeWord && suggestions[0].Readings.Single().Code == "ce'shi", "Whole-word match");
Check(suggestions[1].WholeWord && suggestions[1].Readings.Count == 2, "Polyphonic choices retained");
Check(!suggestions[2].WholeWord && suggestions[2].Readings.Single().Code == "ce'shi'ci'ku", "Dictionary phrase composition");
Check(suggestions[3].Readings.Count == 0 && suggestions[4].Readings.Count == 0, "Unknown and Latin remain manual");
Reject(() => DictionaryEncoding.Parse("wrong\n", queries), "Header mismatch");
Reject(() => DictionaryEncoding.Parse(DictionaryEncoding.Header + "别的\tbie'de\t1\n", queries), "Unexpected words");
Reject(() => DictionaryEncoding.Parse(DictionaryEncoding.Header + "测试\tcs\t1\n", queries), "Shortcut is not pronunciation");
Reject(() => DictionaryEncoding.Parse(DictionaryEncoding.Header + "测试\tce'shi\t1\n测试\tce'shi\t2\n", queries), "Duplicate readings");
Reject(() => DictionaryEncoding.Parse(DictionaryEncoding.Header + "测试\tce'shi\t0\n", queries), "Zero frequency");
var existing = Word("测试", "ce'shi");
var dictionary = new ManagedDictionary(Guid.NewGuid().ToString("N"), "编码测试", "text", "local", "full_pinyin", false, [.. words, existing]);
var applied = DictionaryEncoding.Apply(dictionary, new Dictionary<string, string> { [poly.Id] = " CHONG ", [composed.Id] = "ce shi ci ku", [exact.Id] = "ce'shi" });
Check(applied.Updated == 2 && applied.Duplicates == 1 && !applied.Dictionary.Enabled, "Explicit selection, duplicates and enable state");
Check(applied.Dictionary.Entries.First(word => word.Id == poly.Id) == poly with { Input = "chong" } &&
    applied.Dictionary.Entries.First(word => word.Id == exact.Id) == exact && applied.Dictionary.Entries.Count == dictionary.Entries.Count, "Metadata preserved and duplicates not deleted");
Reject(() => DictionaryEncoding.Apply(dictionary, new Dictionary<string, string> { [existing.Id] = "genggai" }), "Existing code protected");
Reject(() => DictionaryEncoding.Apply(dictionary, new Dictionary<string, string> { [poly.Id] = "a|b" }), "Invalid code");
Reject(() => DictionaryEncoding.Apply(dictionary, new Dictionary<string, string> { [poly.Id] = "" }), "Empty code");
Reject(() => DictionaryEncoding.Apply(dictionary, new Dictionary<string, string> { [Guid.NewGuid().ToString("N")] = "chong" }), "Stale ID");
using (var cancellation = new CancellationTokenSource()) {
    cancellation.Cancel();
    try { DictionaryEncoding.Suggest(words, source, cancellation.Token); throw new Exception("Cancellation ignored"); } catch (OperationCanceledException) { checks++; }
}
var repeated = Word(new string('重', 12));
var many = DictionaryEncoding.Suggest([repeated], new Dictionary<string, IReadOnlyList<DictionaryReading>> { ["重"] = [new("zhong", 10), new("chong", 9)] });
Check(many[0].Readings.Count == 8, "Exponential readings bounded to eight");
if (args.Length == 3 && args[0] == "--live-shell") {
    Environment.SetEnvironmentVariable("OWO_CONFIG_SHELL_PATH", Path.GetFullPath(args[1]));
    var live = await new DictionaryEncodingClient().SuggestAsync([Word("测试方法"), Word("重庆"), Word("银行")], Path.GetFullPath(args[2]), CancellationToken.None);
    Check(live.All(row => row.Readings.Count > 0), "Real mapped lexicon and helper process");
    Check(live[0].Readings.Any(reading => reading.Code == "ce'shi'fang'fa"), "Actual test-method pronunciation");
    foreach (var row in live) Console.WriteLine(row.Word.Text + ": " + string.Join(", ", row.Readings.Select(reading => reading.Code)));
}
Console.WriteLine($"PASS: {checks} dictionary encoding checks; no personal dictionary writes.");
