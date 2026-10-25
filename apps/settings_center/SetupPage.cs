using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Windowing;
using Microsoft.Win32;
using System.Diagnostics;
using Windows.Graphics;
using Windows.Storage.Pickers;
using Windows.System;

namespace OwO_Settings;

public sealed partial class SetupPage : Page
{
    private enum SetupStep { Welcome, Options, Progress }

    private const string Version = "0.2.5";
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
    private readonly TextBlock _availability = new() { TextWrapping = TextWrapping.Wrap, Opacity = 0.75 };
    private readonly Dictionary<SetupStep, UIElement> _pages = new();

    private readonly ProgressBar _progress = new() { IsIndeterminate = true };
    private readonly TextBlock _progressPercentage = new() {
        Text = "0%", FontSize = 14, Opacity = 0.78,
        MinWidth = 48, TextAlignment = TextAlignment.Right,
        HorizontalAlignment = HorizontalAlignment.Right,
        VerticalAlignment = VerticalAlignment.Center
    };
    private readonly TextBlock _stageMessage = new() {
        TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center
    };
    private readonly InfoBar _installStatus = new() {
        IsOpen = true, IsClosable = false, HorizontalContentAlignment = HorizontalAlignment.Stretch
    };
    private readonly ToggleButton _detailsToggle = new() { Content = "显示安装详细信息" };
    private readonly TextBox _installLog = new() {
        IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
        MinHeight = 140, MaxHeight = 220, Visibility = Visibility.Collapsed,
        FontFamily = new FontFamily("Consolas")
    };

