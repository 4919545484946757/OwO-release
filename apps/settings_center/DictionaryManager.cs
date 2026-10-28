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
    private readonly Grid _catalogPage = new() { Margin = new Thickness(16, 0, 16, 16), RowSpacing = 12 };
    private bool _busy;
    internal DictionaryManager() {
        _catalogPage.RowDefinitions.Add(new() { Height = GridLength.Auto });
        _catalogPage.RowDefinitions.Add(new() { Height = GridLength.Auto });
        _catalogPage.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var toolbar = new Grid { ColumnSpacing = 8 };
        toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.Children.Add(_search); Grid.SetColumn(_import, 1); toolbar.Children.Add(_import);
        Grid.SetColumn(_refresh, 2); toolbar.Children.Add(_refresh);
        _catalogPage.Children.Add(toolbar); Grid.SetRow(_notice, 1); _catalogPage.Children.Add(_notice);
        var scroll = new ScrollViewer { Content = _cards, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetRow(scroll, 2); _catalogPage.Children.Add(scroll);
        _cards.SizeChanged += (_, _) => LayoutCards();
        _search.TextChanged += (_, _) => RenderCards();
        _import.Click += async (_, _) => await RunAsync(ImportAsync);
        _refresh.Click += async (_, _) => await RefreshAsync();
        AutomationProperties.SetAutomationId(_search, "DictionarySearch");
        AutomationProperties.SetAutomationId(_import, "DictionaryImport");
        _page.Content = _catalogPage; Content = _page;
    }
    internal async Task RefreshAsync() => await RunAsync(async () => {
        _catalog = await _store.LoadAsync(); _page.Content = _catalogPage; RenderCards();
    });
    private async Task RunAsync(Func<Task> action) {
        if (_busy) return; _busy = true; _import.IsEnabled = _refresh.IsEnabled = false; _cards.IsHitTestVisible = false;
        try { await action(); }
        catch (Exception error) { _notice.Message = error.Message; _notice.Severity = InfoBarSeverity.Error; _notice.IsOpen = true; }
        finally { _busy = false; _import.IsEnabled = _refresh.IsEnabled = true; _cards.IsHitTestVisible = true; }
    }
    private void RenderCards() {
        _cards.Children.Clear();
        var query = _search.Text.Trim();
        bool Match(params string[] values) => values.Any(value => value.Contains(query, StringComparison.OrdinalIgnoreCase));
        if (Match("用户词库", "学习", "宿主")) AddCard("用户词库", "本机学习与手动添加的词语", "新增 · 编辑 · 导入/出 · 清除学习", "user", async () => await OpenUserAsync());
        if (Match("基础中文词典", "内置", "只读")) AddCard("基础中文词典", "随 OwO 安装的只读词典", "不参与用户词库清空操作", "base", async () => {
            var body = new StackPanel { Spacing = 12 };
            body.Children.Add(new TextBlock { Text = "基础中文词典", FontSize = 24 });
            body.Children.Add(new TextBlock { Text = "由 Core 以紧凑只读词典加载，随软件维护，不允许在此编辑或删除。导入的字典独立保存，不会替换基础词典。插件方案的私有词典仍由对应插件管理。", TextWrapping = TextWrapping.Wrap });
            ShowDetail(body); await Task.CompletedTask;
        });
        foreach (var dictionary in _catalog.Dictionaries.Where(value => Match(value.Name, value.Format, value.SourceName))) {
            var pending = dictionary.Entries.Count(word => !DictionaryStore.Usable(word));
            AddCard(dictionary.Name, $"{dictionary.Format} · {dictionary.Entries.Count} 条", (dictionary.Enabled ? "已启用" : "已停用") + (pending > 0 ? $" · {pending} 条待补编码" : ""), dictionary.Id,
                async () => { OpenDictionary(dictionary); await Task.CompletedTask; });
        }
        if (_cards.Children.Count == 0) _cards.Children.Add(new TextBlock { Text = "没有匹配的字典。", Margin = new Thickness(8) });
        LayoutCards();
    }
    private void AddCard(string name, string description, string state, string id, Func<Task> action) {
        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(new SymbolIcon(Symbol.Library) { HorizontalAlignment = HorizontalAlignment.Left });
        body.Children.Add(new TextBlock { Text = name, FontSize = 18, TextTrimming = TextTrimming.CharacterEllipsis });
        body.Children.Add(new TextBlock { Text = description, TextWrapping = TextWrapping.Wrap, Opacity = 0.75 });
        body.Children.Add(new TextBlock { Text = state, FontSize = 12, TextWrapping = TextWrapping.Wrap, Opacity = 0.7 });
        var card = new Button { Content = body, Height = 170, Padding = new Thickness(16),
            CornerRadius = new CornerRadius(10), HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top };
        AutomationProperties.SetAutomationId(card, "DictionaryCard_" + id);
        card.Click += async (_, _) => await RunAsync(action); _cards.Children.Add(card);
    }
    private void LayoutCards() {
        var columns = Math.Max(1, (int)((_cards.ActualWidth + 12) / 252));
        _cards.ColumnDefinitions.Clear(); _cards.RowDefinitions.Clear();
        for (var column = 0; column < columns; ++column) _cards.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        for (var index = 0; index < _cards.Children.Count; ++index) {
            if (index % columns == 0) _cards.RowDefinitions.Add(new() { Height = GridLength.Auto });
            Grid.SetRow((FrameworkElement)_cards.Children[index], index / columns); Grid.SetColumn((FrameworkElement)_cards.Children[index], index % columns);
        }
    }
    private void ShowDetail(FrameworkElement detail, string? title = null) {
        var root = new Grid { Margin = new Thickness(16, 0, 16, 16), RowSpacing = 12 };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var back = new Button { Content = "返回词库管理器" };
        AutomationProperties.SetAutomationId(back, "DictionaryBack");
        back.Click += async (_, _) => await RefreshAsync();
        var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        heading.Children.Add(back);
        if (title is not null) heading.Children.Add(new TextBlock { Text = title, FontSize = 20, VerticalAlignment = VerticalAlignment.Center });
        root.Children.Add(heading);
        Grid.SetRow(detail, 1); root.Children.Add(detail); _page.Content = root;
    }
    private Task OpenUserAsync() {
        var editor = new UserLexiconEditor(); ShowDetail(editor, "用户词库"); _ = editor.RefreshAsync();
        return Task.CompletedTask;
    }
    private void OpenDictionary(ManagedDictionary dictionary) => ShowDetail(new DictionaryDetailEditor(dictionary,
        async updated => {
            _catalog = await _store.SaveAsync(_catalog, _catalog.Dictionaries.Select(value => value.Id == updated.Id ? updated : value).ToArray());
            return updated;
        }, async () => {
            _catalog = await _store.SaveAsync(_catalog, _catalog.Dictionaries.Where(value => value.Id != dictionary.Id).ToArray());
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
