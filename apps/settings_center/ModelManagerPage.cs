using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;

namespace OwO_Settings;

internal sealed class ModelManagerPage : UserControl {
    private readonly ModelManagerStore _store;
    private readonly Action<string, InfoBarSeverity> _notify;
    private TextAssistantSettings _value;
    private readonly StackPanel _profiles = new() { Spacing = 8 };
    private readonly ToggleSwitch _enabled = new() { Header = "选区大模型功能（选中文字后按 v）" };
    private readonly ToggleSwitch _context = new() { Header = "发送同一输入位置附近的局部上下文" };
    private readonly TextBox _preferred = new() { Header = "首选语言", MaxLength = 64 };
    private readonly TextBox _secondary = new() { Header = "次选语言", MaxLength = 64 };
    private readonly ComboBox _polish = new() { Header = "语言润色级别", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ComboBox _code = new() { Header = "代码纠错级别", HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly NumberBox _timeout = new() { Header = "请求超时（秒）", Minimum = 5, Maximum = 120, SmallChange = 5 };
    private readonly NumberBox _tokens = new() { Header = "最大输出 Token 数", Minimum = 128, Maximum = 16384, SmallChange = 128 };
    private bool _loading;
    private bool _dialogOpen;
    internal ModelManagerPage(string configPath, Action<string, InfoBarSeverity> notify) {
        _store = new(ModelManagerStore.RootForConfig(configPath)); _notify = notify;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_enabled, "TextAssistantEnabled");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_context, "TextAssistantContext");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_preferred, "ModelPreferredLanguage");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_secondary, "ModelSecondaryLanguage");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_polish, "ModelPolishLevel");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(_code, "ModelCodeLevel");
        try { _value = _store.Load(); } catch (Exception error) { _value = new(); notify(error.Message, InfoBarSeverity.Error); }
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        var body = new StackPanel { Spacing = 12, Margin = new Thickness(24, 0, 24, 24), MaxWidth = 960 };
        Content = new ScrollViewer { Content = body, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        body.Children.Add(Note("管理用户自己的模型 API，与本地 libime 排序独立。仅在选区存在时按小写 v 打开功能列表，数字 1–4 确认功能；结果直接替换原选区，Esc 取消。"));
        body.Children.Add(_enabled);
        body.Children.Add(_context);
        body.Children.Add(Note("选区最多 8192 字符；局部上下文最多 2048 字符。密码、安全输入、无法确认安全的输入位置不采集。默认关闭，开启前需明确同意向指定服务发送文本。"));
        var add = new Button { Content = "添加模型 API", Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(add, "AddModelApi");
        add.Click += async (_, _) => await EditAsync(null);
        body.Children.Add(add); body.Children.Add(_profiles);
        body.Children.Add(_preferred); body.Children.Add(_secondary);
        foreach (var item in new[] { "一档：仅完善字词、语法，不改原意", "二档：不改原意，适配环境和格式", "三档：优化语义、排版并按完成度扩写" }) _polish.Items.Add(item);
        foreach (var item in new[] { "一档：修复语法、必要符号和变量并注释", "二档：进一步修复逻辑错误并注释", "三档：评估负面影响后优化算法并注释" }) _code.Items.Add(item);
        body.Children.Add(_polish); body.Children.Add(_code);
        body.Children.Add(_timeout); body.Children.Add(_tokens);
        body.Children.Add(Note("代码只生成文本，不编译或执行。模型可能误判、产生不准确内容；请核查结果。API 密钥由 Windows 当前账户加密保存，不随配置导出或安装包分发。"));
        Apply(); RenderProfiles();
        _enabled.Toggled += async (_, _) => {
            if (_loading) return;
            if (_enabled.IsOn && !await ConsentAsync()) { Apply(); return; }
            SaveOptions();
        };
        _context.Toggled += (_, _) => SaveOptions();
        _preferred.LostFocus += (_, _) => SaveOptions(); _secondary.LostFocus += (_, _) => SaveOptions();
        _polish.SelectionChanged += (_, _) => SaveOptions(); _code.SelectionChanged += (_, _) => SaveOptions();
        _timeout.ValueChanged += (_, _) => SaveOptions(); _tokens.ValueChanged += (_, _) => SaveOptions();
    }
    private static TextBlock Note(string text) => new() { Text = text, TextWrapping = TextWrapping.Wrap, Opacity = .75 };
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
        _preferred.Text = _value.PreferredLanguage; _secondary.Text = _value.SecondaryLanguage;
        _polish.SelectedIndex = _value.PolishLevel - 1; _code.SelectedIndex = _value.CodeLevel - 1;
        _timeout.Value = _value.TimeoutSeconds; _tokens.Value = _value.MaxOutputTokens;
        _loading = false;
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
            Content = Note($"将向 {ModelManagerStore.Endpoint(profile.Endpoint).Host}（模型 {profile.Model}）发送你选中的文字、应用程序名称，以及你允许的局部上下文。第三方可能保存这些内容，按其政策处理并收取 API 费用。\n\n选择功能后结果将直接替换选区；可能改变原意或产生错误代码。不会执行代码或系统操作。Esc 可取消本机请求，但已发送的网络请求及费用不保证可撤回。\n\n只有明确选中文字后按 v、再选择功能才发起请求；正常打字不联网。") };
        if (await ShowDialogAsync(dialog) != ContentDialogResult.Primary) return false;
        _value = _value with { PrivacyConsent = true };
        return true;
    }
    private void RenderProfiles() {
        _profiles.Children.Clear();
        if (_value.Profiles.Count == 0) _profiles.Children.Add(Note("尚未添加模型。兼容 Chat Completions 的 HTTPS API，可填写 /v1 基础地址或完整 /chat/completions 地址。"));
        foreach (var profile in _value.Profiles) {
            var panel = new StackPanel { Spacing = 8 };
            panel.Children.Add(new TextBlock { Text = profile.Name + (profile.Id == _value.ActiveProfile ? " · 默认" : ""), FontSize = 17 });
            panel.Children.Add(Note($"{profile.Model} · {ModelManagerStore.Endpoint(profile.Endpoint).Host}"));
            var enabled = new ToggleSwitch { Header = "启用此模型配置", IsOn = profile.Enabled };
            enabled.Toggled += (_, _) => {
                var disablingActive = profile.Id == _value.ActiveProfile && !enabled.IsOn;
                Save(_value with { Profiles = _value.Profiles.Select(p => p.Id == profile.Id ? p with { Enabled = enabled.IsOn } : p).ToList(),
                    Enabled = disablingActive ? false : _value.Enabled, PrivacyConsent = disablingActive ? false : _value.PrivacyConsent });
                Apply(); RenderProfiles();
            };
            panel.Children.Add(enabled);
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            var use = new Button { Content = "设为默认", IsEnabled = profile.Enabled && profile.Id != _value.ActiveProfile };
            use.Click += (_, _) => { if (Save(_value with { ActiveProfile = profile.Id, Enabled = false, PrivacyConsent = false })) { Apply(); RenderProfiles(); } };
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
            actions.Children.Add(use); actions.Children.Add(edit); actions.Children.Add(test); actions.Children.Add(remove);
            panel.Children.Add(actions);
            _profiles.Children.Add(new Border { Child = panel, Padding = new Thickness(16), CornerRadius = new CornerRadius(8),
                Background = (Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"], BorderThickness = new Thickness(1),
                BorderBrush = (Brush)Application.Current.Resources["CardStrokeColorDefaultBrush"] });
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
        content.Children.Add(Note("不会自动请求网络。修改模型或服务地址会关闭选区功能，重新启用时需要再次确认发送目的地。"));
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
