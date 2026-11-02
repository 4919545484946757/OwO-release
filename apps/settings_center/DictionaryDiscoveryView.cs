using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

internal sealed class DictionaryDiscoveryView : UserControl
{
    private readonly DictionarySources _client = new();
    private readonly ComboBox _source = new() { Header = "词库来源", ItemsSource = DictionarySources.Sources, DisplayMemberPath = "Name", SelectedIndex = 0, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox _search = new() { PlaceholderText = "搜索当前来源的词库名称、文件名或分类" };
    private readonly NumberBox _maximum = new() { Header = "本次最多选取词条（按文件顺序，不代表质量排名）", Minimum = 1, Maximum = 100000, Value = 10000, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline };
    private readonly StackPanel _items = new() { Spacing = 10 };
    private readonly TextBlock _description = new() { TextWrapping = TextWrapping.Wrap };
    private readonly InfoBar _status = new() { IsOpen = true, IsClosable = false, Message = "选择来源后刷新目录；仅显式下载时联网，不上传输入、配置或本机词库。" };
    private readonly Button _refresh = new() { Content = "刷新在线目录" }, _cancel = new() { Content = "取消", IsEnabled = false };
    private readonly ProgressBar _progress = new() { Minimum = 0, Maximum = 100, Visibility = Visibility.Collapsed };
    private readonly Func<string, PreparedDictionary, Task> _import;
    private IReadOnlyList<DictionaryDownload> _downloads = [];
    private CancellationTokenSource? _operation;
    internal DictionaryDiscoveryView(Func<string, PreparedDictionary, Task> import) {
        _import = import;
        var body = new StackPanel { Spacing = 12 };
        body.Children.Add(_source); body.Children.Add(_description); body.Children.Add(_search); body.Children.Add(_maximum);
        var commands = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 }; commands.Children.Add(_refresh); commands.Children.Add(_cancel);
        body.Children.Add(commands); body.Children.Add(_progress); body.Children.Add(_status); body.Children.Add(_items);
        Content = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled };
        _source.SelectionChanged += (_, _) => { _downloads = []; Render(); Describe(); };
        _search.TextChanged += (_, _) => Render(); Describe();
        _refresh.Click += async (_, _) => await RunAsync(async token => {
            using var deadline = CancellationTokenSource.CreateLinkedTokenSource(token); deadline.CancelAfter(TimeSpan.FromSeconds(40));
            _downloads = await _client.ListAsync((DictionarySource)_source.SelectedItem, deadline.Token); Render(); Notice($"已获取 {_downloads.Count} 个数据文件。超过大小上限的文件仅供查看，不会降低限制。");
        });
        _cancel.Click += (_, _) => _operation?.Cancel();
        Unloaded += (_, _) => _operation?.Cancel();
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_refresh, "DictionaryCatalogRefresh");
    }
    private void Describe() {
        if (_source.SelectedItem is DictionarySource source) _description.Text = $"{source.Description}\n许可：{source.License}\nGitHub 文件核对上游摘要；CC-CEDICT 仅 HTTPS 与本地摘要，无上游独立校验和。\n导入频率统一为中性权重 1，不伪造为个人学习记录。";
    }
    private void Notice(string message, InfoBarSeverity severity = InfoBarSeverity.Informational) { _status.Message = message; _status.Severity = severity; }
    private void Render() {
        _items.Children.Clear();
        foreach (var item in _downloads.Where(d => (d.Title + " " + d.Name).Contains(_search.Text.Trim(), StringComparison.OrdinalIgnoreCase))) {
            var content = new StackPanel { Spacing = 8 };
            content.Children.Add(new TextBlock { Text = item.Title, FontSize = 18, TextWrapping = TextWrapping.Wrap });
            if (item.Title != item.Name) content.Children.Add(new TextBlock { Text = item.Name, TextWrapping = TextWrapping.Wrap });
            content.Children.Add(new TextBlock { Text = $"{(item.Size > 0 ? $"{item.Size / 1048576.0:F1} MiB" : "动态更新数据")} · {item.Source.License}", TextWrapping = TextWrapping.Wrap });
            var commands = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            var download = new Button { Content = item.Size > DictionarySources.DownloadLimit ? "超过 128 MiB 上限" : "下载并预览", IsEnabled = item.Size <= DictionarySources.DownloadLimit && _operation is null };
            download.Click += async (_, _) => await RunAsync(async token => {
                if (!double.IsFinite(_maximum.Value) || _maximum.Value < 1 || _maximum.Value > 100000) throw new InvalidDataException("请填写 1–100000 的选取上限。");
                var accepted = await new ContentDialog { XamlRoot = XamlRoot, Title = "下载第三方词库", Content = new TextBlock { Text = $"来源：{item.Source.Name}\n许可：{item.Source.License}\n文件：{item.Name}\n\n请阅读上游许可。下载不表示已完成授权审查，也不会启用词库。仅选取设置上限内的数据，其余数量会在导入前明确显示。", TextWrapping = TextWrapping.Wrap }, PrimaryButtonText = "下载并转换", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close }.ShowAsync();
                if (accepted != ContentDialogResult.Primary) return;
                _progress.Visibility = Visibility.Visible; _progress.Value = 0; Notice("正在下载、核对并转换数据……");
                var data = await _client.FetchAsync(item, (int)_maximum.Value, new Progress<double>(v => _progress.Value = v), token);
                token.ThrowIfCancellationRequested(); await _import(item.Source.Name + " · " + item.Name, data);
            });
            var sourcePage = item.Source.Repository == "" ? "https://www.mdbg.net/chinese/dictionary?page=cedict" : "https://github.com/" + item.Source.Repository;
            commands.Children.Add(download); commands.Children.Add(new HyperlinkButton { Content = "来源与许可", NavigateUri = new(sourcePage) });
            content.Children.Add(commands);
            var card = (Border)Microsoft.UI.Xaml.Markup.XamlReader.Load("""
                <Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" Padding="16" CornerRadius="8" BorderThickness="1" Background="{ThemeResource CardBackgroundFillColorDefaultBrush}" BorderBrush="{ThemeResource CardStrokeColorDefaultBrush}"/>
                """); card.Child = content; _items.Children.Add(card);
        }
    }
    private async Task RunAsync(Func<CancellationToken, Task> action) {
        if (_operation is not null) return;
        using var operation = new CancellationTokenSource(TimeSpan.FromMinutes(15)); _operation = operation;
        _source.IsEnabled = false; _refresh.IsEnabled = false; _maximum.IsEnabled = false; _cancel.IsEnabled = true; Render();
        try { await action(operation.Token); }
        catch (OperationCanceledException) { Notice("已取消或超时，未发布不完整词库。"); }
        catch (Exception error) { Notice(error.Message + " 可刷新目录后重试。", InfoBarSeverity.Error); }
        finally { _operation = null; _source.IsEnabled = true; _refresh.IsEnabled = true; _maximum.IsEnabled = true; _cancel.IsEnabled = false; _progress.Visibility = Visibility.Collapsed; Render(); }
    }
}
