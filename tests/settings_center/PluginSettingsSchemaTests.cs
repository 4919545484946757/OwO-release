using OwO_Settings;
using System.Text.Json;
using System.Text.Json.Nodes;

var repository = Path.GetFullPath(args[0]);
var root = Path.Combine(repository, "build", "plugin-theme-schema-" + Guid.NewGuid().ToString("N"));
using var manifest = JsonDocument.Parse(File.ReadAllText(Path.Combine(repository, "plugins", "candidate-glass", "manifest.json")));
var version = manifest.RootElement.GetProperty("version").GetString()!;
var versionRoot = Path.Combine(root, "versions", "org.owo.candidate-glass", version);
Directory.CreateDirectory(versionRoot);
foreach (var file in new[] { "manifest.json", "config.schema.json" })
    File.Copy(Path.Combine(repository, "plugins", "candidate-glass", file), Path.Combine(versionRoot, file));
var data = Path.Combine(root, "data", "org.owo.candidate-glass");
Directory.CreateDirectory(data);
var existing = """{"background_color":"#223344","background_opacity":0.52,"shadow_opacity":0.43}""";
File.WriteAllText(Path.Combine(data, "settings.json"), existing);
var plugin = new PluginVersionSnapshot("org.owo.candidate-glass", "OwO 亚克力候选框", version,
    true, "unverified_package", "critical", true, ["ui.candidate_theme"]);
var model = PluginSettingsModel.Load(root, plugin);
var mode = model.Fields.Single(field => field.Key == "theme_mode");
if (!mode.Options.Select(option => option.Label).SequenceEqual(new[] { "跟随系统", "浅色", "深色" }) ||
    !mode.Options.Select(option => option.Value.GetString()).SequenceEqual(new[] { "system", "light", "dark" }))
    throw new Exception("Localized mode labels changed persisted option values.");
if (model.Values["theme_mode"].GetString() != "system" ||
    model.Values["background_color"].GetString() != "#223344" ||
    model.Values["background_opacity"].GetDouble() != 0.52 ||
    model.Values["shadow_opacity"].GetDouble() != 0.43 ||
    model.Values["light_background_color"].GetString() != "#F5F5F5" ||
    File.ReadAllText(model.ConfigPath) != existing)
    throw new Exception("Loading the dual palette modified or lost old settings.");
foreach (var field in model.Fields.Where(field => field.Format == "color"))
    if (!model.Fields.Any(other => other.Key == field.Key[..^"_color".Length] + "_opacity"))
        throw new Exception("Color/opacity expander pairing is incomplete.");
model.Save(model.Values.ToDictionary(pair => pair.Key,
    pair => JsonSerializer.Deserialize<object>(pair.Value.GetRawText())));
if (PluginSettingsModel.Load(root, plugin).Values["background_color"].GetString() != "#223344")
    throw new Exception("Saving lost old palette values.");
var schemaPath = Path.Combine(versionRoot, "config.schema.json");
var original = JsonNode.Parse(File.ReadAllText(schemaPath))!.AsObject();
foreach (var invalid in new[] { new JsonArray("only one"), new JsonArray("ok", "", "dark"),
                              new JsonArray("ok", new string('x', 129), "dark") }) {
    var schema = original.DeepClone().AsObject();
    schema["properties"]!["theme_mode"]!["enumNames"] = invalid;
    File.WriteAllText(schemaPath, schema.ToJsonString());
    try { PluginSettingsModel.Load(root, plugin); }
    catch (InvalidOperationException) { continue; }
    throw new Exception("Invalid enum labels were accepted.");
}
Console.WriteLine("PASS: dual-palette schema, localized enum labels, legacy value preservation, opacity pairing and invalid-label rejection.");
