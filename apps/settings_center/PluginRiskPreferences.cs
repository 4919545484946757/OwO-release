using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace OwO_Settings;

// Presentation preferences only: never an authorization or an installation receipt.
internal static class PluginRiskPreferences
{
    internal static string PathForUser => Path.Combine(Path.GetDirectoryName(SettingsUiPreferences.DefaultPath)!, "plugin-risk-display.json");
    internal static string Key(string identity, IEnumerable<string> permissions) =>
        Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(identity + "\n" +
            string.Join("\n", permissions.Order(StringComparer.Ordinal)))));
    internal static bool Hidden(string key, string? path = null) {
        try { return File.Exists(path ?? PathForUser) &&
            JsonSerializer.Deserialize<HashSet<string>>(File.ReadAllText(path ?? PathForUser))?.Contains(key) == true; }
        catch { return false; } // Corrupt preferences always show the disclosure.
    }
    internal static void Remember(string key, string? path = null, bool hidden = true) {
        path ??= PathForUser;
        var values = new HashSet<string>();
        try { if (File.Exists(path)) values = JsonSerializer.Deserialize<HashSet<string>>(File.ReadAllText(path)) ?? []; } catch { }
        if (values.Count >= 512) values.Clear();
        if (hidden) values.Add(key); else values.Remove(key);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try { File.WriteAllText(temporary, JsonSerializer.Serialize(values)); File.Move(temporary, path, true); }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
}
