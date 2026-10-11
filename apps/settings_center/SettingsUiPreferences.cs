using System.Text.Json;

namespace OwO_Settings;

internal sealed record SettingsUiPreferences(bool AutoSaveEnabled = true)
{
    internal static SettingsUiPreferences Load(string path) => File.Exists(path)
        ? JsonSerializer.Deserialize<SettingsUiPreferences>(File.ReadAllText(path))
            ?? throw new InvalidDataException("设置中心偏好文件无效。")
        : new();

    internal void Save(string path)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try {
            File.WriteAllText(temporary, JsonSerializer.Serialize(this));
            File.Move(temporary, path, overwrite: true);
        } finally {
            if (File.Exists(temporary)) File.Delete(temporary);
        }
    }
}
