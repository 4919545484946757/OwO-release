using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace OwO_Settings;

internal sealed class UninstallPage : Page
{
    private enum Step { Options, Progress, Finished, Error }
    private readonly bool _example;
    private readonly string _target, _data;
    private readonly Action _close;
    private readonly Func<bool, Action<UninstallProgress>, Task> _execute;
    private readonly Grid _dashboard = new() { ColumnSpacing = 18, RowSpacing = 14 };
    private readonly ColumnDefinition _railColumn = new();
    private readonly Border _rail = new();
    private readonly Border _article = new();
    private readonly ContentControl _body = new() {
        HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top
    };
    private readonly TextBlock _part = Text("");
    private readonly TextBlock _activeStep = Text("", 18, true);
    private readonly TextBlock _choice = Text("");
    private readonly CheckBox _keep = new() { Content = "保留我的用户数据", IsChecked = true };
    private readonly InfoBar _dataWarning = new() { Severity = InfoBarSeverity.Warning, IsClosable = false };
    private readonly Button _primary = new() { MinWidth = 108 };
    private readonly Button _secondary = new() { MinWidth = 80 };
    private readonly TextBlock _headline = Text("", 26, true);
    private readonly TextBlock _caption = Text("");
    private readonly TextBlock _percent = Text("0%", 29, true);
    private readonly ProgressBar _bar = new() { Minimum = 0, Maximum = 100 };
    private readonly Expander _details = new() { Header = "卸载详情", HorizontalAlignment = HorizontalAlignment.Stretch,
        HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox _journal = new() { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
        MinHeight = 116, MaxHeight = 170, FontFamily = new FontFamily("Consolas") };
    private StackPanel? _optionsPanel, _progressPanel;
    private Step _step;
    private int _lastPercent;
    private bool _emptyDirectoryRetained, _dataRemovalStarted, _dataRemoved;
    private bool _compact, _confirming;
    private Func<Task<bool>>? _verifyConfirmation;
    private readonly List<bool> _simulatedDecisions = [];
    private bool _simulateFailure;
    internal bool Busy { get; private set; }

    internal UninstallPage(string? root, string? work, string? preview, Action close)
    {
        _example = preview is not null;
        _close = close;
        _target = _example ? @"C:\示例\OwO\InputMethod\0.3.1" : root ?? throw new ArgumentNullException(nameof(root));
        _data = _example ? @"C:\Users\示例\AppData\Local\OwO\InputMethod" : UninstallOperation.UserDataDirectory;
        _execute = _example ? SimulateAsync :
            (remove, progress) => UninstallOperation.RunAsync(_target, work!, remove, progress);
        _primary.Style = Application.Current.Resources["AccentButtonStyle"] as Style;
        AutomationProperties.SetAutomationId(_keep, "OwOUninstallKeepData");
        AutomationProperties.SetAutomationId(_bar, "OwOUninstallProgress");
        AutomationProperties.SetAutomationId(_details, "OwOUninstallDetails");
        AutomationProperties.SetAutomationId(_primary, "OwOUninstallPrimary");
        _primary.Click += async (_, _) => await PrimaryAsync();
        _secondary.Click += (_, _) => { if (_step == Step.Error) Options(); else _close(); };
        _keep.Checked += (_, _) => ShowDataSelection();
        _keep.Unchecked += (_, _) => ShowDataSelection();
        _details.Content = _journal;
        Content = BuildLayout();
        Options();
        if (preview is "progress" or "success" or "error") {
            ResetProgress();
            Apply(new UninstallProgress(54, "shortcuts"));
            if (preview == "success") Outcome(null);
            if (preview == "error") Outcome("示例：某个应用仍在使用输入法组件。关闭相关应用后可重试。");
        }
    }

    private static TextBlock Text(string value, double size = 14, bool emphasis = false) => new() {
        Text = value, FontSize = size, FontWeight = emphasis ? FontWeights.SemiBold : FontWeights.Normal,
        TextWrapping = TextWrapping.Wrap
    };

    private static Brush? BrushResource(string name) => Application.Current.Resources.TryGetValue(name, out var value)
        ? value as Brush : null;

    private static void Surface(Border card, Thickness inset, bool quiet = false)
    {
        card.CornerRadius = new CornerRadius(16);
        card.BorderThickness = new Thickness(1);
        card.BorderBrush = BrushResource("CardStrokeColorDefaultBrush") ??
            new SolidColorBrush(Windows.UI.Color.FromArgb(38, 128, 128, 128));
        card.Background = BrushResource(quiet ? "LayerFillColorDefaultBrush" : "CardBackgroundFillColorDefaultBrush") ??
            new SolidColorBrush(Windows.UI.Color.FromArgb(quiet ? (byte)10 : (byte)18, 128, 128, 128));
        card.Padding = inset;
    }

    private UIElement BuildLayout()
    {
        var header = new Grid { ColumnSpacing = 12 };
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var identity = new StackPanel { Spacing = 5 };
        var wordmark = Text("OwO / 卸载助手", 21, true);
        identity.Children.Add(wordmark);
        identity.Children.Add(Text("Windows 输入法"));
        header.Children.Add(identity);
        var tag = new Border { Child = Text(_example ? "界面预览" : "本机安装", 12, true),
            VerticalAlignment = VerticalAlignment.Center };
        Surface(tag, new Thickness(12, 6, 12, 6), true);
        Grid.SetColumn(tag, 1); header.Children.Add(tag);

        var summary = new StackPanel { Spacing = 9 };
        _part.Opacity = .74;
        _choice.Opacity = .78;
        summary.Children.Add(_part);
        summary.Children.Add(_activeStep);
        summary.Children.Add(_choice);
        _rail.Child = summary;
        Surface(_rail, new Thickness(17), true);
        _rail.VerticalAlignment = VerticalAlignment.Top;
        _article.Child = _body;
        Surface(_article, new Thickness(22));
        _article.VerticalAlignment = VerticalAlignment.Top;
        _dashboard.ColumnDefinitions.Add(_railColumn);
        _dashboard.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        _dashboard.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        _dashboard.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        _dashboard.Children.Add(_rail);
        _dashboard.Children.Add(_article);
        _dashboard.SizeChanged += (_, args) => Reflow(args.NewSize.Width);
        Reflow(850);

        var main = new StackPanel { Spacing = 20, MaxWidth = 940, Margin = new Thickness(24, 20, 24, 12) };
        main.Children.Add(header);
        main.Children.Add(_dashboard);
        var scroll = new ScrollViewer { Content = main, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        var footer = new Grid { ColumnSpacing = 8, Padding = new Thickness(24, 10, 24, 16),
            HorizontalAlignment = HorizontalAlignment.Stretch, MaxWidth = 988 };
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        footer.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        Grid.SetColumn(_secondary, 1); footer.Children.Add(_secondary);
        Grid.SetColumn(_primary, 2); footer.Children.Add(_primary);
        var shell = new Grid();
        shell.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        shell.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        shell.Children.Add(scroll);
        Grid.SetRow(footer, 1); shell.Children.Add(footer);
        return shell;
    }

    private void Reflow(double width)
    {
        _compact = width < 650;
        _railColumn.Width = new GridLength(_compact ? 0 : 180);
        Grid.SetColumnSpan(_rail, _compact ? 2 : 1);
        Grid.SetRow(_article, _compact ? 1 : 0);
        Grid.SetColumn(_article, _compact ? 0 : 1);
        Grid.SetColumnSpan(_article, _compact ? 2 : 1);
        _article.Padding = new Thickness(_compact ? 16 : 22);
    }

    private void Outlet(Step stage, string section, string label)
    {
        _step = stage;
        _part.Text = section;
        _activeStep.Text = label;
        _secondary.Content = stage == Step.Options ? "取消" : "返回选项";
        _secondary.Visibility = stage is Step.Options or Step.Error ? Visibility.Visible : Visibility.Collapsed;
        _primary.Content = stage switch {
            Step.Options => "卸载", Step.Progress => "卸载中", Step.Error => "重试", _ => "完成"
        };
        _primary.IsEnabled = stage != Step.Progress;
    }

    private void ShowDataSelection()
    {
        var keep = _keep.IsChecked == true;
        _choice.Text = _dataRemoved ? "用户数据已移除" : keep ? "保留用户数据" : "移除用户数据";
        _dataWarning.Message = "取消勾选会永久删除此用户的配置、学习词频、词库、插件及模型设置。";
        _dataWarning.IsOpen = !keep;
    }

    private void Options()
    {
        Outlet(Step.Options, "01 / 03", "卸载选项");
        if (_optionsPanel is null) {
            var panel = new StackPanel { Spacing = 14 };
            panel.Children.Add(Text("卸载 OwO 输入法", 28, true));
            panel.Children.Add(Text("移除程序文件和 Windows 输入法注册。不会改动其他输入法。"));
            panel.Children.Add(Text("建议先保存正在编辑的设置，关闭可能占用输入法组件的应用。", 12));
            var location = Text(_target, 12);
            location.Opacity = .75;
            panel.Children.Add(location);
            var settings = new StackPanel { Spacing = 7 };
            settings.Children.Add(_keep);
            settings.Children.Add(Text("保留配置、用户词库、词频、插件及模型设置，方便以后重新安装。"));
            var card = new Border { Child = settings };
            Surface(card, new Thickness(14), true);
            panel.Children.Add(card);
            panel.Children.Add(_dataWarning);
            _optionsPanel = panel;
        }
        ShowDataSelection();
        _body.Content = _optionsPanel;
    }

    private void ResetProgress()
    {
        Outlet(Step.Progress, "02 / 03", "执行卸载");
        _headline.Text = "正在卸载";
        _caption.Text = _example ? "正在运行模拟卸载，不会改动系统。" : "等待管理员授权，之后会显示实际卸载进度。";
        _lastPercent = 0; _bar.Value = 0; _bar.IsIndeterminate = true; _percent.Text = "0%";
        _emptyDirectoryRetained = false;
        _details.IsExpanded = false;
        _journal.Text = "";
        if (_progressPanel is null) {
            var panel = new StackPanel { Spacing = 15 };
            var line = new Grid { ColumnSpacing = 12 };
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            line.Children.Add(_headline);
            Grid.SetColumn(_percent, 1); line.Children.Add(_percent);
            panel.Children.Add(line);
            panel.Children.Add(_bar);
            panel.Children.Add(_caption);
            panel.Children.Add(_details);
            _progressPanel = panel;
        }
        _body.Content = _progressPanel;
    }

    private void Apply(UninstallProgress progress)
    {
        if (progress.Stage == "complete") {
            _emptyDirectoryRetained = progress.Detail == "empty-directory-retained";
            return; // Only Outcome, after successful backend exit, can show 100%.
        }
        if (progress.Percent < _lastPercent || progress.Percent >= 100) return;
        if (progress.Stage == "remove-data") _dataRemovalStarted = true;
        if (progress.Stage == "data-removed") _dataRemoved = true;
        _bar.IsIndeterminate = false;
        _bar.Value = _lastPercent = progress.Percent;
        _percent.Text = $"{progress.Percent}%";
        _caption.Text = progress.Label;
        _journal.Text += $"[{progress.Percent,3}%] {progress.Label}\n";
        if (progress.Detail.Length != 0) _journal.Text += progress.Detail + "\n";
    }

    private void Outcome(string? error)
    {
        if (error is null) {
            Outlet(Step.Finished, "03 / 03", "卸载完成");
            _headline.Text = "卸载完成";
            _bar.IsIndeterminate = false; _bar.Value = 100; _percent.Text = "100%";
            _caption.Text = _dataRemoved || _keep.IsChecked != true ? "已移除 OwO 用户数据。" :
                _dataRemovalStarted ? $"已保留剩余用户数据：{_data}" : $"已保留用户数据：{_data}";
            if (_emptyDirectoryRetained) _caption.Text += "\n程序文件已删除，仍被占用的空目录已保留。";
            _journal.Text += "[100%] 卸载完成\n";
        } else {
            Outlet(Step.Error, "02 / 03", "卸载未完成");
            _headline.Text = "卸载未完成";
            _bar.IsIndeterminate = false;
            _caption.Text = error;
            if (_dataRemoved) _caption.Text += "\n用户数据已移除；重试不会恢复数据。";
            else if (_dataRemovalStarted) _caption.Text += "\n用户数据可能已部分删除。";
            _journal.Text += error + "\n";
            _details.IsExpanded = true;
        }
    }

    private async Task<bool> ConfirmRemovalAsync()
    {
        if (_verifyConfirmation is not null) return await _verifyConfirmation();
        var dialog = new ContentDialog {
            XamlRoot = XamlRoot,
            Title = "永久删除用户数据？",
            Content = Text($"以下目录中的配置、字典、词频、插件、模型设置及日志将被删除，无法在 OwO 中恢复。\n\n{_data}"),
            PrimaryButtonText = "删除数据并卸载", CloseButtonText = "返回", DefaultButton = ContentDialogButton.Close
        };
        return await dialog.ShowAsync() == ContentDialogResult.Primary;
    }

    private async Task PrimaryAsync()
    {
        if (Busy || _confirming) return;
        if (_step == Step.Finished) { _close(); return; }
        var remove = _keep.IsChecked != true;
        if (remove) {
            _confirming = true;
            try { if (!await ConfirmRemovalAsync()) return; }
            finally { _confirming = false; }
        }
        Busy = true;
        ResetProgress();
        try { await _execute(remove, Apply); Outcome(null); }
        catch (Exception error) { Outcome(error.Message); }
        finally { Busy = false; }
    }

    private async Task SimulateAsync(bool remove, Action<UninstallProgress> progress)
    {
        _simulatedDecisions.Add(remove);
        foreach (var item in new UninstallProgress[] { new(6, "prepare"), new(18, "stop"), new(36, "unregister"),
            new(54, "shortcuts"), new(67, remove ? "remove-data" : "keep-data"), new(79, "program-files") }) {
            await Task.Delay(_verifyConfirmation is null ? 320 : 15);
            progress(item);
        }
        if (_simulateFailure) throw new IOException("测试：程序文件仍被占用。");
    }

    internal async Task VerifyAsync(Action<int> resize)
    {
        if (!_example) throw new InvalidOperationException("UI verification requires a simulation.");
        var checks = new List<string>();
        void Check(bool ok, string label) {
            if (!ok) throw new InvalidOperationException(label);
            checks.Add("PASS " + label);
        }
        resize(960); await Task.Delay(140);
        Check(!_compact && Grid.GetColumn(_article) == 1, "wide dashboard uses two columns");
        resize(520); await Task.Delay(140);
        Check(_compact && Grid.GetRow(_article) == 1, "narrow dashboard stacks the cards");
        Check(_keep.IsChecked == true && !_dataWarning.IsOpen, "keep user data is default");
        Check(UninstallProgress.TryParse("{\"stage\":\"stop\",\"percent\":18,\"detail\":\"\"}", out var packet) &&
              packet?.Percent == 18 && !UninstallProgress.TryParse("{\"stage\":\"stop\",\"percent\":100,\"detail\":\"\"}", out _),
            "progress stream rejects a false terminal percent");
        _keep.IsChecked = false;
        _verifyConfirmation = () => Task.FromResult(false);
        await PrimaryAsync();
        Check(_dataWarning.IsOpen && _step == Step.Options && _simulatedDecisions.Count == 0,
            "declining data deletion never starts operation");
        _verifyConfirmation = () => Task.FromResult(true);
        _simulateFailure = true;
        var failed = PrimaryAsync();
        Check(Busy && _step == Step.Progress && !_primary.IsEnabled, "busy state locks wizard actions");
        await failed;
        Check(!Busy && _step == Step.Error && _bar.Value == 79 && _details.IsExpanded &&
            _journal.Text.Contains("删除用户数据") && _simulatedDecisions is [true],
            "explicit deletion uses deletion stage and failure never shows 100%");
        _details.IsExpanded = false;
        Check(!_details.IsExpanded, "details collapse");
        Options();
        _keep.IsChecked = true;
        _simulateFailure = false;
        await PrimaryAsync();
        Check(_step == Step.Finished && _bar.Value == 100 && _caption.Text.Contains("已保留") &&
            _simulatedDecisions is [true, false], "successful retry preserves data and shows 100%");
        VerificationReport = checks;
    }

    internal IReadOnlyList<string> VerificationReport { get; private set; } = [];
}
