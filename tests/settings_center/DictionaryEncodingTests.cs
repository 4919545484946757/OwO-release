using OwO_Settings;

var checks = 0;
void Check(bool value, string message) { if (!value) throw new Exception(message); checks++; }
void Reject(Action action, string message) { try { action(); } catch (InvalidDataException) { checks++; return; } throw new Exception("Expected rejection: " + message); }
DictionaryWord Word(string text, string input = "") => new(Guid.NewGuid().ToString("N"), input, text, 12, "zh-CN", "原编码", "12345", "noun");
var exact = Word("测试"); var poly = Word("重"); var composed = Word("测试词库"); var missing = Word("𰻞"); var latin = Word("hello");
var words = new[] { exact, poly, composed, missing, latin };
{
    var activationDictionary = new ManagedDictionary(Guid.NewGuid().ToString("N"), "启用测试", "text", "synthetic", "full_pinyin", true, [exact, Word("已有", "yi'you")]);
    var choices = 0; var supplements = 0;
    Task<DictionaryActivationDecision> Choose(int count) { choices++; Check(count == 1, "Prompt counts all pending entries"); return Task.FromResult(new DictionaryActivationDecision(DictionaryActivationChoice.EnableWithoutEncoding, true)); }
    Task<ManagedDictionary?> Supplement(ManagedDictionary staged) { supplements++; Check(!staged.Enabled, "Do not activate before supplement confirmation"); return Task.FromResult<ManagedDictionary?>(staged with { Entries = staged.Entries.Select(word => word.Id == exact.Id ? word with { Input = "ce'shi" } : word).ToArray() }); }
    var direct = await DictionaryActivation.PrepareAsync(activationDictionary, false, true, Choose, Supplement);
    Check(direct.Dictionary == activationDictionary && !direct.PromptBeforeEnablingPending && choices == 1 && supplements == 0, "Explicit direct enable can suppress future prompts without encoding words");
    var generated = await DictionaryActivation.PrepareAsync(activationDictionary, false, true,
        _ => Task.FromResult(new DictionaryActivationDecision(DictionaryActivationChoice.Supplement, true)), Supplement);
    Check(generated.Dictionary is { Enabled: true } && generated.Dictionary.Entries[0].Input == "ce'shi" &&
        !generated.PromptBeforeEnablingPending && supplements == 1, "Accepted encoding and enable are staged together");
    var cancelled = await DictionaryActivation.PrepareAsync(activationDictionary, false, true,
        _ => Task.FromResult(new DictionaryActivationDecision(DictionaryActivationChoice.Cancel, true)), Supplement);
    Check(cancelled.Dictionary is null && cancelled.PromptBeforeEnablingPending && supplements == 1, "Cancel does not change enable state or remember suppression");
    var closed = await DictionaryActivation.PrepareAsync(activationDictionary, false, true,
        _ => Task.FromResult(new DictionaryActivationDecision(DictionaryActivationChoice.Supplement, true)), _ => Task.FromResult<ManagedDictionary?>(null));
    Check(closed.Dictionary is null && closed.PromptBeforeEnablingPending, "Cancelling encoding also cancels enable and preference changes");
    var bypasses = new[] { (activationDictionary, true, true), (activationDictionary with { Enabled = false }, false, true),
        (activationDictionary with { Scheme = "archive" }, false, true), (activationDictionary with { Entries = [Word("已有", "yi'you")] }, false, true), (activationDictionary, false, false) };
    foreach (var (value, enabled, prompt) in bypasses) {
        var result = await DictionaryActivation.PrepareAsync(value, enabled, prompt,
            _ => throw new Exception("Unexpected prompt"), _ => throw new Exception("Unexpected encoding"));
        Check(result.Dictionary == value && result.PromptBeforeEnablingPending == prompt, "No prompt for disabled, already enabled, archive, complete or suppressed dictionary");
    }
    var allPending = await DictionaryActivation.PrepareAsync(activationDictionary with { Entries = [exact, poly] }, false, true,
        count => { Check(count == 2, "All-unencoded dictionary is prompted"); return Task.FromResult(new DictionaryActivationDecision(DictionaryActivationChoice.EnableWithoutEncoding)); }, Supplement);
    Check(allPending.Dictionary is { Enabled: true } && allPending.PromptBeforeEnablingPending, "Explicitly skipping encoding preserves pending words and normal reminders");
    Reject(() => DictionaryActivation.PrepareAsync(activationDictionary, false, true,
        _ => Task.FromResult(new DictionaryActivationDecision((DictionaryActivationChoice)99)), Supplement).GetAwaiter().GetResult(),
        "Unknown activation choice rejected");
    Reject(() => DictionaryActivation.PrepareAsync(activationDictionary, false, true,
        _ => Task.FromResult(new DictionaryActivationDecision(DictionaryActivationChoice.Supplement)),
        staged => Task.FromResult<ManagedDictionary?>(staged with { Id = Guid.NewGuid().ToString("N") })).GetAwaiter().GetResult(),
        "Mismatched encoding result cannot enable a dictionary");
}
if (args.Length == 2 && args[0] is "--seed-ui" or "--seed-thu-ui") {
    var path = Path.GetFullPath(args[1]);
    if (!Path.GetFileName(path).StartsWith("dictionary-encoding-ui-", StringComparison.Ordinal)) throw new ArgumentException("Use a dedicated dictionary-encoding-ui-* fixture.");
    var store = new DictionaryStore(path); var empty = await store.LoadAsync();
    if (empty.Revision != 0) throw new InvalidOperationException("Do not overwrite an existing UI fixture.");
    if (args[0] == "--seed-thu-ui")
        await store.SaveAsync(empty, [new ManagedDictionary(Guid.NewGuid().ToString("N"), "THUOCL 旧格式测试", "text", "THUOCL 专业词库/THUOCL_IT.txt sha256:" + new string('a', 64), "full_pinyin", false,
            [Word("字符串 "), Word("初始化　"), Word("重 "), Word("hello ")])]);
    else
        await store.SaveAsync(empty, [new ManagedDictionary(Guid.NewGuid().ToString("N"), "编码补充测试", "text", "synthetic", "full_pinyin", false, [.. words, Word("唯测库")]),
            new ManagedDictionary(Guid.NewGuid().ToString("N"), "已安装注音参考", "text", "synthetic", "full_pinyin", false, [Word("唯测库", "wei'ce'ku")])]);
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
Check(suggestions[0].AutomaticCode == "ce'shi" && suggestions[1].AutomaticCode is null && suggestions[2].AutomaticCode == "ce'shi'ci'ku", "Automatic unique words and phrases; ambiguous words skipped");
var confident = DictionaryEncoding.Suggest([Word("重庆食品")], new Dictionary<string, IReadOnlyList<DictionaryReading>> {
    ["重庆"] = [new("chong'qing", 1)], ["食品"] = [new("shi'pin", 1)], ["重"] = [new("zhong", uint.MaxValue), new("chong", 1)], ["庆"] = [new("qing", 100000)] });
Check(confident[0].AutomaticCode == "chong'qing'shi'pin" && confident[0].Readings[0].Code == confident[0].AutomaticCode, "Unique phrase evidence resolves character polyphony, not frequency");
var conflicts = DictionaryEncoding.Suggest([Word("甲乙丙")], new Dictionary<string, IReadOnlyList<DictionaryReading>> {
    ["甲乙"] = [new("jia'yi", 1)], ["丙"] = [new("bing", 1)], ["甲"] = [new("jia", 1)], ["乙丙"] = [new("yi'ping", 1000000)] });
Check(conflicts[0].AutomaticCode is null, "Equally supported phrase decompositions disagree: skip");
var truncated = DictionaryEncoding.Suggest([Word("重")], new Dictionary<string, IReadOnlyList<DictionaryReading>> { ["重"] = [new("zhong", 1, true)] });
Check(truncated[0].AutomaticCode is null, "Truncated evidence cannot prove unique pronunciation");
Check(DictionaryEncoding.Parse(DictionaryEncoding.Header + "重\tzhong\t1\t1\n", queries)["重"][0].Uncertain, "Backend uncertainty is preserved");
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
Check(many[0].Readings.Count == 8 && many[0].AutomaticCode is null, "Exponential readings bounded and not guessed automatically");
var padded = Word("字符串 ");
var thu = dictionary with { Name = "可重命名", SourceName = "THUOCL 专业词库/THUOCL_IT.txt sha256:" + new string('a', 64), Entries = [padded, Word("未知 ")] };
var normalized = DictionaryEncoding.PrepareWord(thu, padded);
Check(normalized == padded with { Text = "字符串" } && DictionaryEncoding.Queries([normalized]).Contains("字符串"), "Existing THUOCL column padding is repaired only for lookup");
Check(DictionaryEncoding.PrepareWord(dictionary, padded) == padded && DictionaryEncoding.PrepareWord(thu with { SourceName = "dictionary.txt" }, padded) == padded, "General and Gboard expansion whitespace is preserved");
Check(DictionaryEncoding.PrepareWord(thu, padded with { Input = "zi'fu'chuan" }).Text == padded.Text, "Existing coded terms are not rewritten");
var cleanResult = DictionaryEncoding.Apply(thu, new Dictionary<string, string> { [padded.Id] = "zi'fu'chuan" });
Check(cleanResult.Updated == 1 && cleanResult.CleanedText == 1 && cleanResult.Dictionary.Entries[0] == padded with { Text = "字符串", Input = "zi'fu'chuan" } && cleanResult.Dictionary.Entries[1] == thu.Entries[1], "Only confirmed THUOCL entries are cleaned; skipped records and metadata survive");
var duplicatePadding = DictionaryEncoding.Apply(thu with { Entries = [padded, Word("字符串", "zi'fu'chuan")] }, new Dictionary<string, string> { [padded.Id] = "zi'fu'chuan" });
Check(duplicatePadding.Duplicates == 1 && duplicatePadding.CleanedText == 0 && duplicatePadding.Updated == 0 && duplicatePadding.Dictionary.Entries[0] == padded, "A duplicate after cleanup remains pending without overwriting or deleting data");
if (args.Length == 3 && args[0] == "--installed-thu-report") {
    Environment.SetEnvironmentVariable("OWO_CONFIG_SHELL_PATH", Path.GetFullPath(args[1]));
    var catalog = await new DictionaryStore().LoadAsync();
    var actual = catalog.Dictionaries.Single(value => value.SourceName.StartsWith("THUOCL 专业词库/THUOCL_IT.txt sha256:", StringComparison.Ordinal));
    var pending = actual.Entries.Where(word => !DictionaryStore.Usable(word)).ToArray();
    var prepared = pending.Select(word => DictionaryEncoding.PrepareWord(actual, word)).ToArray();
    var watch = System.Diagnostics.Stopwatch.StartNew();
    var report = await new DictionaryEncodingClient().SuggestAsync(prepared, Path.GetFullPath(args[2]), CancellationToken.None, catalog.Dictionaries);
    var automatic = report.Count(word => word.AutomaticCode is not null);
    Console.WriteLine($"READ ONLY THUOCL IT: pending={pending.Length}; trimmed={pending.Zip(prepared).Count(pair => pair.First.Text != pair.Second.Text)}; automatic={automatic}; skipped={report.Count - automatic}; elapsed_ms={watch.ElapsedMilliseconds}");
    foreach (var word in report.Take(8)) Console.WriteLine($"{word.Word.Text}: {word.AutomaticCode ?? "skipped"}");
    Check(automatic > 0 && report.Count == pending.Length, "Installed THUOCL IT produces automatic codes after padding repair");
    Check(report.First(word => word.Word.Text == "字符串").AutomaticCode == "zi'fu'chuan", "Actual padded THUOCL sample reverse lookup");
    Console.WriteLine("Personal dictionary was read but never saved.");
}
if (args.Length == 3 && args[0] == "--live-shell") {
    Environment.SetEnvironmentVariable("OWO_CONFIG_SHELL_PATH", Path.GetFullPath(args[1]));
    var live = await new DictionaryEncodingClient().SuggestAsync([Word("测试方法"), Word("重庆"), Word("银行")], Path.GetFullPath(args[2]), CancellationToken.None, installed: []);
    Check(live.All(row => row.Readings.Count > 0), "Real mapped lexicon and helper process");
    Check(live[0].Readings.Any(reading => reading.Code == "ce'shi'fang'fa"), "Actual test-method pronunciation");
    foreach (var row in live) Console.WriteLine(row.Word.Text + ": " + string.Join(", ", row.Readings.Select(reading => reading.Code)));
    var reference = new ManagedDictionary(Guid.NewGuid().ToString("N"), "已安装参考", "text", "local", "full_pinyin", false, [Word("唯测库", "wei'ce'ku"), Word("测试", "ceshi"), Word("重", "chong"), Word("重", "zhong")]);
    var pending = new[] { Word("唯测库"), Word("测试"), Word("重") };
    var q = DictionaryEncoding.Queries(pending); var index = DictionaryEncoding.InstalledReferences([reference]);
    var native = await DictionaryEncodingClient.QueryAsync(DictionaryEncoding.Payload(q, index), null, CancellationToken.None);
    var onlyInstalled = DictionaryEncoding.Suggest(pending, DictionaryEncoding.Parse(native, q));
    Check(onlyInstalled[0].AutomaticCode == "wei'ce'ku" && onlyInstalled[1].AutomaticCode == "ce'shi" && onlyInstalled[2].AutomaticCode is null, "Installed-only references, compact code and polyphone skipping");
    var conflictingReference = reference with { Entries = [Word("测试方法", "ce'shi'fang'fa"), Word("测试方法", "ce'si'fang'fa")] };
    var clash = await new DictionaryEncodingClient().SuggestAsync([Word("测试方法")], Path.GetFullPath(args[2]), CancellationToken.None, [conflictingReference]);
    Check(clash[0].Readings.Count == 2 && clash[0].AutomaticCode is null, "Conflicting installed and base pronunciations are not auto-selected");
    var boundaryReference = reference with { Entries = [Word("宽高", "kuangao")] };
    var boundary = await new DictionaryEncodingClient().SuggestAsync([Word("宽高")], Path.GetFullPath(args[2]), CancellationToken.None, [boundaryReference]);
    Check(boundary[0].AutomaticCode == "kuan'gao", "Compact imported spelling does not add guessed boundaries over exact base evidence");
    var preciseReference = reference with { Entries = [Word("宽高", "kuangao"), Word("宽高", "kuan'gao")] };
    var boundaryQueries = DictionaryEncoding.Queries([Word("宽高")]);
    var localBoundaries = await DictionaryEncodingClient.QueryAsync(DictionaryEncoding.Payload(boundaryQueries, DictionaryEncoding.InstalledReferences([preciseReference])), null, CancellationToken.None);
    Check(DictionaryEncoding.Suggest([Word("宽高")], DictionaryEncoding.Parse(localBoundaries, boundaryQueries))[0].AutomaticCode == "kuan'gao", "Installed explicit boundaries take precedence independent of source ordering");
    var batch = Enumerable.Range(0, 1001).Select(_ => Word("测试方法")).ToArray();
    var batched = await new DictionaryEncodingClient().SuggestAsync(batch, Path.GetFullPath(args[2]), CancellationToken.None, installed: []);
    Check(batched.Count == 1001 && batched.All(item => item.AutomaticCode == "ce'shi'fang'fa"), "Entire dictionary automatically split across batch limit");
}
Console.WriteLine($"PASS: {checks} dictionary encoding checks; no personal dictionary writes.");
