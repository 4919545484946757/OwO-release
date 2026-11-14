using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using System.Text;
using System.Text.Json;
using Windows.Storage.Pickers;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private DictionaryManager? _hostDictionaries;
    private readonly Dictionary<(string Scheme, string Version, string Page), SchemeEditor> _schemeEditors = [];
    private string _configurationSchemeId = "full_pinyin";
    private bool _refreshingConfigurationSchemes;
    private InputSchemeOption ConfigurationScheme => _knownSchemes.GetValueOrDefault(_configurationSchemeId)
        ?? InputSchemeSettingsPolicy.Resolve(_configurationSchemeId);
    private bool IsHostConfigurationScope => _configurationSchemeId is "full_pinyin" or "exact_code";
    private bool UsesHostDictionary => _configurationSchemeId == "full_pinyin";
    private bool IsSchemeScopedPage => _currentPage is "candidates" or "intelligence" or "shortcuts" or "user-lexicon";

    private void RefreshConfigurationSchemes() {
        if (SettingsSchemeSelector is null) return;
        var options = _availableSchemeIds.Select(id => _knownSchemes[id]).OrderBy(item => item.OwnerPluginId is null ? 0 : 1)
            .ThenBy(item => item.Id == "full_pinyin" ? 0 : 1).ThenBy(item => item.Name, StringComparer.CurrentCulture).ToArray();
        _refreshingConfigurationSchemes = true;
        try {
            SettingsSchemeSelector.ItemsSource = options;
            SelectConfigurationScheme(options.Any(item => item.Id == _configurationSchemeId) ? _configurationSchemeId : "full_pinyin");
        } finally { _refreshingConfigurationSchemes = false; }
        foreach (var key in _schemeEditors.Keys.ToArray()) {
            var scheme = _knownSchemes.GetValueOrDefault(key.Scheme);
            if (scheme is null || !_availableSchemeIds.Contains(key.Scheme) || !PluginVersionIsCurrent(scheme.OwnerPluginId!, key.Version))
                _schemeEditors.Remove(key);
        }
        if (IsSchemeScopedPage) ShowPage(_currentPage);
        else UpdateConfigurationSchemeBadge();
    }
    private void SelectConfigurationScheme(string id) {
        _configurationSchemeId = id;
        if (SettingsSchemeSelector.ItemsSource is IEnumerable<InputSchemeOption> options)
            SettingsSchemeSelector.SelectedItem = options.FirstOrDefault(item => item.Id == id);
    }
    private void SettingsSchemeSelector_Changed(object sender, SelectionChangedEventArgs args) {
        if (_refreshingConfigurationSchemes || SettingsSchemeSelector.SelectedItem is not InputSchemeOption selected) return;
        _configurationSchemeId = selected.Id;
        CancelShortcutCapture();
        SettingsSearchBox.ItemsSource = null;
        UpdateDependentSettings();
        if (IsSchemeScopedPage || _currentPage == "punctuation")
            ShowPage(_currentPage == "punctuation" && !IsHostConfigurationScope ? "candidates" : _currentPage);
    }
    private bool PluginVersionIsCurrent(string owner, string version) => _installedPlugins.Any(plugin =>
        plugin.Id == owner && plugin.Version == version && plugin.Active && plugin.PermissionsAuthorized && plugin.Permissions.Contains("input.scheme"));

    private void UpdateConfigurationSchemeBadge() {
        if (SettingsPageSchemeBadge is null || SettingsPageSchemeLabel is null) return;
        var visible = IsSchemeScopedPage || _currentPage == "punctuation";
        SettingsPageSchemeBadge.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;
        if (visible) {
            SettingsPageSchemeLabel.Text = $"配置方案：{ConfigurationScheme.Name}";
            ToolTipService.SetToolTip(SettingsPageSchemeBadge, SettingsPageSchemeLabel.Text);
        }
    }

    private void UpdateSchemeScopePage() {
        if (SchemeScopeHost is null || SchemeShortcutHost is null) return;
        var scoped = IsSchemeScopedPage && (_currentPage == "user-lexicon" ? !UsesHostDictionary : !IsHostConfigurationScope);
        SchemeScopeHost.Visibility = scoped && _currentPage != "shortcuts" ? Visibility.Visible : Visibility.Collapsed;
        SchemeShortcutHost.Visibility = scoped && _currentPage == "shortcuts" ? Visibility.Visible : Visibility.Collapsed;
        NativeShortcutHint.Visibility = scoped ? Visibility.Collapsed : Visibility.Visible;
        foreach (var item in _settingsCards.Where(card => _settingsModules.PageFor(card.Key) == "shortcuts" && !ReferenceEquals(card.Card, LanguageShortcutCard)))
            item.Card.Visibility = scoped ? Visibility.Collapsed : Visibility.Visible;
        if (!scoped) return;
        if (_currentPage is "candidates" or "intelligence") GeneralSettingsPage.Visibility = Visibility.Collapsed;
        if (_currentPage == "user-lexicon") {
            UserLexiconPageHost.Visibility = Visibility.Collapsed;
            _hostDictionaries?.CancelDiscoveryForScopeChange();
        }
        var host = _currentPage == "shortcuts" ? SchemeShortcutHost : SchemeScopeHost;
        host.Content = BuildSchemeScopeContent();
        CurrentPageDescription.Text = _currentPage == "shortcuts"
            ? "方案快捷键仅对所选方案生效；切换输入方案仍是全局快捷键。"
            : "此页只显示所选方案的配置，不会切换正在使用的输入方案。";
    }

    private FrameworkElement BuildSchemeScopeContent() {
        var scheme = ConfigurationScheme;
        if (scheme.OwnerPluginId is not { } owner)
            return SchemeNote(_currentPage == "shortcuts" ? "纯净键盘不接管组合快捷键。" : "此方案未声明独立配置或用户词库。", _currentPage != "shortcuts");
        var plugin = _installedPlugins.FirstOrDefault(item => item.Id == owner && item.Active && item.PermissionsAuthorized && item.Permissions.Contains("input.scheme"));
        if (plugin is null) return SchemeNote("方案未启用或未获授权。", _currentPage != "shortcuts");
        var key = (scheme.Id, plugin.Version, _currentPage);
        if (_schemeEditors.TryGetValue(key, out var existing)) return existing.View;
        try {
            var profile = SchemePackageProfile.Load(_pluginClient.StorePath, owner, plugin.Version, scheme.Id);
            if (profile is null) return SchemeNote("此版本未声明此方案的设置。请在插件管理器查看它的通用设置。", _currentPage != "shortcuts");
            var panel = new StackPanel { Spacing = 8, MaxWidth = 960 };
            FrameworkElement view = _currentPage == "shortcuts" ? panel : new ScrollViewer { Content = panel, Padding = new Thickness(32, 0, 32, 24), HorizontalContentAlignment = HorizontalAlignment.Stretch };
            var editor = new SchemeEditor(view, panel);
            if (_currentPage == "shortcuts") BuildSchemeShortcuts(editor, profile, plugin);
            else if (_currentPage == "user-lexicon") BuildSchemeDictionaries(panel, profile, plugin);
            else {
                if (_currentPage == "candidates") BuildSchemePresentation(editor, profile, plugin);
                BuildSchemeFields(editor, _currentPage == "candidates" ? profile.Document.CandidateFields : profile.Document.LearningFields, plugin);
                if (editor.Save is not null) AddSchemeSave(editor);
            }
            _schemeEditors.Add(key, editor);
            return view;
        } catch (Exception error) { return SchemeNote($"方案设置无法加载：{error.Message}", _currentPage != "shortcuts"); }
    }

    private static TextBlock SchemeNote(string text, bool inset = false) => new() {
        Text = text, TextWrapping = TextWrapping.Wrap, Margin = inset ? new Thickness(32, 0, 32, 0) : new Thickness(0)
    };
    private sealed class SchemeEditor(FrameworkElement view, StackPanel panel) {
        internal FrameworkElement View { get; } = view;
        internal StackPanel Panel { get; } = panel;
        internal Button? SaveButton;
        internal Func<Task>? Save;
        internal bool Dirty, Saving;
        internal int Revision;
    }
    private void WatchSchemeEditor(SchemeEditor editor) {
        editor.Dirty = true;
        editor.Revision++;
        if (AutoSaveSettings.IsOn) { _autoSaveTimer.Stop(); _autoSaveTimer.Start(); }
    }
    private void AddSchemeSave(SchemeEditor editor) {
        var button = new Button { Content = "保存此方案", Visibility = AutoSaveSettings.IsOn ? Visibility.Collapsed : Visibility.Visible };
        editor.SaveButton = button;
        button.Click += async (_, _) => await SaveSchemeEditorAsync(editor);
        editor.Panel.Children.Add(button);
    }
    private async Task<bool> SaveSchemeEditorAsync(SchemeEditor editor) {
        if (!editor.Dirty || editor.Saving || editor.Save is null) return true;
        editor.Saving = true;
        var revision = editor.Revision;
        try {
            await editor.Save();
            if (revision == editor.Revision) editor.Dirty = false;
            ShowStatus("方案设置已保存。", InfoBarSeverity.Success);
            return true;
        } catch (Exception error) { ShowStatus($"方案设置未保存：{error.Message}", InfoBarSeverity.Error); return false; }
        finally { editor.Saving = false; }
    }
    private void BuildSchemePresentation(SchemeEditor editor, SchemePackageProfile profile, PluginVersionSnapshot plugin) {
        var defaults = SchemePresentation.Read(profile.PresentationDefaultsPath);
        if (defaults is null) return;
        var value = SchemePresentation.Read(profile.PresentationPath) ?? defaults;
        var preview = new ScrollNeutralComboBox { Header = "编码预览显示位置", ItemsSource = new[] { "候选栏", "输入位置" },
            SelectedIndex = value.Preview == "input" ? 1 : 0 };
        var page = new NumberBox { Header = "每页候选数", Minimum = 1, Maximum = 9, Value = value.PageSize, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
        var wrap = new NumberBox { Header = "长候选换行长度", Minimum = 4, Maximum = 64, Value = value.WrapLength, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
        AutomationProperties.SetAutomationId(preview, "SchemePresentationPreview");
        AutomationProperties.SetAutomationId(page, "SchemePresentationPageSize");
        AutomationProperties.SetAutomationId(wrap, "SchemePresentationWrapLength");
        var dirty = false;
        void Changed() { dirty = true; WatchSchemeEditor(editor); }
        preview.SelectionChanged += (_, _) => Changed();
        page.ValueChanged += (_, _) => Changed();
        wrap.ValueChanged += (_, _) => Changed();
        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(preview); panel.Children.Add(page); panel.Children.Add(wrap);
        panel.Children.Add(SchemeNote("仅作用于当前方案，下次开始输入时生效。目标应用不支持输入位置预览时，会回退到候选栏。"));
        editor.Panel.Children.Add(new Border { Style = (Style)Resources["SettingsCardStyle"], Child = panel });
        editor.Save = () => {
            if (!dirty) return Task.CompletedTask;
            if (!PluginVersionIsCurrent(plugin.Id, plugin.Version)) throw new IOException("方案所属插件版本已变更。");
            var size = checked((int)SettingsModules.UnsignedNumber(page.Value, 1, 9, "每页候选数"));
            var length = checked((int)SettingsModules.UnsignedNumber(wrap.Value, 4, 64, "长候选换行长度"));
            SchemePackageProfile.WriteFile(profile.PresentationPath, new SchemePresentation(preview.SelectedIndex == 1 ? "input" : "candidate", size, length).Encode());
            dirty = false;
            return Task.CompletedTask;
        };
    }
    private void BuildSchemeFields(SchemeEditor editor, IReadOnlyList<string> keys, PluginVersionSnapshot plugin) {
        if (keys.Count == 0) { if (editor.Save is null) editor.Panel.Children.Add(SchemeNote("方案包未声明本页配置项。")); return; }
        SchemePackageProfile.CheckPath(Path.Combine(_pluginClient.StorePath, "versions", plugin.Id, plugin.Version, "manifest.json"));
        var model = PluginSettingsModel.Load(_pluginClient.StorePath, plugin);
        var reads = new Dictionary<string, Func<object?>>(StringComparer.Ordinal);
        var changes = new HashSet<string>(StringComparer.Ordinal);
        void Edited(string key) { changes.Add(key); WatchSchemeEditor(editor); }
        foreach (var key in keys) {
            var field = model.Fields.SingleOrDefault(item => item.Key == key) ?? throw new InvalidDataException($"配置结构没有声明字段 {key}。");
            var value = model.Values[key];
            var group = new StackPanel { Spacing = 4 };
            group.Children.Add(SchemeNote(field.Title));
            if (field.Description.Length != 0) group.Children.Add(SchemeNote(field.Description));
            if (field.Options.Count != 0) {
                var options = new ScrollNeutralComboBox { ItemsSource = field.Options, DisplayMemberPath = nameof(PluginSettingOption.Label),
                    SelectedItem = field.Options.FirstOrDefault(item => item.Value.GetRawText() == value.GetRawText()) ?? field.Options[0] };
                var previous = options.SelectedItem;
                options.SelectionChanged += (_, _) => { if (!Equals(previous, options.SelectedItem)) { previous = options.SelectedItem; Edited(key); } };
                group.Children.Add(options);
                reads.Add(key, () => ((PluginSettingOption)options.SelectedItem).Value);
            } else if (field.Type == "boolean") {
                var toggle = new ToggleSwitch { IsOn = value.ValueKind == JsonValueKind.True, HorizontalAlignment = HorizontalAlignment.Left };
                AutomationProperties.SetName(toggle, field.Title);
                AutomationProperties.SetAutomationId(toggle, $"SchemeField:{key}");
                var previous = toggle.IsOn;
                toggle.Toggled += (_, _) => { if (previous != toggle.IsOn) { previous = toggle.IsOn; Edited(key); } };
                group.Children.Add(toggle); reads.Add(key, () => toggle.IsOn);
            } else if (field.Type is "integer" or "number") {
                var number = new NumberBox { Value = value.ValueKind == JsonValueKind.Number ? value.GetDouble() : 0,
                    Minimum = field.Minimum ?? double.MinValue, Maximum = field.Maximum ?? double.MaxValue, SmallChange = field.Step ?? 1,
                    SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
                var previous = number.Value;
                number.ValueChanged += (_, _) => { if (!previous.Equals(number.Value)) { previous = number.Value; Edited(key); } };
                group.Children.Add(number);
                reads.Add(key, () => !double.IsFinite(number.Value) ? throw new InvalidDataException($"{field.Title}必须为有限数值。")
                    : field.Type == "integer" ? (object)checked((long)Math.Round(number.Value)) : number.Value);
            } else {
                var text = new TextBox { Text = value.ValueKind == JsonValueKind.String ? value.GetString() ?? "" : "", MaxLength = 4096 };
                var previous = text.Text;
                text.TextChanged += (_, _) => { if (previous != text.Text) { previous = text.Text; Edited(key); } };
                group.Children.Add(text); reads.Add(key, () => text.Text);
            }
            editor.Panel.Children.Add(new Border { Style = (Style)Resources["SettingsCardStyle"], Child = group });
        }
        var savePresentation = editor.Save;
        editor.Save = async () => {
            if (savePresentation is not null) await savePresentation();
            if (changes.Count == 0) return;
            if (!PluginVersionIsCurrent(plugin.Id, plugin.Version)) throw new IOException("方案所属插件版本已变更。");
            SchemePackageProfile.CheckPath(model.ConfigPath);
            model.SavePatch(changes.ToDictionary(key => key, key => reads[key]()));
            changes.Clear();
        };
    }

    private void BuildSchemeShortcuts(SchemeEditor editor, SchemePackageProfile profile, PluginVersionSnapshot plugin) {
        if (!profile.HasShortcuts) { editor.Panel.Children.Add(SchemeNote("方案包未声明独立按键，运行时沿用旧版兼容绑定。")); return; }
        string Read(string path) {
            SchemePackageProfile.CheckPath(path);
            using var file = File.OpenRead(path);
            if (file.Length is <= 0 or > 4096) throw new InvalidDataException("方案快捷键文件无效。");
            var bytes = new byte[(int)file.Length]; file.ReadExactly(bytes);
            if (bytes.Any(value => value > 127)) throw new InvalidDataException("方案快捷键必须为 ASCII。");
            return Encoding.ASCII.GetString(bytes);
        }
        Dictionary<string, string> Parse(string text) {
            var lines = text.Replace("\r\n", "\n").TrimEnd('\n').Split('\n');
            if (lines[0] != SchemePackageProfile.ShortcutHeader) throw new InvalidDataException("方案快捷键声明版本无效。");
            var result = new Dictionary<string, string>();
            foreach (var line in lines.Skip(1)) {
                var fields = line.Split('=', 2);
                if (fields.Length != 2 || !SchemePackageProfile.Actions.Contains(fields[0]) || !result.TryAdd(fields[0], fields[1]))
                    throw new InvalidDataException("方案快捷键动作无效或重复。");
            }
            return result;
        }
        var defaultsText = Read(profile.DefaultsPath);
        var defaults = Parse(defaultsText);
        Dictionary<string, string> saved;
        try { saved = File.Exists(profile.ShortcutPath) ? Parse(Read(profile.ShortcutPath)) : []; }
        catch (Exception) { saved = []; }
        editor.Panel.Children.Add(SchemeNote("使用 Ctrl+…、PageUp 等键名；多组用分号隔开，留空即关闭该动作。“切换输入方案”属于 OwO 全局。"));
        var inputs = new Dictionary<string, TextBox>();
        foreach (var (id, keys) in defaults) {
            var input = new TextBox { Header = id switch {
                "raw" => "原样上屏", "cursor_left" => "编码光标左移", "cursor_right" => "编码光标右移",
                "previous_page" => "上一页", "next_page" => "下一页", "expanded_previous_page" => "展开并选中上一候选页", _ => "展开并选中下一候选页"
            }, Text = saved.GetValueOrDefault(id, keys), MaxLength = 256 };
            var previous = input.Text;
            input.TextChanged += (_, _) => { if (previous != input.Text) { previous = input.Text; WatchSchemeEditor(editor); } };
            inputs.Add(id, input); editor.Panel.Children.Add(input);
        }
        if (inputs.Count == 0) return;
        editor.Save = async () => {
            var proposed = SchemePackageProfile.ShortcutHeader + "\n" + string.Concat(inputs.Select(item => $"{item.Key}={item.Value.Text.Trim()}\n"));
            await _client.ValidateSchemeShortcutsAsync(defaultsText, proposed);
            if (!PluginVersionIsCurrent(plugin.Id, plugin.Version)) throw new IOException("方案所属插件版本已变更。");
            SchemePackageProfile.WriteFile(profile.ShortcutPath, Encoding.ASCII.GetBytes(proposed));
        };
        AddSchemeSave(editor);
    }

    private void BuildSchemeDictionaries(StackPanel panel, SchemePackageProfile profile, PluginVersionSnapshot plugin) {
        if (profile.Document.LexiconExtension is not { } extension) { panel.Children.Add(SchemeNote("方案包未声明用户词库格式或在线来源。")); return; }
        panel.Children.Add(SchemeNote(profile.Document.LexiconHelp ?? $"用户词库格式：{extension}。由此方案读取，不会导入全拼词库。"));
        var controls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        bool busy = false;
        async Task Run(Func<Task> operation) {
            if (busy) return;
            busy = true;
            try { if (!PluginVersionIsCurrent(plugin.Id, plugin.Version)) throw new IOException("方案所属插件版本已变更。"); await operation(); }
            catch (Exception error) { ShowStatus(error.Message, InfoBarSeverity.Error); }
            finally { busy = false; }
        }
        var import = new Button { Content = "导入用户词库" }; controls.Children.Add(import);
        import.Click += async (_, _) => await Run(async () => {
            var picker = new FileOpenPicker(); picker.FileTypeFilter.Add(extension); DictionaryManager.InitializePicker(picker);
            var file = await picker.PickSingleFileAsync(); if (file is null) return;
            if (!file.Name.EndsWith(extension, StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("词库扩展名不匹配。");
            if (File.Exists(profile.UserLexiconPath) && !await ConfirmAsync("替换方案用户词库", "仅替换此方案的文件。建议先导出现有词库。", "替换")) return;
            var bytes = await SchemePackageProfile.ReadLexiconAsync(file.Path);
            if (!PluginVersionIsCurrent(plugin.Id, plugin.Version)) throw new IOException("方案所属插件版本已变更。");
            SchemePackageProfile.WriteFile(profile.UserLexiconPath, bytes);
            ShowStatus("方案用户词库已导入，由方案读取。", InfoBarSeverity.Success);
        });
        var export = new Button { Content = "导出用户词库" }; controls.Children.Add(export);
        async Task Export(string path, string filename) {
            if (!File.Exists(path)) { ShowStatus("此方案暂无可导出的词条。", InfoBarSeverity.Informational); return; }
            var picker = new FileSavePicker { SuggestedFileName = filename }; picker.FileTypeChoices.Add("方案词库", [extension]);
            DictionaryManager.InitializePicker(picker); var file = await picker.PickSaveFileAsync(); if (file is null) return;
            var bytes = await SchemePackageProfile.ReadLexiconAsync(path);
            SchemePackageProfile.WriteFile(file.Path, bytes);
            ShowStatus("词库已导出。", InfoBarSeverity.Success);
        }
        async Task Remove(string path, string title, string explanation) {
            SchemePackageProfile.CheckPath(path);
            if (!File.Exists(path)) { ShowStatus("此方案暂无可移除的词库文件。", InfoBarSeverity.Informational); return; }
            if (!await ConfirmAsync(title, explanation, "移除")) return;
            if (!PluginVersionIsCurrent(plugin.Id, plugin.Version)) throw new IOException("方案所属插件版本已变更。");
            SchemePackageProfile.CheckPath(path);
            File.Delete(path);
            ShowStatus("已移除。", InfoBarSeverity.Success);
        }
        export.Click += async (_, _) => await Run(() => Export(profile.UserLexiconPath, "user-lexicon"));
        var clear = new Button { Content = "移除用户词库" }; controls.Children.Add(clear);
        clear.Click += async (_, _) => await Run(() => Remove(profile.UserLexiconPath, "移除方案用户词库",
            "仅移除当前方案的手动导入词库。不会删除学习记录。"));
        panel.Children.Add(controls);
        if (profile.LearnedLexiconPath is { } learned) {
            var learning = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            var exportLearned = new Button { Content = "导出学习记录" };
            var clearLearned = new Button { Content = "清除学习记录" };
            AutomationProperties.SetAutomationId(exportLearned, "SchemeExportLearned");
            AutomationProperties.SetAutomationId(clearLearned, "SchemeClearLearned");
            exportLearned.Click += async (_, _) => await Run(() => Export(learned, "learned"));
            clearLearned.Click += async (_, _) => await Run(() => Remove(learned, "清除方案学习记录",
                "仅删除当前方案的本地学习记录，建议先导出。不会删除手动导入词库。"));
            learning.Children.Add(exportLearned); learning.Children.Add(clearLearned); panel.Children.Add(learning);
        }
        if (profile.Document.Sources.Length == 0) { panel.Children.Add(SchemeNote("方案包未声明可下载词库。")); return; }
        foreach (var source in profile.Document.Sources) {
            var path = profile.DownloadPath(source);
            var title = SchemeNote(source.Title);
            var details = SchemeNote($"许可：{source.License} · {source.Bytes:N0} 字节");
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            var download = new Button();
            AutomationProperties.SetAutomationId(download, $"SchemeDownload:{source.Id}");
            AutomationProperties.SetName(download, $"下载 {source.Title}");
            var remove = new Button { Content = "移除" };
            void Refresh() {
                var installed = File.Exists(path);
                download.Content = installed ? "重新下载" : "下载";
                remove.Visibility = installed ? Visibility.Visible : Visibility.Collapsed;
            }
            Refresh();
            actions.Children.Add(download); actions.Children.Add(remove);
            panel.Children.Add(title); panel.Children.Add(details); panel.Children.Add(actions);
            download.Click += async (_, _) => await Run(async () => {
                if (!await ConfirmAsync("下载方案词库", $"{source.Title}\n{source.License}\n{source.Url}\n仅安装到当前方案。", "下载并校验")) return;
                using var http = new HttpClient(new HttpClientHandler { AllowAutoRedirect = false });
                using var timeout = new CancellationTokenSource(TimeSpan.FromMinutes(2));
                var bytes = await SchemePackageProfile.DownloadAsync(source, http, timeout.Token);
                if (!PluginVersionIsCurrent(plugin.Id, plugin.Version)) throw new IOException("方案所属插件版本已变更。");
                SchemePackageProfile.WriteFile(path, bytes);
                Refresh();
                ShowStatus("词库校验通过，已保存到当前方案。", InfoBarSeverity.Success);
            });
            remove.Click += async (_, _) => await Run(async () => {
                await Remove(path, "移除已下载词库", $"仅移除当前方案的 {source.Title}，可再次下载。");
                Refresh();
            });
        }
    }
}
