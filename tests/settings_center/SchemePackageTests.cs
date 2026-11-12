using OwO_Settings;
using System.Net;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

var root = Path.Combine(Path.GetTempPath(), "owo-scheme-package-" + Guid.NewGuid().ToString("N"));
var metadata = Path.Combine(root, "versions", "org.test", "1.0.0", "input-schemes", "table", "settings.json");
var bytes = Encoding.UTF8.GetBytes("code\tword\n");
var source = new SchemeDictionarySource("custom", "Fixture", "MIT", $"https://raw.githubusercontent.com/org/repo/{new string('a', 40)}/fixture.tsv", bytes.Length, Convert.ToHexString(SHA256.HashData(bytes)));
var document = new SchemePackageDocument(1, "table", ["show_prefix"], ["learn"], ".tsv", [source]);
var options = new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
void Write(SchemePackageDocument doc) => SchemePackageProfile.WriteFile(metadata, JsonSerializer.SerializeToUtf8Bytes(doc, options));
SchemePackageProfile? Load(string id = "plugin:org.test/table") => SchemePackageProfile.Load(root, "org.test", "1.0.0", id);
void Check(bool condition) { if (!condition) throw new Exception("Scheme package assertion failed."); }
void Reject(Action action) { try { action(); } catch (InvalidDataException) { return; } throw new Exception("Unsafe profile was accepted."); }
try {
    Check(Load() is null);
    Write(document);
    var profile = Load()!;
    Check(profile.Document.CandidateFields.Single() == "show_prefix" && profile.Document.LearningFields.Single() == "learn");
    Check(profile.UserLexiconPath == Path.Combine(root, "data", "org.test", "input-schemes", "table", "user-lexicon.tsv"));
    Check(profile.DownloadPath(source) != profile.UserLexiconPath && Load("plugin:org.test/other") is null);
    Reject(() => Load("plugin:org.test/../table"));
    Reject(() => Load("plugin:org.other/table"));
    Write(document with { Sources = [source with { Url = "https://localhost/a" }] }); Reject(() => Load());
    Write(document with { LexiconExtension = "../user.tsv" }); Reject(() => Load());
    Write(document with { LearningFields = document.CandidateFields }); Reject(() => Load());
    using var client = new HttpClient(new Handler(bytes));
    Check((await SchemePackageProfile.DownloadAsync(source, client, CancellationToken.None)).SequenceEqual(bytes));
    try { await SchemePackageProfile.DownloadAsync(source with { Sha256 = new string('0', 64) }, client, CancellationToken.None); throw new Exception("Wrong digest accepted."); }
    catch (InvalidDataException) { }
    SchemePackageProfile.WriteFile(profile.UserLexiconPath, bytes);
    Check((await SchemePackageProfile.ReadLexiconAsync(profile.UserLexiconPath)).SequenceEqual(bytes));
    Console.WriteLine("PASS: scheme identity, scope isolation, declared sources, fixed-commit URL, digest, private lexicon round trip.");
} finally { if (Directory.Exists(root)) Directory.Delete(root, recursive: true); }

sealed class Handler(byte[] bytes) : HttpMessageHandler {
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) =>
        Task.FromResult(new HttpResponseMessage(HttpStatusCode.OK) { Content = new ByteArrayContent(bytes) });
}
