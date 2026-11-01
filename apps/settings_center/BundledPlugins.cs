using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Text.RegularExpressions;

namespace OwO_Settings;

internal sealed record BundledPlugin(string Id, string Version, string Name, bool Active,
    string Package, string Sha256, string? Settings, string? SettingsSha256);
internal sealed record BundledPluginCatalog(int SchemaVersion, IReadOnlyList<BundledPlugin> Plugins);

// Bundling stores intent, never another machine's authorization. Every new
// package goes through the native inspector and the existing consent-bound
// installer; only the caller's explicit per-package confirmation permits it.
internal static class BundledPlugins
{
    private static readonly JsonSerializerOptions Json = new() {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow, MaxDepth = 12
    };
    internal static string Root(string packageRoot) => Path.Combine(Path.GetFullPath(packageRoot), "bundled-plugins");
    internal static BundledPluginCatalog Load(string packageRoot) {
        var root = Root(packageRoot);
        var manifest = Path.Combine(root, "manifest.json");
        if (!File.Exists(manifest)) return new(1, []);
        var bytes = Read(root, "manifest.json", 1024 * 1024);
        var catalog = JsonSerializer.Deserialize<BundledPluginCatalog>(bytes, Json)
            ?? throw new InvalidDataException("随包插件目录为空。");
        if (catalog.SchemaVersion != 1 || catalog.Plugins is null || catalog.Plugins.Count > 128)
            throw new InvalidDataException("随包插件目录版本或数量无效。");
        var identities = new HashSet<string>(StringComparer.Ordinal);
        var active = new HashSet<string>(StringComparer.Ordinal);
        foreach (var item in catalog.Plugins) {
            if (item is null || item.Id is null || !Regex.IsMatch(item.Id, "^[a-z0-9][a-z0-9.-]{0,127}$") || item.Id.Contains("..") ||
                item.Version is null || !Regex.IsMatch(item.Version, "^[0-9A-Za-z][0-9A-Za-z.+-]{0,63}$") ||
                string.IsNullOrWhiteSpace(item.Name) || item.Name.Length > 256 || item.Name.Any(char.IsControl) ||
                !identities.Add(item.Id + "/" + item.Version) || (item.Active && !active.Add(item.Id)) ||
                item.Package != $"packages/{item.Id}-{item.Version}.owopkg" ||
                (item.Settings is not null && item.Settings != $"config/{item.Id}.json") ||
                (item.Settings is null) != (item.SettingsSha256 is null))
                throw new InvalidDataException("随包插件标识、路径或启用状态无效。");
            Verify(root, item.Package, item.Sha256, 256L * 1024 * 1024);
            if (item.Settings is not null) Verify(root, item.Settings, item.SettingsSha256!, 256 * 1024);
        }
        return catalog;
    }
    private static string Resolve(string root, string relative) {
        var path = Path.GetFullPath(Path.Combine(root, relative));
        var boundary = Path.GetFullPath(root);
        if (!path.StartsWith(boundary + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
            throw new InvalidDataException("随包插件路径越界。");
        for (var current = path; current.Length >= boundary.Length; current = Path.GetDirectoryName(current)!) {
            if ((File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0)
                throw new InvalidDataException("随包插件不能包含链接。");
            if (current.Equals(boundary, StringComparison.OrdinalIgnoreCase)) break;
        }
        return path;
    }
    internal static byte[] Read(string root, string relative, long maximum) {
        using var input = File.OpenRead(Resolve(root, relative));
        if (input.Length <= 0 || input.Length > maximum) throw new InvalidDataException("随包插件文件大小无效。");
        using var output = new MemoryStream(); input.CopyTo(output);
        if (output.Length > maximum) throw new InvalidDataException("随包插件文件大小发生变化。");
        return output.ToArray();
    }
    private static void Verify(string root, string relative, string hash, long maximum) {
        using var input = File.OpenRead(Resolve(root, relative));
        if (input.Length <= 0 || input.Length > maximum || hash is null || !Regex.IsMatch(hash, "^[0-9a-f]{64}$") ||
            !Convert.ToHexString(SHA256.HashData(input)).Equals(hash, StringComparison.OrdinalIgnoreCase))
            throw new InvalidDataException("随包插件校验失败，未安装插件。");
    }
    internal static Dictionary<string, object?> Settings(PluginSettingsModel model, byte[] bytes) {
        using var document = JsonDocument.Parse(bytes, new JsonDocumentOptions { MaxDepth = 16 });
        if (document.RootElement.ValueKind != JsonValueKind.Object) throw new InvalidDataException("随包插件配置必须是对象。");
        var values = model.DefaultValues().ToDictionary(pair => pair.Key, pair => (object?)pair.Value, StringComparer.Ordinal);
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var property in document.RootElement.EnumerateObject()) {
            var field = model.Fields.FirstOrDefault(value => value.Key == property.Name);
            if (field is null || !seen.Add(property.Name)) throw new InvalidDataException("随包插件配置包含未知或重复字段。");
            var value = property.Value;
            var valid = field.Type switch {
                "boolean" => value.ValueKind is JsonValueKind.True or JsonValueKind.False,
                "string" => value.ValueKind == JsonValueKind.String && value.GetString()!.Length <= 4096 &&
                    (field.Format != "color" || Regex.IsMatch(value.GetString()!, "^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$")),
                "integer" or "number" => value.ValueKind == JsonValueKind.Number && value.TryGetDouble(out var number) && double.IsFinite(number) &&
                    (field.Type != "integer" || number == Math.Truncate(number)) &&
                    (field.Minimum is null || number >= field.Minimum) && (field.Maximum is null || number <= field.Maximum),
                _ => false
            };
            if (!valid || (field.Options.Count > 0 && !field.Options.Any(option => JsonElement.DeepEquals(option.Value, value))))
                throw new InvalidDataException($"随包插件配置字段无效：{field.Key}");
            values[property.Name] = value.Clone();
        }
        return values;
    }
    internal static async Task<IReadOnlyList<string>> ImportAsync(string packageRoot, PluginShellClient client,
        Func<BundledPlugin, PluginInstallPreviewSnapshot, Task<bool>> confirm, IProgress<string>? progress = null) {
        var catalog = Load(packageRoot);
        var root = Root(packageRoot);
        var outcomes = new List<string>();
        foreach (var item in catalog.Plugins) {
            try {
                var installed = await client.LoadAsync();
                if (installed.Plugins.Any(value => value.Id == item.Id && value.Version == item.Version)) {
                    outcomes.Add($"保留已有版本、设置与启用状态：{item.Name} {item.Version}"); continue;
                }
                var path = Path.Combine(root, item.Package);
                var preview = await client.InspectInstallAsync(path);
                if (preview.PluginId != item.Id || preview.Version != item.Version)
                    throw new InvalidDataException("随包插件身份与原生预检不一致。");
                if (!await confirm(item, preview)) { outcomes.Add($"已跳过：{item.Name}"); continue; }
                progress?.Report($"正在安装随包插件：{item.Name}");
                // Recheck payload/config hashes after the user reads the prompt.
                Verify(root, item.Package, item.Sha256, 256L * 1024 * 1024);
                await client.InstallRiskAsync(path, preview.InventorySha256);
                var current = (await client.LoadAsync()).Plugins.Single(value => value.Id == item.Id && value.Version == item.Version);
                if (item.Settings is not null) {
                    Verify(root, item.Settings, item.SettingsSha256!, 256 * 1024);
                    var model = PluginSettingsModel.Load(client.StorePath, current);
                    if (!File.Exists(model.ConfigPath)) model.Save(Settings(model, Read(root, item.Settings, 256 * 1024)));
                }
                if (item.Active) await client.ActivateAsync(item.Id, item.Version, preview.Permissions.Contains("agent.ipc"));
                outcomes.Add($"已安装{(item.Active ? "并启用" : "，保持停用")}：{item.Name} {item.Version}");
            } catch (Exception error) { outcomes.Add($"插件未完整恢复，请在设置中心检查：{item.Name}：{error.Message}"); }
        }
        return outcomes;
    }
}
