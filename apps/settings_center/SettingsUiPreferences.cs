using System.Text.Json;

namespace OwO_Settings;

internal sealed record SettingsUiPreferences(bool AutoSaveEnabled = true, string LogoTheme = "system")
{
    internal static string DefaultPath => Path.Combine(Path.GetDirectoryName(
        Environment.GetEnvironmentVariable("OWO_CONFIG_PATH") ?? Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "OwO", "InputMethod", "config", "owo.conf"))!, "settings-ui.json");

    internal static SettingsUiPreferences Load(string path) {
        var value = File.Exists(path)
        ? JsonSerializer.Deserialize<SettingsUiPreferences>(File.ReadAllText(path))
            ?? throw new InvalidDataException("设置中心偏好文件无效。")
        : new();
        if (!LogoAssets.IsValid(value.LogoTheme)) throw new InvalidDataException("Logo 外观选项无效。");
        return value;
    }

    internal void Save(string path)
    {
        if (!LogoAssets.IsValid(LogoTheme)) throw new InvalidDataException("Logo 外观选项无效。");
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

internal static class LogoAssets
{
    internal static bool IsValid(string? mode) => mode is "system" or "light" or "dark";
    internal static bool IsLight(string mode, bool systemLight) => mode switch {
        "light" => true, "dark" => false, "system" => systemLight,
        _ => throw new InvalidDataException("Logo 外观选项无效。")
    };
    internal static string Svg(string mode, bool systemLight, bool rounded) =>
        $"Logo{(rounded ? "Rounded" : "Square")}{(IsLight(mode, systemLight) ? "Light" : "Dark")}.svg";
    internal static string Icon(string mode, bool systemLight) =>
        $"AppIcon.{(IsLight(mode, systemLight) ? "Light" : "Dark")}.ico";
}
