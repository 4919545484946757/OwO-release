using OwO_Settings;

var checks = 0;
void Check(bool value) { checks++; if (!value) throw new Exception($"Plugin manager policy assertion {checks} failed."); }
var active = new PluginVersionSnapshot("org.example.a", "主题插件", "1.9.0", true, "unverified_package", "low", true, ["ui.candidate_theme"]);
var next = active with { Version = "1.10.0", Active = false, PermissionsAuthorized = false };
var other = active with { Id = "org.example.b", Name = "符号插件", Version = "2.0.0", Active = false, Permissions = ["ui.unicode_palette"] };
var installed = new[] { active, next, other };
Check(PluginManagerCatalog.CompareVersions("1.10.0", "1.9.0") > 0);
Check(PluginManagerCatalog.CompareVersions("1.0.0", "1.0.0-rc.1") > 0);
Check(PluginManagerCatalog.CompareVersions("1.0.0-rc.10", "1.0.0-rc.2") > 0);
Check(PluginManagerCatalog.CompareVersions("1.0.0+build.2", "1.0.0+build.1") == 0);
Check(PluginManagerCatalog.Latest(installed.Where(p => p.Id == active.Id)) == next);
Check(PluginManagerCatalog.Filter(installed, "", "all", false).Count == 2);
Check(PluginManagerCatalog.Filter(installed, "", "enabled", false).Single() == active);
Check(PluginManagerCatalog.Filter(installed, "", "unauthorized", true).Single() == next);
Check(PluginManagerCatalog.Filter(installed, "符号", "all", true).Single() == other);
Check(PluginManagerCatalog.Filter(installed, "theme", "all", true).Count == 2);
Check(PluginManagerCatalog.Filter(installed, "missing", "all", true).Count == 0);
foreach (var history in new[] { false, true }) {
    Check(PluginManagerCatalog.DisplayVersions(installed, history)
        .SequenceEqual(PluginManagerCatalog.Filter(installed, "", "all", history)));
}
Check(PluginManagerCatalog.DisplayVersions(installed, false).Contains(active));
Check(!PluginManagerCatalog.DisplayVersions(installed, false).Contains(next));
Check(PluginManagerCatalog.DisplayVersions(installed, true).Contains(next));
var stopped = installed.Select(p => p with { Active = false }).ToArray();
Check(PluginManagerCatalog.DisplayVersions(stopped, false).Single(p => p.Id == active.Id).Version == next.Version);
Check(PluginManagerCatalog.DisplayVersions([], false).Count == 0);
Check(!PluginManagerCatalog.ContainsVersion(installed.Where(p => p.Id != active.Id), active));
Check(!PluginManagerCatalog.ContainsVersion(installed.Where(p => p.Version != active.Version), active));
Check(PluginManagerCatalog.ContainsVersion(stopped, active)); // Stopping is not uninstalling.
Check(PluginManagerCatalog.DisplayVersions(installed, false).Count == 2 &&
    PluginManagerCatalog.Filter(installed, "missing", "all", false).Count == 0); // Local search does not hide global settings.
var asset = new OnlineAsset("theme.owopkg", new("https://github.com/4919545484946757/OwO-release/releases/download/v1.0.0/theme.owopkg"), 1, new('0', 64));
var resource = new OnlineResource(active.Id, "主题", "plugin", "1.9.0", "GPL-3.0-only", asset);
Check(PluginManagerCatalog.OnlineState(resource, installed) == "已安装 · 已启用");
Check(PluginManagerCatalog.OnlineState(resource with { Version = "1.10.0" }, installed) == "已安装 · 未启用");
Check(PluginManagerCatalog.OnlineState(resource with { Version = "1.11.0" }, installed) == "可更新");
Check(PluginManagerCatalog.OnlineState(resource with { Version = "1.8.0" }, installed) == "本机版本较新");
Check(PluginManagerCatalog.OnlineState(resource with { Id = "org.example.new" }, installed) == "可安装");
Console.WriteLine($"Plugin manager: {checks} checks passed (shared navigation/card versions, search, history, stop/remove and update states). No plugin or network mutations.");
