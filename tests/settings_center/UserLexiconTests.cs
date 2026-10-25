using OwO_Settings;
using System.Text;
using System.Text.Json;

var root = Path.Combine(Path.GetTempPath(), "owo-lexicon-files-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(root);
async Task Reject(string contents, string extension = ".json") {
    var path = Path.Combine(root, "invalid" + extension);
    await File.WriteAllTextAsync(path, contents, new UTF8Encoding(false));
    try { await UserLexiconClient.ReadImportAsync(path); } catch { return; }
    throw new Exception("Invalid import was accepted.");
}
try {
    var words = new List<LexiconWord> { new("nihao", "你好", 3), new("ceshi", "测试\"😀", 1) };
    var file = Path.Combine(root, "词库.json");
    await File.WriteAllTextAsync(file, JsonSerializer.Serialize(new LexiconFile(1, "OwO.UserLexicon", words)));
    var parsed = await UserLexiconClient.ReadImportAsync(file);
    if (!parsed.SequenceEqual(words)) throw new Exception("JSON export/import round trip failed.");
    var tsv = Path.Combine(root, "词库.tsv");
    await File.WriteAllTextAsync(tsv, "input\ttext\tcount\r\nNI'HAO\t你好\t3\r\n");
    var table = await UserLexiconClient.ReadImportAsync(tsv);
    if (table.Count != 1 || UserLexiconClient.CanonicalInput(table[0].Input) != "nihao") throw new Exception("TSV/BOM/Chinese path failed.");
    await Reject("{}");
    await Reject("{\"schema_version\":2,\"kind\":\"OwO.UserLexicon\",\"entries\":[]}");
    await Reject("{\"schema_version\":1,\"kind\":\"OwO.UserLexicon\",\"kind\":\"OwO.UserLexicon\",\"entries\":[]}");
    await Reject("{\"schema_version\":1,\"kind\":\"OwO.UserLexicon\",\"entries\":[],\"extra\":true}");
    await Reject(JsonSerializer.Serialize(new LexiconFile(1, "OwO.UserLexicon", [new("nihao", "你好", 129)])));
    await Reject(JsonSerializer.Serialize(new LexiconFile(1, "OwO.UserLexicon", [new("NI'HAO", "你好", 1), new("nihao", "你好", 2)])));
    await Reject(JsonSerializer.Serialize(new LexiconFile(1, "OwO.UserLexicon", [new("test", "错误\n词语", 2)])));
    await Reject("bad\t错误\t0\n", ".tsv");
    await Reject("bad\t多列\t2\textra\n", ".tsv");
    await Reject(new string('x', 4 * 1024 * 1024 + 1));
    var invalidUtf8 = Path.Combine(root, "encoding.tsv");
    await File.WriteAllBytesAsync(invalidUtf8, [0xff, 0xfe, 0xff]);
    var rejected = false;
    try { await UserLexiconClient.ReadImportAsync(invalidUtf8); } catch { rejected = true; }
    if (!rejected) throw new Exception("Invalid encoding was accepted.");
    Console.WriteLine("User lexicon JSON/TSV, round trip, validation and privacy format contracts passed.");
} finally {
    var resolved = Path.GetFullPath(root);
    if (!resolved.StartsWith(Path.GetFullPath(Path.GetTempPath()), StringComparison.OrdinalIgnoreCase) ||
        !Path.GetFileName(resolved).StartsWith("owo-lexicon-files-", StringComparison.Ordinal)) throw new Exception("Unsafe test cleanup path.");
    Directory.Delete(resolved, true);
}
