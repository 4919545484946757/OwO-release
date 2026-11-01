using OwO_Settings;
using System.Security.Cryptography;
using System.Text.Json;

if (args.Length != 2) throw new ArgumentException("Provide an exported package root and the native plugin shell.");
var root = Path.Combine(Path.GetTempPath(), "owo-bundle-tests-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(root);
var package = Path.Combine(root, "package");
var source = BundledPlugins.Root(args[0]);
foreach (var file in Directory.EnumerateFiles(source, "*", SearchOption.AllDirectories)) {
    var target = Path.Combine(BundledPlugins.Root(package), Path.GetRelativePath(source, file));
    Directory.CreateDirectory(Path.GetDirectoryName(target)!); File.Copy(file, target);
}
var checks = 0;
void Check(bool condition, string message) { if (!condition) throw new Exception(message); checks++; }
void Reject(Action action, string message) {
    try { action(); } catch (Exception error) when (error is InvalidDataException or JsonException or IOException) { checks++; return; }
    throw new Exception("Expected rejection: " + message);
}
var json = new JsonSerializerOptions { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };
var manifest = Path.Combine(BundledPlugins.Root(package), "manifest.json");
var original = File.ReadAllBytes(manifest);
void Write(BundledPluginCatalog value) => File.WriteAllBytes(manifest, JsonSerializer.SerializeToUtf8Bytes(value, json));
Environment.SetEnvironmentVariable("OWO_PLUGIN_SHELL_PATH", Path.GetFullPath(args[1]));
Environment.SetEnvironmentVariable("OWO_PLUGIN_STORE_PATH", Path.Combine(root, "store"));
try {
    var catalog = BundledPlugins.Load(package);
    Check(catalog.Plugins.Select(p => p.Id).ToHashSet().SetEquals(new[] { "org.owo.candidate-glass", "org.owo.unicode-palette" }), "Only the two requested plugins are bundled");
    Check(catalog.Plugins.All(p => p.Active), "Enabled intent preserved");
    Check(catalog.Plugins.Single(p => p.Id == "org.owo.candidate-glass").Settings is not null, "Current acrylic settings included");
    Check(!Directory.EnumerateFiles(BundledPlugins.Root(package), "*", SearchOption.AllDirectories)
        .Any(p => Path.GetFileName(p).Contains("record") || p.Contains("authorizations") || p.Contains("history")), "No foreign grants or history included");
    Check(BundledPlugins.Load(Path.Combine(root, "empty")).Plugins.Count == 0, "Optional missing bundle is harmless");
    Write(catalog with { SchemaVersion = 99 }); Reject(() => BundledPlugins.Load(package), "Unsupported version");
    Write(catalog with { Plugins = [catalog.Plugins[0], catalog.Plugins[0]] }); Reject(() => BundledPlugins.Load(package), "Duplicate identity");
    Write(catalog with { Plugins = [catalog.Plugins[0] with { Package = "../outside.owopkg" }] }); Reject(() => BundledPlugins.Load(package), "Path traversal");
    Write(catalog with { Plugins = [catalog.Plugins[0] with { Id = "../outside" }] }); Reject(() => BundledPlugins.Load(package), "Unsafe id");
    File.WriteAllBytes(manifest, original);
    var firstFile = Path.Combine(BundledPlugins.Root(package), catalog.Plugins[0].Package);
    var firstBytes = File.ReadAllBytes(firstFile);
    File.WriteAllBytes(firstFile, [.. firstBytes, 0]); Reject(() => BundledPlugins.Load(package), "Package tamper");
    File.WriteAllBytes(firstFile, firstBytes);
    var client = new PluginShellClient();
    var confirmations = 0;
    var declined = await BundledPlugins.ImportAsync(package, client, (_, _) => { confirmations++; return Task.FromResult(false); });
    Check(confirmations == 2 && declined.All(line => line.StartsWith("已跳过")), "Explicit per-plugin denial skips both");
    Check((await client.LoadAsync()).Plugins.Count == 0, "No install/authorization/activation on denial");
    var accepted = await BundledPlugins.ImportAsync(package, client, (_, preview) => {
        Check(preview.Permissions.All(p => p is "ui.candidate_theme" or "ui.unicode_palette"), "Isolated test grants only known resource capabilities");
        return Task.FromResult(true);
    });
    Check(accepted.All(line => line.StartsWith("已安装")), "Authorized import succeeds");
    var installed = await client.LoadAsync();
    Check(installed.Plugins.Count == 2 && installed.Plugins.All(p => p.Active && p.PermissionsAuthorized), "Fresh grants bound by native installer; active intent restored");
    var acrylic = installed.Plugins.Single(p => p.Id == "org.owo.candidate-glass");
    var model = PluginSettingsModel.Load(client.StorePath, acrylic);
    Check(File.Exists(model.ConfigPath), "Acrylic configuration restored before activation");
    Reject(() => BundledPlugins.Settings(model, "{\"unknown\":true}"u8.ToArray()), "Unknown config field");
    var flag = model.Fields.First(field => field.Type == "boolean");
    Reject(() => BundledPlugins.Settings(model, JsonSerializer.SerializeToUtf8Bytes(new Dictionary<string, object> { [flag.Key] = 12 })), "Invalid field type");
    var before = SHA256.HashData(File.ReadAllBytes(model.ConfigPath));
    await client.DeactivateAsync("org.owo.unicode-palette", "1.0.0");
    var repeated = await BundledPlugins.ImportAsync(package, client, (_, _) => throw new Exception("Existing version must not be reauthorized"));
    Check(repeated.All(line => line.StartsWith("保留已有")) && before.SequenceEqual(SHA256.HashData(File.ReadAllBytes(model.ConfigPath))), "Reinstall preserves local configuration");
    Check(!(await client.LoadAsync()).Plugins.Single(p => p.Id == "org.owo.unicode-palette").Active, "Reinstall preserves local disabled state");
    Environment.SetEnvironmentVariable("OWO_PLUGIN_STORE_PATH", Path.Combine(root, "tamper-store"));
    var tamperClient = new PluginShellClient();
    var mutationCount = 0;
    await BundledPlugins.ImportAsync(package, tamperClient, (item, _) => {
        if (mutationCount++ == 0) {
            var path = Path.Combine(BundledPlugins.Root(package), item.Package);
            File.WriteAllBytes(path, [.. File.ReadAllBytes(path), 0]);
            return Task.FromResult(true);
        }
        return Task.FromResult(false);
    });
    Check((await tamperClient.LoadAsync()).Plugins.Count == 0, "Tamper after consent cannot publish or activate");
    Console.WriteLine($"PASS: {checks} bundled-plugin privacy, integrity, explicit consent, real native import, configuration and enabled-state checks; isolated stores only.");
} finally {
    if (!Path.GetFileName(root).StartsWith("owo-bundle-tests-", StringComparison.Ordinal)) throw new Exception("Unsafe test cleanup");
    Directory.Delete(root, true);
}
