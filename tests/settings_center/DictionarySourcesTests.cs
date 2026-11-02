using OwO_Settings;
using System.Text;
using System.IO.Compression;
using System.Security.Cryptography;
using System.Net;

void Check(bool value) { if (!value) throw new Exception("Dictionary source assertion failed."); }
if (args.Length == 2 && args[0] == "--moegirl-file") {
    var data = await File.ReadAllBytesAsync(args[1]);
    var digest = Convert.ToHexString(SHA256.HashData(data)).ToLowerInvariant();
    var source = DictionarySources.Sources.Single(s => s.Id == "moegirl");
    var download = new DictionaryDownload(source, "moegirl.dict.yaml",
        new("https://github.com/outloudvi/mw2fcitx/releases/download/20261024/moegirl.dict.yaml"), data.Length, digest, false);
    var result = DictionarySources.Prepare(download, data, DictionaryStore.MaximumEntries, digest, CancellationToken.None);
    foreach (var word in result.Data.Entries) DictionaryStore.ValidateWord(word);
    var longCodes = result.Data.Entries.Where(word => word.OriginalInput.Length > 256).ToArray();
    Check(longCodes.Length > 0 && longCodes.All(word => word.Input == ""));
    var selected = DictionarySources.Prepare(download, data, 50000, digest, CancellationToken.None);
    Check(selected.Available == result.Available && selected.Data.Entries.Count == Math.Min(50000, result.Data.Entries.Count));
    var local = DictionaryFormats.Parse(new UTF8Encoding(false, true).GetString(data), "moegirl.dict.yaml");
    Check(local.Entries.Count == result.Data.Entries.Count && local.Pending == result.Data.Pending);
    Check(local.Entries.Where(word => word.OriginalInput.Length > 256).Select(word => word.OriginalInput)
        .SequenceEqual(longCodes.Select(word => word.OriginalInput)));
    Console.WriteLine($"Moegirl full offline conversion: available={result.Available}; imported={result.Data.Entries.Count}; pending={result.Data.Pending}; long_original_codes={longCodes.Length}; sha256={digest}; no installation.");
    Console.WriteLine($"Default selection={selected.Data.Entries.Count}; pending={selected.Data.Pending}; local Rime import={local.Entries.Count}.");
    return;
}
if (args.Contains("--live")) {
    using var client = new DictionarySources();
    foreach (var source in DictionarySources.Sources) {
        using var deadline = new CancellationTokenSource(TimeSpan.FromMinutes(3));
        var files = await client.ListAsync(source, deadline.Token);
        var choice = files.Where(f => f.Size <= 8 * 1024 * 1024).OrderBy(f => f.Size).FirstOrDefault();
        Check(choice is not null);
        var data = await client.FetchAsync(choice!, 3, null, deadline.Token);
        Check(data.Data.Entries.Count is > 0 and <= 3);
        Console.WriteLine($"{source.Name}: files={files.Count}; sampled={data.Data.Entries.Count}; pending={data.Data.Pending}; available={data.Available}; file={choice!.Name}; no installation.");
    }
    return;
}
foreach (var bad in new[] { "http://github.com/iDvel/rime-ice/a", "https://github.com.evil/a", "https://github.com/evil/repo/a", "https://user@www.mdbg.net/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.txt.gz", "https://raw.githubusercontent.com/evil/repo/main/a" }) {
    try { DictionarySources.ValidateUrl(new(bad)); throw new Exception("Unsafe origin accepted"); } catch (InvalidDataException) { }
}
var ice = DictionarySources.Sources[0];
var item = new DictionaryDownload(ice, "test.dict.yaml", new("https://raw.githubusercontent.com/iDvel/rime-ice/main/cn_dicts/test.dict.yaml"), 0, "", true);
var bytes = Encoding.UTF8.GetBytes("---\nname: test\n...\n测试\tce shi\t9999999\n词库\tci ku\t100\n无编码\n");
var prepared = DictionarySources.Prepare(item, bytes, 2, "test", CancellationToken.None);
Check(prepared.Available == 3 && prepared.Data.Entries.Count == 2 && prepared.Data.Pending == 0);
Check(prepared.Data.Entries[0].Input == "ce'shi" && prepared.Data.Entries[0].Count == 1);
prepared = DictionarySources.Prepare(item, bytes, 3, "test", CancellationToken.None);
Check(prepared.Data.Pending == 1);
var longReading = string.Join(" ", Enumerable.Repeat("bu yao", 39));
Check(longReading.Length == 272);
var moegirl = item with { Source = DictionarySources.Sources.Single(s => s.Id == "moegirl"), Name = "moegirl.dict.yaml" };
prepared = DictionarySources.Prepare(moegirl, Encoding.UTF8.GetBytes(
    "---\nname: fixture\n...\n前条\tqian tiao\n" + new string('测', 78) + "\t" + longReading + "\n后条\thou tiao\n"), 10, "test", CancellationToken.None);
