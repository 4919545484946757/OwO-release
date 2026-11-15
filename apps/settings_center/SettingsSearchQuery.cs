using System.Text;

namespace OwO_Settings;

internal enum SearchArea { Page, Setting, Shortcut, Plugin }
internal sealed record SearchTerm(string Title, string Keywords, string PageTitle, SearchArea Area);

internal static class SettingsSearchQuery
{
    private sealed record QueryWord(string Text, string[] Mapped);
    private static readonly string[][] Aliases = [
        ["翻页", "分页", "切页", "上一页", "下一页", "paging", "fanye"],
        ["快捷键", "热键", "按键绑定", "keyboard shortcut", "hotkey", "kuaijiejian"],
        ["词库", "字典", "词表", "dictionary", "lexicon", "ciku"],
        ["学习", "词频", "记忆", "learning", "frequency", "cipin"],
        ["模型", "大模型", "AI", "LLM", "model", "moxing"],
        ["拼音", "全拼", "pinyin"],
        ["输入方案", "输入模式", "切换语言", "input scheme", "shurufangan"],
        ["候选", "选词", "候选栏", "candidate", "houxuan"],
        ["符号", "标点", "punctuation", "biaodian"],
        ["插件", "扩展", "plugin", "extension", "chajian"],
        ["自动保存", "自动存档", "autosave", "zidongbaocun"],
        ["通用设置", "常规设置", "general", "preferences", "tongyong"],
        ["调试", "性能", "测速", "延迟", "benchmark", "timing", "diaoshi"],
        ["纠错", "拼写修正", "correction", "jiucuo"]
    ];
    private static readonly (string[] Names, SearchArea Area)[] Scopes = [
        (["page", "页面"], SearchArea.Page), (["setting", "设置"], SearchArea.Setting),
        (["key", "shortcut", "快捷键"], SearchArea.Shortcut), (["plugin", "插件"], SearchArea.Plugin)
    ];
    private static readonly char[] WordSeparators = [' ', '\t', '·', '/', '.', ',', '，', ';', '；', ':', '：'];
    private static readonly string[][] NormalAliases = Aliases.Select(row => row.Select(Normalize).Distinct().ToArray()).ToArray();

    internal static string Namespace(SearchArea area) => area switch {
        SearchArea.Page => "page", SearchArea.Shortcut => "key", SearchArea.Plugin => "plugin", _ => "setting"
    };
    private static string Normalize(string value) => new(value.Normalize(NormalizationForm.FormKC)
        .ToLowerInvariant().Where(ch => char.IsLetterOrDigit(ch) || "-=[]".Contains(ch)).ToArray());

    internal static IReadOnlyList<T> Find<T>(IEnumerable<T> items, string raw, Func<T, SearchTerm> index, int limit = 12)
    {
        var query = raw.Trim().Normalize(NormalizationForm.FormKC);
        if (query.Length == 0 || limit <= 0) return [];
        if (query.Length > 128) query = query[..(char.IsHighSurrogate(query[127]) ? 127 : 128)];
        SearchArea? scope = null;
        var colon = query.IndexOf(':');
        if (colon > 0) {
            var prefix = query[..colon].Trim(' ', '`');
            foreach (var entry in Scopes) if (entry.Names.Contains(prefix, StringComparer.OrdinalIgnoreCase)) {
                scope = entry.Area;
                query = query[(colon + 1)..].TrimStart(':', ' ');
                break;
            }
        }
        var words = query.Split(WordSeparators, StringSplitOptions.RemoveEmptyEntries)
            .Select(Normalize).Where(value => value.Length != 0).Distinct().Take(8)
            .Select(word => new QueryWord(word, NormalAliases
                .Where(row => row.Contains(word) || row.Any(alias => IsNear(word, alias)))
                .SelectMany(row => row).Where(alias => alias.Length >= 2).Distinct().ToArray())).ToArray();
        var plain = Normalize(query);
        if (words.Length == 0 && scope is null) return [];
        var groups = items.Select((item, order) => (Item: item, Term: index(item), Order: order))
            .Where(row => scope is null || row.Term.Area == scope)
            .Select(row => (row.Item, row.Term.Area, row.Order, Score: Rank(row.Term, words, plain)))
            .Where(row => row.Score >= 0).GroupBy(row => row.Area)
            .OrderByDescending(group => group.Max(row => row.Score)).ThenBy(group => group.Key).ToArray();
        return groups.SelectMany(group => group.OrderByDescending(row => row.Score).ThenBy(row => row.Order)
                .Take(groups.Length > 1 ? Math.Min(limit, 5) : limit))
            .Take(limit).Select(row => row.Item).ToArray();
    }

    private static int Rank(SearchTerm term, QueryWord[] words, string query)
    {
        if (words.Length == 0) return 0;
        var title = Normalize(term.Title);
        var page = Normalize(term.PageTitle);
        var description = Normalize(term.Keywords);
        var candidates = term.Keywords.Split(WordSeparators, StringSplitOptions.RemoveEmptyEntries)
            .Take(32).Select(Normalize).Append(title).Append(page).ToArray();
        var score = term.Area == SearchArea.Page ? 12 : 0;
        foreach (var (word, aliases) in words) {
            var direct = title == word ? 130 : title.StartsWith(word, StringComparison.Ordinal) ? 100 :
                title.Contains(word, StringComparison.Ordinal) ? 85 :
                page == word ? 72 : page.Contains(word, StringComparison.Ordinal) ? 63 :
                description.Contains(word, StringComparison.Ordinal) ? 55 : 0;
            if (direct != 0) { score += direct; continue; }
            var mapped = aliases.Any(alias => title.Contains(alias, StringComparison.Ordinal) ||
                page.Contains(alias, StringComparison.Ordinal) || description.Contains(alias, StringComparison.Ordinal));
            if (mapped) { score += 36; continue; }
            if (candidates.Any(candidate => IsNear(word, candidate))) { score += 24; continue; }
            return -1;
        }
        return score + (title == query ? 36 : title.Contains(query, StringComparison.Ordinal) ? 14 : 0);
    }

    private static bool IsNear(string query, string target)
    {
        // Limited typo matching (including one adjacent transposition). Short
        // Chinese labels use explicit aliases instead of broad edit distance.
        if (query.Length < 3 || query.Length > 24 || target.Length > 26 ||
            !query.All(ch => char.IsAsciiLetterOrDigit(ch)) || !target.All(ch => char.IsAsciiLetterOrDigit(ch))) return false;
        var budget = query.Length >= 6 ? 2 : 1;
        if (Math.Abs(query.Length - target.Length) > budget) return false;
        Span<int> earlier = stackalloc int[27];
        Span<int> previous = stackalloc int[27];
        Span<int> current = stackalloc int[27];
        for (var j = 0; j <= target.Length; ++j) previous[j] = j;
        for (var i = 1; i <= query.Length; ++i) {
            current[0] = i;
            for (var j = 1; j <= target.Length; ++j) {
                current[j] = Math.Min(Math.Min(previous[j] + 1, current[j - 1] + 1),
                    previous[j - 1] + (query[i - 1] == target[j - 1] ? 0 : 1));
                if (i > 1 && j > 1 && query[i - 1] == target[j - 2] && query[i - 2] == target[j - 1])
                    current[j] = Math.Min(current[j], earlier[j - 2] + 1);
            }
            previous.CopyTo(earlier);
            current.CopyTo(previous);
        }
        return previous[target.Length] <= budget;
    }
}
