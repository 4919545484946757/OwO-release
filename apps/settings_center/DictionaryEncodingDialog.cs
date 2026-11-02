using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Windows.Storage.Pickers;

namespace OwO_Settings;

internal sealed class DictionaryEncodingDialog
{
    private readonly IReadOnlyList<DictionaryWord> _pending;
    private readonly ContentDialog _dialog;
    private readonly NumberBox _limit = new() { Header = "本批最多处理条数", Minimum = 1, Maximum = DictionaryEncoding.MaximumBatch, Value = 500, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
    private readonly Button _generate = new() { Content = "生成编码建议" }, _pick = new() { Content = "选择参考词典" }, _cancel = new() { Content = "取消生成", Visibility = Visibility.Collapsed };
    private readonly Button _previous = new() { Content = "上一页", IsEnabled = false }, _next = new() { Content = "下一页", IsEnabled = false };
    private readonly TextBlock _reference = new() { Text = "参考：当前 Core 的基础中文词典（只用于注音，不改变启用状态）", TextWrapping = TextWrapping.Wrap, Opacity = 0.75 };
    private readonly TextBlock _summary = new() { TextWrapping = TextWrapping.Wrap };
    private readonly InfoBar _notice = new() { IsOpen = false, IsClosable = false };
    private readonly ProgressBar _progress = new() { IsIndeterminate = true, Visibility = Visibility.Collapsed };
    private readonly ListView _list = new() { SelectionMode = ListViewSelectionMode.None, MaxHeight = 290, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly List<Row> _rows = [];
    private CancellationTokenSource? _operation;
    private string? _referencePath;
    private int _page;
    private bool _closed;
    private sealed class Row(DictionaryEncodingSuggestion suggestion) {
        internal DictionaryEncodingSuggestion Suggestion = suggestion;
        internal string Code = suggestion.Readings.FirstOrDefault()?.Code ?? "";
        internal bool Selected = suggestion.WholeWord && suggestion.Readings.Count == 1;
    }
    private DictionaryEncodingDialog(XamlRoot root, IReadOnlyList<DictionaryWord> pending) {
        _pending = pending;
        var body = new StackPanel { Spacing = 10, MinWidth = 280, MaxWidth = 620 };
        body.Children.Add(new TextBlock { Text = "仅补充待编码词条，不覆盖已有编码。整词唯一读音默认选中；多音及分词推导项请核对。所有改动须点击保存后才会写入。", TextWrapping = TextWrapping.Wrap });
        body.Children.Add(_reference); body.Children.Add(_limit);
        var commands = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        commands.Children.Add(_generate); commands.Children.Add(_pick); commands.Children.Add(_cancel); body.Children.Add(commands);
        commands.SizeChanged += (_, args) => commands.Orientation = args.NewSize.Width < 480 ? Orientation.Vertical : Orientation.Horizontal;
        body.Children.Add(_progress); body.Children.Add(_notice); body.Children.Add(_summary);
        var select = new CommandBar { DefaultLabelPosition = CommandBarDefaultLabelPosition.Right };
        // Compact buttons wrap through CommandBar's overflow in narrow windows.
        void SelectionCommand(string label, Action action) {
            var command = new AppBarButton { Label = label, Icon = new SymbolIcon(Symbol.Accept) };
            command.Click += (_, _) => action(); select.PrimaryCommands.Add(command);
        }
        SelectionCommand("仅选整词唯一读音", () => Select(row => row.Suggestion.WholeWord && row.Suggestion.Readings.Count == 1));
        SelectionCommand("选中全部可用建议", () => Select(row => row.Code.Length > 0));
        SelectionCommand("全部取消", () => Select(_ => false));
        select.IsEnabled = false;
        body.Children.Add(select); body.Children.Add(_list);
        var pages = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 }; pages.Children.Add(_previous); pages.Children.Add(_next); body.Children.Add(pages);
        _dialog = new() { XamlRoot = root, Title = "补充词库编码", Content = new ScrollViewer { Content = body, MaxHeight = 650, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled },
            PrimaryButtonText = "保存选中编码", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close, IsPrimaryButtonEnabled = false };
        AutomationProperties.SetAutomationId(_dialog, "DictionaryEncodingDialog");
        AutomationProperties.SetAutomationId(_generate, "DictionaryEncodingGenerate");
        AutomationProperties.SetAutomationId(_list, "DictionaryEncodingPreview");
        _generate.Click += async (_, _) => {
            select.IsEnabled = false; await GenerateAsync(); select.IsEnabled = _rows.Count > 0 && _operation is null;
        };
        _pick.Click += async (_, _) => {
            try {
                var picker = new FileOpenPicker(); picker.FileTypeFilter.Add(".owolx"); DictionaryManager.InitializePicker(picker);
                var file = await picker.PickSingleFileAsync(); if (file is null) return;
                _referencePath = file.Path; _reference.Text = "参考词典：" + file.Path;
                _rows.Clear(); Render(); select.IsEnabled = false;
            } catch (Exception error) { Notice(error.Message); }
        };
        _cancel.Click += (_, _) => _operation?.Cancel();
        _previous.Click += (_, _) => { _page--; Render(); }; _next.Click += (_, _) => { _page++; Render(); };
        _dialog.PrimaryButtonClick += (_, args) => {
            try { foreach (var row in _rows.Where(row => row.Selected)) DictionaryStore.ValidateWord(row.Suggestion.Word with { Input = DictionaryEncoding.Normalize(row.Code) }); }
            catch (Exception error) { args.Cancel = true; Notice(error.Message); }
        };
        _dialog.Closing += (_, _) => { _closed = true; _operation?.Cancel(); };
        _dialog.Closed += (_, _) => _closed = true;
        Render();
    }
    private void Notice(string message, InfoBarSeverity severity = InfoBarSeverity.Error) { _notice.Message = message; _notice.Severity = severity; _notice.IsOpen = true; }
    private async Task GenerateAsync() {
        if (_operation is not null) return;
        if (!double.IsFinite(_limit.Value) || _limit.Value != Math.Truncate(_limit.Value) || _limit.Value is < 1 or > DictionaryEncoding.MaximumBatch) { Notice("请填写 1–1000 的整数批量。"); return; }
        using var operation = new CancellationTokenSource(); _operation = operation;
        _generate.IsEnabled = _pick.IsEnabled = _limit.IsEnabled = false; _dialog.IsPrimaryButtonEnabled = false;
        _cancel.Visibility = _progress.Visibility = Visibility.Visible; _notice.IsOpen = false;
        _rows.Clear(); _page = 0; Render();
        try {
            var suggestions = await new DictionaryEncodingClient().SuggestAsync(_pending.Take((int)_limit.Value).ToArray(), _referencePath, operation.Token);
            operation.Token.ThrowIfCancellationRequested();
            if (_closed) return;
            _rows.AddRange(suggestions.Select(value => new Row(value)));
            Notice("建议已生成。多音字和推导结果须核对；无建议的词条可手动填写，或保留待补编码。", InfoBarSeverity.Informational);
        } catch (OperationCanceledException) { if (!_closed) Notice("生成已取消或超时，词库未修改。", InfoBarSeverity.Informational); }
        catch (Exception error) { if (!_closed) Notice(error.Message + " 可先启动 Core，或选择本机 .owolx 参考词典重试。"); }
        finally {
            _operation = null;
            if (!_closed) { _generate.IsEnabled = _pick.IsEnabled = _limit.IsEnabled = true; _cancel.Visibility = _progress.Visibility = Visibility.Collapsed; Render(); }
        }
    }
    private void Select(Func<Row, bool> match) { if (_operation is not null) return; foreach (var row in _rows) row.Selected = match(row); Render(); }
    private void UpdateSummary() {
        _summary.Text = $"待补 {_pending.Count} 条 · 本批 {_rows.Count} 条 · 有建议 {_rows.Count(row => row.Suggestion.Readings.Count > 0)} 条 · 已选 {_rows.Count(row => row.Selected)} 条 · 第 {_page + 1} 页";
        _dialog.IsPrimaryButtonEnabled = _operation is null && _rows.Any(row => row.Selected) && _rows.Where(row => row.Selected).All(row => !string.IsNullOrWhiteSpace(row.Code));
    }
    private void Render() {
        _page = Math.Clamp(_page, 0, Math.Max(0, (_rows.Count - 1) / 50));
        _list.Items.Clear();
        foreach (var row in _rows.Skip(_page * 50).Take(50)) {
            var panel = new StackPanel { Spacing = 5, Margin = new Thickness(0, 4, 0, 8) };
            var selected = new CheckBox { Content = row.Suggestion.Word.Text, IsChecked = row.Selected };
            selected.Checked += (_, _) => { row.Selected = true; UpdateSummary(); }; selected.Unchecked += (_, _) => { row.Selected = false; UpdateSummary(); };
            panel.Children.Add(selected);
            var field = new Grid { ColumnSpacing = 8 }; field.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); field.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var code = new TextBox { Text = row.Code, PlaceholderText = "拼音编码，例如 ce'shi；可手动修改", MaxLength = 256 };
            code.TextChanged += (_, _) => { row.Code = code.Text; UpdateSummary(); };
            field.Children.Add(code);
            if (row.Suggestion.Readings.Count > 1) {
                var menu = new MenuFlyout();
                foreach (var reading in row.Suggestion.Readings) { var item = new MenuFlyoutItem { Text = reading.Code }; item.Click += (_, _) => code.Text = reading.Code; menu.Items.Add(item); }
                var choose = new Button { Content = "读音 ▾", Flyout = menu }; Grid.SetColumn(choose, 1); field.Children.Add(choose);
            }
            panel.Children.Add(field);
            panel.Children.Add(new TextBlock { Text = row.Suggestion.Readings.Count == 0 ? "未找到读音，请手动补充" :
                $"{(row.Suggestion.WholeWord ? "整词匹配" : "分词／单字推导（需确认）")} · {row.Suggestion.Readings.Count} 种建议（最多 8 种）", Opacity = 0.7, TextWrapping = TextWrapping.Wrap });
            _list.Items.Add(new ListViewItem { Content = panel, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        }
        _previous.IsEnabled = _operation is null && _page > 0; _next.IsEnabled = _operation is null && (_page + 1) * 50 < _rows.Count;
        UpdateSummary();
    }
    internal static async Task<IReadOnlyDictionary<string, string>?> ShowAsync(XamlRoot root, IReadOnlyList<DictionaryWord> pending) {
        var view = new DictionaryEncodingDialog(root, pending);
        if (await view._dialog.ShowAsync() != ContentDialogResult.Primary) return null;
        return view._rows.Where(row => row.Selected).ToDictionary(row => row.Suggestion.Word.Id, row => row.Code, StringComparer.Ordinal);
    }
}