Check(prepared.Available == 3 && prepared.Data.Entries.Count == 3 && prepared.Data.Pending == 1);
Check(prepared.Data.Entries[1].Input == "" && prepared.Data.Entries[1].OriginalInput == longReading &&
    prepared.Data.Entries[1].Text == new string('测', 78) && prepared.Data.Entries[2].Input == "hou'tiao");
prepared = DictionarySources.Prepare(moegirl, Encoding.UTF8.GetBytes(
    "---\nname: fixture\n...\n测试\t" + longReading + "\n正常\tzheng chang\n"), 1, "test", CancellationToken.None);
Check(prepared.Available == 2 && prepared.Data.Entries.Count == 1 && prepared.Data.Pending == 1);
var thu = item with { Source = DictionarySources.Sources[3], Name = "THUOCL_IT.txt" };
prepared = DictionarySources.Prepare(thu, Encoding.UTF8.GetBytes("人工智能\t99999\n"), 3, "test", CancellationToken.None);
Check(prepared.Data.Pending == 1 && prepared.Data.Entries[0].Text == "人工智能");
prepared = DictionarySources.Prepare(thu, Encoding.UTF8.GetBytes("字符串 \t1234\n 初始化　\t999\n数组\t456\n \t1\n"), 10, "test", CancellationToken.None);
Check(prepared.Data.Pending == 3 && prepared.Available == 3 && prepared.Data.Entries.Select(word => word.Text).SequenceEqual(new[] { "字符串", "初始化", "数组" }));
using var zipped = new MemoryStream();
using (var gzip = new GZipStream(zipped, CompressionMode.Compress, true)) gzip.Write(Encoding.UTF8.GetBytes("# test\n女 女 [nu:3] /female/\n綠 绿 [lu:4] /green/\n"));
prepared = DictionarySources.Prepare(item with { Source = DictionarySources.Sources[4] }, zipped.ToArray(), 3, "test", CancellationToken.None);
Check(prepared.Data.Entries[0].Input == "nv" && prepared.Data.Entries[1].Input == "lv" && prepared.Data.Entries[1].Text == "绿");
using var fake = new DictionarySources(new Handler(bytes));
using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA1);
hash.AppendData(Encoding.ASCII.GetBytes($"blob {bytes.Length}\0")); hash.AppendData(bytes);
var valid = item with { Size = bytes.Length, Hash = Convert.ToHexString(hash.GetHashAndReset()) };
Check((await fake.FetchAsync(valid, 2, null, CancellationToken.None)).Available == 3);
try { await fake.FetchAsync(valid with { Hash = new('0', 40) }, 2, null, CancellationToken.None); throw new Exception("Hash mismatch accepted"); } catch (InvalidDataException) { }
using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
try { await fake.FetchAsync(valid, 2, null, cancelled.Token); throw new Exception("Cancellation ignored"); } catch (OperationCanceledException) { }
Console.WriteLine("Dictionary sources: origins, Rime/THUOCL/CEDICT conversion, selection cap, neutral weights, Git digest, mismatch and cancellation passed.");

sealed class Handler(byte[] data) : HttpMessageHandler {
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken token) { token.ThrowIfCancellationRequested(); return Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) { Content = new ByteArrayContent(data) }); }
}
