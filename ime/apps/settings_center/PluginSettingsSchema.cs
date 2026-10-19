using System.Text.Json;
using System.Text.RegularExpressions;

namespace OwO_Settings;

internal sealed record PluginSettingOption(string Label, JsonElement Value);

internal sealed record PluginSettingField(
    string Key,
    string Title,
    string Description,
    string Type,
    string Format,
    JsonElement DefaultValue,
    double? Minimum,
    double? Maximum,
    double? Step,
    IReadOnlyList<PluginSettingOption> Options);

internal sealed class PluginSettingsModel
{
    private static readonly Regex SafeKey = new("^[A-Za-z0-9_.-]{1,64}$",
        RegexOptions.CultureInvariant);

    internal required PluginVersionSnapshot Plugin { get; init; }
    internal required string Title { get; init; }
    internal required string Description { get; init; }
    internal required string ConfigPath { get; init; }
    internal required IReadOnlyList<PluginSettingField> Fields { get; init; }
    internal required Dictionary<string, JsonElement> Values { get; init; }

    internal static PluginSettingsModel Load(string storePath, PluginVersionSnapshot plugin)
    {
        var versionRoot = Path.GetFullPath(Path.Combine(
            storePath, "versions", plugin.Id, plugin.Version));
        var manifestPath = Path.Combine(versionRoot, "manifest.json");
        using var manifest = ReadBoundedJson(manifestPath, 64 * 1024);
        var schemaRelative = manifest.RootElement.GetProperty("config_schema").GetString()
            ?? throw new InvalidOperationException("插件清单未声明配置结构。" );
        var schemaPath = Path.GetFullPath(Path.Combine(versionRoot, schemaRelative));
        if (!schemaPath.StartsWith(versionRoot + Path.DirectorySeparatorChar,
                                   StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("插件配置结构路径越过版本目录。" );

        using var schema = ReadBoundedJson(schemaPath, 256 * 1024);
        var root = schema.RootElement;
        if (root.TryGetProperty("type", out var rootType) && rootType.GetString() != "object")
            throw new InvalidOperationException("插件配置结构顶层必须是 object。" );
        var title = Text(root, "title", $"{plugin.Name} 设置", 128);
        var description = Text(root, "description", "由插件提供的声明式配置页面。", 512);
        var fields = new List<PluginSettingField>();
        if (root.TryGetProperty("properties", out var properties)) {
            if (properties.ValueKind != JsonValueKind.Object)
                throw new InvalidOperationException("插件配置 properties 必须是对象。" );
            foreach (var property in properties.EnumerateObject()) {
                if (fields.Count >= 64 || !SafeKey.IsMatch(property.Name) ||
                    property.Value.ValueKind != JsonValueKind.Object)
                    throw new InvalidOperationException("插件配置字段数量、名称或结构无效。" );
                var value = property.Value;
                var type = Text(value, "type", "string", 16);
                if (type is not ("boolean" or "string" or "integer" or "number"))
                    throw new InvalidOperationException($"插件设置 {property.Name} 使用了不支持的类型。" );
                var format = Text(value, "format", "", 16);
                if (format is not ("" or "color") || (format == "color" && type != "string"))
                    throw new InvalidOperationException($"插件设置 {property.Name} 使用了不支持的格式。" );
                var defaultValue = value.TryGetProperty("default", out var defaultElement)
                    ? defaultElement.Clone() : DefaultFor(type);
                var options = new List<PluginSettingOption>();
                if (value.TryGetProperty("enum", out var enumElement)) {
                    if (enumElement.ValueKind != JsonValueKind.Array || enumElement.GetArrayLength() > 32)
                        throw new InvalidOperationException($"插件设置 {property.Name} 的枚举无效。" );
                    foreach (var option in enumElement.EnumerateArray())
                        options.Add(new PluginSettingOption(Display(option), option.Clone()));
                }
                fields.Add(new PluginSettingField(
                    property.Name,
                    Text(value, "title", property.Name, 128),
                    Text(value, "description", "", 512),
                    type,
                    format,
                    defaultValue,
                    Number(value, "minimum"),
                    Number(value, "maximum"),
                    Number(value, "multipleOf"),
                    options));
            }
        }

        var dataRoot = Path.GetFullPath(Path.Combine(storePath, "data", plugin.Id));
        var configPath = Path.Combine(dataRoot, "settings.json");
        var values = new Dictionary<string, JsonElement>(StringComparer.Ordinal);
        if (File.Exists(configPath)) {
            using var config = ReadBoundedJson(configPath, 256 * 1024);
            if (config.RootElement.ValueKind == JsonValueKind.Object) {
                foreach (var property in config.RootElement.EnumerateObject())
                    if (fields.Any(field => field.Key == property.Name))
                        values[property.Name] = property.Value.Clone();
            }
        }
        foreach (var field in fields)
            if (!values.ContainsKey(field.Key)) values[field.Key] = field.DefaultValue.Clone();
        return new PluginSettingsModel {
            Plugin = plugin, Title = title, Description = description,
            ConfigPath = configPath, Fields = fields, Values = values,
        };
    }

    internal void Save(IReadOnlyDictionary<string, object?> values)
    {
        var directory = Path.GetDirectoryName(ConfigPath)
            ?? throw new InvalidOperationException("插件配置目录无效。" );
        Directory.CreateDirectory(directory);
        var temporary = ConfigPath + ".tmp";
        var json = JsonSerializer.Serialize(values, new JsonSerializerOptions { WriteIndented = true });
        File.WriteAllText(temporary, json);
        File.Move(temporary, ConfigPath, true);
    }

    internal Dictionary<string, JsonElement> DefaultValues() => Fields.ToDictionary(
        field => field.Key, field => field.DefaultValue.Clone(), StringComparer.Ordinal);

    private static JsonDocument ReadBoundedJson(string path, int maximumBytes)
    {
        var info = new FileInfo(path);
        if (!info.Exists || info.Length <= 0 || info.Length > maximumBytes)
            throw new InvalidOperationException($"插件配置文件缺失或大小无效：{path}" );
        return JsonDocument.Parse(File.ReadAllBytes(path), new JsonDocumentOptions {
            AllowTrailingCommas = false, CommentHandling = JsonCommentHandling.Disallow,
            MaxDepth = 32,
        });
    }

    private static string Text(JsonElement element, string name, string fallback, int maximum)
    {
        if (!element.TryGetProperty(name, out var value)) return fallback;
        var text = value.ValueKind == JsonValueKind.String ? value.GetString() : null;
        if (string.IsNullOrWhiteSpace(text) || text.Length > maximum)
            throw new InvalidOperationException($"插件配置字段 {name} 无效。" );
        return text;
    }

    private static double? Number(JsonElement element, string name) =>
        element.TryGetProperty(name, out var value) && value.TryGetDouble(out var number)
            ? number : null;

    private static JsonElement DefaultFor(string type) => JsonSerializer.SerializeToElement(
        type switch { "boolean" => (object)false, "integer" => 0, "number" => 0.0, _ => "" });

    private static string Display(JsonElement value) => value.ValueKind switch {
        JsonValueKind.String => value.GetString() ?? "",
        JsonValueKind.True => "开", JsonValueKind.False => "关",
        _ => value.GetRawText(),
    };
}
