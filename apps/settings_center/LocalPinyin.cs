namespace OwO_Settings;

internal static class LocalPinyin {
    private static readonly Lazy<HashSet<string>> Syllables = new(() => {
        using var input = typeof(LocalPinyin).Assembly.GetManifestResourceStream("OwO.FullPinyinSyllables")
            ?? throw new InvalidDataException("Missing shared full-pinyin syllables.");
        using var reader = new StreamReader(input);
        return reader.ReadToEnd().Split('"').Where((_, index) => index % 2 != 0).ToHashSet(StringComparer.Ordinal);
    });
    private static readonly Lazy<Dictionary<string, string>> Readings = new(() => {
        using var input = typeof(LocalPinyin).Assembly.GetManifestResourceStream("OwO.ZeroPronunciations")
            ?? throw new InvalidDataException("Missing zero candidate pronunciation index.");
        using var reader = new StreamReader(input);
        var entries = new Dictionary<string, string>(StringComparer.Ordinal);
        while (reader.ReadLine() is { } line) {
            if (line.Length == 0 || line[0] == '#') continue;
            var delimiter = line.IndexOf(' ');
            entries.Add(line[..delimiter], line[(delimiter + 1)..]);
        }
        return entries;
    });

    private static string NormalizeSyllable(string word) => word switch { "lue" => "lve", "nue" => "nve", _ => word };
    private static bool CompleteSyllable(string word) => word is not ("m" or "n" or "ng" or "hm" or "hng") &&
        Syllables.Value.Contains(word);

    // An exact input can have several segmentations (e.g. xian / xi'an).
    // Accept any full reading of the proposed text, without crossing a
    // manually typed apostrophe or allowing one character to cover two syllables.
    private static int AdvanceSeparator(string spelling, int end) =>
        end < spelling.Length && spelling[end] == '\'' ? end + 1 : end;

    internal static bool CanVerify(string raw) {
        var spelling = raw.ToLowerInvariant();
        if (spelling.Length == 0 || spelling[0] == '\'' || spelling.Contains("''", StringComparison.Ordinal)) return false;
        var ends = new bool[spelling.Length + 1]; ends[0] = true;
        for (int i = 0; i < spelling.Length; ++i) {
            if (!ends[i]) continue;
            for (int length = 1; length <= 6 && i + length <= spelling.Length; ++length) {
                if (spelling[i + length - 1] == '\'') break;
                if (CompleteSyllable(NormalizeSyllable(spelling.Substring(i, length))))
                    ends[AdvanceSeparator(spelling, i + length)] = true;
            }
        }
        return ends[^1];
    }

    internal static bool Matches(string raw, string candidate) {
        var spelling = raw.ToLowerInvariant();
        var current = new bool[spelling.Length + 1]; current[0] = true;
        var next = new bool[current.Length];
        foreach (var character in candidate) {
            Array.Clear(next);
            var found = false;
            for (int i = 0; i < spelling.Length; ++i) {
                if (!current[i]) continue;
                for (int length = 1; length <= 6 && i + length <= spelling.Length; ++length) {
                    if (spelling[i + length - 1] == '\'') break;
                    var word = NormalizeSyllable(spelling.Substring(i, length));
                    if (!CompleteSyllable(word) || !Readings.Value.TryGetValue(word, out var chars) || !chars.Contains(character)) continue;
                    next[AdvanceSeparator(spelling, i + length)] = true;
                    found = true;
                }
            }
            if (!found) return false;
            (current, next) = (next, current);
        }
        return candidate.Length > 0 && current[^1];
    }

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
                    var word = NormalizeSyllable(part.Substring(i, length));
                    if (ways[i + length] == 0 || !CompleteSyllable(word)) continue;
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
