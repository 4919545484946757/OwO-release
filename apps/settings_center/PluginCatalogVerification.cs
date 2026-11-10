using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json;

namespace OwO_Settings;

public sealed partial class MainPage
{
    // Opt-in real WinUI regression; only uniquely named disposable fixture plugins.
    internal async Task VerifyPluginCatalogAsync()
    {
        var root = Path.GetDirectoryName(_client.ConfigPath)!;
        if (!Path.GetFileName(Path.GetDirectoryName(root)!).StartsWith("plugin-catalog-ui-", StringComparison.Ordinal) ||
            !Path.GetFullPath(_pluginClient.StorePath).StartsWith(Path.GetDirectoryName(root)! + Path.DirectorySeparatorChar,
                StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("Isolated catalog fixture required.");
        var deadline = DateTime.UtcNow.AddSeconds(20);
        while ((!_settingsLoaded || _settingsBusy || _installedPlugins.Count < 2) && DateTime.UtcNow < deadline)
            await Task.Delay(100);
        if (_installedPlugins.Count != 2 || _installedPlugins.Select(p => p.Id).Distinct().Count() != 1 ||
            !_installedPlugins.All(p => p.Id.StartsWith("org.owo.fixture.catalog-", StringComparison.Ordinal)))
            throw new InvalidOperationException("Expected two versions of one disposable fixture plugin.");
        var checks = new List<string>();
        void Check(bool value, string label) {
            if (!value) throw new InvalidOperationException(label);
            checks.Add(label);
        }
        void Aligned(int expected) => Check(_visiblePlugins.Count == expected && _pluginPages.Count == expected &&
            _visiblePlugins.All(p => _pluginPages.ContainsKey($"plugin:{p.Id}:{p.Version}")), "cards/navigation aligned: " + expected);
        Dictionary<string, object?> Values(PluginSettingsModel model) => model.Values.ToDictionary(p => p.Key, p => (object?)p.Value);
        var old = _installedPlugins.Single(p => !p.Active);
        var latest = _installedPlugins.Single(p => p.Active);
        Aligned(1);
        Check(_pluginPages.Values.Single().Version == latest.Version, "default navigation selects active version");
        PluginHistory.IsChecked = true;
        Aligned(2);
        Check(_pluginPagesGroup!.MenuItems.OfType<NavigationViewItem>().All(n =>
            (n.Content is TextBlock text ? text.Text : n.Content.ToString()!).Contains("1.")),
            "navigation distinguishes exact versions");
        SelectPluginNavigation($"plugin:{old.Id}:{old.Version}");
        await ShowPluginSettingsAsync(old);
        var editor = _pluginSettings!;
        var firstControl = _pluginConfigControls.Values.First();
        await LoadPluginsAsync();
        Check(ReferenceEquals(editor, _pluginSettings) && ReferenceEquals(firstControl, _pluginConfigControls.Values.First()),
            "unchanged refresh preserves editor and controls");
        SettingsNavigation.SelectedItem = PluginNavigationItem;
        ShowPage("plugins");
        await Task.Delay(150);
        PluginSearch.Text = "no-such-plugin";
        await Task.Delay(100); // WinUI dispatches TextChanged after the Text assignment.
        Check(_visiblePlugins.Count == 0 && _pluginPages.Count == 2,
            $"local search does not discard global navigation (cards={_visiblePlugins.Count}, navigation={_pluginPages.Count})");
        PluginSearch.Text = "";
        await Task.Delay(100);
        SelectPluginNavigation($"plugin:{old.Id}:{old.Version}");
        await ShowPluginSettingsAsync(old);
        PluginHistory.IsChecked = false;
        Aligned(1);
        Check(_currentPage == "plugins" && _pluginSettings is null && _pluginConfigControls.Count == 0,
            "hiding historical version closes its editor");
        PluginHistory.IsChecked = true;
        SelectPluginNavigation($"plugin:{old.Id}:{old.Version}");
        await ShowPluginSettingsAsync(old);
        editor = _pluginSettings!;
        editor.Save(Values(editor));
        var retained = File.ReadAllText(editor.ConfigPath);
        _pendingPluginSaves[editor] = Values(editor);
        await _pluginClient.UninstallAsync(old.Id, old.Version);
        await LoadPluginsAsync();
        Aligned(1);
        Check(_pluginSettings is null && _pendingPluginSaves.Count == 0 && _currentPage == "plugins",
            "removed version closes editor and discards pending write");
        Check(!_pluginSearchItems.Any(p => p.Page == $"plugin:{old.Id}:{old.Version}"), "removed version leaves no searchable configuration");
        await NavigateToSearchItemAsync(new SearchItem("stale suggestion", "", $"plugin:{old.Id}:{old.Version}"));
        Check(_currentPage == "plugins" && _pluginSettings is null, "stale search suggestion returns to manager");
        await FlushAutoSaveAsync();
        Check(File.ReadAllText(editor.ConfigPath) == retained, "uninstall preserves personal configuration");
        SelectPluginNavigation($"plugin:{latest.Id}:{latest.Version}");
        await ShowPluginSettingsAsync(latest);
        await _pluginClient.DeactivateAsync(latest.Id, latest.Version);
        await LoadPluginsAsync();
        Check(_pluginPages.Values.Single().Active == false && _visiblePlugins.Single().Active == false &&
            _pluginSettings is not null, "stopped but installed version remains configurable");
        _pendingPluginSaves[_pluginSettings!] = Values(_pluginSettings!);
        await _pluginClient.UninstallAsync(latest.Id, latest.Version);
        await LoadPluginsAsync();
        Aligned(0);
        Check(_pluginPagesGroup is null && _pluginSettings is null && _pendingPluginSaves.Count == 0 &&
            PluginConfigFields.Children.Count == 0 && PluginConfigPage.Visibility == Visibility.Collapsed && _currentPage == "plugins",
            "last uninstall removes navigation, editor, fields and pending writes");
        File.WriteAllText(Path.Combine(root, "plugin-catalog-report.json"), JsonSerializer.Serialize(checks));
    }
}
