using OwO_Settings;
using System.Text;
using System.IO.Compression;
using System.Security.Cryptography;
using System.Net;

void Check(bool value) { if (!value) throw new Exception("Dictionary source assertion failed."); }
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
