using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

internal sealed class DictionaryDetailEditor : UserControl
{
    private ManagedDictionary _dictionary;
    private readonly Func<ManagedDictionary, Task<ManagedDictionary>> _save;
    private readonly Func<Task> _delete;
    private readonly TextBox _query = new() { PlaceholderText = "搜索词语、编码或语言" };
    private readonly TextBlock _summary = new() { TextWrapping = TextWrapping.Wrap };
    private readonly ListView _words = new() { SelectionMode = ListViewSelectionMode.Single };
    private readonly ToggleSwitch _enabled = new() { Header = "启用字典", OnContent = "开", OffContent = "关" };
    private readonly Action<string, InfoBarSeverity> _reportStatus;
    private readonly Button _previous = new() { Content = "上一页" }, _next = new() { Content = "下一页" };
    private readonly Grid _root = new() { RowSpacing = 10 };
    private readonly CommandBar _commands = new() { DefaultLabelPosition = CommandBarDefaultLabelPosition.Right };
    private int _page;
    private bool _updating, _busy;
    internal DictionaryDetailEditor(ManagedDictionary dictionary, Func<ManagedDictionary, Task<ManagedDictionary>> save, Func<Task> delete, Action<string, InfoBarSeverity> reportStatus) {
        _dictionary = dictionary; _save = save; _delete = delete; _reportStatus = reportStatus;
        foreach (var height in new[] { GridLength.Auto, GridLength.Auto, GridLength.Auto, new GridLength(1, GridUnitType.Star), GridLength.Auto })
            _root.RowDefinitions.Add(new() { Height = height });
        var commands = _commands;
        void Command(string label, Symbol icon, Func<Task> action, bool secondary = false) {
            var button = new AppBarButton { Label = label, Icon = new SymbolIcon(icon) };
            button.Click += async (_, _) => await RunAsync(action);
            if (secondary) commands.SecondaryCommands.Add(button); else commands.PrimaryCommands.Add(button);
        }
        Command("新增", Symbol.Add, async () => await EditAsync(null));
        Command("编辑", Symbol.Edit, async () => await EditAsync(Selected()));
        Command("删除词条", Symbol.Delete, DeleteWordAsync);
        Command("合并导入", Symbol.OpenFile, MergeAsync);
        Command("补充编码", Symbol.Edit, SupplementAsync);
        Command("导出", Symbol.Save, ExportAsync);
        Command("重命名", Symbol.Rename, RenameAsync, true);
        Command("删除字典", Symbol.Delete, DeleteAsync, true);
        _root.Children.Add(commands);
        var header = new Grid { ColumnSpacing = 12 }; header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        header.Children.Add(_summary); Grid.SetColumn(_enabled, 1); header.Children.Add(_enabled);
        Grid.SetRow(header, 1); _root.Children.Add(header);
        Grid.SetRow(_query, 2); _root.Children.Add(_query); Grid.SetRow(_words, 3); _root.Children.Add(_words);
        var footer = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        footer.Children.Add(_previous); footer.Children.Add(_next); Grid.SetRow(footer, 4); _root.Children.Add(footer);
        _query.TextChanged += (_, _) => { _page = 0; Render(); };
        _previous.Click += (_, _) => { _page = Math.Max(0, _page - 1); Render(); };
        _next.Click += (_, _) => { _page++; Render(); };
        _words.DoubleTapped += async (_, _) => await RunAsync(async () => await EditAsync(Selected()));
        _enabled.Toggled += async (_, _) => {
            if (_updating || _busy) return;
            await RunAsync(async () => await SaveAsync(_dictionary with { Enabled = _enabled.IsOn }));
            Render();
        };
        Content = _root; Render();
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_enabled, "DictionaryEnabled");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_query, "DictionaryWordSearch");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_words, "DictionaryWords");
    }
    private DictionaryWord Selected() => (_words.SelectedItem as ListViewItem)?.Tag as DictionaryWord ?? throw new InvalidOperationException("请先选择词条。" );
    private ContentDialog Dialog(string title, object content, string primary) => new() {
        XamlRoot = XamlRoot, Title = title, Content = content, PrimaryButtonText = primary,
        CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close
    };
    private async Task RunAsync(Func<Task> action) {
        if (_busy) return; _busy = true; _commands.IsEnabled = _enabled.IsEnabled = _query.IsEnabled = _words.IsEnabled = false;
        try { await action(); }
        catch (Exception error) { _reportStatus(error.Message, InfoBarSeverity.Error); }
        finally { _busy = false; _commands.IsEnabled = _enabled.IsEnabled = _query.IsEnabled = _words.IsEnabled = true; }
    }
    private async Task SaveAsync(ManagedDictionary dictionary) { _dictionary = await _save(dictionary); Render(); }
    private void Render() {
        _updating = true; _enabled.IsOn = _dictionary.Enabled; _updating = false;
        var query = _query.Text.Trim();
        var filtered = _dictionary.Entries.Where(word => word.Text.Contains(query, StringComparison.OrdinalIgnoreCase) || word.Input.Contains(query, StringComparison.OrdinalIgnoreCase) || word.Locale.Contains(query, StringComparison.OrdinalIgnoreCase)).ToArray();
        _page = Math.Min(_page, Math.Max(0, (filtered.Length - 1) / 50)); _words.Items.Clear();
        foreach (var word in filtered.Skip(_page * 50).Take(50)) {
            var row = new Grid { ColumnSpacing = 12, Padding = new Thickness(4, 8, 4, 8) };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = new GridLength(2, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            row.Children.Add(new TextBlock { Text = DictionaryStore.Usable(word) ? word.Input : "待补编码", TextTrimming = TextTrimming.CharacterEllipsis });
            var text = new TextBlock { Text = word.Text, TextTrimming = TextTrimming.CharacterEllipsis }; Grid.SetColumn(text, 1); row.Children.Add(text);
            var metadata = new TextBlock { Text = $"{word.Locale}  ·  {word.Count}" }; Grid.SetColumn(metadata, 2); row.Children.Add(metadata);
            var item = new ListViewItem { Content = row, Tag = word, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            ToolTipService.SetToolTip(item, $"{word.Text}\n编码：{word.Input}\n原编码：{word.OriginalInput}\n原权重：{word.OriginalWeight}"); _words.Items.Add(item);
        }
        _summary.Text = $"{_dictionary.Name} · {_dictionary.Format} · {_dictionary.Entries.Count} 条\n适用宿主完整拼音；{_dictionary.Entries.Count(word => !DictionaryStore.Usable(word))} 条待补编码 · 筛选 {filtered.Length} 条 · 第 {_page + 1} 页";
        _previous.IsEnabled = _page > 0; _next.IsEnabled = (_page + 1) * 50 < filtered.Length;
    }
    private async Task EditAsync(DictionaryWord? original) {
        var input = new TextBox { Header = "编码（英文字母及 '；可留空稍后补全）", Text = original?.Input ?? "", MaxLength = 256 };
        var text = new TextBox { Header = "词语", Text = original?.Text ?? "", MaxLength = 1024 };
        var locale = new TextBox { Header = "语言标记（可选）", Text = original?.Locale ?? "", MaxLength = 128 };
        var tag = new TextBox { Header = "词性／标签（可选）", Text = original?.Tag ?? "", MaxLength = 128 };
        var weight = new NumberBox { Header = "权重（1–128）", Minimum = 1, Maximum = 128, Value = original?.Count ?? 1 };
        var fields = new StackPanel { Spacing = 10, MinWidth = 280 }; fields.Children.Add(input); fields.Children.Add(text); fields.Children.Add(locale); fields.Children.Add(tag); fields.Children.Add(weight);
        if (await Dialog(original is null ? "新增词条" : "编辑词条", fields, "保存").ShowAsync() != ContentDialogResult.Primary) return;
        if (!double.IsFinite(weight.Value) || weight.Value != Math.Truncate(weight.Value)) throw new InvalidDataException("权重须为整数。" );
        var word = new DictionaryWord(original?.Id ?? Guid.NewGuid().ToString("N"), input.Text.Trim().ToLowerInvariant(), text.Text, checked((uint)weight.Value), locale.Text.Trim(), original?.OriginalInput ?? "", original?.OriginalWeight ?? "", tag.Text);
        DictionaryStore.ValidateWord(word);
        var entries = _dictionary.Entries.Where(value => value.Id != word.Id).Append(word).ToArray();
        if (entries.Count(value => DictionaryStore.WordIdentity(value) == DictionaryStore.WordIdentity(word)) > 1)
            throw new InvalidDataException("同编码、词语和语言的词条已存在。" );
        await SaveAsync(_dictionary with { Entries = entries });
    }
    private async Task DeleteWordAsync() {
        var word = Selected();
        if (await Dialog("删除词条", $"从本字典删除“{word.Text}”？不会删除用户词库中已学习的同名词。", "删除").ShowAsync() != ContentDialogResult.Primary) return;
        await SaveAsync(_dictionary with { Entries = _dictionary.Entries.Where(value => value.Id != word.Id).ToArray() });
    }
    private async Task RenameAsync() {
        if (await DictionaryActions.RenameAsync(XamlRoot, _dictionary.Name) is { } name) await SaveAsync(_dictionary with { Name = name });
    }
    private async Task SupplementAsync() {
        var query = _query.Text.Trim();
        var pending = _dictionary.Entries.Where(word => !DictionaryStore.Usable(word) &&
            (word.Text.Contains(query, StringComparison.OrdinalIgnoreCase) || word.Locale.Contains(query, StringComparison.OrdinalIgnoreCase))).ToArray();
        if (pending.Length == 0) throw new InvalidOperationException("当前筛选下没有待补编码词条；已有编码不会被覆盖。");
        if (await DictionaryEncodingDialog.SupplementAsync(XamlRoot, _dictionary, pending, persist: async value => {
            if (value.Updated > 0) _dictionary = await _save(value.Dictionary);
        }) is not { } result) return;
        Render();
        _reportStatus($"已补充 {result.Updated} 条编码；{result.Duplicates} 条因与已有词条重复而保留待补状态。" +
            (result.CleanedText > 0 ? $"同时清理 {result.CleanedText} 条 THUOCL 格式空白。" : "") + "词频、原始字段与启用状态未修改。", InfoBarSeverity.Success);
    }
    private async Task DeleteAsync() {
        if (await DictionaryActions.ConfirmDeleteAsync(XamlRoot, _dictionary.Name)) await _delete();
    }
    private async Task MergeAsync() {
        var selected = await DictionaryManager.PickImportAsync(); if (selected is null) return;
        var data = selected.Value.Data;
        if (await Dialog("合并导入", $"识别为 {data.Format}：{data.Entries.Count} 条，其中 {data.Pending} 条待补编码。合并仅影响此字典，同编码、词语、语言的权重取较高值。", "确认合并").ShowAsync() != ContentDialogResult.Primary) return;
        var merged = _dictionary.Entries.Concat(data.Entries).GroupBy(DictionaryStore.WordIdentity)
            .Select(group => group.OrderByDescending(word => word.Count).First()).ToArray();
        await SaveAsync(_dictionary with { Entries = merged });
    }
    private async Task ExportAsync() {
        if (!await DictionaryActions.ExportAsync(_dictionary)) return;
        _reportStatus("已导出。本文件包含个人词语，请妥善保管。", InfoBarSeverity.Success);
    }
}
