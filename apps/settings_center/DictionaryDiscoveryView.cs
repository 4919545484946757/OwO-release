using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

internal sealed class DictionaryDiscoveryView : UserControl
{
    private readonly DictionarySources _client = new();
    private DictionarySource? _source;
    private readonly Grid _sourcePage = new() { RowSpacing = 12 };
    private readonly Grid _sourceCards = new() { ColumnSpacing = 12, RowSpacing = 12 };
    private readonly Grid _detailPage = new() { RowSpacing = 12, Visibility = Visibility.Collapsed };
    private readonly TextBlock _sourceTitle = new() { FontSize = 18, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center };
    private readonly Button _back = new() { Content = "返回词库来源" };
    private readonly TextBox _search = new() { PlaceholderText = "搜索当前来源的词库名称、文件名或分类" };
    private readonly NumberBox _maximum = new() { Header = "本次最多选取词条（按文件顺序，不代表质量排名）", Minimum = 1, Maximum = DictionaryStore.MaximumEntries, Value = 50000, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
    private readonly Grid _items = new() { ColumnSpacing = 12, RowSpacing = 12 };
    private readonly TextBlock _empty = new() { Text = "没有匹配的词库，试试清空搜索。", TextWrapping = TextWrapping.Wrap };
    private readonly Dictionary<string, Grid> _cardCache = new();
    private readonly Dictionary<string, Button> _downloadButtons = new();
    private readonly TextBlock _description = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Action<string, InfoBarSeverity> _reportStatus;
    private readonly DownloadProgressView _downloadProgress = new("DictionaryDownload");
    private readonly ProgressBar _progress = new() { IsIndeterminate = true, Visibility = Visibility.Collapsed };
    private readonly Func<string, CancellationToken, DownloadOperation> _beginDownload;
    private DownloadOperation? _download;
    private readonly Func<string, PreparedDictionary, Task> _import;
    private IReadOnlyList<DictionaryDownload> _downloads = [];
    private CancellationTokenSource? _operation;
    private CancellationTokenSource? _listing;
    private bool _started;
    internal async Task EnsureLoadedAsync() {
        if (_source is not null && (!_started || _listing?.IsCancellationRequested == true || _downloads.Count == 0 && _listing is null && _operation is null)) await RefreshDirectoryAsync();
    }
    internal void CancelPending() { _listing?.Cancel(); if (_download is null) _operation?.Cancel(); }
    internal DictionaryDiscoveryView(Func<string, PreparedDictionary, Task> import, Action<string, InfoBarSeverity> reportStatus, Func<string, CancellationToken, DownloadOperation> beginDownload) {
        _import = import; _reportStatus = reportStatus; _beginDownload = beginDownload;
        var body = _detailPage;
        for (var index = 0; index < 3; index++) body.RowDefinitions.Add(new() { Height = GridLength.Auto });
        body.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var toolbar = new Grid { ColumnSpacing = 12, RowSpacing = 8 };
        toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto }); toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(2, GridUnitType.Star) });
        _search.VerticalAlignment = VerticalAlignment.Bottom;
        toolbar.Children.Add(_back); Grid.SetColumn(_sourceTitle, 1); toolbar.Children.Add(_sourceTitle);
        Grid.SetRow(_search, 1); Grid.SetColumnSpan(_search, 3); toolbar.Children.Add(_search);
        body.Children.Add(toolbar);
        var settings = new StackPanel { Spacing = 10 }; settings.Children.Add(_description); settings.Children.Add(_maximum);
        var options = new Expander { Header = "来源说明与导入选项", Content = settings, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetRow(options, 1); body.Children.Add(options);
        var progress = new StackPanel(); progress.Children.Add(_progress); progress.Children.Add(_downloadProgress);
        Grid.SetRow(progress, 2); body.Children.Add(progress);
        var scroll = new ScrollViewer { Content = _items, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        scroll.LayoutUpdated += (_, _) => {
            var width = scroll.ViewportWidth;
            if (width > 0 && double.IsFinite(width) && (double.IsNaN(_items.Width) || Math.Abs(_items.Width - width) > 0.5)) {
                _items.Width = width; DictionaryCardLayout.Arrange(_items);
            }
        };
        _items.SizeChanged += (_, _) => DictionaryCardLayout.Arrange(_items);
        Grid.SetRow(scroll, 3); body.Children.Add(scroll);
        BuildSourcePage();
        var pages = new Grid(); pages.Children.Add(_sourcePage); pages.Children.Add(_detailPage); Content = pages;
        _back.Click += (_, _) => {
            CancelPending(); _source = null; _started = false;
            _detailPage.Visibility = Visibility.Collapsed; _sourcePage.Visibility = Visibility.Visible;
        };
        _search.TextChanged += (_, _) => Render(); Describe();
        Unloaded += (_, _) => CancelPending();
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_back, "DictionarySourceBack");
    }
    private void BuildSourcePage() {
        _sourcePage.RowDefinitions.Add(new() { Height = GridLength.Auto });
        _sourcePage.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        _sourcePage.Children.Add(new TextBlock { Text = "选择词库来源", FontSize = 20 });
        foreach (var source in DictionarySources.Sources) {
            var content = new StackPanel { Spacing = 10 };
            content.Children.Add(new SymbolIcon(Symbol.Library) { HorizontalAlignment = HorizontalAlignment.Left });
            content.Children.Add(new TextBlock { Text = source.Name, FontSize = 18, TextWrapping = TextWrapping.Wrap, MaxLines = 2, TextTrimming = TextTrimming.CharacterEllipsis });
            content.Children.Add(new TextBlock { Text = source.Description, TextWrapping = TextWrapping.Wrap, MaxLines = 3, TextTrimming = TextTrimming.CharacterEllipsis, Opacity = 0.75 });
            content.Children.Add(new TextBlock { Text = source.License, FontSize = 12, TextWrapping = TextWrapping.Wrap, MaxLines = 2, TextTrimming = TextTrimming.CharacterEllipsis, Opacity = 0.7 });
            var card = new Button { Content = content, Height = DictionaryCardLayout.Height, Padding = new(16), CornerRadius = new(10), MinWidth = 0,
                HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(card, "DictionarySource_" + source.Id);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(card, source.Name);
            ToolTipService.SetToolTip(card, source.Description + "\n许可：" + source.License);
            card.Click += async (_, _) => {
                CancelPending(); _source = source; _started = false;
                _downloads = []; _cardCache.Clear(); _downloadButtons.Clear(); _search.Text = "";
                _sourceTitle.Text = source.Name; Describe(); Render();
                _sourcePage.Visibility = Visibility.Collapsed; _detailPage.Visibility = Visibility.Visible;
                await RefreshDirectoryAsync();
            };
            _sourceCards.Children.Add(card);
        }
        var scroll = new ScrollViewer { Content = _sourceCards, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        scroll.LayoutUpdated += (_, _) => {
            var width = scroll.ViewportWidth;
            if (width > 0 && double.IsFinite(width) && (double.IsNaN(_sourceCards.Width) || Math.Abs(_sourceCards.Width - width) > 0.5)) {
                _sourceCards.Width = width; DictionaryCardLayout.Arrange(_sourceCards);
            }
        };
        _sourceCards.SizeChanged += (_, _) => DictionaryCardLayout.Arrange(_sourceCards);
        Grid.SetRow(scroll, 1); _sourcePage.Children.Add(scroll);
    }
    private async Task RefreshDirectoryAsync() {
        if (_source is not { } source) return;
        _listing?.Cancel();
        using var request = new CancellationTokenSource(TimeSpan.FromSeconds(40)); _listing = request; _started = true;
        _downloads = []; Render(); Notice($"{source.Name} · 正在获取目录……");
        _progress.IsIndeterminate = true; _progress.Visibility = Visibility.Visible;
        try {
            var files = await _client.ListAsync(source, request.Token);
            if (!ReferenceEquals(_listing, request) || request.IsCancellationRequested || !Equals(_source, source)) return;
            _downloads = files; Notice($"{source.Name} · 已获取 {files.Count} 个数据文件。");
        } catch (OperationCanceledException) {
            if (ReferenceEquals(_listing, request)) Notice("目录请求已取消或超时，可切换来源重试。");
        } catch (Exception error) {
            if (ReferenceEquals(_listing, request)) Notice(error.Message + " 可切换来源重试。", InfoBarSeverity.Error);
        } finally {
            if (ReferenceEquals(_listing, request)) { _listing = null; _progress.Visibility = Visibility.Collapsed; _progress.IsIndeterminate = false; Render(); }
        }
    }
    private void Describe() {
        if (_source is { } source) _description.Text = $"{source.Description}\n许可：{source.License}\nGitHub 文件核对上游摘要；CC-CEDICT 仅 HTTPS 与本地摘要，无上游独立校验和。\n导入频率统一为中性权重 1，不伪造为个人学习记录。";
    }
    private void Notice(string message, InfoBarSeverity severity = InfoBarSeverity.Informational) => _reportStatus(message, severity);
    private void Render() {
        var desired = new List<UIElement>();
        foreach (var item in _downloads.Where(d => (d.Title + " " + d.Name).Contains(_search.Text.Trim(), StringComparison.OrdinalIgnoreCase))) {
            var key = item.Source.Id + ":" + item.Name + ":" + item.Hash;
            if (_cardCache.TryGetValue(key, out var cached)) {
                _downloadButtons[key].IsEnabled = item.Size <= DictionarySources.DownloadLimit && _operation is null;
                desired.Add(cached); continue;
            }
            var content = new StackPanel { Spacing = 8 };
            content.Children.Add(new SymbolIcon(Symbol.Library) { HorizontalAlignment = HorizontalAlignment.Left });
            content.Children.Add(new TextBlock { Text = item.Title, FontSize = 18, TextTrimming = TextTrimming.CharacterEllipsis });
            content.Children.Add(new TextBlock { Text = item.Source.Name + " · " + item.Name, MaxLines = 2, TextTrimming = TextTrimming.CharacterEllipsis, TextWrapping = TextWrapping.Wrap, Opacity = 0.75 });
            content.Children.Add(new TextBlock { Text = $"{(item.Size > 0 ? $"{item.Size / 1048576.0:F1} MiB" : "动态更新数据")} · {item.Source.License}", MaxLines = 2, TextTrimming = TextTrimming.CharacterEllipsis, FontSize = 12, TextWrapping = TextWrapping.Wrap, Opacity = 0.7 });
            var download = new Button { Content = item.Size > DictionarySources.DownloadLimit ? "文件超过上限" : "下载并预览", MinWidth = 0, Margin = new Thickness(13, 0, 13, 0), HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Center, IsEnabled = item.Size <= DictionarySources.DownloadLimit && _operation is null };
            download.Click += async (_, _) => await RunAsync(async token => {
                if (!double.IsFinite(_maximum.Value) || _maximum.Value < 1 || _maximum.Value > DictionaryStore.MaximumEntries) throw new InvalidDataException($"请填写 1–{DictionaryStore.MaximumEntries} 的选取上限。");
                var accepted = await new ContentDialog { XamlRoot = XamlRoot, Title = "下载第三方词库", Content = new TextBlock { Text = $"来源：{item.Source.Name}\n许可：{item.Source.License}\n文件：{item.Name}\n\n请阅读上游许可。下载不表示已完成授权审查，也不会启用词库。仅选取设置上限内的数据，其余数量会在导入前明确显示。", TextWrapping = TextWrapping.Wrap }, PrimaryButtonText = "下载并转换", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close }.ShowAsync();
                if (accepted != ContentDialogResult.Primary) return;
                using var download = _beginDownload(item.Name, token); _download = download; _downloadProgress.Bind(download);
                PreparedDictionary data;
                try { data = await download.RunAsync(ct => _client.FetchAsync(item, (int)_maximum.Value, null, ct, download)); }
                finally { _download = null; _downloadProgress.Bind(null); }
                token.ThrowIfCancellationRequested(); await _import(item.Source.Name + " · " + item.Name, data);
            });
            var sourcePage = item.Source.Repository == "" ? "https://www.mdbg.net/chinese/dictionary?page=cedict" : "https://github.com/" + item.Source.Repository;
            async Task DetailsAsync() => await RunAsync(async _ => {
                var details = new StackPanel { Spacing = 12 };
                details.Children.Add(new TextBlock { Text = $"{item.Title}\n{item.Name}\n{item.Source.Name}\n许可：{item.Source.License}\n\n{item.Source.Description}\n\n下载后仍需预览确认，默认不启用。", TextWrapping = TextWrapping.Wrap });
                details.Children.Add(new HyperlinkButton { Content = "打开上游来源与许可", NavigateUri = new(sourcePage) });
                await new ContentDialog { XamlRoot = XamlRoot, Title = "词库详情", Content = new ScrollViewer { Content = details, MaxHeight = 420 }, CloseButtonText = "关闭" }.ShowAsync();
            });
            var card = new Button { Content = content, Padding = new Thickness(16, 16, 16, 72), CornerRadius = new(10), BorderThickness = new(0), MinWidth = 0, HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top };
            card.Click += async (_, _) => await DetailsAsync();
            var host = new Grid { Height = DictionaryCardLayout.Height }; host.Children.Add(card);
            var brush = new Microsoft.UI.Xaml.Media.SolidColorBrush(Windows.UI.Color.FromArgb(120, 0, 0, 0));
            host.Children.Add(new Border { Height = 64, VerticalAlignment = VerticalAlignment.Bottom, Margin = new(3, 0, 3, 3), CornerRadius = new(0, 0, 7, 7), Background = brush, Child = download });
            var flyout = new MenuFlyout(); var entry = new MenuFlyoutItem { Text = "详情与许可", Icon = new SymbolIcon(Symbol.Document) }; entry.Click += async (_, _) => await DetailsAsync(); flyout.Items.Add(entry);
            var menu = new Button { Content = new FontIcon { Glyph = "\uE712", FontSize = 16, IsTextScaleFactorEnabled = false }, Flyout = flyout, Padding = new(0), Width = 32, Height = 32, MinWidth = 0, MinHeight = 0, HorizontalContentAlignment = HorizontalAlignment.Center, VerticalContentAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top, Margin = new(8) };
            host.Children.Add(menu); host.Children.Add(new Border { BorderBrush = brush, BorderThickness = new(3), CornerRadius = new(10), IsHitTestVisible = false });
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(host, "OnlineDictionaryCard_" + item.Source.Id + "_" + item.Name);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(card, item.Title);
            ToolTipService.SetToolTip(card, item.Title + "\n" + item.Name);
            _cardCache[key] = host; _downloadButtons[key] = download; desired.Add(host);
        }
        if (_downloads.Count > 0 && desired.Count == 0) desired.Add(_empty);
        // Keep unchanged card instances during progress, refresh and tab switches.
        for (var index = _items.Children.Count - 1; index >= 0; index--)
            if (!desired.Contains(_items.Children[index])) _items.Children.RemoveAt(index);
        for (var index = 0; index < desired.Count; index++) {
            if (index < _items.Children.Count && ReferenceEquals(_items.Children[index], desired[index])) continue;
            if (_items.Children.Contains(desired[index])) _items.Children.Remove(desired[index]);
            _items.Children.Insert(index, desired[index]);
        }
        DictionaryCardLayout.Arrange(_items);
    }
    private async Task RunAsync(Func<CancellationToken, Task> action) {
        if (_operation is not null || _listing is not null) return;
        using var operation = new CancellationTokenSource(); _operation = operation;
        _back.IsEnabled = false; _maximum.IsEnabled = false; Render();
        try { await action(operation.Token); }
        catch (OperationCanceledException) { Notice("已取消或超时，未发布不完整词库。"); }
        catch (Exception error) { Notice(error.Message + " 可返回来源重新进入后重试。", InfoBarSeverity.Error); }
        finally { _operation = null; _back.IsEnabled = true; _maximum.IsEnabled = true; _progress.Visibility = Visibility.Collapsed; Render(); }
    }
}