    private SetupStep _step;
    private bool _languageReady;
    private bool _packageReady;
    private bool _checking;
    private bool _installing;
    internal bool InstallationInProgress => _installing;
    private static string WindowsPowerShell => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.System),
        "WindowsPowerShell", "v1.0", "powershell.exe");

    public SetupPage()
    {
        _packageRoot = Path.GetFullPath(App.InstallPackageRoot ?? Path.Combine(AppContext.BaseDirectory, ".."));
        _previewStage = App.InstallPreviewStage;
        foreach (var option in new[] { _startMenuShortcut, _desktopShortcut, _taskbarGuide }) {
            option.Content = new TextBlock {
                Text = option.Content.ToString(), TextWrapping = TextWrapping.Wrap
            };
            option.HorizontalAlignment = HorizontalAlignment.Stretch;
            option.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        }
        _consent.HorizontalAlignment = HorizontalAlignment.Stretch;
        _consent.HorizontalContentAlignment = HorizontalAlignment.Stretch;
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

        var footer = new Grid { Padding = new Thickness(24, 10, 24, 14), ColumnSpacing = 10 };
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
        _primary.Style = Application.Current.Resources["AccentButtonStyle"] as Style;
        bool? footerNarrow = null;
        footer.SizeChanged += (_, args) => {
            var narrow = args.NewSize.Width < 560;
            if (footerNarrow == narrow) return;
            footerNarrow = narrow;
            footer.Padding = new Thickness(narrow ? 12 : 24, 10, narrow ? 12 : 24, 14);
            footer.RowDefinitions.Clear();
            if (narrow) {
                for (var index = 0; index < 4; index++)
                    footer.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            }
            var buttons = new[] { _back, _cancel, _openSettings, _primary };
            for (var index = 0; index < buttons.Length; index++) {
                Grid.SetRow(buttons[index], narrow ? index : 0);
                Grid.SetColumn(buttons[index], narrow ? 0 : index);
                Grid.SetColumnSpan(buttons[index], narrow ? 4 : 1);
                buttons[index].HorizontalAlignment = narrow ? HorizontalAlignment.Stretch : HorizontalAlignment.Right;
            }
        };
        Grid.SetRow(footer, 1);
        root.Children.Add(footer);
        return root;
    }

    private StackPanel PageBody(string title, string subtitle)
    {
        var body = new StackPanel {
            Spacing = 14, Margin = new Thickness(24, 24, 24, 20), MaxWidth = 940,
            HorizontalAlignment = HorizontalAlignment.Stretch
        };
        body.Children.Add(new TextBlock {
            Text = title, FontSize = 30,
            TextWrapping = TextWrapping.Wrap,
            FontWeight = Microsoft.UI.Text.FontWeights.Normal
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
        // All installer states share one step indicator, including checking,
        // installing and the success/error pages built outside PageBody.
        if (content is StackPanel body)
            body.Children.Insert(0, WizardSteps(_step));
        if (content is FrameworkElement element) {
            element.Margin = new Thickness(0);
            element.HorizontalAlignment = HorizontalAlignment.Stretch;
        }
        // ScrollViewer can measure unconstrained content at its desired width.
        // Supply a finite viewport width so long paths, cards and images wrap
        // before arranging rather than expanding past the window edge.
        var viewport = new Grid { Padding = new Thickness(24, 18, 24, 16) };
        viewport.Children.Add(content);
        var scroll = new ScrollViewer {
            Content = viewport, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollMode = ScrollMode.Enabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalContentAlignment = HorizontalAlignment.Stretch
        };
        scroll.SizeChanged += (_, args) => {
            viewport.Width = Math.Max(0, args.NewSize.Width - 16);
            var inset = args.NewSize.Width < 600 ? 12 : 24;
            viewport.Padding = new Thickness(inset, 18, inset, 16);
        };
        _pageHost.Children.Add(scroll);
        _pages[_step] = scroll;
    }

    private bool RestorePage()
    {
        if (!_pages.TryGetValue(_step, out var page)) return false;
        _pageHost.Children.Clear();
        _pageHost.Children.Add(page);
        return true;
    }

    private void ShowWelcome()
    {
        _step = SetupStep.Welcome;
        ResizeInstallerWindow(760);
        var body = PageBody("欢迎使用 OwO", "面向 Windows 11 的模块化中文输入法。");
        var introduction = new StackPanel { Spacing = 14 };
        introduction.Children.Add(new TextBlock {
            Text = "OwO", FontSize = 52, CharacterSpacing = 80,
            Foreground = AccentBrush()
        });
        introduction.Children.Add(new TextBlock {
            Text = "让每一次表达，更贴近你的习惯。", FontSize = 24,
            TextWrapping = TextWrapping.Wrap
        });
        introduction.Children.Add(new TextBlock {
            Text = "从拼音输入到个性化候选，OwO 将本地词库、用户学习和可选智能排序融入日常输入。你也可以通过插件定制外观与功能。",
            TextWrapping = TextWrapping.Wrap, Opacity = 0.78, MaxWidth = 760
        });
        introduction.Children.Add(new TextBlock {
            Text = $"版本 {Version}  ·  Windows 11 x64", FontSize = 12, Opacity = 0.65
        });
        body.Children.Add(Card(introduction));
        var features = ResponsiveColumns(
            FeatureCard("\uE8D2", "流畅表达", "拼音候选、快捷选词与用户词库，适配日常输入。"),
            FeatureCard("\uE753", "本地优先", "基础输入无需联网，词库与学习数据保存在本机。"),
            FeatureCard("\uE713", "按需扩展", "在设置中心调整输入偏好、管理主题和功能插件。"));
        body.Children.Add(features);
        body.Children.Add(new TextBlock {
            Text = "接下来：选择安装位置与快捷方式，再查看安装进度。安装完成后，按 Win＋Space 切换到 OwO。",
            TextWrapping = TextWrapping.Wrap, Opacity = 0.7, Margin = new Thickness(0, 4, 0, 0)
        });
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
        ResizeInstallerWindow(760);
        if (!RestorePage()) SetPage(BuildOptionsBody());

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
        _recheckLanguage.IsEnabled = _previewStage is null && !_checking;
        UpdateInstallAvailability();
    }

    private UIElement BuildOptionsBody()
    {
        var body = PageBody("安装选项", "选择安装位置、安装选项和快捷方式设置。");

        var installation = new StackPanel { Spacing = 10 };
        installation.Children.Add(SectionTitle("安装位置"));
        var location = new Grid { ColumnSpacing = 10 };
        location.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        location.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        location.Children.Add(_installDirectory);
        Grid.SetColumn(_browseDirectory, 1);
        location.Children.Add(_browseDirectory);
        installation.Children.Add(location);
        installation.Children.Add(new TextBlock {
            Text = "浏览时会在所选文件夹下创建 OwO 目录，也可填写完整路径。已有安装或可验证的失败残留会改用新的同级目录，不覆盖原文件；其他非空目录不能使用。",
            TextWrapping = TextWrapping.Wrap, FontSize = 12, Opacity = 0.7
        });
        body.Children.Add(Card(installation));

        var options = new StackPanel { Spacing = 10 };
        options.Children.Add(SectionTitle("快捷入口"));
        options.Children.Add(_startMenuShortcut);
        options.Children.Add(_desktopShortcut);
        options.Children.Add(_taskbarGuide);
        options.Children.Add(new TextBlock {
            Text = "Windows 任务栏固定由你手动完成，安装器可在完成后提供操作提示。",
            TextWrapping = TextWrapping.Wrap, FontSize = 12, Opacity = 0.7
        });

        var languageText = new StackPanel { Spacing = 4 };
        languageText.Children.Add(SectionTitle("安装检查"));
        languageText.Children.Add(new TextBlock {
            Text = "中文（简体，中国）与基本输入支持",
            TextWrapping = TextWrapping.Wrap
        });
        languageText.Children.Add(_languageStatus);
        var languageActions = ResponsiveColumns(_recheckLanguage, _openLanguageSettings);
        languageActions.Margin = new Thickness(0, 8, 0, 0);
        languageText.Children.Add(languageActions);
        languageText.Children.Add(_packageStatus);
        body.Children.Add(ResponsiveColumns(Card(options), Card(languageText)));
        body.Children.Add(new TextBlock {
            Text = "安装到当前用户账户；注册输入法组件时，Windows 会请求管理员授权。无需将中文设为系统显示语言。",
            TextWrapping = TextWrapping.Wrap, FontSize = 12, Opacity = 0.7
        });

        _consent.Content = new TextBlock {
            Text = "我确认安装来源可信，并同意注册输入法组件和启动当前用户后台服务。",
            TextWrapping = TextWrapping.Wrap
        };
        body.Children.Add(_consent);
        body.Children.Add(_availability);
        return body;
    }

    private void ShowProgress(string message, bool indeterminate = true)
    {
        _step = SetupStep.Progress;
        if (!RestorePage()) SetPage(BuildProgressBody());
        _progress.IsIndeterminate = indeterminate;
        _progress.Minimum = 0;
        _progress.Maximum = 100;
        _progress.Value = 0;
        _progressPercentage.Text = indeterminate ? "—" : "0%";
        _installLog.Text = "";
        _installStatus.Severity = InfoBarSeverity.Informational;
        _stageMessage.Text = message;
        SetDetailsVisibility(false);

        _back.Visibility = Visibility.Collapsed;
        _cancel.Visibility = Visibility.Visible;
        _cancel.Content = _previewStage is null ? "取消" : "关闭预览";
        _cancel.IsEnabled = _previewStage is not null;
        _openSettings.Visibility = Visibility.Collapsed;
        _primary.Visibility = Visibility.Collapsed;
    }

    private UIElement BuildProgressBody()
    {
        var body = new StackPanel {
            Spacing = 12, Margin = new Thickness(24, 18, 24, 16), MaxWidth = 940,
            HorizontalAlignment = HorizontalAlignment.Center
        };
        body.Children.Add(ProgressBanner());
        body.Children.Add(_progress);
        _progressPercentage.Text = "0%";
        var stageRow = new Grid {
            ColumnSpacing = 20, Padding = new Thickness(4, 8, 16, 8),
            MinHeight = 40, HorizontalAlignment = HorizontalAlignment.Stretch
        };
        stageRow.SizeChanged += (_, args) => {
            var compact = args.NewSize.Width < 480;
            stageRow.Padding = new Thickness(4, 8, compact ? 10 : 16, 8);
            stageRow.ColumnSpacing = compact ? 12 : 20;
        };
        stageRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        stageRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        stageRow.Children.Add(_stageMessage);
        Grid.SetColumn(_progressPercentage, 1);
        stageRow.Children.Add(_progressPercentage);
        _installStatus.Content = stageRow;
        _installStatus.Severity = InfoBarSeverity.Informational;
        body.Children.Add(_installStatus);
        body.Children.Add(new TextBlock {
            Text = "正在安装 OwO 输入法", FontSize = 24,
            TextWrapping = TextWrapping.Wrap,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
            Margin = new Thickness(0, 2, 0, 0)
        });
        body.Children.Add(new TextBlock {
            Text = "请稍候，安装完成前不要关闭此窗口。",
            TextWrapping = TextWrapping.Wrap, Opacity = 0.78
        });
        body.Children.Add(_detailsToggle);
        body.Children.Add(_installLog);
        return body;
    }

    private async void PrimaryButton_Click(object sender, RoutedEventArgs e)
    {
        await AdvanceAsync();
    }

    private async Task AdvanceAsync()
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
        // One in-flight check survives Back/Next navigation; it updates the
        // cached options page, never a retired control tree.
        if (_checking || _installing || _previewStage is not null) return;
        _checking = true;
        _languageReady = false;
        _recheckLanguage.IsEnabled = false;
        UpdateInstallAvailability();
        _languageStatus.Text = "正在检查…";
        _packageStatus.Severity = InfoBarSeverity.Informational;
        _packageStatus.Message = "正在验证安装包结构。";
        var required = new[] { "bin", "data", "model", "settings", "scripts", "LICENSES",
            "scripts/Install-OwO.ps1", "scripts/Install-Prerequisites.ps1", "README.zh-CN.md", "THIRD-PARTY-NOTICES.md",
            "Open-OwO-Settings.cmd", "Uninstall-OwO.cmd" };
        var missing = required.Where(item => !Directory.Exists(Path.Combine(_packageRoot, item)) &&
                                             !File.Exists(Path.Combine(_packageRoot, item))).ToArray();
        _packageReady = App.BundledInstaller is not null || missing.Length == 0;
        _packageStatus.Severity = _packageReady ? InfoBarSeverity.Success : InfoBarSeverity.Error;
        _packageStatus.Message = App.BundledInstaller is not null
            ? "安装文件已内置，点击安装后校验并解压。" : _packageReady ? "安装包结构完整。" :
            $"安装包不完整，缺少：{string.Join("、", missing)}";
        try {
            var result = await RunProcessAsync(WindowsPowerShell, [
                "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
                SetupPrerequisites.LanguageProbe
            ], TimeSpan.FromSeconds(20));
            _languageReady = SetupPrerequisites.LanguageReady(result.ExitCode, result.Output);
            _languageStatus.Text = _languageReady
                ? "已检测到当前用户的简体中文输入支持。"
                : result.ExitCode == 1 && result.Output.Trim() == "OWO_LANGUAGE_MISSING"
                    ? "尚未检测到所需语言。请安装语言和基本输入组件后重新检查。"
                    : "语言检测失败，请重新检查。诊断：" + result.Error.Trim();
        } catch (Exception exception) {
            _languageStatus.Text = $"语言检测失败（并非确认缺少语言包）：{exception.Message}。请重新检查。";
        } finally {
            _checking = false;
            _recheckLanguage.IsEnabled = _previewStage is null;
        }
        _openLanguageSettings.Visibility = _languageReady ? Visibility.Collapsed : Visibility.Visible;
        UpdateInstallAvailability();
    }

    private void UpdateInstallAvailability()
    {
        if (_step != SetupStep.Options) return;
        var pathValid = TryGetInstallDirectory(out _, out var pathError);
        var reason = SetupPrerequisites.BlockingReason(_previewStage is not null,
            _checking, _languageReady, _packageReady, _consent.IsChecked == true,
            pathValid, pathError);
        var architectureError = SetupPrerequisites.ArchitectureError(
            System.Runtime.InteropServices.RuntimeInformation.OSArchitecture);
        if (_previewStage is null && architectureError.Length != 0) reason = architectureError;
        _primary.IsEnabled = !_installing && reason.Length == 0;
        _availability.Text = reason.Length == 0 ? "已准备就绪，可以安装。" : reason;
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
        UpdateInstallAvailability();
        if (!_primary.IsEnabled || _installing) return;
        if (!TryGetInstallDirectory(out var installDirectory, out var pathError)) {
            _packageStatus.Severity = InfoBarSeverity.Error;
            _packageStatus.Message = pathError;
            return;
        }
        _installing = true;
        try {
            ShowProgress("正在复制文件、注册 TSF 并启动服务…");
            if (App.BundledInstaller is { } payload) {
                _progress.IsIndeterminate = false;
                var preparing = true;
                try {
                    await payload.PrepareAsync(new Progress<InstallerPayloadProgress>(update => {
                        if (!preparing) return;
                        _stageMessage.Text = update.Message;
                        _progress.Value = update.Percent;
                        _progressPercentage.Text = $"{update.Percent}%";
                    }));
                } finally { preparing = false; }
                _progress.IsIndeterminate = true;
                _progressPercentage.Text = "—";
                _stageMessage.Text = "正在安装文件、注册 TSF 并启动服务…";
            }
            var arguments = new List<string> {
                "-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                "-File", Path.Combine(_packageRoot, "scripts", "Install-OwO.ps1"),
                "-InstallDirectory", installDirectory, "-ReportProgress"
            };
            // Bundled-payload preparation has its own progress scale.
            _progress.Value = 0;
            if (_startMenuShortcut.IsChecked != true) arguments.Add("-NoSettingsStartMenuShortcut");
            if (_desktopShortcut.IsChecked == true) arguments.Add("-CreateSettingsDesktopShortcut");
            var receiving = true;
            var progress = new Progress<InstallerProcessLine>(line => {
                if (!receiving) return;
                ShowBackendLine(line);
            });
            InstallerProcessResult result;
            try { result = await InstallerProcessRunner.RunAsync(WindowsPowerShell, arguments, lines: progress); }
            finally { receiving = false; }
            App.LogStage($"Installer backend exited: {result.ExitCode}");
            _installLog.Text = string.Join(Environment.NewLine,
                new[] { result.Output, result.Error }.Where(value => !string.IsNullOrWhiteSpace(value)));
            if (result.ExitCode != 0)
                throw new InvalidOperationException($"安装后端退出码：{result.ExitCode}");
            _progress.IsIndeterminate = false;
            _progress.Value = 100;
            _progressPercentage.Text = "100%";
            _installStatus.Severity = InfoBarSeverity.Success;
            _stageMessage.Text = _taskbarGuide.IsChecked == true
                ? "安装成功。打开设置中心后，可右键任务栏图标并选择“固定到任务栏”。"
                : "安装成功。现在可以按 Win＋Space 选择 OwO 输入法。";
            _openSettings.Visibility = Visibility.Visible;
            _cancel.Content = "完成";
            _cancel.IsEnabled = true;
            Environment.ExitCode = 0;
        } catch (Exception exception) {
            _progress.IsIndeterminate = false;
            _installStatus.Severity = InfoBarSeverity.Error;
            _stageMessage.Text = $"安装失败：{exception.Message}";
            _installLog.Text = string.IsNullOrWhiteSpace(_installLog.Text)
                ? exception.ToString() : _installLog.Text + Environment.NewLine + exception;
            SetDetailsVisibility(true);
            _cancel.Content = "关闭";
            _cancel.IsEnabled = true;
            _back.Visibility = Visibility.Visible;
            _back.IsEnabled = true;
            _back.Content = "返回选项";
            Environment.ExitCode = 1;
        } finally {
            _installing = false;
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
                ShowProgress("正在复制文件、注册 TSF 并启动服务…", false);
                _progress.Value = 46;
                _progressPercentage.Text = "46%";
                _installLog.Text = "正在准备安装目录…\r\n正在复制程序文件…\r\n正在注册输入法组件…";
                break;
            case "success":
                ShowProgress("安装成功。现在可以按 Win＋Space 选择 OwO 输入法。", false);
                _progress.Value = 100;
                _progressPercentage.Text = "100%";
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

    private void ShowBackendLine(InstallerProcessLine line)
    {
        App.LogStage("Installer backend: " + line.Text);
        if (!line.IsError && InstallerProcessRunner.TryProgress(line.Text, out var percent, out var stage)) {
            if (percent < _progress.Value) return;
            _progress.IsIndeterminate = false;
            _progress.Value = percent;
            _progressPercentage.Text = $"{percent}%";
            _stageMessage.Text = stage;
        } else {
            var text = _installLog.Text + line.Text + Environment.NewLine;
            _installLog.Text = text.Length > 65536 ? text[^65536..] : text;
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
        if (_step == SetupStep.Progress)
            ResizeInstallerWindow(visible ? 750 : 670, preservePosition: true);
    }

    private static void ResizeInstallerWindow(int preferredHeight, bool preservePosition = false)
    {
        var window = ((App)Application.Current).MainWindow;
        var display = DisplayArea.GetFromWindowId(window.AppWindow.Id, DisplayAreaFallback.Primary);
        var workArea = display.WorkArea;
        // AppWindow uses physical pixels; XAML layout sizes are logical pixels.
        var scale = window.Content.XamlRoot?.RasterizationScale ?? 1;
        if (preservePosition) {
            var position = window.AppWindow.Position;
            var availableHeight = Math.Max(1, workArea.Y + workArea.Height - position.Y);
            var targetHeight = Math.Min((int)Math.Round(preferredHeight * scale), availableHeight);
            // Resize preserves the top-left origin and the existing width;
            // details affect only the vertical expansion area.
            window.AppWindow.Resize(new SizeInt32(window.AppWindow.Size.Width, targetHeight));
            return;
        }
        var width = Math.Min((int)Math.Round(1040 * scale), Math.Max(1, workArea.Width - 48));
        var height = Math.Min((int)Math.Round(preferredHeight * scale), Math.Max(1, workArea.Height - 48));
        var currentPosition = window.AppWindow.Position;
        var currentSize = window.AppWindow.Size;
        var centeredX = currentPosition.X + (currentSize.Width - width) / 2;
        var centeredY = currentPosition.Y + (currentSize.Height - height) / 2;
        var x = Math.Clamp(centeredX, workArea.X, workArea.X + workArea.Width - width);
        var y = Math.Clamp(centeredY, workArea.Y, workArea.Y + workArea.Height - height);
        window.AppWindow.MoveAndResize(new RectInt32(x, y, width, height));
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

    private static Brush AccentBrush() =>
        (Brush)Application.Current.Resources["SystemControlForegroundAccentBrush"];

    private static UIElement WizardSteps(SetupStep current)
    {
        var labels = new[] { "开始", "安装选项", "安装进度" };
        var grid = new Grid { ColumnSpacing = 12, Margin = new Thickness(0, 0, 0, 8) };
        for (var index = 0; index < labels.Length; index++) {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var panel = new StackPanel { Spacing = 8 };
            panel.Children.Add(new TextBlock {
                Text = $"0{index + 1}  {labels[index]}", FontSize = 12,
                TextWrapping = TextWrapping.Wrap,
                Opacity = index == (int)current ? 1 : 0.55
            });
            panel.Children.Add(new Border {
                Height = 3, CornerRadius = new CornerRadius(2),
                Background = index == (int)current ? AccentBrush() :
                    new SolidColorBrush(Windows.UI.Color.FromArgb(35, 128, 128, 128))
            });
            Grid.SetColumn(panel, index);
            grid.Children.Add(panel);
        }
        return grid;
    }

    private static Grid ResponsiveColumns(params FrameworkElement[] items)
    {
        var grid = new Grid { ColumnSpacing = 12, RowSpacing = 12 };
        void Arrange(bool horizontal)
        {
            grid.ColumnDefinitions.Clear();
            grid.RowDefinitions.Clear();
            for (var index = 0; index < items.Length; index++) {
                if (horizontal) grid.ColumnDefinitions.Add(new ColumnDefinition {
                    Width = new GridLength(1, GridUnitType.Star)
                });
                else grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
                Grid.SetColumn(items[index], horizontal ? index : 0);
                Grid.SetRow(items[index], horizontal ? 0 : index);
            }
        }
        foreach (var item in items) grid.Children.Add(item);
        var horizontal = true;
        Arrange(horizontal);
        grid.SizeChanged += (_, args) => {
            var next = args.NewSize.Width >= 720;
            if (next == horizontal) return;
            horizontal = next;
            Arrange(horizontal);
        };
        return grid;
    }

    private static Border FeatureCard(string glyph, string title, string description)
    {
        var content = new StackPanel { Spacing = 10 };
        content.Children.Add(new FontIcon {
            Glyph = glyph, FontSize = 22, Foreground = AccentBrush(),
            HorizontalAlignment = HorizontalAlignment.Left
        });
        content.Children.Add(new TextBlock {
            Text = title, FontSize = 17,
            FontWeight = Microsoft.UI.Text.FontWeights.Normal
        });
        content.Children.Add(new TextBlock {
            Text = description, TextWrapping = TextWrapping.Wrap, Opacity = 0.7
        });
        return Card(content);
    }

    private static Border ProgressBanner() => new() {
        Height = 238,
        CornerRadius = new CornerRadius(12),
        Child = new Image {
            Source = new BitmapImage(new Uri(Path.Combine(App.ResourceDirectory, "Assets", "SetupProgressBanner.png"))),
            Stretch = Stretch.UniformToFill,
            HorizontalAlignment = HorizontalAlignment.Stretch,
            VerticalAlignment = VerticalAlignment.Center
        }
    };

    private static TextBlock SectionTitle(string text) => new() {
        Text = text, FontSize = 18, FontWeight = Microsoft.UI.Text.FontWeights.Normal,
        Margin = new Thickness(0, 0, 0, 6)
    };

    private static Border Card(UIElement content) => new() {
        Child = content, Padding = new Thickness(18), CornerRadius = new CornerRadius(8),
        BorderThickness = new Thickness(1),
        Background = new SolidColorBrush(Windows.UI.Color.FromArgb(12, 128, 128, 128)),
        BorderBrush = new SolidColorBrush(Windows.UI.Color.FromArgb(30, 128, 128, 128))
    };

    private static void CloseWindow() => ((App)Application.Current).MainWindow.Close();

    private static Task<InstallerProcessResult> RunProcessAsync(string executable,
        IEnumerable<string> arguments, TimeSpan? timeout = null)
        => InstallerProcessRunner.RunAsync(executable, arguments, timeout);
}
