namespace OwO_Settings;

internal static class ShortcutSettingsValidation
{
    internal static void Validate(SettingsSnapshot snapshot)
    {
        var shortcuts = new List<string>();
        if (snapshot.CorrectionShortcutEnabled) shortcuts.AddRange(snapshot.CorrectionShortcuts);
        if (snapshot.LanguageShortcutEnabled) shortcuts.AddRange(snapshot.LanguageShortcuts);
        if (snapshot.RawInputShortcutEnabled) shortcuts.AddRange(snapshot.RawInputShortcuts);
        if (snapshot.CursorLeftShortcutEnabled) shortcuts.AddRange(snapshot.CursorLeftShortcuts);
        if (snapshot.CursorRightShortcutEnabled) shortcuts.AddRange(snapshot.CursorRightShortcuts);
        if (snapshot.PreviousPageShortcutEnabled) shortcuts.AddRange(snapshot.PreviousPageShortcuts);
        if (snapshot.NextPageShortcutEnabled) shortcuts.AddRange(snapshot.NextPageShortcuts);
        if (snapshot.ExpandedPreviousPageShortcutEnabled) shortcuts.AddRange(snapshot.ExpandedPreviousPageShortcuts);
        if (snapshot.ExpandedNextPageShortcutEnabled) shortcuts.AddRange(snapshot.ExpandedNextPageShortcuts);
        if (snapshot.TextNavigationShortcutEnabled)
            shortcuts.AddRange(snapshot.TextNavigationShortcuts);
        if (snapshot.PunctuationShortcutEnabled) {
            var punctuation = snapshot.PunctuationShortcuts;
            if (punctuation.Count == 0)
                throw new InvalidOperationException("启用中文标点快捷键前，请先添加至少一种绑定方案。");
            shortcuts.AddRange(punctuation);
        }
        if (shortcuts.Count != shortcuts.Distinct(StringComparer.Ordinal).Count())
            throw new InvalidOperationException("启用的快捷键不能重复。");
    }
}
