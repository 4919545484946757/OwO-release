using OwO_Settings;
using System.IO.Compression;
using System.Text;

if (args.Length == 2 && args[0] == "--seed-ui") {
    var fixture = Path.GetFullPath(args[1]);
    if (!Path.GetFileName(fixture).StartsWith("dictionary-manager-ui-", StringComparison.Ordinal)) throw new ArgumentException("Use an isolated UI fixture directory.");
    var store = new DictionaryStore(Path.Combine(fixture, "dictionaries"));
    var data = DictionaryFormats.Parse("# Gboard Dictionary version:2\nceshiciku\t测试词库\tzh-CN\tnoun\n\t无编码示例\tzh-CN\n", "dictionary.txt");
    var dictionary = new ManagedDictionary(new string('1', 32), "测试字典", data.Format, "dictionary.txt", "full_pinyin", true, data.Entries);
    await store.SaveAsync(await store.LoadAsync(), [dictionary]);
    Console.WriteLine("Seeded isolated dictionary UI fixture.");
    return;
}

var root = Path.Combine(Path.GetTempPath(), "owo-dictionary-tests-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(root);
var checks = 0;
void Check(bool condition, string message) { if (!condition) throw new Exception(message); checks++; }
async Task Reject(Func<Task> action, string message) {
    try { await action(); } catch (Exception error) when (error is InvalidDataException or IOException or System.Text.Json.JsonException or KeyNotFoundException or DecoderFallbackException) { checks++; return; }
    throw new Exception("Expected rejection: " + message);
}
void Bad(string data, string name = "bad.tsv") => DictionaryFormats.Parse(data, name);
try {
    var v1 = DictionaryFormats.Parse("# Gboard Dictionary version:1\nnh\t你好\tzh-CN\n\t没有编码\tzh-CN\nabc123\t带数字快捷词\ten\n", "dictionary.txt");
    Check(v1.Entries.Count == 3 && v1.Pending == 2 && v1.Entries[0].Input == "nh" && v1.Entries[2].OriginalInput == "abc123", "Gboard v1 shortcuts and missing encodings preserved");
    var v2 = DictionaryFormats.Parse("# Gboard Dictionary version:2\n# Gboard Dictionary format:shortcut\tword\tlanguage_tag\tpos_tag\ncs\t测试\tzh-CN\tnoun\n", "dictionary.txt");
    Check(v2.Entries[0].Locale == "zh-CN" && v2.Entries[0].Tag == "noun", "Gboard v2 locale and POS");
    Check(DictionaryFormats.Parse("# Gboard Dictionary version:2\ncs\t 测试 \tzh-CN\tnoun\ncs\t 测试 \tzh-CN\tverb\n", "dictionary.txt").Entries.Count == 2,
        "Different POS entries are retained");
    Check(DictionaryFormats.Parse("# Gboard Dictionary version:1\nx\t hello \ten\n", "dictionary.txt").Entries[0].Text == " hello ", "Expansion whitespace is preserved");
    var rime = DictionaryFormats.Parse("---\nname: fixture\ncolumns:\n  - text\n  - code\n  - weight\n...\n测试词库\tce shi ci ku\t1000\n测试词库\tce shi ci ku\t20\n", "fixture.dict.yaml");
    Check(rime.Entries.Count == 1 && rime.Duplicates == 1 && rime.AdjustedWeights == 1 && rime.Entries[0].Input == "ce'shi'ci'ku" && rime.Entries[0].OriginalWeight == "1000", "Rime columns, whitespace, duplicates, original frequency");
    var inline = DictionaryFormats.Parse("---\ncolumns: [code, text, weight]\n...\nce shi\t测试\t12\n", "fixture.yaml");
    Check(inline.Entries[0].Count == 12, "Rime inline columns");
    var csv = DictionaryFormats.Parse("word,input,frequency,locale\n\"hello, world\",hello,8,en\n\"say \"\"hello\"\"\",say,2,en\n", "fixture.csv");
    Check(csv.Entries[0].Text == "hello, world" && csv.Entries[1].Text == "say \"hello\"", "CSV quotes and column order");
    Check(DictionaryFormats.Parse("foo\tinput\t2\n", "fixture.tsv").Entries[0].Text == "input", "Word input is not misdetected as a header");
    Check(DictionaryFormats.Parse("词语一\nword two\n", "words.txt").Pending == 2, "Word-only text retained");
    var native = DictionaryFormats.Parse("{\"schema_version\":1,\"kind\":\"OwO.UserLexicon\",\"entries\":[{\"input\":\"ce'shi\",\"text\":\"测试\",\"count\":7}]}", "dictionary.json");
    Check(native.Entries[0].Count == 7, "Legacy OwO JSON");
    var dictionary = new ManagedDictionary(Guid.NewGuid().ToString("N"), "隔离测试", "Gboard", "dictionary.txt", "full_pinyin", true, [.. v1.Entries, .. v2.Entries]);
    foreach (var tsv in new[] { false, true }) {
        var imported = DictionaryFormats.Parse(DictionaryFormats.Export(dictionary, tsv), tsv ? "roundtrip.tsv" : "roundtrip.json");
        Check(imported.Entries.Count == dictionary.Entries.Count && imported.Pending == 2 && imported.Entries.Any(word => word.Tag == "noun") && imported.Entries.Any(word => word.OriginalInput == "abc123"), "Format roundtrip preserves unencoded metadata");
    }
    await Reject(() => { Bad("# Gboard Dictionary version:9\na\tb\ten\n", "dictionary.txt"); return Task.CompletedTask; }, "Unknown Gboard version");
    await Reject(() => { Bad("word,input\n\"unclosed,x\n", "bad.csv"); return Task.CompletedTask; }, "Malformed CSV");
    await Reject(() => { Bad("code\tword\tcount\nfoo\tbad\tnan\n"); return Task.CompletedTask; }, "Bad numeric weight");
    await Reject(() => { Bad("---\nname: missing\n", "bad.yaml"); return Task.CompletedTask; }, "Missing Rime body");
    await Reject(() => { Bad("{\"schema_version\":1,\"schema_version\":1,\"kind\":\"OwO.UserLexicon\",\"entries\":[]}", "bad.json"); return Task.CompletedTask; }, "Duplicate JSON header");
    var zip = Path.Combine(root, "gboard.zip");
    using (var archive = ZipFile.Open(zip, ZipArchiveMode.Create)) {
        await using var writer = new StreamWriter(archive.CreateEntry("dictionary.txt").Open(), new UTF8Encoding(false));
        await writer.WriteAsync("# Gboard Dictionary version:1\ncs\t测试\tzh-CN\n");
    }
    Check((await DictionaryFormats.ReadAsync(zip)).Entries.Count == 1, "Gboard ZIP streaming");
    var utf16 = Path.Combine(root, "utf16.txt");
    await File.WriteAllTextAsync(utf16, "ce'shi\t测试\t2\n", Encoding.Unicode);
    Check((await DictionaryFormats.ReadAsync(utf16)).Entries[0].Input == "ce'shi", "UTF-16 BOM import");
    var unsafeZip = Path.Combine(root, "unsafe.zip");
    using (var archive = ZipFile.Open(unsafeZip, ZipArchiveMode.Create)) archive.CreateEntry("../dictionary.txt");
    await Reject(() => DictionaryFormats.ReadAsync(unsafeZip), "Zip path traversal");
    var store = new DictionaryStore(Path.Combine(root, "store"));
    var empty = await store.LoadAsync();
    var catalog = await store.SaveAsync(empty, [dictionary]);
    Check((await store.LoadAsync()).Dictionaries.Count == 1 && catalog.Revision == 1, "Atomic catalog save");
    string Active() => File.ReadAllText(Path.Combine(store.Root, "snapshots", File.ReadAllText(Path.Combine(store.Root, "current")), "active.tsv"));
    Check(Active().Contains("nh\t你好\t1") && !Active().Contains("没有编码") && !Active().Contains("abc123"), "Only usable words are active, pending rows retained");
    var firstPointer = File.ReadAllText(Path.Combine(store.Root, "current"));
    await Reject(() => store.SaveAsync(empty, []), "Stale revision rejected");
    await Reject(() => store.SaveAsync(catalog, [dictionary with { Name = "" }]), "Invalid mutation rejected");
    Check(File.ReadAllText(Path.Combine(store.Root, "current")) == firstPointer, "Failed operations do not publish a snapshot");
    catalog = await store.SaveAsync(catalog, [dictionary with { Enabled = false }]);
    Check(Active() == "OWO_DICTIONARIES_V1\n", "Disable removes active candidates only");
    catalog = await store.SaveAsync(catalog, []);
    Check((await store.LoadAsync()).Dictionaries.Count == 0 && File.Exists(Path.Combine(store.Root, "snapshots", firstPointer, "catalog.json")), "Delete retains recovery snapshot");
    Check(!Directory.EnumerateFiles(store.Root, "*.tmp").Any(), "No abandoned pointer temporary files");
    Console.WriteLine($"PASS: {checks} dictionary format/store checks, isolated files only.");
} finally { Directory.Delete(root, true); }
