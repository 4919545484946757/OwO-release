using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.Win32;
using System.Diagnostics;
using Windows.System;

namespace OwO_Settings;

public sealed class SetupPage : Page
{
    private readonly string _packageRoot;
    private readonly TextBlock _languageStatus = new() { Text = "正在检查…", TextWrapping = TextWrapping.Wrap };
    private readonly Button _openLanguageSettings = new() { Content = "打开语言设置", Visibility = Visibility.Collapsed };
    private readonly Button _recheckLanguage = new() { Content = "重新检查" };
    private readonly InfoBar _packageStatus = new() { IsOpen = true, IsClosable = false, Severity = InfoBarSeverity.Informational, Message = "正在验证安装包结构。" };
    private readonly CheckBox _startMenuShortcut = new() { Content = "在开始菜单添加 OwO 设置中心", IsChecked = true };
    private readonly CheckBox _desktopShortcut = new() { Content = "在桌面添加 OwO 设置中心" };
    private readonly CheckBox _taskbarGuide = new() { Content = "安装完成后显示任务栏固定提示" };
    private readonly CheckBox _consent = new();
    private readonly ProgressBar _progress = new() { IsIndeterminate = true, Visibility = Visibility.Collapsed };
    private readonly InfoBar _installStatus = new() { IsOpen = false, IsClosable = false };
    private readonly TextBox _installLog = new() { Header = "安装详情", IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 180, Visibility = Visibility.Collapsed, FontFamily = new FontFamily("Consolas") };
    private readonly Button _close = new() { Content = "取消" };
    private readonly Button _openSettings = new() { Content = "打开设置中心", Visibility = Visibility.Collapsed };
    private readonly Button _install = new() { Content = "安装", IsEnabled = false };
    private bool _languageReady;
    private bool _packageReady;

    public SetupPage()
    {
        _packageRoot = Path.GetFullPath(App.InstallPackageRoot ??
            Path.Combine(AppContext.BaseDirectory, ".."));
        Content = BuildContent();
        _openLanguageSettings.Click += OpenLanguageSettingsButton_Click;
        _recheckLanguage.Click += RecheckLanguageButton_Click;
        _install.Click += InstallButton_Click;
        _openSettings.Click += OpenSettingsButton_Click;
        _close.Click += (_, _) => CloseWindow();
        _consent.Checked += (_, _) => UpdateInstallAvailability();
        _consent.Unchecked += (_, _) => UpdateInstallAvailability();
        // Environment validation is started after the first dispatcher turn so
        // unpackaged WinUI has finished attaching the page to its XamlRoot.
        Loaded += (_, _) => DispatcherQueue.TryEnqueue(async () =>
            await CheckEnvironmentAsync());
    }

