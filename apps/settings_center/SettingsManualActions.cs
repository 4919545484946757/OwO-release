namespace OwO_Settings;

internal readonly record struct SettingsManualActions(bool SaveReload, bool ResetPage, bool PluginSettings)
{
    internal static SettingsManualActions ForPage(string page, bool autoSaveEnabled) {
        if (autoSaveEnabled) return default;
        var configuration = page is "schemes" or "candidates" or "intelligence" or "shortcuts" or "punctuation";
        return new(configuration, configuration || page is "plugins" or "general", page.StartsWith("plugin:", StringComparison.Ordinal));
    }
}
