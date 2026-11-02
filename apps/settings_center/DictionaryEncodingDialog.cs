using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Windows.Storage.Pickers;

namespace OwO_Settings;

internal sealed class DictionaryEncodingDialog
{
    private readonly IReadOnlyList<DictionaryWord> _pending;
    private readonly ContentDialog _dialog;
    private readonly NumberBox _limit = new() { Header = "本次处理上限（自动分批）", Minimum = 1, Maximum = DictionaryStore.MaximumEntries, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
    private readonly Button _generate = new() { Content = "自动补全编码" }, _pick = new() { Content = "另选参考词典" }, _cancel = new() { Content = "取消生成", Visibility = Visibility.Collapsed };
    private readonly Button _previous = new() { Content = "上一页", IsEnabled = false }, _next = new() { Content = "下一页", IsEnabled = false };
    private readonly TextBlock _reference = new() { Text = "自动参考基础词典和已安装的注音词库（含停用项，只读，不改变启用状态）；不读取用户学习记录。", TextWrapping = TextWrapping.Wrap, Opacity = 0.75 };
    private readonly TextBlock _summary = new() { TextWrapping = TextWrapping.Wrap };
    private readonly InfoBar _notice = new() { IsOpen = false, IsClosable = false };
    private readonly ProgressBar _progress = new() { IsIndeterminate = true, Visibility = Visibility.Collapsed };
    private readonly ListView _list = new() { SelectionMode = ListViewSelectionMode.None, MaxHeight = 290, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly List<Row> _rows = [];
    private readonly CommandBar _selectionCommands = new() { DefaultLabelPosition = CommandBarDefaultLabelPosition.Right };
    private readonly ToggleSwitch _manual = new() { Header = "手动处理已跳过项", OnContent = "显示", OffContent = "隐藏" };
    private readonly Expander _preview = new() { Header = "查看自动补全明细", HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private CancellationTokenSource? _operation;
    private string? _referencePath;
    private int _page;
    private bool _closed;
    private sealed class Row(DictionaryEncodingSuggestion suggestion) {
        internal DictionaryEncodingSuggestion Suggestion = suggestion;
        internal string Code = suggestion.AutomaticCode ?? suggestion.Readings.FirstOrDefault()?.Code ?? "";
        internal bool Selected = suggestion.AutomaticCode is not null;
    }
    private DictionaryEncodingDialog(XamlRoot root, IReadOnlyList<DictionaryWord> pending, int padding) {
        _pending = pending;
        _limit.Value = pending.Count;
        _generate.Style = Application.Current.Resources["AccentButtonStyle"] as Style;
        // Keep native focus/disabled/high-contrast states while making this
        // primary action consistently blue instead of the user's accent hue.
        _generate.Resources = (ResourceDictionary)Microsoft.UI.Xaml.Markup.XamlReader.Load("""
            <ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                                xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml">
                <ResourceDictionary.ThemeDictionaries>
                    <ResourceDictionary x:Key="Light">
                        <SolidColorBrush x:Key="AccentButtonBackground" Color="#0067C0"/>
                        <SolidColorBrush x:Key="AccentButtonBackgroundPointerOver" Color="#1976C7"/>
                        <SolidColorBrush x:Key="AccentButtonBackgroundPressed" Color="#3185CE"/>
                        <SolidColorBrush x:Key="AccentButtonForeground" Color="White"/>
                        <SolidColorBrush x:Key="AccentButtonForegroundPointerOver" Color="White"/>
                        <SolidColorBrush x:Key="AccentButtonForegroundPressed" Color="#CCFFFFFF"/>
                    </ResourceDictionary>
                    <ResourceDictionary x:Key="Dark">
                        <SolidColorBrush x:Key="AccentButtonBackground" Color="#60CDFF"/>
                        <SolidColorBrush x:Key="AccentButtonBackgroundPointerOver" Color="#58BDEB"/>
                        <SolidColorBrush x:Key="AccentButtonBackgroundPressed" Color="#50AED9"/>
                        <SolidColorBrush x:Key="AccentButtonForeground" Color="Black"/>
                        <SolidColorBrush x:Key="AccentButtonForegroundPointerOver" Color="Black"/>
                        <SolidColorBrush x:Key="AccentButtonForegroundPressed" Color="#CC000000"/>
                    </ResourceDictionary>
                    <ResourceDictionary x:Key="HighContrast">
                        <StaticResource x:Key="AccentButtonBackground" ResourceKey="SystemControlHighlightAccentBrush"/>
                        <StaticResource x:Key="AccentButtonBackgroundPointerOver" ResourceKey="SystemControlForegroundAccentBrush"/>
                        <StaticResource x:Key="AccentButtonBackgroundPressed" ResourceKey="SystemControlBackgroundBaseMediumLowBrush"/>
                        <StaticResource x:Key="AccentButtonForeground" ResourceKey="SystemControlHighlightAltAccentBrush"/>
                        <StaticResource x:Key="AccentButtonForegroundPointerOver" ResourceKey="SystemControlBackgroundChromeWhiteBrush"/>
                        <StaticResource x:Key="AccentButtonForegroundPressed" ResourceKey="SystemControlHighlightBaseHighBrush"/>
                    </ResourceDictionary>
                </ResourceDictionary.ThemeDictionaries>
            </ResourceDictionary>
            """);
        var body = new StackPanel { Spacing = 10, MinWidth = 280, MaxWidth = 620 };
        body.Children.Add(new TextBlock { Text = "自动分批补全整本词库，不必逐条勾选。优先使用整词注音及能确定读音的词语组合；多音、冲突或未知项目默认跳过，可选手动补充。保存前不会写入，也不覆盖已有编码。", TextWrapping = TextWrapping.Wrap });
        if (padding > 0) body.Children.Add(new InfoBar { IsOpen = true, IsClosable = false, Severity = InfoBarSeverity.Informational,
            Message = $"检测到 {padding} 条旧版 THUOCL 词条带有格式空白，已按去除首尾空白的正文查询。仅保存选中项时同步清理；跳过或取消的词条保持原样。" });
        body.Children.Add(_reference); body.Children.Add(_limit);
        var commands = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        commands.Children.Add(_generate); commands.Children.Add(_pick); commands.Children.Add(_cancel); body.Children.Add(commands);
        commands.SizeChanged += (_, args) => commands.Orientation = args.NewSize.Width < 480 ? Orientation.Vertical : Orientation.Horizontal;
        body.Children.Add(_progress); body.Children.Add(_notice); body.Children.Add(_summary);
        var select = _selectionCommands;
        // Compact buttons wrap through CommandBar's overflow in narrow windows.
        void SelectionCommand(string label, Action action) {
            var command = new AppBarButton { Label = label, Icon = new SymbolIcon(Symbol.Accept) };
            command.Click += (_, _) => action(); select.PrimaryCommands.Add(command);
        }
        SelectionCommand("仅选自动确定项", () => Select(row => row.Suggestion.AutomaticCode is not null));
        SelectionCommand("全部取消", () => Select(_ => false));
        select.IsEnabled = false;
        body.Children.Add(select); body.Children.Add(_manual); _preview.Content = _list; body.Children.Add(_preview);
        var pages = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 }; pages.Children.Add(_previous); pages.Children.Add(_next); body.Children.Add(pages);
        _dialog = new() { XamlRoot = root, Title = "补充词库编码", Content = new ScrollViewer { Content = body, MaxHeight = 650, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled },
            PrimaryButtonText = "保存补全结果", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close, IsPrimaryButtonEnabled = false };
        AutomationProperties.SetAutomationId(_dialog, "DictionaryEncodingDialog");
        AutomationProperties.SetAutomationId(_generate, "DictionaryEncodingGenerate");
        AutomationProperties.SetAutomationId(_list, "DictionaryEncodingPreview");
        AutomationProperties.SetAutomationId(_manual, "DictionaryEncodingManualSkipped");
        _generate.Click += async (_, _) => await GenerateAsync();
        _pick.Click += async (_, _) => {
            try {
                var picker = new FileOpenPicker(); picker.FileTypeFilter.Add(".owolx"); DictionaryManager.InitializePicker(picker);
                var file = await picker.PickSingleFileAsync(); if (file is null) return;
                _referencePath = file.Path; _reference.Text = "参考词典：" + file.Path;
                _rows.Clear(); Render(); select.IsEnabled = false;
            } catch (Exception error) { Notice(error.Message); }
        };
        _cancel.Click += (_, _) => _operation?.Cancel();
        _manual.Toggled += (_, _) => { _page = 0; _preview.IsExpanded = _manual.IsOn; Render(); };
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
        if (!double.IsFinite(_limit.Value) || _limit.Value != Math.Truncate(_limit.Value) || _limit.Value is < 1 or > DictionaryStore.MaximumEntries) { Notice("请填写有效整数上限，最多 500000 条。"); return; }
        using var operation = new CancellationTokenSource(); _operation = operation;
        _selectionCommands.IsEnabled = false;
        _generate.IsEnabled = _pick.IsEnabled = _limit.IsEnabled = _manual.IsEnabled = false; _dialog.IsPrimaryButtonEnabled = false;
        _cancel.Visibility = _progress.Visibility = Visibility.Visible; _notice.IsOpen = false;
        _rows.Clear(); _page = 0; Render();
        try {
            var progress = new Progress<DictionaryEncodingProgress>(value => {
                if (_closed || !ReferenceEquals(_operation, operation)) return;
                _summary.Text = $"正在分析 {value.Completed} / {value.Total} 条……"; _reference.Text = value.Sources;
            });
            var suggestions = await new DictionaryEncodingClient().SuggestAsync(_pending.Take((int)_limit.Value).ToArray(), _referencePath, operation.Token, progress: progress);
            operation.Token.ThrowIfCancellationRequested();
            if (_closed) return;
            _rows.AddRange(suggestions.Select(value => new Row(value)));
            Notice("已自动选中能够确定读音的项目，统一保存即可。其余项目默认跳过；如有需要，打开“手动处理已跳过项”。", InfoBarSeverity.Informational);
        } catch (OperationCanceledException) { if (!_closed) Notice("生成已取消或超时，词库未修改。", InfoBarSeverity.Informational); }
        catch (Exception error) { if (!_closed) Notice(error.Message + " 可先启动 Core，或选择本机 .owolx 参考词典重试。"); }
        finally {
            _operation = null;
            if (!_closed) { _generate.IsEnabled = _pick.IsEnabled = _limit.IsEnabled = _manual.IsEnabled = true; _selectionCommands.IsEnabled = _rows.Count > 0; _cancel.Visibility = _progress.Visibility = Visibility.Collapsed; Render(); }
        }
    }
    private void Select(Func<Row, bool> match) { if (_operation is not null) return; foreach (var row in _rows) row.Selected = match(row); Render(); }
    private void UpdateSummary() {
        _summary.Text = $"待补 {_pending.Count} 条 · 已分析 {_rows.Count} 条 · 自动可补 {_rows.Count(row => row.Suggestion.AutomaticCode is not null)} 条 · 默认跳过 {_rows.Count(row => row.Suggestion.AutomaticCode is null)} 条 · 将保存 {_rows.Count(row => row.Selected)} 条";
        _dialog.IsPrimaryButtonEnabled = _operation is null && _rows.Any(row => row.Selected) && _rows.Where(row => row.Selected).All(row => !string.IsNullOrWhiteSpace(row.Code));
    }
    private void Render() {
        var visible = _rows.Where(row => _manual.IsOn == (row.Suggestion.AutomaticCode is null)).ToArray();
        _preview.Header = _manual.IsOn ? $"已跳过项（可选手动） · 第 {_page + 1} 页" : $"自动补全明细（无需逐条操作） · 第 {_page + 1} 页";
        _page = Math.Clamp(_page, 0, Math.Max(0, (visible.Length - 1) / 50));
        _list.Items.Clear();
        foreach (var row in visible.Skip(_page * 50).Take(50)) {
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
                $"{(row.Suggestion.AutomaticCode is not null ? "读音已确定，可自动补全" : row.Suggestion.WholeWord ? "多音／来源冲突，默认跳过" : "无法唯一确定，默认跳过")} · {row.Suggestion.Readings.Count} 种建议（最多 8 种）", Opacity = 0.7, TextWrapping = TextWrapping.Wrap });
            _list.Items.Add(new ListViewItem { Content = panel, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        }
        _previous.IsEnabled = _operation is null && _page > 0; _next.IsEnabled = _operation is null && (_page + 1) * 50 < visible.Length;
        UpdateSummary();
    }
    internal static async Task<DictionaryEncodingResult?> SupplementAsync(XamlRoot root, ManagedDictionary dictionary,
        IReadOnlyList<DictionaryWord>? pending = null, bool autoStart = false) {
        pending ??= dictionary.Entries.Where(word => !DictionaryStore.Usable(word)).ToArray();
        if (pending.Count == 0) throw new InvalidOperationException("没有待补编码词条；已有编码不会被覆盖。");
        var prepared = pending.Select(word => DictionaryEncoding.PrepareWord(dictionary, word)).ToArray();
        var padding = pending.Zip(prepared).Count(pair => pair.First.Text != pair.Second.Text);
        if (await ShowAsync(root, prepared, padding, autoStart) is not { } selected) return null;
        return DictionaryEncoding.Apply(dictionary, selected);
    }
    internal static async Task<IReadOnlyDictionary<string, string>?> ShowAsync(XamlRoot root, IReadOnlyList<DictionaryWord> pending, int padding = 0, bool autoStart = false) {
        var view = new DictionaryEncodingDialog(root, pending, padding);
        if (autoStart) view._dialog.Opened += async (_, _) => await view.GenerateAsync();
        if (await view._dialog.ShowAsync() != ContentDialogResult.Primary) return null;
        return view._rows.Where(row => row.Selected).ToDictionary(row => row.Suggestion.Word.Id, row => row.Code, StringComparer.Ordinal);
    }
}