    private UIElement BuildContent()
    {
        var root = new Grid();
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var body = new StackPanel { Spacing = 12, Margin = new Thickness(40, 28, 40, 28),
                                    MaxWidth = 820, HorizontalAlignment = HorizontalAlignment.Center };
        body.Children.Add(new TextBlock { Text = "安装 OwO 输入法", FontSize = 28,
                                          FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        body.Children.Add(new TextBlock { Text = "Windows 11 · WinUI 3 安装向导" });
        body.Children.Add(new TextBlock { Text = "向导会先检查系统依赖和安装包，再由受控安装后端完成文件校验、TSF 注册和服务启动。",
            TextWrapping = TextWrapping.Wrap });
        body.Children.Add(SectionTitle("环境检查"));

        var languageGrid = new Grid { ColumnSpacing = 16 };
        languageGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        languageGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var languageText = new StackPanel { Spacing = 4 };
        languageText.Children.Add(new TextBlock { Text = "中文（简体，中国）与基本输入支持",
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        languageText.Children.Add(_languageStatus);
        languageGrid.Children.Add(languageText);
        var languageActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        languageActions.Children.Add(_recheckLanguage);
        languageActions.Children.Add(_openLanguageSettings);
        Grid.SetColumn(languageActions, 1);
        languageGrid.Children.Add(languageActions);
        body.Children.Add(Card(languageGrid));
        body.Children.Add(_packageStatus);
        body.Children.Add(SectionTitle("安装选项"));
        var options = new StackPanel { Spacing = 12 };
        options.Children.Add(_startMenuShortcut);
        options.Children.Add(_desktopShortcut);
        options.Children.Add(_taskbarGuide);
        body.Children.Add(Card(options));
        _consent.Content = new TextBlock { Text = "我确认安装来源可信，并同意注册输入法组件和启动当前用户后台服务。", TextWrapping = TextWrapping.Wrap };
        body.Children.Add(_consent);
        body.Children.Add(_progress);
        body.Children.Add(_installStatus);
        body.Children.Add(_installLog);

        var scroll = new ScrollViewer { Content = body, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        root.Children.Add(scroll);
        var actions = new StackPanel { Orientation = Orientation.Horizontal,
            HorizontalAlignment = HorizontalAlignment.Right, Spacing = 10,
            Margin = new Thickness(24, 14, 24, 14) };
        actions.Children.Add(_close);
        actions.Children.Add(_openSettings);
        actions.Children.Add(_install);
        Grid.SetRow(actions, 1);
        root.Children.Add(actions);
        return root;
    }

    private static TextBlock SectionTitle(string text) => new() {
        Text = text, FontSize = 20, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
        Margin = new Thickness(0, 12, 0, 0)
    };

    private static Border Card(UIElement content) => new() {
        Child = content, Padding = new Thickness(18), CornerRadius = new CornerRadius(8),
        BorderThickness = new Thickness(1)
    };

    private async Task CheckEnvironmentAsync()
    {
        var required = new[] { "bin", "data", "model", "settings", "scripts",
                               "README.zh-CN.md", "THIRD-PARTY-NOTICES.md" };
        var missing = required.Where(item => !Directory.Exists(Path.Combine(_packageRoot, item)) &&
                                             !File.Exists(Path.Combine(_packageRoot, item))).ToArray();
        _packageReady = missing.Length == 0 &&
            File.Exists(Path.Combine(_packageRoot, "scripts", "Install-OwO.ps1"));
        _packageStatus.Severity = _packageReady ? InfoBarSeverity.Success : InfoBarSeverity.Error;
        _packageStatus.Message = _packageReady ? "安装包结构完整。" :
            $"安装包不完整，缺少：{string.Join("、", missing)}";
        try {
            var result = await RunProcessAsync("powershell.exe", [
                "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
                "$ok = $false; foreach ($language in Get-WinUserLanguageList) { " +
                "if ($language.LanguageTag -in @('zh-Hans-CN','zh-CN')) { $ok = $true } }; " +
                "if (-not $ok) { exit 1 }"
            ]);
            _languageReady = result.ExitCode == 0;
        } catch { _languageReady = false; }
        _languageStatus.Text = _languageReady
            ? "已安装，可以注册并激活 OwO 输入法。"
            : "尚未检测到所需语言。请安装语言和基本输入组件后重新检查。";
        _openLanguageSettings.Visibility = _languageReady ? Visibility.Collapsed : Visibility.Visible;
        UpdateInstallAvailability();
    }

    private void UpdateInstallAvailability() => _install.IsEnabled =
        _languageReady && _packageReady && _consent.IsChecked == true;

    private async void OpenLanguageSettingsButton_Click(object sender, RoutedEventArgs e) =>
        await Launcher.LaunchUriAsync(new Uri("ms-settings:regionlanguage"));

    private async void RecheckLanguageButton_Click(object sender, RoutedEventArgs e) =>
        await CheckEnvironmentAsync();

    private async void InstallButton_Click(object sender, RoutedEventArgs e)
    {
        _install.IsEnabled = false;
        _close.IsEnabled = false;
        _progress.Visibility = Visibility.Visible;
        _installStatus.IsOpen = true;
        _installStatus.Severity = InfoBarSeverity.Informational;
        _installStatus.Message = "正在安装、校验并启用 OwO 输入法…";
        try {
            var arguments = new List<string> {
                "-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                "-File", Path.Combine(_packageRoot, "scripts", "Install-OwO.ps1")
            };
            if (_startMenuShortcut.IsChecked != true) arguments.Add("-NoSettingsStartMenuShortcut");
            if (_desktopShortcut.IsChecked == true) arguments.Add("-CreateSettingsDesktopShortcut");
            var result = await RunProcessAsync("powershell.exe", arguments);
            _installLog.Text = string.Join(Environment.NewLine,
                new[] { result.Output, result.Error }.Where(value => !string.IsNullOrWhiteSpace(value)));
            _installLog.Visibility = Visibility.Visible;
            if (result.ExitCode != 0)
                throw new InvalidOperationException($"安装后端退出码：{result.ExitCode}");
            _installStatus.Severity = InfoBarSeverity.Success;
            _installStatus.Message = _taskbarGuide.IsChecked == true
                ? "安装成功。打开设置中心后，可右键任务栏图标并选择“固定到任务栏”。"
                : "安装成功。现在可以按 Win＋Space 选择 OwO 输入法。";
            _install.Visibility = Visibility.Collapsed;
            _openSettings.Visibility = Visibility.Visible;
            _close.Content = "完成";
            Environment.ExitCode = 0;
        } catch (Exception error) {
            _installStatus.Severity = InfoBarSeverity.Error;
            _installStatus.Message = $"安装失败：{error.Message}";
            _installLog.Visibility = Visibility.Visible;
            _install.IsEnabled = true;
            Environment.ExitCode = 1;
        } finally {
            _progress.Visibility = Visibility.Collapsed;
            _close.IsEnabled = true;
        }
    }

    private void OpenSettingsButton_Click(object sender, RoutedEventArgs e)
    {
        using var key = Registry.CurrentUser.OpenSubKey(
            @"Software\Microsoft\Windows\CurrentVersion\Uninstall\OwO Input Method");
        var root = key?.GetValue("InstallLocation") as string;
        var executable = string.IsNullOrWhiteSpace(root) ? null :
            Path.Combine(root, "settings", "OwO.Settings.exe");
        if (executable is not null && File.Exists(executable)) {
            Process.Start(new ProcessStartInfo(executable) { UseShellExecute = true });
            CloseWindow();
        }
    }

    private static void CloseWindow() => ((App)Application.Current).MainWindow.Close();

    private sealed record ProcessResult(int ExitCode, string Output, string Error);

    private static async Task<ProcessResult> RunProcessAsync(string executable,
                                                              IEnumerable<string> arguments)
    {
        var start = new ProcessStartInfo(executable) {
            UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
        };
        foreach (var argument in arguments) start.ArgumentList.Add(argument);
        using var process = Process.Start(start) ??
            throw new InvalidOperationException($"无法启动 {executable}。");
        var output = process.StandardOutput.ReadToEndAsync();
        var error = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();
        return new ProcessResult(process.ExitCode, await output, await error);
    }
}
