using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.Win32;
using System.Diagnostics;
using Windows.Storage.Pickers;
using Windows.System;

namespace OwO_Settings;

public sealed class SetupPage : Page
{
    private enum SetupStep { Welcome, Options, Progress }

    private const string Version = "0.2.3";
    private readonly string _packageRoot;
    private readonly string? _previewStage;
    private readonly Grid _pageHost = new();
    private readonly Button _back = new() { Content = "上一步", Visibility = Visibility.Collapsed };
    private readonly Button _cancel = new() { Content = "取消" };
    private readonly Button _primary = new() { Content = "下一步" };
    private readonly Button _openSettings = new() { Content = "打开设置中心", Visibility = Visibility.Collapsed };

    private readonly TextBox _installDirectory = new();
    private readonly Button _browseDirectory = new() { Content = "浏览…" };
    private readonly TextBlock _languageStatus = new() { Text = "尚未检查", TextWrapping = TextWrapping.Wrap };
    private readonly Button _openLanguageSettings = new() { Content = "打开语言设置", Visibility = Visibility.Collapsed };
    private readonly Button _recheckLanguage = new() { Content = "重新检查" };
    private readonly InfoBar _packageStatus = new() { IsOpen = true, IsClosable = false };
    private readonly CheckBox _startMenuShortcut = new() { Content = "在开始菜单添加 OwO 设置中心", IsChecked = true };
    private readonly CheckBox _desktopShortcut = new() { Content = "在桌面添加 OwO 设置中心" };
    private readonly CheckBox _taskbarGuide = new() { Content = "安装完成后显示任务栏固定提示" };
    private readonly CheckBox _consent = new();

    private readonly ProgressBar _progress = new() { IsIndeterminate = true };
    private readonly InfoBar _installStatus = new() { IsOpen = true, IsClosable = false };
    private readonly ToggleButton _detailsToggle = new() { Content = "显示安装详细信息" };
    private readonly TextBox _installLog = new() {
        IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
        MinHeight = 190, MaxHeight = 300, Visibility = Visibility.Collapsed,
        FontFamily = new FontFamily("Consolas")
    };

    private SetupStep _step;
    private bool _languageReady;
    private bool _packageReady;

    public SetupPage()
    {
        _packageRoot = Path.GetFullPath(App.InstallPackageRoot ?? Path.Combine(AppContext.BaseDirectory, ".."));
        _previewStage = App.InstallPreviewStage;
        _installDirectory.Text = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Programs", "OwO", "InputMethod", Version);
        Content = BuildShell();

        _back.Click += BackButton_Click;
        _cancel.Click += (_, _) => CloseWindow();
        _primary.Click += PrimaryButton_Click;
        _openSettings.Click += OpenSettingsButton_Click;
        _browseDirectory.Click += BrowseDirectoryButton_Click;
        _openLanguageSettings.Click += async (_, _) =>
            await Launcher.LaunchUriAsync(new Uri("ms-settings:regionlanguage"));
        _recheckLanguage.Click += async (_, _) => await CheckEnvironmentAsync();
        _consent.Checked += (_, _) => UpdateInstallAvailability();
        _consent.Unchecked += (_, _) => UpdateInstallAvailability();
        _installDirectory.TextChanged += (_, _) => UpdateInstallAvailability();
        _detailsToggle.Checked += (_, _) => SetDetailsVisibility(true);
        _detailsToggle.Unchecked += (_, _) => SetDetailsVisibility(false);

        Loaded += (_, _) => DispatcherQueue.TryEnqueue(() => {
            if (_previewStage is null) ShowWelcome();
            else ApplyPreviewStage(_previewStage);
        });
    }

    private UIElement BuildShell()
    {
        var root = new Grid();
        root.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        root.Children.Add(_pageHost);

        var footer = new Grid { Padding = new Thickness(32, 14, 32, 18), ColumnSpacing = 10 };
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        footer.Children.Add(_back);
        Grid.SetColumn(_cancel, 1);
        footer.Children.Add(_cancel);
        Grid.SetColumn(_openSettings, 2);
        footer.Children.Add(_openSettings);
        Grid.SetColumn(_primary, 3);
        footer.Children.Add(_primary);
        Grid.SetRow(footer, 1);
        root.Children.Add(footer);
        return root;
    }

