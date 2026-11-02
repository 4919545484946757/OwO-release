using OwO_Settings;

var configurationPages = new[] { "schemes", "candidates", "intelligence", "shortcuts", "punctuation" };
var pages = configurationPages.Concat(new[] { "general", "plugins", "user-lexicon", "plugin:example:1.0.0", "unknown" }).ToArray();
var checks = 0;
foreach (var page in pages) {
    foreach (var automatic in new[] { true, false, true, false }) {
        var actions = SettingsManualActions.ForPage(page, automatic);
        if (automatic) {
            if (actions != default) throw new Exception("Auto-save must hide every manual settings action: " + page);
        } else {
            var configuration = configurationPages.Contains(page);
            if (actions.SaveReload != configuration || actions.ResetPage != (configuration || page is "general" or "plugins") ||
                actions.PluginSettings != page.StartsWith("plugin:", StringComparison.Ordinal))
                throw new Exception("Manual mode lost the page-specific action rules: " + page);
        }
        checks++;
    }
}
Console.WriteLine($"PASS: {checks} auto-save/manual action visibility cases; no config writes.");
