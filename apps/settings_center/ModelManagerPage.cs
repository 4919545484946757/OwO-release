using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace OwO_Settings;

internal sealed partial class ModelManagerPage : UserControl {
    private readonly ModelManagerStore _store;
    private readonly Func<string, CancellationToken, DownloadOperation> _startDownload;
    private readonly StackPanel _localRows = new() { Spacing = 8 };
    private readonly TextBlock _zeroSource = new() { TextWrapping = TextWrapping.Wrap, Opacity = .75 };
    private readonly Action<string, InfoBarSeverity> _notify;
    private TextAssistantSettings _value;
    private readonly StackPanel _profiles = new() { Spacing = 8 };
    private readonly TextBlock _profileCount = new() { Opacity = .75, TextWrapping = TextWrapping.Wrap };
    private readonly ToggleSwitch _enabled = new() { Header = "选区大模型功能", MinWidth = 0 };
    private readonly ToggleSwitch _context = new() { Header = "发送局部上下文", MinWidth = 0 };
    private readonly ToggleSwitch _zero = new() { Header = "启用 0 号候选", MinWidth = 0 };
    private readonly ScrollNeutralComboBox _zeroMode = new() { Header = "候选生成模式", PlaceholderText = "仅拼音", ItemsSource = new[] { "能效优先", "性能优先" }, MinWidth = 160 };
    private readonly TextBlock _zeroHint = new() { TextWrapping = TextWrapping.Wrap, Opacity = .75 };
    private readonly TextBox _preferred = new() { Header = "首选语言", MaxLength = 64 };
    private readonly TextBox _secondary = new() { Header = "次选语言", MaxLength = 64 };
    private readonly ScrollNeutralComboBox _polish = new() { Header = "语言润色级别", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ScrollNeutralComboBox _code = new() { Header = "代码纠错级别", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly NumberBox _timeout = new() { Header = "请求超时（秒）", Minimum = 5, Maximum = 120, SmallChange = 5 };
    private readonly NumberBox _tokens = new() { Header = "最大输出 Token 数", Minimum = 128, Maximum = 16384, SmallChange = 128 };
    private bool _loading;
    private bool _dialogOpen;
    private bool _localBusy;
    internal ModelManagerPage(string configPath, Action<string, InfoBarSeverity> notify,
        Func<string, CancellationToken, DownloadOperation> startDownload) {
        _store = new(ModelManagerStore.RootForConfig(configPath)); _notify = notify;
        _startDownload = startDownload;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_enabled, "TextAssistantEnabled");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_context, "TextAssistantContext");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_zero, "ZeroCandidateEnabled");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_zeroMode, "ZeroCandidateMode");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_preferred, "ModelPreferredLanguage");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_secondary, "ModelSecondaryLanguage");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_polish, "ModelPolishLevel");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_code, "ModelCodeLevel");
        try { _value = ModelManagerStore.UseCoreRanking(_store.Load()); }
        catch (Exception error) { _value = ModelManagerStore.UseCoreRanking(new()); notify(error.Message, InfoBarSeverity.Error); }
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        var body = new StackPanel { Spacing = 16, Margin = new Thickness(16, 4, 16, 24), MaxWidth = 1040 };
        Content = new ScrollViewer { Content = body, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var intro = Stack(7, Heading("在输入位置调用模型", 24),
            Note("选中文字后按 v，再按 1–4 选择翻译、润色、代码纠错或内容生成。结果直接替换选区；Esc 取消。"));
        body.Children.Add(Card(ResponsivePair(intro, _enabled, 540, fitTrailing: true), padding: 20));
        var add = new Button { Content = "添加模型 API", Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(add, "AddModelApi");
        add.Click += async (_, _) => await EditAsync(null);
        body.Children.Add(Card(Stack(12, ResponsivePair(Stack(4, Heading("API 连接"), _profileCount), add, 380, fitTrailing: true), _profiles)));
        body.Children.Add(Section("0 号候选（暂停）", "自动生成已暂停，等待本地句法搭配的耗时与质量评估。原有配置已保留，本地模型调试和选区功能可继续使用。", _zeroSource, _zero, _zeroMode, _zeroHint));
        body.Children.Add(Section("本地模型", "一键下载 Qwen3-4B 权重及共享 llama.cpp CPU 运行时，用于 0 号候选与离线调试对话。", _localRows));
        foreach (var item in new[] { "一档 · 字词与语法", "二档 · 语法与格式", "三档 · 语义优化与扩写" }) _polish.Items.Add(item);
        foreach (var item in new[] { "一档 · 语法修复与注释", "二档 · 逻辑修复与注释", "三档 · 算法优化与注释" }) _code.Items.Add(item);
        body.Children.Add(Heading("功能偏好", 20));
        body.Children.Add(ResponsivePair(
            Section("翻译目标", "选区语言与首选语言不同则翻译为首选；相同则翻译为次选。", _preferred, _secondary),
            Section("润色与代码纠错", "润色前两档保留原意；代码结果只生成文本，不会编译或执行。", _polish, _code), 640));
        body.Children.Add(ResponsivePair(
            Section("隐私与传输", "最多发送 8192 字符的选区；可另带同一输入位置附近最多 2048 字符的上下文。密码等安全输入不采集。", _context),
            Section("请求预算", "请求独立于本地 libime 排序。API 密钥按当前 Windows 账户加密，不随配置导出。", _timeout, _tokens), 640));
        Apply(); RenderProfiles(); RenderLocalModels();
        _enabled.Toggled += async (_, _) => {
            if (_loading) return;
            if (_enabled.IsOn && !await ConsentAsync()) { Apply(); return; }
            SaveOptions();
        };
        _context.Toggled += (_, _) => SaveOptions();
        _zero.Toggled += async (_, _) => {
            if (_loading) return;
            if (!_zero.IsOn) { Save(_value with { ZeroCandidateEnabled = false }); Apply(); return; }
            var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = "启用本地 0 号候选？", PrimaryButtonText = "启用",
                CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close,
                Content = Note("只在本机对拼音候选排序，不调用云端 API。可按档位使用少量前文，数字会脱敏；密码框、终端及未验证控件不读取。结果按 0 或点击才会上屏。") };
            if (await ShowDialogAsync(dialog) == ContentDialogResult.Primary)
                Save(_value with { ZeroCandidateEnabled = true });
            Apply();
        };
        _zeroMode.SelectionChanged += (_, _) => {
            if (!_loading && _zeroMode.IsEnabled && _zeroMode.SelectedIndex >= 0)
                Save(_value with { ZeroCandidatePerformance = _zeroMode.SelectedIndex == 1,
                    ZeroCandidateNoContext = false });
        };
        _preferred.LostFocus += (_, _) => SaveOptions(); _secondary.LostFocus += (_, _) => SaveOptions();
        _polish.SelectionChanged += (_, _) => SaveOptions(); _code.SelectionChanged += (_, _) => SaveOptions();
        _timeout.ValueChanged += (_, _) => SaveOptions(); _tokens.ValueChanged += (_, _) => SaveOptions();
    }
    private static TextBlock Note(string text) => new() { Text = text, TextWrapping = TextWrapping.Wrap, Opacity = .75 };
    private static TextBlock Heading(string text, double size = 17) => new() { Text = text, FontSize = size, TextWrapping = TextWrapping.Wrap };
    private static StackPanel Stack(double spacing, params UIElement[] children) {
        var panel = new StackPanel { Spacing = spacing };
        foreach (var child in children) panel.Children.Add(child);
        return panel;
    }
    private static Border Card(UIElement child, double padding = 16) => new() {
        Child = child, Padding = new Thickness(padding), CornerRadius = new CornerRadius(10),
        Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"], BorderThickness = new Thickness(1),
        BorderBrush = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"]
    };
    private static Border Section(string title, string description, params UIElement[] fields) =>
        Card(Stack(12, Stack(4, Heading(title), Note(description)), Stack(10, fields)));
    private static Grid ResponsivePair(FrameworkElement first, FrameworkElement second, double breakpoint, bool fitTrailing = false) {
        var grid = new Grid { RowSpacing = 10 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(0) });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        grid.Children.Add(first); grid.Children.Add(second); Grid.SetRow(second, 1);
        if (fitTrailing) second.VerticalAlignment = VerticalAlignment.Center;
        bool wide = false;
        grid.SizeChanged += (_, args) => {
            var next = args.NewSize.Width >= breakpoint;
            if (wide == next) return;
            wide = next;
            grid.ColumnSpacing = wide ? 12 : 0;
            grid.RowSpacing = wide ? 0 : 10;
            grid.ColumnDefinitions[1].Width = !wide ? new GridLength(0) : fitTrailing ? GridLength.Auto : new GridLength(1, GridUnitType.Star);
            Grid.SetRow(second, wide ? 0 : 1); Grid.SetColumn(second, wide ? 1 : 0);
        };
        return grid;
    }
    private async Task<ContentDialogResult> ShowDialogAsync(ContentDialog dialog) {
        if (_dialogOpen) return ContentDialogResult.None;
        _dialogOpen = true;
        try { return await dialog.ShowAsync(); }
        catch { _notify("无法打开确认窗口，请稍后重试。", InfoBarSeverity.Warning); return ContentDialogResult.None; }
        finally { _dialogOpen = false; }
    }
    private void Apply() {
        _loading = true;
        _enabled.IsOn = _value.Enabled; _context.IsOn = _value.IncludeContext;
        _zero.IsOn = false;
        _zero.IsEnabled = false;
        _zeroSource.Text = "状态：已暂停";
        _zeroMode.IsEnabled = false;
        _zeroMode.SelectedIndex = _value.ZeroCandidateNoContext ? -1 : _value.ZeroCandidatePerformance ? 1 : 0;
        _zeroHint.Text = "输入拼音时不会启动 0 号候选请求。";
        _preferred.Text = _value.PreferredLanguage; _secondary.Text = _value.SecondaryLanguage;
        _polish.SelectedIndex = _value.PolishLevel - 1; _code.SelectedIndex = _value.CodeLevel - 1;
        _timeout.Value = _value.TimeoutSeconds; _tokens.Value = _value.MaxOutputTokens;
        _loading = false;
    }
    private LocalModelStore StoreFor(LocalModel model) => LocalModelStore.ForModel(_store.Root, _value.LocalModelStorageParent, model);
    private async Task ChooseLocalStorageAsync() {
        if (_localBusy) return;
        try {
            var picker = new Windows.Storage.Pickers.FolderPicker {
                SuggestedStartLocation = Windows.Storage.Pickers.PickerLocationId.ComputerFolder
            };
            picker.FileTypeFilter.Add("*");
            WinRT.Interop.InitializeWithWindow.Initialize(picker,
                WinRT.Interop.WindowNative.GetWindowHandle(((App)Application.Current).MainWindow));
            if (await picker.PickSingleFolderAsync() is { } folder) UpdateLocalStorage(folder.Path);
        } catch { _notify("无法打开目录选择器。", InfoBarSeverity.Warning); }
    }
    private void UpdateLocalStorage(string path) {
        if (_localBusy || string.Equals(_value.LocalModelStorageParent, path, StringComparison.OrdinalIgnoreCase)) return;
        if (path.Length != 0 && !LocalModelStore.CanChooseStorage(path)) {
            _notify("请选择本地磁盘上的普通文件夹。", InfoBarSeverity.Warning); return;
        }
        if (Save(_value with { LocalModelStorageParent = path })) {
            Apply(); RenderLocalModels();
        }
    }
    private void RenderLocalModels() {
        _localRows.Children.Clear();
        if (!LocalModelStore.Supported) _localRows.Children.Add(Note("本地推理当前支持 Windows x64 和 AVX2 CPU。云端功能不受影响。"));
        var localRoot = StoreFor(LocalModelStore.Catalog[0]).Root;
        var folder = new Button { Content = "选择下载目录…", IsEnabled = !_localBusy };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(folder, "LocalModelStoragePicker");
        folder.Click += async (_, _) => await ChooseLocalStorageAsync();
        var reset = new Button { Content = "使用默认目录", IsEnabled = !_localBusy && _value.LocalModelStorageParent.Length != 0 };
        reset.Click += (_, _) => UpdateLocalStorage("");
        var location = Stack(6, folder, reset); location.Orientation = Orientation.Horizontal;
        _localRows.Children.Add(Stack(6, Note($"Qwen3-4B 储存目录：{localRoot}\n更改目录不会移动或删除已下载文件。"), location));
        var legacy = LocalModelStore.Legacy;
        var models = LocalModelStore.Catalog.AsEnumerable();
        if (StoreFor(legacy).Installed(legacy) || _value.ZeroCandidateLocalModel == legacy.Id) models = models.Append(legacy);
        foreach (var model in models) {
            var modelStore = StoreFor(model);
            bool installed = modelStore.Installed(model);
            var status = installed ? "已下载" :
                model.Id == legacy.Id ? "旧模型文件未找到。" : "权重约 2.50 GB · 未安装共享运行时时另需下载约 24 MB";
            var actions = Stack(8); actions.Orientation = Orientation.Horizontal;
            var get = new Button { Content = "一键下载", IsEnabled = LocalModelStore.Supported && !_localBusy };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(get, "DownloadLocalModel-" + model.Id);
            get.Click += async (_, _) => await DownloadLocalAsync(model);
            if (installed) {
                var remove = new Button { Content = "移除模型", IsEnabled = !_localBusy };
                remove.Click += async (_, _) => {
                    var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = $"移除 {model.Name}？", Content = "仅删除这份模型权重；共享运行时和 API 配置会保留。",
                        PrimaryButtonText = "移除", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close };
                    if (await ShowDialogAsync(dialog) != ContentDialogResult.Primary) return;
                    try {
                        modelStore.Remove(model); Apply(); RenderLocalModels(); _notify("已移除本地模型。", InfoBarSeverity.Success);
                    } catch (IOException) { Apply(); _notify("模型文件正在使用或无法移除，请停止输入后重试。", InfoBarSeverity.Warning); }
                    catch (UnauthorizedAccessException) { Apply(); _notify("当前账户无法移除模型文件。", InfoBarSeverity.Warning); }
                };
                actions.Children.Add(remove);
            } else if (model.Id != legacy.Id) actions.Children.Add(get);
            var debug = new Button { Content = "调试对话…" };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(debug, "DebugLocalModel-" + model.Id);
            debug.Click += async (_, _) => await ShowLocalDebugAsync(model);
            _localRows.Children.Add(Card(Stack(6, Heading(model.Name, 16), Note(model.Description + "\n" + status), actions, debug), 12));
        }
    }
    private async Task DownloadLocalAsync(LocalModel model) {
        if (_localBusy) return;
        _localBusy = true; RenderLocalModels();
        try {
            using var operation = _startDownload(model.Name, CancellationToken.None);
            var store = StoreFor(model);
            await operation.RunAsync(token => store.InstallAsync(model, operation, token));
        } catch (OperationCanceledException) { }
        catch (Exception error) {
            _notify(error is InvalidOperationException or InvalidDataException ? error.Message : "本地模型下载失败，请检查网络、磁盘空间和访问权限。", InfoBarSeverity.Error);
        } finally { _localBusy = false; Apply(); RenderLocalModels(); }
    }
    private bool Save(TextAssistantSettings value) {
        try { _store.Save(value); _value = value; _notify("模型配置已保存。", InfoBarSeverity.Success); return true; }
        catch (Exception error) { _notify(error.Message, InfoBarSeverity.Error); Apply(); return false; }
    }
    private void SaveOptions() {
        if (_loading) return;
        if (!double.IsFinite(_timeout.Value) || !double.IsFinite(_tokens.Value)) return;
        Save(_value with { Enabled = _enabled.IsOn, IncludeContext = _context.IsOn,
            PreferredLanguage = _preferred.Text.Trim(), SecondaryLanguage = _secondary.Text.Trim(),
            PolishLevel = _polish.SelectedIndex + 1, CodeLevel = _code.SelectedIndex + 1,
            TimeoutSeconds = (int)_timeout.Value, MaxOutputTokens = (int)_tokens.Value });
    }
    private async Task<bool> ConsentAsync() {
        var profile = _value.Profiles.FirstOrDefault(p => p.Id == _value.ActiveProfile && p.Enabled);
        if (profile is null || profile.ProtectedKey.Length == 0) { _notify("请先添加带密钥的模型并设为默认。", InfoBarSeverity.Warning); return false; }
        var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = "启用选区大模型功能", PrimaryButtonText = "我同意并启用", CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
            Content = Note($"将向 {ModelManagerStore.Endpoint(profile.Endpoint).Host}（模型 {profile.Model}）发送你选中的文字、应用程序名称，以及你允许的局部上下文。第三方可能保存这些内容，按其政策处理并收取 API 费用。\n\n选择功能后结果将直接替换选区；可能改变原意或产生错误代码。不会执行代码或系统操作。Esc 可取消本机请求，但已发送的网络请求及费用不保证可撤回。\n\n选区功能仅在明确选中文字、按 v 并选择功能后发起请求。0 号候选另行授权。") };
        if (await ShowDialogAsync(dialog) != ContentDialogResult.Primary) return false;
        _value = _value with { PrivacyConsent = true };
        return true;
    }
    private void RenderProfiles() {
        _profiles.Children.Clear();
        _profileCount.Text = $"{_value.Profiles.Count} 个连接 · {_value.Profiles.Count(profile => profile.Enabled)} 个启用";
        if (_value.Profiles.Count == 0) _profiles.Children.Add(Note("尚未添加模型。兼容 Chat Completions 的 HTTPS API，可填写 /v1 基础地址或完整 /chat/completions 地址。"));
        foreach (var profile in _value.Profiles) {
            var details = Stack(4, Heading(profile.Name + (profile.Id == _value.ActiveProfile ? " · 默认" : "")),
                Note($"{profile.Model} · {ModelManagerStore.Endpoint(profile.Endpoint).Host}"));
            var enabled = new ToggleSwitch { Header = "启用此连接", IsOn = profile.Enabled, MinWidth = 0 };
            enabled.Toggled += (_, _) => {
                var disablingActive = profile.Id == _value.ActiveProfile && !enabled.IsOn;
                Save(_value with { Profiles = _value.Profiles.Select(p => p.Id == profile.Id ? p with { Enabled = enabled.IsOn } : p).ToList(),
                    Enabled = disablingActive ? false : _value.Enabled, PrivacyConsent = disablingActive ? false : _value.PrivacyConsent });
                Apply(); RenderProfiles();
            };
            var use = new Button { Content = "设为默认", IsEnabled = profile.Enabled && profile.Id != _value.ActiveProfile };
            use.Click += (_, _) => {
                if (Save(_value with { ActiveProfile = profile.Id, Enabled = false, PrivacyConsent = false })) { Apply(); RenderProfiles(); }
            };
            var edit = new Button { Content = "编辑" }; edit.Click += async (_, _) => await EditAsync(profile);
            var test = new Button { Content = "测试连接", IsEnabled = profile.Enabled };
            test.Click += async (_, _) => {
                var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = "测试模型连接", Content = "仅向此 API 发送固定测试文字“连接测试”，不发送选区和上下文；可能产生费用。", PrimaryButtonText = "发送测试", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close };
                if (await ShowDialogAsync(dialog) != ContentDialogResult.Primary) return;
                test.IsEnabled = false;
                try {
                    using var cancellation = new CancellationTokenSource(TimeSpan.FromSeconds(_value.TimeoutSeconds));
                    await TextAssistantApi.ExecuteAsync(_value with { ActiveProfile = profile.Id, Enabled = true, PrivacyConsent = true, IncludeContext = false },
                        new("translate", "连接测试", "", "OwO.Settings"), cancellation.Token);
                    _notify("模型连接成功。", InfoBarSeverity.Success);
                } catch (Exception error) { _notify(error is InvalidOperationException ? error.Message : "连接失败，请检查网络、API 地址、密钥和模型。", InfoBarSeverity.Error); }
                finally { test.IsEnabled = true; }
            };
            var remove = new Button { Content = "删除" }; remove.Click += async (_, _) => {
                var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = "删除模型？", Content = "将删除此模型及本机保存的加密 API 密钥。", PrimaryButtonText = "删除", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close };
                if (await ShowDialogAsync(dialog) != ContentDialogResult.Primary) return;
                if (Save(_value with { Profiles = _value.Profiles.Where(p => p.Id != profile.Id).ToList(),
                    ActiveProfile = _value.ActiveProfile == profile.Id ? "" : _value.ActiveProfile, Enabled = false, PrivacyConsent = false })) { Apply(); RenderProfiles(); }
            };
            var primary = Stack(8, edit, test); primary.Orientation = Orientation.Horizontal;
            var secondary = Stack(8, use, remove); secondary.Orientation = Orientation.Horizontal;
            _profiles.Children.Add(Card(Stack(10, ResponsivePair(details, enabled, 430, fitTrailing: true),
                ResponsivePair(primary, secondary, 430, fitTrailing: true)), 12));
        }
    }
    private async Task EditAsync(UserModelProfile? existing) {
        var name = new TextBox { Header = "显示名称", Text = existing?.Name ?? "", MaxLength = 80 };
        var endpoint = new TextBox { Header = "HTTPS API 地址", Text = existing?.Endpoint ?? "", PlaceholderText = "https://服务地址/v1", MaxLength = 2048 };
        var model = new TextBox { Header = "模型 ID", Text = existing?.Model ?? "", MaxLength = 160 };
        var key = new PasswordBox { Header = "API 密钥（留空保留已保存密钥）", MaxLength = 4096 };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(name, "ModelApiName");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(endpoint, "ModelApiEndpoint");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(model, "ModelApiModel");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(key, "ModelApiKey");
        var legacy = new ToggleSwitch { Header = "使用 max_tokens（关闭则使用 max_completion_tokens）", IsOn = existing?.LegacyTokenLimit ?? true };
        var content = new StackPanel { Spacing = 12 }; content.Children.Add(name); content.Children.Add(endpoint); content.Children.Add(model); content.Children.Add(key); content.Children.Add(legacy);
        content.Children.Add(Note("保存不会立即联网。API 连接只用于选区文字助手。"));
        var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = existing is null ? "添加模型 API" : "编辑模型 API", Content = content,
            PrimaryButtonText = "保存", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close };
        dialog.PrimaryButtonClick += (_, e) => {
            try {
                var encrypted = key.Password.Length == 0 ? existing?.ProtectedKey ?? "" : ModelKeyProtection.Protect(key.Password);
                if (encrypted.Length == 0) throw new InvalidOperationException("请填写 API 密钥。");
                var profile = new UserModelProfile(existing?.Id ?? Guid.NewGuid().ToString("N"), name.Text.Trim(), endpoint.Text.Trim(), model.Text.Trim(), encrypted, true, legacy.IsOn);
                var profiles = _value.Profiles.Where(p => p.Id != profile.Id).Append(profile).ToList();
                var value = _value with { Profiles = profiles, ActiveProfile = _value.ActiveProfile.Length == 0 ? profile.Id : _value.ActiveProfile, Enabled = false, PrivacyConsent = false };
                ModelManagerStore.Validate(value);
                if (!Save(value)) { e.Cancel = true; return; }
                key.Password = ""; Apply(); RenderProfiles();
            } catch (Exception error) { e.Cancel = true; _notify(error.Message, InfoBarSeverity.Error); }
        };
        await ShowDialogAsync(dialog); key.Password = "";
    }
}
