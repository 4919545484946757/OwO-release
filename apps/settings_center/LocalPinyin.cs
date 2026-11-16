namespace OwO_Settings;

internal static class LocalPinyin {
    private static readonly Lazy<HashSet<string>> Syllables = new(() => {
        using var input = typeof(LocalPinyin).Assembly.GetManifestResourceStream("OwO.FullPinyinSyllables")
            ?? throw new InvalidDataException("Missing shared full-pinyin syllables.");
        using var reader = new StreamReader(input);
        return reader.ReadToEnd().Split('"').Where((_, index) => index % 2 != 0).ToHashSet(StringComparer.Ordinal);
    });

    // Annotate only an unambiguous, shortest cover of complete syllables.
    // Do not force a length for shorthand, incomplete spellings, "xian"-like
    // single syllables, or several equally short segmentations.
    internal static (string Spaced, int Count) Describe(string pinyin) {
        var words = new List<string>();
        foreach (var part in pinyin.ToLowerInvariant().Split('\'')) {
            if (part.Length == 0) return (pinyin, 0);
            var best = new int[part.Length + 1];
            Array.Fill(best, part.Length + 1); best[^1] = 0;
            var next = new int[best.Length];
            var ways = new int[best.Length]; ways[^1] = 1;
            for (int i = part.Length - 1; i >= 0; --i) {
                for (int length = 1; length <= 6 && i + length <= part.Length; ++length) {
                    var word = part.Substring(i, length);
                    if (ways[i + length] == 0 || word is "m" or "n" or "ng" or "hm" or "hng" ||
                        !Syllables.Value.Contains(word)) continue;
                    var steps = 1 + best[i + length];
                    if (steps < best[i]) { best[i] = steps; next[i] = length; ways[i] = ways[i + length]; }
                    else if (steps == best[i]) ways[i] = 2;
                }
            }
            if (ways[0] != 1) return (pinyin, 0);
            for (int i = 0; i < part.Length; i += next[i]) words.Add(part.Substring(i, next[i]));
        }
        return (string.Join(' ', words), words.Count is >= 2 and <= 24 ? words.Count : 0);
    }
}
