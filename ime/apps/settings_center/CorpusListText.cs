namespace OwO_Settings;

// Executable allow/exclude lists are edited as one semicolon-separated line.
internal static class CorpusListText
{
    internal static IReadOnlyList<string> Split(string? value) =>
        (value ?? string.Empty).Split(';', StringSplitOptions.RemoveEmptyEntries |
                                              StringSplitOptions.TrimEntries);
}