    private StackPanel PageBody(string title, string subtitle)
    {
        var body = new StackPanel {
            Spacing = 14, Margin = new Thickness(48, 40, 48, 28), MaxWidth = 850,
            HorizontalAlignment = HorizontalAlignment.Center
        };
        body.Children.Add(new TextBlock {
            Text = title, FontSize = 30,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold
        });
        body.Children.Add(new TextBlock {
            Text = subtitle, FontSize = 16, Opacity = 0.78,
            TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 12)
        });
        return body;
    }

    private void SetPage(UIElement content)
    {
        _pageHost.Children.Clear();
        _pageHost.Children.Add(new ScrollViewer {
            Content = content,
            HorizontalContentAlignment = HorizontalAlignment.Stretch
        });
    }

    private void ShowWelcome()
    {
        _step = SetupStep.Welcome;
        var body = PageBody("欢迎安装 OwO 输入法", "面向 Windows 11 的模块化中文输入法。");
        body.Children.Add(new TextBlock {
            Text = "OwO 提供本地拼音候选、用户学习、上下文排序和插件扩展能力。安装向导将在后续步骤中让你选择安装位置和快捷方式。",
            TextWrapping = TextWrapping.Wrap, FontSize = 17, MaxWidth = 720,
            Margin = new Thickness(0, 14, 0, 18)
        });
        var features = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        features.Children.Add(FeatureCard("原生输入体验", "使用 Windows TSF 接入常用应用。"));
        features.Children.Add(FeatureCard("本地优先", "基础候选与用户数据保存在本机。"));
        features.Children.Add(FeatureCard("可扩展", "可安装候选框皮肤和功能插件。"));
        body.Children.Add(features);
        SetPage(body);
        _back.Visibility = Visibility.Collapsed;
        _cancel.Visibility = Visibility.Visible;
        _cancel.Content = _previewStage is null ? "取消" : "关闭预览";
        _openSettings.Visibility = Visibility.Collapsed;
        _primary.Visibility = Visibility.Visible;
        _primary.Content = "下一步";
        _primary.IsEnabled = _previewStage is null;
    }

    private void ShowOptions()
    {
        _step = SetupStep.Options;
        var body = PageBody("安装选项", "选择安装位置、安装选项和快捷方式设置。");

        body.Children.Add(SectionTitle("安装位置"));
        var location = new Grid { ColumnSpacing = 10 };
        location.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        location.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        location.Children.Add(_installDirectory);
        Grid.SetColumn(_browseDirectory, 1);
        location.Children.Add(_browseDirectory);
        body.Children.Add(location);

        body.Children.Add(SectionTitle("快捷方式"));
        var options = new StackPanel { Spacing = 10 };
        options.Children.Add(_startMenuShortcut);
        options.Children.Add(_desktopShortcut);
        options.Children.Add(_taskbarGuide);
        body.Children.Add(Card(options));

        body.Children.Add(SectionTitle("安装检查"));
        var languageText = new StackPanel { Spacing = 4 };
        languageText.Children.Add(new TextBlock {
            Text = "中文（简体，中国）与基本输入支持",
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold
        });
        languageText.Children.Add(_languageStatus);
        var languageActions = new StackPanel {
            Orientation = Orientation.Horizontal, Spacing = 8,
            Margin = new Thickness(0, 8, 0, 0)
        };
        languageActions.Children.Add(_recheckLanguage);
        languageActions.Children.Add(_openLanguageSettings);
        languageText.Children.Add(languageActions);
        body.Children.Add(Card(languageText));
        body.Children.Add(_packageStatus);

        _consent.Content = new TextBlock {
            Text = "我确认安装来源可信，并同意注册输入法组件和启动当前用户后台服务。",
            TextWrapping = TextWrapping.Wrap
        };
        body.Children.Add(_consent);
        SetPage(body);

        _back.Visibility = Visibility.Visible;
        _back.Content = "上一步";
        _back.IsEnabled = _previewStage is null;
        _cancel.IsEnabled = true;
        _cancel.Visibility = Visibility.Visible;
        _cancel.Content = _previewStage is null ? "取消" : "关闭预览";
        _openSettings.Visibility = Visibility.Collapsed;
        _primary.Visibility = Visibility.Visible;
        _primary.Content = "安装";
        _browseDirectory.IsEnabled = _previewStage is null;
        _installDirectory.IsReadOnly = _previewStage is not null;
        _startMenuShortcut.IsEnabled = _previewStage is null;
        _desktopShortcut.IsEnabled = _previewStage is null;
        _taskbarGuide.IsEnabled = _previewStage is null;
        _consent.IsEnabled = _previewStage is null;
        _recheckLanguage.IsEnabled = _previewStage is null;
        UpdateInstallAvailability();
    }

    private void ShowProgress(string message, bool indeterminate = true)
    {
        _step = SetupStep.Progress;
        var body = PageBody("正在安装 OwO 输入法", "请稍候，安装完成前不要关闭此窗口。");
        _progress.IsIndeterminate = indeterminate;
        if (!indeterminate) _progress.Value = 0;
        body.Children.Add(_progress);
        _installStatus.Severity = InfoBarSeverity.Informational;
        _installStatus.Message = message;
        body.Children.Add(_installStatus);
        body.Children.Add(_detailsToggle);
        body.Children.Add(_installLog);
        SetPage(body);

        _back.Visibility = Visibility.Collapsed;
        _cancel.Visibility = Visibility.Visible;
        _cancel.Content = _previewStage is null ? "取消" : "关闭预览";
        _cancel.IsEnabled = _previewStage is not null;
        _openSettings.Visibility = Visibility.Collapsed;
        _primary.Visibility = Visibility.Collapsed;
    }

    private async void PrimaryButton_Click(object sender, RoutedEventArgs e)
    {
        if (_previewStage is not null) return;
        if (_step == SetupStep.Welcome) {
            ShowOptions();
            await CheckEnvironmentAsync();
            return;
        }
        if (_step == SetupStep.Options) await InstallAsync();
    }

    private void BackButton_Click(object sender, RoutedEventArgs e)
    {
        if (_step == SetupStep.Progress) ShowOptions();
        else ShowWelcome();
    }

    private async Task CheckEnvironmentAsync()
    {
        _languageStatus.Text = "正在检查…";
        _packageStatus.Severity = InfoBarSeverity.Informational;
        _packageStatus.Message = "正在验证安装包结构。";
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

    private void UpdateInstallAvailability()
    {
        if (_step != SetupStep.Options) return;
        _primary.IsEnabled = _previewStage is null && _languageReady && _packageReady &&
            _consent.IsChecked == true && TryGetInstallDirectory(out _, out _);
    }

    private async void BrowseDirectoryButton_Click(object sender, RoutedEventArgs e)
    {
        var picker = new FolderPicker { SuggestedStartLocation = PickerLocationId.ComputerFolder };
        picker.FileTypeFilter.Add("*");
        var hwnd = WinRT.Interop.WindowNative.GetWindowHandle(((App)Application.Current).MainWindow);
        WinRT.Interop.InitializeWithWindow.Initialize(picker, hwnd);
        var folder = await picker.PickSingleFolderAsync();
        if (folder is not null) {
            var isExistingOwO = File.Exists(Path.Combine(folder.Path, "bin", "OwO.TSF.dll")) &&
                File.Exists(Path.Combine(folder.Path, "scripts", "Uninstall-OwO.ps1"));
            _installDirectory.Text = isExistingOwO ? folder.Path :
                Path.Combine(folder.Path, "OwO", "InputMethod", Version);
        }
    }

    private bool TryGetInstallDirectory(out string directory, out string error)
    {
        directory = "";
        error = "";
        try {
            if (string.IsNullOrWhiteSpace(_installDirectory.Text)) {
                error = "请选择安装位置。";
                return false;
            }
            directory = Path.GetFullPath(_installDirectory.Text.Trim());
            var root = Path.GetPathRoot(directory);
            if (string.Equals(directory.TrimEnd(Path.DirectorySeparatorChar),
                    root?.TrimEnd(Path.DirectorySeparatorChar), StringComparison.OrdinalIgnoreCase)) {
                error = "不能安装到磁盘根目录。";
                return false;
            }
            var package = _packageRoot.TrimEnd(Path.DirectorySeparatorChar);
            var target = directory.TrimEnd(Path.DirectorySeparatorChar);
            if (target.Equals(package, StringComparison.OrdinalIgnoreCase) ||
                target.StartsWith(package + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase) ||
                package.StartsWith(target + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase)) {
                error = "安装位置不能与安装包目录重叠。";
                return false;
            }
            return true;
        } catch (Exception exception) {
            error = $"安装路径无效：{exception.Message}";
            return false;
        }
    }

    private async Task InstallAsync()
    {
        if (!TryGetInstallDirectory(out var installDirectory, out var pathError)) {
            _packageStatus.Severity = InfoBarSeverity.Error;
            _packageStatus.Message = pathError;
            return;
        }
        ShowProgress("正在复制文件、注册 TSF 并启动服务…");
        try {
            var arguments = new List<string> {
                "-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                "-File", Path.Combine(_packageRoot, "scripts", "Install-OwO.ps1"),
                "-InstallDirectory", installDirectory
            };
            if (_startMenuShortcut.IsChecked != true) arguments.Add("-NoSettingsStartMenuShortcut");
            if (_desktopShortcut.IsChecked == true) arguments.Add("-CreateSettingsDesktopShortcut");
            var result = await RunProcessAsync("powershell.exe", arguments);
            _installLog.Text = string.Join(Environment.NewLine,
                new[] { result.Output, result.Error }.Where(value => !string.IsNullOrWhiteSpace(value)));
            if (result.ExitCode != 0)
                throw new InvalidOperationException($"安装后端退出码：{result.ExitCode}");
            _progress.IsIndeterminate = false;
            _progress.Value = 100;
            _installStatus.Severity = InfoBarSeverity.Success;
            _installStatus.Message = _taskbarGuide.IsChecked == true
                ? "安装成功。打开设置中心后，可右键任务栏图标并选择“固定到任务栏”。"
                : "安装成功。现在可以按 Win＋Space 选择 OwO 输入法。";
            _openSettings.Visibility = Visibility.Visible;
            _cancel.Content = "完成";
            _cancel.IsEnabled = true;
            Environment.ExitCode = 0;
        } catch (Exception exception) {
            _progress.IsIndeterminate = false;
            _progress.Value = 0;
            _installStatus.Severity = InfoBarSeverity.Error;
            _installStatus.Message = $"安装失败：{exception.Message}";
            _installLog.Text = string.IsNullOrWhiteSpace(_installLog.Text)
                ? exception.ToString() : _installLog.Text + Environment.NewLine + exception;
            SetDetailsVisibility(true);
            _cancel.Content = "关闭";
            _cancel.IsEnabled = true;
            _back.Visibility = Visibility.Visible;
            _back.IsEnabled = true;
            _back.Content = "返回选项";
            Environment.ExitCode = 1;
        }
    }

    private void ApplyPreviewStage(string stage)
    {
        switch (stage) {
            case "start":
                ShowWelcome();
                break;
            case "options":
            case "ready":
                SetPreviewPrerequisitesReady();
                _consent.IsChecked = true;
                ShowOptions();
                break;
            case "checking":
                ShowOptions();
                _languageStatus.Text = "正在检查中文语言和基本输入组件…";
                _packageStatus.Severity = InfoBarSeverity.Informational;
                _packageStatus.Message = "正在验证安装包结构和文件完整性。";
                break;
            case "progress":
            case "installing":
                ShowProgress("正在复制文件、注册 TSF 并启动服务…");
                _installLog.Text = "正在准备安装目录…\r\n正在复制程序文件…\r\n正在注册输入法组件…";
                break;
            case "success":
                ShowProgress("安装成功。现在可以按 Win＋Space 选择 OwO 输入法。", false);
                _progress.Value = 100;
                _installStatus.Severity = InfoBarSeverity.Success;
                _installLog.Text = "TSF 配置已注册。\r\nCore Service 已启动。\r\n设置中心快捷入口已创建。";
                _openSettings.Visibility = Visibility.Visible;
                _openSettings.IsEnabled = false;
                break;
            case "error":
                ShowProgress("安装失败：示例安装后端错误。", false);
                _installStatus.Severity = InfoBarSeverity.Error;
                _installLog.Text = "示例错误：TSF registration failed。\r\n阶段预览未执行任何系统操作。";
                SetDetailsVisibility(true);
                break;
            default:
                throw new ArgumentException($"未知安装预览阶段：{stage}");
        }
    }

    private void SetPreviewPrerequisitesReady()
    {
        _languageReady = true;
        _packageReady = true;
        _languageStatus.Text = "已安装，可以注册并激活 OwO 输入法。";
        _packageStatus.Severity = InfoBarSeverity.Success;
        _packageStatus.Message = "安装包结构完整。";
    }

    private void SetDetailsVisibility(bool visible)
    {
        _detailsToggle.IsChecked = visible;
        _detailsToggle.Content = visible ? "隐藏安装详细信息" : "显示安装详细信息";
        _installLog.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;
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

    private static Border FeatureCard(string title, string description)
    {
        var content = new StackPanel { Spacing = 6, Width = 220 };
        content.Children.Add(new TextBlock {
            Text = title, FontSize = 17,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold
        });
        content.Children.Add(new TextBlock { Text = description, TextWrapping = TextWrapping.Wrap });
        return Card(content);
    }

    private static TextBlock SectionTitle(string text) => new() {
        Text = text, FontSize = 20, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
        Margin = new Thickness(0, 10, 0, 0)
    };

    private static Border Card(UIElement content) => new() {
        Child = content, Padding = new Thickness(18), CornerRadius = new CornerRadius(8),
        BorderThickness = new Thickness(1)
    };

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
