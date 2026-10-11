using OwO_Settings;

var root = Path.Combine(Path.GetTempPath(), "owo-ui-preferences-" + Guid.NewGuid().ToString("N"));
var path = Path.Combine(root, "config", "settings-ui.json");
try {
    if (!SettingsUiPreferences.Load(path).AutoSaveEnabled) throw new Exception("Default must enable auto-save");
    new SettingsUiPreferences(false).Save(path);
    if (SettingsUiPreferences.Load(path).AutoSaveEnabled) throw new Exception("Disabled state was not persisted");
    new SettingsUiPreferences(true).Save(path);
    if (!SettingsUiPreferences.Load(path).AutoSaveEnabled) throw new Exception("Enabled state was not persisted");
    if (Directory.EnumerateFiles(Path.GetDirectoryName(path)!, "*.tmp").Any()) throw new Exception("Temporary file leaked");
    File.WriteAllText(path, "broken");
    var rejected = false;
    try { SettingsUiPreferences.Load(path); } catch (System.Text.Json.JsonException) { rejected = true; }
    if (!rejected || File.ReadAllText(path) != "broken") throw new Exception("Malformed preference was silently overwritten");
    Console.WriteLine("Settings UI preference tests passed.");
} finally {
    if (Directory.Exists(root)) Directory.Delete(root, recursive: true);
}
