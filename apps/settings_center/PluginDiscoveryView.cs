using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;

namespace OwO_Settings;

internal sealed class PluginDiscoveryView : UserControl
{
    private readonly OnlineDistribution _client = new();
    private readonly StackPanel _cards = new() { Spacing = 10 };
    private readonly Action<string, InfoBarSeverity> _reportStatus;
    private readonly DownloadProgressView _progress = new("PluginDownload");
    private readonly Func<string, CancellationToken, DownloadOperation> _beginDownload;
    private DownloadOperation? _download;
    private readonly Button _refresh = new() { Content = "刷新在线目录" };
    private readonly CheckBox _updatesOnly = new() { Content = "仅显示可更新", VerticalAlignment = VerticalAlignment.Center };
    private readonly List<Button> _actions = [];
    private readonly Func<string, string, Task> _install;
    private readonly Action _showInstalled;
    private IReadOnlyList<OnlineResource> _resources = [];
    private IReadOnlyList<PluginVersionSnapshot> _installed = [];
    private CancellationTokenSource? _operation;
    private string _query = "";
    private readonly string _downloads = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OwO", "InputMethod", "downloads");

    internal PluginDiscoveryView(Func<string, string, Task> install, Action showInstalled, Action<string, InfoBarSeverity> reportStatus, Func<string, CancellationToken, DownloadOperation> beginDownload) {
        _install = install; _showInstalled = showInstalled; _reportStatus = reportStatus; _beginDownload = beginDownload;
        var body = new StackPanel { Spacing = 12 };
        body.Children.Add(new TextBlock { Text = $"官方插件目录 · {OnlineDistribution.Repository}", TextWrapping = TextWrapping.Wrap });
        var toolbar = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        toolbar.Children.Add(_refresh); body.Children.Add(toolbar);
        body.Children.Add(_updatesOnly); body.Children.Add(_progress); body.Children.Add(_cards);
        Content = body;
        AutomationProperties.SetAutomationId(_refresh, "PluginCatalogRefresh");
        _refresh.Click += async (_, _) => await RunAsync(async token => {
            Notice("正在查询官方插件目录……");
            using var query = CancellationTokenSource.CreateLinkedTokenSource(token); query.CancelAfter(TimeSpan.FromSeconds(30));
            var release = await _client.LatestAsync(query.Token);
            var resources = await _client.ResourcesAsync(release, query.Token);
            _resources = resources.Where(r => r.Kind == "plugin").ToList(); Render();
            Notice($"目录 {release.Tag} · {_resources.Count} 个插件。下载会校验大小及 SHA-256，安装仍需独立确认。", InfoBarSeverity.Success);
        });
        _updatesOnly.Checked += (_, _) => Render(); _updatesOnly.Unchecked += (_, _) => Render();
        Unloaded += (_, _) => { if (_download is null) _operation?.Cancel(); };
    }
    internal void SetInstalled(IReadOnlyList<PluginVersionSnapshot> values) { _installed = values; Render(); }
    internal void SetQuery(string query) { _query = query.Trim(); Render(); }
    private void Notice(string message, InfoBarSeverity severity = InfoBarSeverity.Informational) {
        _reportStatus(message, severity);
    }
    private void Render() {
        _cards.Children.Clear(); _actions.Clear();
        var matched = _resources.Where(r => $"{r.Name} {r.Id} {r.Version}".Contains(_query, StringComparison.OrdinalIgnoreCase))
            .Where(r => _updatesOnly.IsChecked != true || PluginManagerCatalog.OnlineState(r, _installed) == "可更新").ToList();
        foreach (var resource in matched) {
            var state = PluginManagerCatalog.OnlineState(resource, _installed);
            var content = new StackPanel { Spacing = 8 };
            content.Children.Add(new TextBlock { Text = resource.Name, FontSize = 18, TextWrapping = TextWrapping.Wrap });
            content.Children.Add(new TextBlock { Text = $"{resource.Id}\n{resource.Version} · {state} · {resource.Asset.Size / 1048576.0:F1} MiB", TextWrapping = TextWrapping.Wrap });
            var commands = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            var download = new Button { Content = state == "可更新" ? "下载更新" : state == "可安装" ? "下载并安装" : "管理本机版本", IsEnabled = _operation is null };
            download.Click += async (_, _) => {
                if (state is not ("可安装" or "可更新")) { _showInstalled(); return; }
                await RunAsync(async token => {
                    var detail = $"{resource.Name} {resource.Version}\n许可：{resource.License}\n文件：{resource.Asset.Name}\nSHA-256：{resource.Asset.Sha256}\n\n仅下载和校验。随后会预检真实包权限并单独征求安装授权；不会沿用旧版授权。";
                    if (!await ConfirmAsync("下载插件", detail, "下载并校验")) return;
                    token.ThrowIfCancellationRequested();
                    using var downloadTask = _beginDownload(resource.Asset.Name, token); _download = downloadTask; _progress.Bind(downloadTask);
                    string path;
                    try { path = await downloadTask.RunAsync(ct => _client.DownloadAsync(resource.Asset, _downloads, null, ct, downloadTask)); }
                    finally { _download = null; _progress.Bind(null); }
                    token.ThrowIfCancellationRequested();
                    Notice("下载校验通过，请在安装对话框中核对权限。", InfoBarSeverity.Success);
                    await _install(resource.Name, path);
                });
            };
            var details = new Button { Content = "详情", IsEnabled = _operation is null };
            details.Click += async (_, _) => await RunAsync(async _ => {
                await ConfirmAsync("插件详情", $"{resource.Name}\n{resource.Id} {resource.Version}\n状态：{state}\n许可：{resource.License}\n来源：{OnlineDistribution.Repository}\nSHA-256：{resource.Asset.Sha256}\n\n目录不授予任何权限。实际申请的权限与信任状态在下载后的包预检中显示。", "知道了");
            });
            _actions.Add(download); _actions.Add(details); commands.Children.Add(download); commands.Children.Add(details); content.Children.Add(commands);
            var card = (Border)Microsoft.UI.Xaml.Markup.XamlReader.Load("""
                <Border xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
                        Padding="16" CornerRadius="8" BorderThickness="1"
                        Background="{ThemeResource CardBackgroundFillColorDefaultBrush}"
                        BorderBrush="{ThemeResource CardStrokeColorDefaultBrush}" />
                """);
            card.Child = content;
            _cards.Children.Add(card);
        }
        if (_resources.Count != 0 && matched.Count == 0) _cards.Children.Add(new TextBlock { Text = "没有匹配的在线插件。" });
    }
    private async Task RunAsync(Func<CancellationToken, Task> action) {
        if (_operation is not null) return;
        using var operation = new CancellationTokenSource(); _operation = operation;
        _refresh.IsEnabled = false; foreach (var actionButton in _actions) actionButton.IsEnabled = false;
        try { await action(operation.Token); }
        catch (OperationCanceledException) { Notice("已取消或超时，本机插件保持原状态。可刷新目录或重试。"); }
        catch (Exception error) { Notice($"操作失败：{error.Message} 可重试；不会自动降低校验要求。", InfoBarSeverity.Error); }
        finally { _operation = null; _refresh.IsEnabled = true; _progress.Bind(null); Render(); }
    }
    private async Task<bool> ConfirmAsync(string title, string text, string primary) =>
        await new ContentDialog { XamlRoot = XamlRoot, Title = title,
            Content = new ScrollViewer { Content = new TextBlock { Text = text, TextWrapping = TextWrapping.Wrap }, MaxHeight = 440 },
            PrimaryButtonText = primary, CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close }.ShowAsync() == ContentDialogResult.Primary;
}
