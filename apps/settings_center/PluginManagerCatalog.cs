namespace OwO_Settings;

// Pure presentation policy: never grants permissions or changes plugin state.
internal static class PluginManagerCatalog
{
    internal static int CompareVersions(string left, string right) {
        var l = left.Split('+', 2)[0].Split('-', 2); var r = right.Split('+', 2)[0].Split('-', 2);
        if (Version.TryParse(l[0], out var lv) && Version.TryParse(r[0], out var rv)) {
            var order = lv.CompareTo(rv); if (order != 0) return order;
            if (l.Length != r.Length) return l.Length == 1 ? 1 : -1;
            if (l.Length == 1) return 0;
            var lp = l[1].Split('.'); var rp = r[1].Split('.');
            for (var i = 0; i < Math.Min(lp.Length, rp.Length); i++) {
                var ln = lp[i].Length > 0 && lp[i].All(char.IsAsciiDigit);
                var rn = rp[i].Length > 0 && rp[i].All(char.IsAsciiDigit);
                if (ln != rn) return ln ? -1 : 1;
                var part = ln ? lp[i].Length.CompareTo(rp[i].Length) : 0;
                if (part == 0) part = StringComparer.Ordinal.Compare(lp[i], rp[i]);
                if (part != 0) return part;
            }
            return lp.Length.CompareTo(rp.Length);
        }
        return StringComparer.OrdinalIgnoreCase.Compare(left, right);
    }
    internal static PluginVersionSnapshot? Latest(IEnumerable<PluginVersionSnapshot> values) =>
        values.OrderByDescending(p => p.Version, Comparer<string>.Create(CompareVersions)).FirstOrDefault();

    internal static IReadOnlyList<PluginVersionSnapshot> Filter(IEnumerable<PluginVersionSnapshot> source,
        string query, string filter, bool history) {
        var all = source.ToList();
        IEnumerable<PluginVersionSnapshot> selected = history ? all : all.GroupBy(p => p.Id, StringComparer.Ordinal)
            .Select(g => g.FirstOrDefault(p => p.Active) ?? Latest(g)!);
        query = query.Trim();
        return selected.Where(p => filter switch {
            "enabled" => p.Active, "disabled" => !p.Active, "unauthorized" => !p.PermissionsAuthorized, _ => true
        }).Where(p => query.Length == 0 || $"{p.Name} {p.Id} {p.Version} {p.Detail} {string.Join(' ', p.Permissions)}".Contains(query, StringComparison.OrdinalIgnoreCase))
            .OrderByDescending(p => p.Active).ThenBy(p => p.Name, StringComparer.CurrentCulture)
            .ThenByDescending(p => p.Version, Comparer<string>.Create(CompareVersions)).ToList();
    }

    internal static string OnlineState(OnlineResource resource, IEnumerable<PluginVersionSnapshot> installed) {
        var versions = installed.Where(p => p.Id == resource.Id).ToList();
        var exact = versions.FirstOrDefault(p => p.Version == resource.Version);
        if (exact is not null) return exact.Active ? "已安装 · 已启用" : "已安装 · 未启用";
        var latest = Latest(versions);
        if (latest is null) return "可安装";
        return CompareVersions(resource.Version, latest.Version) > 0 ? "可更新" : "本机版本较新";
    }
}
