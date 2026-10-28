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
if (!model.Pages.Select(page => page.Id).SequenceEqual(new[] { "common", "light", "dark" }) ||
    model.Fields.Any(field => field.Key != "theme_mode" && field.PageId.Length == 0) ||
    model.Fields.Where(field => field.Key.StartsWith("light_", StringComparison.Ordinal)).Any(field => field.PageId != "light"))
    throw new Exception("Plugin pages were not declared or assigned correctly.");
var mode = model.Fields.Single(field => field.Key == "theme_mode");
if (!mode.Options.Select(option => option.Label).SequenceEqual(new[] { "跟随系统", "浅色", "深色" }) ||
    !mode.Options.Select(option => option.Value.GetString()).SequenceEqual(new[] { "system", "light", "dark" }))
    throw new Exception("Localized mode labels changed persisted option values.");
if (model.Values["theme_mode"].GetString() != "system" ||
    model.Values["background_color"].GetString() != "#223344" ||
    model.Values["background_opacity"].GetDouble() != 0.52 ||
    model.Values["shadow_opacity"].GetDouble() != 0.43 ||
    model.Values["light_background_color"].GetString() != "#F5F5F5" ||
    model.Values["light_candidate_block_opacity"].GetDouble() != 0.05 ||
    File.ReadAllText(model.ConfigPath) != existing)
    throw new Exception("Loading the dual palette modified or lost old settings.");
foreach (var field in model.Fields.Where(field => field.Format == "color"))
    if (!model.Fields.Any(other => other.Key == field.Key[..^"_color".Length] + "_opacity"))
        throw new Exception("Color/opacity expander pairing is incomplete.");
model.Save(model.Values.ToDictionary(pair => pair.Key,
    pair => JsonSerializer.Deserialize<object>(pair.Value.GetRawText())));
if (PluginSettingsModel.Load(root, plugin).Values["background_color"].GetString() != "#223344")
    throw new Exception("Saving lost old palette values.");
var roles = new[] { "background", "border", "divider", "pinyin_text", "candidate_text",
                    "candidate_block", "candidate_number", "shadow", "candidate_badge", "selected_page",
                    "hover_block", "pressed_block", "button_background", "button_border", "button_text", "secondary_text", "caret" };
// Save through the same model as the editor, in an isolated store. Every role
// needs its own color AND opacity in each palette, not a shared opacity key.
var edited = model.Values.ToDictionary(pair => pair.Key,
    pair => JsonSerializer.Deserialize<object>(pair.Value.GetRawText()));
for (var index = 0; index < roles.Length; ++index) {
    var role = roles[index];
    foreach (var prefix in new[] { "", "light_" }) {
        if (!model.Fields.Any(field => field.Key == prefix + role + "_color" && field.Format == "color") ||
            !model.Fields.Any(field => field.Key == prefix + role + "_opacity" && field.Type == "number"))
            throw new Exception($"Missing independent palette controls: {prefix}{role}");
        edited[prefix + role + "_color"] = $"#{(prefix.Length == 0 ? 0x100000 : 0x200000) + index:X6}";
        edited[prefix + role + "_opacity"] = (index + 1) / (prefix.Length == 0 ? 20.0 : 40.0);
    }
}
foreach (var themeMode in new[] { "system", "light", "dark" }) {
    edited["theme_mode"] = themeMode;
    model.Save(edited);
    var saved = PluginSettingsModel.Load(root, plugin);
    foreach (var pair in edited)
        if (saved.Values[pair.Key].GetRawText() != JsonSerializer.Serialize(pair.Value))
            throw new Exception($"Saving/switching themes changed {pair.Key}.");
}
foreach (var prefix in new[] { "light_", "" }) {
    var opposite = prefix.Length == 0 ? "light_" : "";
    var before = PluginSettingsModel.Load(root, plugin);
    foreach (var role in roles) {
        edited[prefix + role + "_color"] = "#405060";
        edited[prefix + role + "_opacity"] = 0.37;
    }
    model.Save(edited);
    var after = PluginSettingsModel.Load(root, plugin);
    foreach (var role in roles) foreach (var suffix in new[] { "_color", "_opacity" })
        if (after.Values[opposite + role + suffix].GetRawText() != before.Values[opposite + role + suffix].GetRawText())
            throw new Exception("Editing one palette modified the other palette.");
}
var schemaPath = Path.Combine(versionRoot, "config.schema.json");
var original = JsonNode.Parse(File.ReadAllText(schemaPath))!.AsObject();
foreach (var invalidKind in new[] { "unknown-page", "duplicate-page", "invalid-pages" }) {
    var schema = original.DeepClone().AsObject();
    if (invalidKind == "unknown-page") schema["properties"]!["light_background_color"]!["x-page"] = "missing";
    else if (invalidKind == "duplicate-page") schema["x-pages"]!.AsArray().Add(schema["x-pages"]![0]!.DeepClone());
    else schema["x-pages"] = new JsonObject();
    File.WriteAllText(schemaPath, schema.ToJsonString());
    try { PluginSettingsModel.Load(root, plugin); }
    catch (InvalidOperationException) { continue; }
    throw new Exception("Invalid plugin pages were accepted.");
}
foreach (var invalid in new[] { new JsonArray("only one"), new JsonArray("ok", "", "dark"),
                              new JsonArray("ok", new string('x', 129), "dark") }) {
    var schema = original.DeepClone().AsObject();
    schema["properties"]!["theme_mode"]!["enumNames"] = invalid;
    File.WriteAllText(schemaPath, schema.ToJsonString());
    try { PluginSettingsModel.Load(root, plugin); }
    catch (InvalidOperationException) { continue; }
    throw new Exception("Invalid enum labels were accepted.");
}
// Schemas without paging metadata retain the original flat editor contract.
var legacy = original.DeepClone().AsObject();
legacy.Remove("x-pages");
foreach (var property in legacy["properties"]!.AsObject()) property.Value!.AsObject().Remove("x-page");
File.WriteAllText(schemaPath, legacy.ToJsonString());
if (PluginSettingsModel.Load(root, plugin).Pages.Count != 0)
    throw new Exception("Legacy schemas unexpectedly require pages.");
Console.WriteLine("PASS: 17 independent color/opacity pairs per palette, declarative pages, palette isolation and legacy schemas.");
