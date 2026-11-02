using OwO_Settings;
using System.Text.Json;

var root = Path.Combine(Path.GetTempPath(), "owo-logo-preferences-" + Guid.NewGuid().ToString("N"));
var path = Path.Combine(root, "settings-ui.json");
try {
    var riskPath = Path.Combine(root, "risk.json");
    var riskKey = PluginRiskPreferences.Key("id/1/hash", ["a", "b"]);
    if (PluginRiskPreferences.Hidden(riskKey, riskPath)) throw new Exception("Fresh risk details hidden.");
    PluginRiskPreferences.Remember(riskKey, riskPath);
    if (!PluginRiskPreferences.Hidden(riskKey, riskPath)) throw new Exception("Risk display preference not saved.");
    PluginRiskPreferences.Remember(riskKey, riskPath, hidden: false);
    if (PluginRiskPreferences.Hidden(riskKey, riskPath)) throw new Exception("Risk display preference cannot be reset.");
    PluginRiskPreferences.Remember(riskKey, riskPath);
    if (riskKey != PluginRiskPreferences.Key("id/1/hash", ["b", "a"])) throw new Exception("Permission order changed identity.");
    foreach (var changed in new[] { PluginRiskPreferences.Key("id/2/hash", ["a", "b"]), PluginRiskPreferences.Key("id/1/otherhash", ["a", "b"]), PluginRiskPreferences.Key("id/1/hash", ["a", "b", "c"]) })
        if (PluginRiskPreferences.Hidden(changed, riskPath)) throw new Exception("New version/package/permission reused risk suppression.");
    File.WriteAllText(riskPath, "invalid");
    if (PluginRiskPreferences.Hidden(riskKey, riskPath)) throw new Exception("Corrupt risk preferences must show details.");
    if (SettingsUiPreferences.Load(path).LogoTheme != "system") throw new Exception("New users must follow system theme.");
    Directory.CreateDirectory(root);
    File.WriteAllText(path, "{\"AutoSaveEnabled\":false}");
    var legacy = SettingsUiPreferences.Load(path);
    if (legacy.AutoSaveEnabled || legacy.LogoTheme != "system") throw new Exception("Legacy auto-save preference was lost.");
    foreach (var mode in new[] { "system", "light", "dark" }) {
        (legacy with { LogoTheme = mode }).Save(path);
        var saved = SettingsUiPreferences.Load(path);
        if (saved.AutoSaveEnabled || saved.LogoTheme != mode) throw new Exception("Logo save modified auto-save.");
        (saved with { AutoSaveEnabled = true }).Save(path);
        if (SettingsUiPreferences.Load(path).LogoTheme != mode) throw new Exception("Auto-save change lost logo theme.");
        foreach (var systemLight in new[] { false, true }) {
            var light = mode == "light" || mode == "system" && systemLight;
            foreach (var rounded in new[] { false, true }) {
                var expected = $"Logo{(rounded ? "Small" : "Square")}{(light ? "Light" : "Dark")}.svg";
                if (LogoAssets.Svg(mode, systemLight, rounded) != expected) throw new Exception("Wrong SVG theme resolution.");
            }
            if (LogoAssets.Icon(mode, systemLight) != $"AppIcon.{(light ? "Light" : "Dark")}.ico")
                throw new Exception("Wrong window icon resolution.");
        }
    }
    foreach (var invalid in new[] { "\"other\"", "null" }) {
        File.WriteAllText(path, "{\"LogoTheme\":" + invalid + "}");
        try { SettingsUiPreferences.Load(path); throw new Exception("Invalid logo preference accepted."); }
        catch (InvalidDataException) { }
    }
    File.WriteAllText(path, "invalid");
    try { SettingsUiPreferences.Load(path); throw new Exception("Broken preference overwritten."); }
    catch (JsonException) { }
    if (File.ReadAllText(path) != "invalid") throw new Exception("Read changed preferences.");
    Console.WriteLine("PASS: logo theme defaults, legacy preferences, independent persistence and SVG/ICO selection.");
} finally { if (Directory.Exists(root)) Directory.Delete(root, true); }
