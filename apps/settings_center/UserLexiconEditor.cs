using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System.Text;
using System.Text.Json;
using Windows.Storage.Pickers;

namespace OwO_Settings;

internal sealed class UserLexiconEditor : UserControl
{
    private readonly UserLexiconClient _client = new();
    private readonly TextBox _search = new() { PlaceholderText = "搜索编码或词语", MinWidth = 0 };
    private readonly ListView _list = new() { SelectionMode = ListViewSelectionMode.Single };
    private readonly TextBlock _summary = new() { TextWrapping = TextWrapping.Wrap };
    private readonly InfoBar _notice = new() { IsOpen = false, IsClosable = false };
    private readonly CommandBar _commands = new() { DefaultLabelPosition = CommandBarDefaultLabelPosition.Right };
    private readonly Button _previous = new() { Content = "上一页" }, _next = new() { Content = "下一页" };
    private int _offset, _total;
    private bool _busy;
    private const int PageSize = 50;
    internal UserLexiconEditor()
    {
        var root = new Grid { Margin = new Thickness(16, 0, 16, 16), RowSpacing = 10 };
        foreach (var size in new[] { GridLength.Auto, GridLength.Auto, GridLength.Auto, new GridLength(1, GridUnitType.Star), GridLength.Auto })
            root.RowDefinitions.Add(new RowDefinition { Height = size });
        root.Children.Add(_notice);
        AddCommand("新增", Symbol.Add, async () => await EditAsync(null));
        AddCommand("编辑", Symbol.Edit, async () => await EditAsync(Selected()));
        AddCommand("删除", Symbol.Delete, DeleteAsync);
        AddCommand("导入", Symbol.OpenFile, ImportAsync);
        AddCommand("导出", Symbol.Save, ExportAsync);
        AddCommand("刷新", Symbol.Refresh, async () => await LoadAsync());
        AddCommand("清空全部学习", Symbol.Clear, ClearAsync, true);
        Grid.SetRow(_commands, 1); root.Children.Add(_commands);
        var filter = new Grid { ColumnSpacing = 8 };
        filter.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        filter.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var find = new Button { Content = "搜索" };
        find.Click += async (_, _) => await RunAsync(async () => { _offset = 0; await LoadAsync(); });
        _search.KeyDown += async (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) {
            e.Handled = true; await RunAsync(async () => { _offset = 0; await LoadAsync(); });
        } };
        filter.Children.Add(_search); Grid.SetColumn(find, 1); filter.Children.Add(find);
        Grid.SetRow(filter, 2); root.Children.Add(filter);
        Grid.SetRow(_list, 3); root.Children.Add(_list);
        _list.DoubleTapped += async (_, _) => await RunAsync(async () => await EditAsync(Selected()));
        var footer = new Grid { ColumnSpacing = 8 };
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        footer.Children.Add(_summary); Grid.SetColumn(_previous, 1); footer.Children.Add(_previous);
        Grid.SetColumn(_next, 2); footer.Children.Add(_next);
        _previous.Click += async (_, _) => await RunAsync(async () => { _offset = Math.Max(0, _offset - PageSize); await LoadAsync(); });
        _next.Click += async (_, _) => await RunAsync(async () => { _offset += PageSize; await LoadAsync(); });
        Grid.SetRow(footer, 4); root.Children.Add(footer);
        Content = root;
    }
    private void AddCommand(string label, Symbol icon, Func<Task> action, bool secondary = false)
    {
        var button = new AppBarButton { Label = label, Icon = new SymbolIcon(icon) };
        button.Click += async (_, _) => await RunAsync(action);
        if (secondary) _commands.SecondaryCommands.Add(button); else _commands.PrimaryCommands.Add(button);
    }
    private LexiconWord Selected() => (_list.SelectedItem as ListViewItem)?.Tag as LexiconWord
        ?? throw new InvalidOperationException("请先选择一个词条。");
    internal Task RefreshAsync() => RunAsync(LoadAsync);
    private async Task RunAsync(Func<Task> action)
    {
        if (_busy) return;
        _busy = true; _commands.IsEnabled = _list.IsEnabled = _search.IsEnabled = false;
        try { await action(); }
        catch (Exception error) { Notify(error.Message, InfoBarSeverity.Error); }
        finally { _busy = false; _commands.IsEnabled = _list.IsEnabled = _search.IsEnabled = true;
            _previous.IsEnabled = _offset > 0; _next.IsEnabled = _offset + PageSize < _total; }
    }
    private void Notify(string message, InfoBarSeverity severity = InfoBarSeverity.Success)
    { _notice.Message = message; _notice.Severity = severity; _notice.IsOpen = true; }
    private async Task LoadAsync()
    {
        var result = await _client.RequestAsync("list", query: _search.Text, offset: _offset);
        _total = result.Total;
        if (_offset >= _total && _offset > 0) { _offset = 0; await LoadAsync(); return; }
        _list.Items.Clear();
        foreach (var word in result.Words) {
            var row = new Grid { ColumnSpacing = 12, Padding = new Thickness(4, 8, 4, 8), HorizontalAlignment = HorizontalAlignment.Stretch };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(2, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.Children.Add(new TextBlock { Text = word.Input, TextTrimming = TextTrimming.CharacterEllipsis });
            var label = new TextBlock { Text = word.Text, TextTrimming = TextTrimming.CharacterEllipsis };
            Grid.SetColumn(label, 1); row.Children.Add(label);
            var weight = new TextBlock { Text = $"权重 {word.Count}" }; Grid.SetColumn(weight, 2); row.Children.Add(weight);
            var item = new ListViewItem { Content = row, Tag = word, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            ToolTipService.SetToolTip(item, $"{word.Input} → {word.Text}；权重 {word.Count}");
            _list.Items.Add(item);
        }
        _summary.Text = $"共 {_total} 条 · 第 {_offset / PageSize + 1} 页 · 每页 {PageSize} 条";
        _notice.IsOpen = false;
    }
    private ContentDialog Dialog(string title, object content, string primary) => new() {
        XamlRoot = XamlRoot, Title = title, Content = content, PrimaryButtonText = primary,
        CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close
    };
    private async Task EditAsync(LexiconWord? original)
    {
        var input = new TextBox { Header = "编码（拼音/英文字母编码）", Text = original?.Input ?? "", MaxLength = 256 };
        var text = new TextBox { Header = "词语", Text = original?.Text ?? "", MaxLength = 256 };
        var count = new NumberBox { Header = "权重（1–128）", Minimum = 1, Maximum = 128, Value = original?.Count ?? 2,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
        var fields = new StackPanel { Spacing = 12, MinWidth = 280 };
        fields.Children.Add(input); fields.Children.Add(text); fields.Children.Add(count);
        fields.Children.Add(new TextBlock { Text = "手动保存视为明确确认，直接生效；不会模拟输入或上屏。", TextWrapping = TextWrapping.Wrap });
        if (await Dialog(original is null ? "新增用户词" : "编辑用户词", fields, "保存").ShowAsync() != ContentDialogResult.Primary) return;
        if (double.IsNaN(count.Value) || count.Value != Math.Truncate(count.Value)) throw new InvalidDataException("权重必须为整数。");
        var word = new LexiconWord(input.Text.Trim(), text.Text, checked((uint)count.Value));
        UserLexiconClient.Validate(word);
        var latest = await _client.RequestAsync("get", original: original ?? word);
        if (original is not null && latest.Words.Count == 0) throw new IOException("原词条已被删除，请刷新后重新新增。");
        if ((original is null && latest.Words.Count != 0) ||
            (original is not null && latest.Words[0].Count != original.Count)) {
            if (await Dialog("词条已发生变化", "此词已存在，或权重已被学习更新。是否以刚才填写的内容覆盖？", "确认覆盖").ShowAsync() != ContentDialogResult.Primary) return;
        }
        if (original is not null && (UserLexiconClient.CanonicalInput(original.Input), original.Text) !=
            (UserLexiconClient.CanonicalInput(word.Input), word.Text)) {
            var target = await _client.RequestAsync("get", original: word);
            if (target.Revision != latest.Revision) throw new IOException("编辑期间词库发生变化，请刷新后重试。");
            if (target.Words.Count != 0 && await Dialog("目标词条已存在", "新编码和词语已有记录，是否以填写的权重覆盖，并删除原绑定？", "确认合并").ShowAsync() != ContentDialogResult.Primary) return;
        }
        await _client.RequestAsync("save", latest.Revision, original: original, words: [word]);
        await LoadAsync(); Notify("词条已保存并立即生效，旧版本已保留为词频备份。");
    }
    private async Task DeleteAsync()
    {
        var word = Selected();
        if (await Dialog("删除用户词", $"删除“{word.Text}”（{word.Input}）及该编码对应的学习偏好？基础词典不会修改。", "删除").ShowAsync() != ContentDialogResult.Primary) return;
        var latest = await _client.RequestAsync("get", original: word);
        await _client.RequestAsync("delete", latest.Revision, original: word);
        await LoadAsync(); Notify("已删除选中词条的学习数据；基础词典仍可能生成同名候选。");
    }
    private async Task ClearAsync()
    {
        var fields = new StackPanel { Spacing = 10 };
        fields.Children.Add(new TextBlock { Text = "将清空全部用户词条、全局词频和语言上下文学习。基础词典、设置和插件不受影响。会保留操作前的 .bak 备份，因此不是安全抹除；继续输入后仍会重新学习。此操作不会在自动保存时触发。输入“清空”继续。", TextWrapping = TextWrapping.Wrap });
        var confirm = new TextBox { PlaceholderText = "输入：清空" }; fields.Children.Add(confirm);
        var dialog = Dialog("清空全部用户学习", fields, "确认清空"); dialog.IsPrimaryButtonEnabled = false;
        confirm.TextChanged += (_, _) => dialog.IsPrimaryButtonEnabled = confirm.Text == "清空";
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        var latest = await _client.RequestAsync("list", limit: 1);
        await _client.RequestAsync("clear", latest.Revision, mode: "clear-confirmed");
        _offset = 0; await LoadAsync(); Notify("全部用户学习已清空；操作前的数据已保留为词频 .bak 备份。");
    }
    private static void InitializePicker(object picker) => WinRT.Interop.InitializeWithWindow.Initialize(picker,
        WinRT.Interop.WindowNative.GetWindowHandle(((App)Application.Current).MainWindow));
    private async Task ImportAsync()
    {
        var picker = new FileOpenPicker(); picker.FileTypeFilter.Add(".json"); picker.FileTypeFilter.Add(".tsv"); InitializePicker(picker);
        var file = await picker.PickSingleFileAsync(); if (file is null) return;
        var words = await UserLexiconClient.ReadImportAsync(file.Path);
        var mode = new ComboBox { Header = "导入方式", SelectedIndex = 0, HorizontalAlignment = HorizontalAlignment.Stretch };
        mode.Items.Add("合并：保留现有词条，同编码同词语以导入权重更新");
        mode.Items.Add("替换：清空现有词库和学习数据，再导入");
        if (await Dialog("选择导入方式", mode, "校验并预览").ShowAsync() != ContentDialogResult.Primary) return;
        var selected = mode.SelectedIndex == 0 ? "merge" : "replace-confirmed";
        var latest = await _client.RequestAsync("list", limit: 1);
        var preview = await _client.RequestAsync("preview", latest.Revision, mode: selected, words: words);
        var warning = $"文件包含 {words.Count} 条：新增 {preview.Added} 条，更新 {preview.Updated} 条。\n" +
            (selected == "merge" ? "现有其他词条与学习记录将保留。" : "警告：将先清空全部现有用户词条、词频和语言上下文学习。") +
            "\n本次导入需单独确认。导入文件不包含应用上下文，操作前将备份原数据。";
        if (await Dialog("确认导入用户词库", warning, "确认导入").ShowAsync() != ContentDialogResult.Primary) return;
        await _client.RequestAsync("import", preview.Revision, mode: selected, words: words);
        _offset = 0; await LoadAsync(); Notify($"已导入 {words.Count} 条词语。");
    }
    private async Task ExportAsync()
    {
        var picker = new FileSavePicker { SuggestedFileName = $"OwO-用户词库-{DateTime.Now:yyyyMMdd}" };
        picker.FileTypeChoices.Add("OwO 用户词库 JSON", [".json"]);
        picker.FileTypeChoices.Add("制表符文本 TSV", [".tsv"]);
        InitializePicker(picker);
        var file = await picker.PickSaveFileAsync(); if (file is null) return;
        var words = new List<LexiconWord>();
        ulong? revision = null;
        for (var offset = 0;; offset += 200) {
            var page = await _client.RequestAsync("list", revision, offset: offset, limit: 200);
            revision = page.Revision; words.AddRange(page.Words);
            if (words.Count >= page.Total) break;
            if (page.Words.Count == 0) throw new IOException("导出分页异常，请刷新重试。");
        }
        var output = Path.GetExtension(file.Path).Equals(".tsv", StringComparison.OrdinalIgnoreCase)
            ? "input\ttext\tcount\n" + string.Concat(words.Select(word => $"{word.Input}\t{word.Text}\t{word.Count}\n"))
            : JsonSerializer.Serialize(new LexiconFile(1, "OwO.UserLexicon", words), new JsonSerializerOptions { WriteIndented = true });
        var temporary = file.Path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try { await File.WriteAllTextAsync(temporary, output, new UTF8Encoding(false)); File.Move(temporary, file.Path, true); }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
        Notify($"已导出 {words.Count} 条，仅包含编码、词语和权重，不包含应用上下文。");
    }
}
