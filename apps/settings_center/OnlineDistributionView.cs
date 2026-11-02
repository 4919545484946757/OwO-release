using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Diagnostics;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Security.Cryptography;

namespace OwO_Settings;

internal sealed class OnlineDistributionView : UserControl
{
    private readonly OnlineDistribution _client = new();
    private readonly StackPanel _items = new() { Spacing = 8 };
    private readonly InfoBar _status = new() { IsOpen = false, IsClosable = true };
    private readonly ProgressBar _progress = new() { Minimum = 0, Maximum = 100, Visibility = Visibility.Collapsed };
    private readonly Button _check = new() { Content = "检查 GitHub 更新与资源" }, _cancel = new() { Content = "取消", IsEnabled = false };
    private readonly Func<string, string, Task> _plugin, _dictionary;
    private readonly List<Button> _actions = [];
    private CancellationTokenSource? _operation;
    private readonly string _downloads = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OwO", "InputMethod", "downloads");
    private static readonly Version Current = new((Assembly.GetExecutingAssembly().GetName().Version ?? new(0, 0, 0)).ToString(3));
    internal OnlineDistributionView(Func<string, string, Task> plugin, Func<string, string, Task> dictionary) {
        _plugin = plugin; _dictionary = dictionary;
        var body = new StackPanel { Spacing = 10 };
        body.Children.Add(new TextBlock { Text = "更新与资源下载", FontSize = 20 });
        body.Children.Add(new TextBlock { Text = $"当前版本 {Current} · 来源 {OnlineDistribution.Repository}\n联网仅用于查询版本及下载资源，不上传输入内容。下载校验不能替代发布者签名与插件权限审核。", TextWrapping = TextWrapping.Wrap });
        var commands = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        commands.Children.Add(_check); commands.Children.Add(_cancel); body.Children.Add(commands);
        body.Children.Add(_progress); body.Children.Add(_status); body.Children.Add(_items);
        Content = new Border { Child = body, Padding = new(16), CornerRadius = new(8), BorderThickness = new(1) };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_check, "OnlineCheckUpdates");
        _check.Click += async (_, _) => await RunAsync(CheckAsync);
        _cancel.Click += (_, _) => _operation?.Cancel();
        Unloaded += (_, _) => _operation?.Cancel();
    }
    private void Notice(string message, InfoBarSeverity severity = InfoBarSeverity.Informational) {
        _status.Message = message; _status.Severity = severity; _status.IsOpen = true;
    }
    private async Task RunAsync(Func<CancellationToken, Task> action) {
        if (_operation is not null) return;
        using var operation = new CancellationTokenSource(TimeSpan.FromMinutes(15)); _operation = operation;
        _check.IsEnabled = false; _cancel.IsEnabled = true; foreach (var button in _actions) button.IsEnabled = false;
        try { await action(operation.Token); }
        catch (OperationCanceledException) { Notice("操作已取消或超时；原安装和已启用资源保持不变。"); }
        catch (Exception error) { Notice($"联网操作失败：{error.Message} 可重新检查或重试。", InfoBarSeverity.Error); }
        finally { _operation = null; _check.IsEnabled = true; _cancel.IsEnabled = false; _progress.Visibility = Visibility.Collapsed; foreach (var button in _actions) button.IsEnabled = true; }
    }
    private async Task CheckAsync(CancellationToken token) {
        Notice("正在查询官方 GitHub 正式版本……");
        using var query = CancellationTokenSource.CreateLinkedTokenSource(token); query.CancelAfter(TimeSpan.FromSeconds(30));
        var release = await _client.LatestAsync(query.Token);
        var resources = await _client.ResourcesAsync(release, query.Token);
        _items.Children.Clear(); _actions.Clear();
        var isNativeX64 = RuntimeInformation.OSArchitecture == Architecture.X64;
        if (release.Version > Current) {
            Notice($"发现新版本 {release.Tag}。下载后仍需确认启动安装助手；不会自动覆盖当前程序。");
            var expected = $"OwO-Input-Method-{release.Version.ToString(3)}-windows-x64-Setup.exe";
            var setup = release.Assets.SingleOrDefault(a => a.Name == expected);
            if (isNativeX64 && setup is not null && setup.Sha256.Length == 64)
                AddAction($"下载并安装 {release.Tag}（{setup.Size / 1048576.0:F1} MiB）", async ct => await InstallUpdateAsync(setup, ct));
            else _items.Children.Add(new TextBlock { Text = "此版本未提供当前架构可校验的安装包；请在官方发行页检查。", TextWrapping = TextWrapping.Wrap });
        } else Notice(release.Version < Current ? $"官方最新版本为 {release.Tag}，低于本机 {Current}，不会降级。" : "当前已是最新正式版本。");
        var page = new HyperlinkButton { Content = "打开官方发行页", NavigateUri = release.Page };
        _items.Children.Add(page);
        foreach (var resource in resources) {
            var label = resource.Kind switch { "plugin" => "插件", "dictionary" => "词库", _ => "模型（下载后不自动切换）" };
            _items.Children.Add(new TextBlock { Text = $"{label} · {resource.Name} {resource.Version}\n许可：{resource.License} · {resource.Asset.Size / 1048576.0:F1} MiB", TextWrapping = TextWrapping.Wrap });
            AddAction($"下载{label}", async ct => {
                if (!await ConfirmAsync("下载资源", $"{resource.Name}\n{resource.Asset.Name}\n许可：{resource.License}\nSHA-256：{resource.Asset.Sha256}\n\n下载后仍需单独确认安装、导入或模型切换。", "下载并校验")) return;
                var path = await DownloadAsync(resource.Asset, ct);
                ct.ThrowIfCancellationRequested();
                if (resource.Kind == "plugin") await _plugin(resource.Name, path);
                else if (resource.Kind == "dictionary") await _dictionary(resource.Name, path);
                else Notice($"模型已校验并保存：{path}。未更改当前模型；须在兼容性及许可证确认后另行启用。", InfoBarSeverity.Success);
            });
        }
        if (resources.Count == 0) _items.Children.Add(new TextBlock { Text = "此发行版尚未提供可校验的插件/词库/模型目录。", TextWrapping = TextWrapping.Wrap });
    }
    private void AddAction(string label, Func<CancellationToken, Task> action) {
        var button = new Button { Content = label, HorizontalAlignment = HorizontalAlignment.Left, IsEnabled = _operation is null };
        button.Click += async (_, _) => await RunAsync(action); _actions.Add(button); _items.Children.Add(button);
    }
    private async Task<string> DownloadAsync(OnlineAsset asset, CancellationToken token) {
        _progress.Value = 0; _progress.Visibility = Visibility.Visible; Notice($"正在下载并校验 {asset.Name}……");
        var result = await _client.DownloadAsync(asset, _downloads, new Progress<double>(value => _progress.Value = value), token);
        Notice($"已校验：{asset.Name}", InfoBarSeverity.Success); return result;
    }
    private async Task InstallUpdateAsync(OnlineAsset asset, CancellationToken token) {
        if (!await ConfirmAsync("下载 OwO 更新", $"将下载 {asset.Name}。\nSHA-256：{asset.Sha256}\n\n不提供静默升级；下载后可确认打开安装助手。请先保存工作。", "下载")) return;
        var path = await DownloadAsync(asset, token);
        if (!await ConfirmAsync("启动更新安装助手", "请保存工作并关闭设置中心。安装助手会再次确认安装选项；若 Windows 提示发布者未知，请核对官方来源后决定是否继续。", "打开安装助手")) return;
        token.ThrowIfCancellationRequested();
        using var locked = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        if (!Convert.ToHexString(await SHA256.HashDataAsync(locked, token)).Equals(asset.Sha256, StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("安装包已改变，拒绝启动。");
        Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
    }
    private async Task<bool> ConfirmAsync(string title, string text, string primary) =>
        await new ContentDialog { XamlRoot = XamlRoot, Title = title,
            Content = new TextBlock { Text = text, TextWrapping = TextWrapping.Wrap },
            PrimaryButtonText = primary, CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close }.ShowAsync() == ContentDialogResult.Primary;
}
