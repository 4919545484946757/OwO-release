using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Windows.Storage.Pickers;

namespace OwO_Settings;

internal sealed class DictionaryManager : UserControl
{
    private readonly DictionaryStore _store = new();
    private DictionaryCatalog _catalog = new(1, 0, []);
    private readonly ContentControl _page = new() { HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private readonly TextBox _search = new() { PlaceholderText = "搜索字典名称、格式或来源" };
    private readonly Grid _cards = new() { ColumnSpacing = 12, RowSpacing = 12 };
    private readonly InfoBar _notice = new() { IsOpen = false, IsClosable = true };
    private readonly Button _import = new() { Content = "导入字典" }, _refresh = new() { Content = "刷新" };
    private readonly Button _restore = new() { Content = "恢复内置词库", Visibility = Visibility.Collapsed };
    private readonly Grid _catalogPage = new() { Margin = new Thickness(16, 0, 16, 16), RowSpacing = 12 };
    private readonly Dictionary<string, CardView> _cardViews = new(StringComparer.Ordinal);
    private readonly List<UIElement> _desiredCards = [];
    private readonly TextBlock _empty = new() { Text = "没有匹配的字典。", Margin = new Thickness(8) };
    private sealed class CardView(Grid host, Button card, ToggleSwitch toggle, TextBlock title, TextBlock description, TextBlock state)
    {
        internal Grid Host { get; } = host;
        internal Button Card { get; } = card;
        internal ToggleSwitch Toggle { get; } = toggle;
        internal TextBlock Title { get; } = title;
        internal TextBlock Description { get; } = description;
        internal TextBlock State { get; } = state;
        internal Button Menu { get; set; } = null!;
        internal Func<Task> Open = () => Task.CompletedTask, Export = () => Task.CompletedTask,
            Delete = () => Task.CompletedTask, Rename = () => Task.CompletedTask;
        internal Func<bool, Task> SetEnabled = _ => Task.CompletedTask;
        internal bool Updating;
    }
    private bool _busy;
    internal DictionaryManager() {
        _catalogPage.RowDefinitions.Add(new() { Height = GridLength.Auto });
        _catalogPage.RowDefinitions.Add(new() { Height = GridLength.Auto });
        _catalogPage.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var toolbar = new Grid { ColumnSpacing = 8, RowSpacing = 8 };
        toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto }); toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.Children.Add(_search); Grid.SetColumnSpan(_search, 4);
        Grid.SetRow(_import, 1); _import.HorizontalAlignment = HorizontalAlignment.Left; toolbar.Children.Add(_import);
        Grid.SetRow(_refresh, 1); Grid.SetColumn(_refresh, 2); toolbar.Children.Add(_refresh);
        Grid.SetRow(_restore, 1); Grid.SetColumn(_restore, 3); toolbar.Children.Add(_restore);
        _catalogPage.Children.Add(toolbar); Grid.SetRow(_notice, 1); _catalogPage.Children.Add(_notice);
        var scroll = new ScrollViewer { Content = _cards, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        scroll.LayoutUpdated += (_, _) => {
            // Fixed grid columns otherwise preserve their old desired width
            // when the window shrinks; UIA sees only a clipped card fragment.
            var width = scroll.ViewportWidth;
            if (width > 0 && double.IsFinite(width) &&
                (double.IsNaN(_cards.Width) || Math.Abs(_cards.Width - width) > 0.5)) {
                _cards.Width = width; LayoutCards();
            }
        };
        Grid.SetRow(scroll, 2); _catalogPage.Children.Add(scroll);
        _cards.SizeChanged += (_, _) => LayoutCards();
        _search.TextChanged += (_, _) => RenderCards();
        _import.Click += async (_, _) => await RunAsync(ImportAsync);
        _refresh.Click += async (_, _) => await RefreshAsync();
        AutomationProperties.SetAutomationId(_restore, "DictionaryRestoreBuiltins");
        AutomationProperties.SetAutomationId(_search, "DictionarySearch");
        AutomationProperties.SetAutomationId(_import, "DictionaryImport");
        AutomationProperties.SetAutomationId(_refresh, "DictionaryRefresh");
        _page.Content = _catalogPage; Content = _page;
    }
    internal async Task RefreshAsync() => await RunAsync(async () => {
        _catalog = await _store.LoadAsync();
        if (!ReferenceEquals(_page.Content, _catalogPage)) _page.Content = _catalogPage;
        RenderCards();
    });
    private async Task RunAsync(Func<Task> action) {
        // Serialize actions without a disabled/enabled pulse on the toolbar;
        // even a fast snapshot read otherwise retriggers WinUI color transitions.
        if (_busy) return; _busy = true;
        try { await action(); }
        catch (Exception error) { _notice.Message = error.Message; _notice.Severity = InfoBarSeverity.Error; _notice.IsOpen = true; }
        finally { _busy = false; }
    }
    private void RenderCards() {
        _desiredCards.Clear();
        var query = _search.Text.Trim();
        bool Match(params string[] values) => values.Any(value => value.Contains(query, StringComparison.OrdinalIgnoreCase));
        var restore = new MenuFlyout();
        foreach (var builtin in _catalog.Builtins) {
            if (builtin.Deleted) {
                var item = new MenuFlyoutItem { Text = builtin.Name };
                AutomationProperties.SetAutomationId(item, "DictionaryRestore_" + builtin.Id);
                item.Click += async (_, _) => await RunAsync(async () => {
                    await SaveBuiltinAsync(builtin with { Deleted = false, Enabled = false }); RenderCards();
                });
                restore.Items.Add(item); continue;
            }
            if (!Match(builtin.Name, builtin.Id == "user" ? "用户词库 学习 宿主" : "基础中文词典 内置 只读")) continue;
            AddCard(builtin.Name, builtin.Id == "user" ? "本机学习与手动添加的词语" : "随 OwO 安装的只读词典",
                builtin.Id == "user" ? "新增 · 编辑 · 导入/出 · 清除学习" : "紧凑只读 · 随软件维护", builtin.Id,
                () => OpenBuiltinAsync(builtin), builtin.Enabled,
                async enabled => await SaveBuiltinAsync(builtin with { Enabled = enabled }),
                async () => {
                    var exported = builtin.Id == "user" ? await DictionaryActions.ExportUserAsync(builtin.Name) : await DictionaryActions.ExportBaseAsync(builtin.Name);
                    if (exported) Notify("已导出字典；请妥善保管，并在分发基础词典时遵守原词典许可。");
                },
                async () => { if (await DictionaryActions.ConfirmDeleteAsync(XamlRoot, builtin.Name, true)) await SaveBuiltinAsync(builtin with { Enabled = false, Deleted = true }); },
                async () => { if (await DictionaryActions.RenameAsync(XamlRoot, builtin.Name) is { } name) await SaveBuiltinAsync(builtin with { Name = name }); });
        }
        _restore.Flyout = restore; _restore.Visibility = restore.Items.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        foreach (var dictionary in _catalog.Dictionaries.Where(value => Match(value.Name, value.Format, value.SourceName))) {
            var pending = dictionary.Entries.Count(word => !DictionaryStore.Usable(word));
            AddCard(dictionary.Name, $"{dictionary.Format} · {dictionary.Entries.Count} 条", (dictionary.Enabled ? "已启用" : "已停用") + (pending > 0 ? $" · {pending} 条待补编码" : ""), dictionary.Id,
                async () => { OpenDictionary(dictionary); await Task.CompletedTask; }, dictionary.Enabled,
                async enabled => await SaveDictionaryAsync(dictionary with { Enabled = enabled }),
                async () => { if (await DictionaryActions.ExportAsync(dictionary)) Notify("已导出。本文件包含个人词语，请妥善保管。"); },
                async () => { if (await DictionaryActions.ConfirmDeleteAsync(XamlRoot, dictionary.Name)) await DeleteDictionaryAsync(dictionary.Id); },
                async () => { if (await DictionaryActions.RenameAsync(XamlRoot, dictionary.Name) is { } name) await SaveDictionaryAsync(dictionary with { Name = name }); });
        }
        if (_desiredCards.Count == 0) _desiredCards.Add(_empty);
        // Keep unchanged cards attached to the visual tree: resetting Children
        // retriggers transitions and loses keyboard focus, hover and UIA identity.
        for (var index = _cards.Children.Count - 1; index >= 0; --index)
            if (!_desiredCards.Contains(_cards.Children[index])) _cards.Children.RemoveAt(index);
        for (var index = 0; index < _desiredCards.Count; ++index) {
            var card = _desiredCards[index];
            if (index < _cards.Children.Count && ReferenceEquals(_cards.Children[index], card)) continue;
            var oldIndex = _cards.Children.IndexOf(card);
            if (oldIndex >= 0) _cards.Children.RemoveAt(oldIndex);
            _cards.Children.Insert(index, card);
        }
        while (_cards.Children.Count > _desiredCards.Count) _cards.Children.RemoveAt(_cards.Children.Count - 1);
        var retained = _catalog.Dictionaries.Select(value => value.Id)
            .Concat(_catalog.Builtins.Where(value => !value.Deleted).Select(value => value.Id)).ToHashSet(StringComparer.Ordinal);
        foreach (var id in _cardViews.Keys.Where(id => !retained.Contains(id)).ToArray()) _cardViews.Remove(id);
        LayoutCards();
    }
    private void Notify(string message) { _notice.Message = message; _notice.Severity = InfoBarSeverity.Success; _notice.IsOpen = true; }
    private async Task SaveBuiltinAsync(BuiltinDictionary dictionary) =>
        _catalog = await _store.SaveAsync(_catalog, _catalog.Dictionaries, _catalog.Builtins.Select(value => value.Id == dictionary.Id ? dictionary : value).ToArray());
    private async Task SaveDictionaryAsync(ManagedDictionary dictionary) =>
        _catalog = await _store.SaveAsync(_catalog, _catalog.Dictionaries.Select(value => value.Id == dictionary.Id ? dictionary : value).ToArray());
    private async Task DeleteDictionaryAsync(string id) =>
        _catalog = await _store.SaveAsync(_catalog, _catalog.Dictionaries.Where(value => value.Id != id).ToArray());
    private void AddCard(string name, string description, string state, string id, Func<Task> action,
        bool enabled, Func<bool, Task> setEnabled, Func<Task> export, Func<Task> delete, Func<Task> rename) {
        void Update(CardView view) {
            view.Open = action; view.SetEnabled = setEnabled; view.Export = export; view.Delete = delete; view.Rename = rename;
            view.Updating = true;
            try {
                if (view.Title.Text != name) {
                    view.Title.Text = name;
                    AutomationProperties.SetName(view.Card, name);
                    AutomationProperties.SetName(view.Toggle, "启用" + name);
                    AutomationProperties.SetName(view.Menu, "管理" + name);
                }
                if (view.Description.Text != description) view.Description.Text = description;
                if (view.State.Text != state) view.State.Text = state;
                if (view.Toggle.IsOn != enabled) view.Toggle.IsOn = enabled;
            } finally { view.Updating = false; }
            _desiredCards.Add(view.Host);
        }
        if (_cardViews.TryGetValue(id, out var existing)) { Update(existing); return; }
        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(new SymbolIcon(Symbol.Library) { HorizontalAlignment = HorizontalAlignment.Left });
        var title = new TextBlock { Text = name, FontSize = 18, TextTrimming = TextTrimming.CharacterEllipsis };
        var descriptionText = new TextBlock { Text = description, MaxLines = 2, TextTrimming = TextTrimming.CharacterEllipsis, TextWrapping = TextWrapping.Wrap, Opacity = 0.75 };
        var stateText = new TextBlock { Text = state, MaxLines = 2, TextTrimming = TextTrimming.CharacterEllipsis, FontSize = 12, TextWrapping = TextWrapping.Wrap, Opacity = 0.7 };
        body.Children.Add(title); body.Children.Add(descriptionText); body.Children.Add(stateText);
        var operationBrush = new Microsoft.UI.Xaml.Media.SolidColorBrush(
            Windows.UI.Color.FromArgb(51, 0, 0, 0));
        var card = new Button { Content = body, Padding = new Thickness(16, 16, 16, 72),
            CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(0), MinWidth = 0,
            HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top };
        AutomationProperties.SetAutomationId(card, "DictionaryCard_" + id);
        AutomationProperties.SetName(card, name);
        var host = new Grid { Height = 228 }; host.Children.Add(card);
        // Sibling controls, never interactive children of the navigation button.
        // Clicking the switch/menu must not enter the detail page.
        var toggle = new ToggleSwitch { IsOn = enabled, OnContent = "已启用", OffContent = "已停用",
            MinWidth = 0, HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(16, 0, 12, 0) };
        AutomationProperties.SetAutomationId(toggle, "DictionaryCardEnabled_" + id);
        AutomationProperties.SetName(toggle, "启用" + name);
        var view = new CardView(host, card, toggle, title, descriptionText, stateText);
        card.Click += async (_, _) => await RunAsync(view.Open);
        toggle.Toggled += async (_, _) => {
            if (_busy || view.Updating) return;
            var value = toggle.IsOn;
            await RunAsync(async () => await view.SetEnabled(value)); RenderCards();
        };
        // Tint only the footer background, not the switch or its status text.
        // Keep it a sibling of the navigation button so toggling never opens details.
        var actions = new Border {
            Height = 64, VerticalAlignment = VerticalAlignment.Bottom,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            Margin = new Thickness(1, 0, 1, 1), CornerRadius = new CornerRadius(0, 0, 9, 9),
            Background = operationBrush, Child = toggle,
        };
        AutomationProperties.SetAutomationId(actions, "DictionaryCardActions_" + id);
        host.Children.Add(actions);
        var flyout = new MenuFlyout();
        void Item(string text, Symbol icon, string command, Func<Task> execute) {
            var item = new MenuFlyoutItem { Text = text, Icon = new SymbolIcon(icon) };
            AutomationProperties.SetAutomationId(item, "DictionaryCard" + command + "_" + id);
            item.Click += async (_, _) => { await RunAsync(execute); RenderCards(); };
            flyout.Items.Add(item);
        }
        Item("导出", Symbol.Save, "Export", () => view.Export());
        Item("重命名", Symbol.Rename, "Rename", () => view.Rename());
        flyout.Items.Add(new MenuFlyoutSeparator());
        Item("删除", Symbol.Delete, "Delete", () => view.Delete());
        var menu = new Button { Content = new FontIcon { Glyph = "\uE712", FontSize = 16, IsTextScaleFactorEnabled = false },
            Flyout = flyout, Padding = new Thickness(0), Width = 32, Height = 32, MinWidth = 0, MinHeight = 0,
            HorizontalContentAlignment = HorizontalAlignment.Center, VerticalContentAlignment = VerticalAlignment.Center,
            HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(8) };
        AutomationProperties.SetAutomationId(menu, "DictionaryCardMenu_" + id);
        AutomationProperties.SetName(menu, "管理" + name); ToolTipService.SetToolTip(menu, "字典操作");
        host.Children.Add(menu);
        // An overlay draws inside the existing card bounds without changing
        // layout, intercepting input or inheriting the button's hover stroke.
        host.Children.Add(new Border {
            BorderBrush = operationBrush, BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(10), IsHitTestVisible = false,
        });
        view.Menu = menu;
        host.SizeChanged += (_, e) => {
            // An expanded navigation pane can leave a very narrow content
            // viewport. Keep the switch itself inside the card, dropping its
            // redundant status text before reducing the card's side insets.
            var compact = e.NewSize.Width < 140;
            var inset = Math.Clamp((e.NewSize.Width - 44) / 2, 0, 16);
            toggle.OnContent = compact ? "" : "已启用";
            toggle.OffContent = compact ? "" : "已停用";
            toggle.Margin = new Thickness(inset, 0, inset, 0);
        };
        _cardViews.Add(id, view); Update(view);
    }
    private void LayoutCards() {
        const double preferredCardWidth = 228;
        var availableWidth = double.IsNaN(_cards.Width) ? _cards.ActualWidth : _cards.Width;
        var columns = Math.Max(1, (int)((availableWidth + 12) / (preferredCardWidth + 12)));
        var cardWidth = Math.Max(0, Math.Min(preferredCardWidth, (availableWidth - 12 * (columns - 1)) / columns));
        while (_cards.ColumnDefinitions.Count > columns) _cards.ColumnDefinitions.RemoveAt(_cards.ColumnDefinitions.Count - 1);
        while (_cards.ColumnDefinitions.Count < columns) _cards.ColumnDefinitions.Add(new());
        foreach (var column in _cards.ColumnDefinitions)
            if (column.Width.GridUnitType != GridUnitType.Pixel || Math.Abs(column.Width.Value - cardWidth) > 0.01)
                column.Width = new GridLength(cardWidth);
        var rows = (_cards.Children.Count + columns - 1) / columns;
        while (_cards.RowDefinitions.Count > rows) _cards.RowDefinitions.RemoveAt(_cards.RowDefinitions.Count - 1);
        while (_cards.RowDefinitions.Count < rows) _cards.RowDefinitions.Add(new() { Height = GridLength.Auto });
        for (var index = 0; index < _cards.Children.Count; ++index) {
            var card = (FrameworkElement)_cards.Children[index];
            if (Grid.GetRow(card) != index / columns) Grid.SetRow(card, index / columns);
            if (Grid.GetColumn(card) != index % columns) Grid.SetColumn(card, index % columns);
        }
    }
    private void ShowDetail(FrameworkElement detail, string? title = null) {
        var root = new Grid { Margin = new Thickness(16, 0, 16, 16), RowSpacing = 12 };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var back = new Button { Content = "返回词库管理器" };
        AutomationProperties.SetAutomationId(back, "DictionaryBack");
        back.Click += async (_, _) => await RefreshAsync();
        var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        if (title is not null) heading.Children.Add(new TextBlock { Text = title, FontSize = 20, VerticalAlignment = VerticalAlignment.Center });
        heading.Children.Add(back);
        root.Children.Add(heading);
        Grid.SetRow(detail, 1); root.Children.Add(detail); _page.Content = root;
    }
    private Task OpenBuiltinAsync(BuiltinDictionary dictionary) {
        if (dictionary.Id == "user") {
            var editor = new UserLexiconEditor(); ShowDetail(editor, dictionary.Name); _ = editor.RefreshAsync();
        } else {
            var body = new StackPanel { Spacing = 12 };
            body.Children.Add(new TextBlock { Text = dictionary.Name, FontSize = 24 });
            body.Children.Add(new TextBlock { Text = "由 Core 以紧凑只读词典加载。卡片菜单可导出原始 .owolx 文件、重命名或移除该候选来源；移除不会擦除安装文件，可通过恢复入口重新加入。导入字典和插件私有字典独立管理。", TextWrapping = TextWrapping.Wrap });
            ShowDetail(body);
        }
        return Task.CompletedTask;
    }
    private void OpenDictionary(ManagedDictionary dictionary) => ShowDetail(new DictionaryDetailEditor(dictionary,
        async updated => {
            await SaveDictionaryAsync(updated);
            return updated;
        }, async () => {
            await DeleteDictionaryAsync(dictionary.Id);
            _page.Content = _catalogPage; RenderCards();
        }));
    internal static void InitializePicker(object picker) => WinRT.Interop.InitializeWithWindow.Initialize(picker,
        WinRT.Interop.WindowNative.GetWindowHandle(((App)Application.Current).MainWindow));
    internal static async Task<(string Name, string SourceName, DictionaryImport Data)?> PickImportAsync() {
        var picker = new FileOpenPicker(); foreach (var extension in DictionaryFormats.Extensions) picker.FileTypeFilter.Add(extension);
        InitializePicker(picker); var file = await picker.PickSingleFileAsync();
        return file is null ? null : (Path.GetFileNameWithoutExtension(file.Name), file.Name, await DictionaryFormats.ReadAsync(file.Path));
    }
    private async Task ImportAsync() {
        var selection = await PickImportAsync(); if (selection is null) return;
        var (name, sourceName, data) = selection.Value;
        var title = new TextBox { Header = "字典名称", Text = name.Length > 128 ? name[..128] : name, MaxLength = 128 };
        var enable = new CheckBox { Content = "导入后启用可用编码词条（适用于宿主完整拼音）", IsChecked = data.Pending < data.Entries.Count };
        var fields = new StackPanel { Spacing = 12 };
        fields.Children.Add(title); fields.Children.Add(new TextBlock { Text = $"识别格式：{data.Format}\n词条 {data.Entries.Count} 条；待补编码 {data.Pending} 条；已合并重复项 {data.Duplicates} 条；权重调整 {data.AdjustedWeights} 条。\n无有效编码的词条会完整保留，但不会自动猜测读音或进入候选。不会修改用户词库或插件私有字典。", TextWrapping = TextWrapping.Wrap }); fields.Children.Add(enable);
        if (await new ContentDialog { XamlRoot = XamlRoot, Title = "预览并导入字典", Content = fields,
            PrimaryButtonText = "确认导入", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close }.ShowAsync() != ContentDialogResult.Primary) return;
        var dictionary = new ManagedDictionary(Guid.NewGuid().ToString("N"), title.Text.Trim(), data.Format, sourceName, "full_pinyin", enable.IsChecked == true, data.Entries);
        _catalog = await _store.SaveAsync(_catalog, [.. _catalog.Dictionaries, dictionary]); RenderCards();
        _notice.Message = "字典已独立保存；启用词条会在后续输入时加载。"; _notice.Severity = InfoBarSeverity.Success; _notice.IsOpen = true;
    }
}
