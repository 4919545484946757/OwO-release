using Microsoft.UI.Xaml.Controls;
using System.Text.Json;
using Windows.Storage.Pickers;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private async Task LoadPluginsAsync()
    {
        SetBusy(true);
        try {
            var value = await _pluginClient.LoadAsync();
            PluginVersions.ItemsSource = value.Plugins;
            RecoveryItems.ItemsSource = value.Recovery;
            RebuildPluginNavigation(value.Plugins);
            NoPlugins.Visibility = value.Plugins.Count == 0
                ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
            NoRecovery.Visibility = value.Recovery.Count == 0
                ? Microsoft.UI.Xaml.Visibility.Visible : Microsoft.UI.Xaml.Visibility.Collapsed;
            ShowStatus($"插件状态已刷新：{value.Plugins.Count} 个版本，{value.Recovery.Count} 个恢复项。",
                       InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private void RebuildPluginNavigation(IReadOnlyList<PluginVersionSnapshot> plugins)
    {
        foreach (var item in SettingsNavigation.MenuItems.OfType<NavigationViewItem>()
                     .Where(item => item.Tag is string tag &&
                                    tag.StartsWith("plugin:", StringComparison.Ordinal)).ToArray())
            SettingsNavigation.MenuItems.Remove(item);
        if (_pluginPagesHeader is not null) SettingsNavigation.MenuItems.Remove(_pluginPagesHeader);
        _pluginPages.Clear();
        _pluginSearchItems.Clear();
        if (plugins.Count == 0) return;
        _pluginPagesHeader = new NavigationViewItemHeader { Content = "插件设置" };
        SettingsNavigation.MenuItems.Add(_pluginPagesHeader);
        foreach (var plugin in plugins.OrderByDescending(value => value.Active)
                                      .ThenBy(value => value.Name, StringComparer.Ordinal)) {
            var page = $"plugin:{plugin.Id}:{plugin.Version}";
            _pluginPages[page] = plugin;
            SettingsNavigation.MenuItems.Add(new NavigationViewItem {
                Content = plugin.Name, Tag = page,
                Icon = new FontIcon { Glyph = "\uE74C" },
            });
            _pluginSearchItems.Add(new SearchItem(plugin.Name,
                $"插件 {plugin.Id} {plugin.Version}", page));
            try {
                var model = PluginSettingsModel.Load(_pluginClient.StorePath, plugin);
                foreach (var field in model.Fields)
                    _pluginSearchItems.Add(new SearchItem(field.Title,
                        $"{plugin.Name} {field.Description}", page, null, field.Key));
            } catch {
                // A damaged or legacy schema remains manageable from the main plugin page.
            }
        }
    }

    private async void PluginVersionSettings_Click(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginVersionSnapshot plugin }) return;
        var page = $"plugin:{plugin.Id}:{plugin.Version}";
        var navigation = SettingsNavigation.MenuItems.OfType<NavigationViewItem>()
            .FirstOrDefault(value => Equals(value.Tag, page));
        if (navigation is not null) SettingsNavigation.SelectedItem = navigation;
        await ShowPluginSettingsAsync(plugin);
    }

    private Task ShowPluginSettingsAsync(PluginVersionSnapshot plugin,
                                          string? focusField = null)
    {
        try {
            _pluginSettings = PluginSettingsModel.Load(_pluginClient.StorePath, plugin);
            _currentPage = $"plugin:{plugin.Id}:{plugin.Version}";
            UpdateSchemeHintVisibility();
            PluginConfigFields.Children.Clear();
            _pluginConfigControls.Clear();
            _pluginConfigExpanders.Clear();
            _buildingPluginSettings = true;
            try { AddPluginSettingFields(); }
            finally { _buildingPluginSettings = false; }
            GeneralSettingsPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            UserLexiconPageHost.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            ShortcutSettingsPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            PluginSettingsPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            ChinesePunctuationPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            GeneralPreferencesPage.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            PluginConfigPage.Visibility = Microsoft.UI.Xaml.Visibility.Visible;
            ResetPageButton.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            SaveButton.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            ReloadButton.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            CurrentPageTitle.Text = _pluginSettings.Title;
            CurrentPageDescription.Text = $"{plugin.Name} {plugin.Version} · {plugin.Id}";
            PluginConfigNotice.Message = _pluginSettings.Description;
            PluginConfigNotice.Severity = InfoBarSeverity.Informational;
            PluginConfigSaveButton.IsEnabled = _pluginSettings.Fields.Count != 0;
            if (_pluginSettings.Fields.Count == 0) {
                PluginConfigNotice.Message = "该插件没有声明可配置字段。";
            }
            if (focusField is not null && _pluginConfigControls.TryGetValue(focusField, out var target))
                DispatcherQueue.TryEnqueue(() => {
                    if (_pluginConfigExpanders.TryGetValue(focusField, out var expander))
                        expander.IsExpanded = true;
                    target.StartBringIntoView();
                    target.Focus(Microsoft.UI.Xaml.FocusState.Programmatic);
                });
        } catch (Exception error) {
            ShowPage("plugins");
            SettingsNavigation.SelectedItem = PluginNavigationItem;
            ShowStatus($"无法打开插件设置：{error.Message}", InfoBarSeverity.Error);
        }
        return Task.CompletedTask;
    }

    private void AddPluginSettingFields()
    {
        var grouped = new HashSet<string>(StringComparer.Ordinal);
        foreach (var field in _pluginSettings!.Fields) {
            if (grouped.Contains(field.Key)) continue;
            PluginSettingField? opacity = null;
            if (field.Format == "color" && field.Key.EndsWith("_color", StringComparison.Ordinal)) {
                var opacityKey = field.Key[..^"_color".Length] + "_opacity";
                opacity = _pluginSettings.Fields.FirstOrDefault(value =>
                    value.Key == opacityKey && value.Type is "integer" or "number");
                if (opacity is not null) grouped.Add(opacity.Key);
            }
            AddPluginSettingField(field, opacity);
        }
    }

    private static StackPanel PluginSettingText(PluginSettingField field)
    {
        var title = new TextBlock { Text = field.Title, FontWeight =
            Microsoft.UI.Text.FontWeights.SemiBold };
        var text = new StackPanel { Spacing = 3 };
        text.Children.Add(title);
        if (!string.IsNullOrWhiteSpace(field.Description)) text.Children.Add(new TextBlock {
            Text = field.Description, TextWrapping = Microsoft.UI.Xaml.TextWrapping.Wrap,
            Foreground = (Microsoft.UI.Xaml.Media.Brush)Microsoft.UI.Xaml.Application.Current.Resources[
                "TextFillColorSecondaryBrush"], FontSize = 12,
        });
        return text;
    }

    private Microsoft.UI.Xaml.FrameworkElement CreatePluginSettingControl(
        PluginSettingField field)
    {
        var value = _pluginSettings!.Values[field.Key];
        Microsoft.UI.Xaml.FrameworkElement control;
        if (field.Options.Count != 0) {
            var combo = new ComboBox { ItemsSource = field.Options,
                DisplayMemberPath = nameof(PluginSettingOption.Label) };
            combo.SelectedItem = field.Options.FirstOrDefault(option =>
                option.Value.GetRawText() == value.GetRawText()) ?? field.Options[0];
            control = combo;
        } else if (field.Format == "color") {
            var color = value.ValueKind == JsonValueKind.String
                ? ParsePluginColor(value.GetString()) : Windows.UI.Color.FromArgb(255, 0, 0, 0);
            control = new ColorPicker {
                Color = color,
                IsAlphaEnabled = false,
                IsAlphaSliderVisible = false,
                IsAlphaTextInputVisible = false,
                IsColorChannelTextInputVisible = true,
                IsHexInputVisible = true,
                IsColorSpectrumVisible = true,
                IsColorSliderVisible = true,
                MinWidth = 300,
            };
        } else if (field.Type == "boolean") {
            control = new ToggleSwitch { IsOn = value.ValueKind == JsonValueKind.True,
                OnContent = "开", OffContent = "关" };
        } else if (field.Type is "integer" or "number") {
            control = new NumberBox { SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline,
                Minimum = field.Minimum ?? double.MinValue,
                Maximum = field.Maximum ?? double.MaxValue,
                SmallChange = field.Step ?? 1.0,
                Value = value.TryGetDouble(out var number) ? number : 0 };
        } else {
            control = new TextBox { Text = value.ValueKind == JsonValueKind.String
                ? value.GetString() ?? "" : "" };
        }
        control.VerticalAlignment = Microsoft.UI.Xaml.VerticalAlignment.Center;
        ObserveAutoSave(control, plugin: true);
        _pluginConfigControls[field.Key] = control;
        return control;
    }

    private void AddPluginSettingField(PluginSettingField field,
                                       PluginSettingField? opacityField = null)
    {
        var firstColorField = field.Format == "color" &&
            !_pluginConfigControls.Values.Any(value => value is ColorPicker);
        var text = PluginSettingText(field);
        var grid = new Grid { ColumnSpacing = 24 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new Microsoft.UI.Xaml.GridLength(1,
            Microsoft.UI.Xaml.GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = Microsoft.UI.Xaml.GridLength.Auto });
        var control = CreatePluginSettingControl(field);
        Microsoft.UI.Xaml.FrameworkElement cardContent;
        var cardPadding = new Microsoft.UI.Xaml.Thickness(16, 12, 8, 12);
        if (field.Format == "color") {
            var colorControls = new StackPanel { Spacing = 12 };
            colorControls.Children.Add(control);
            if (opacityField is not null) {
                colorControls.Children.Add(new Border {
                    Height = 1,
                    Background = (Microsoft.UI.Xaml.Media.Brush)
                        Microsoft.UI.Xaml.Application.Current.Resources[
                            "DividerStrokeColorDefaultBrush"],
                });
                var opacityGrid = new Grid { ColumnSpacing = 24 };
                opacityGrid.ColumnDefinitions.Add(new ColumnDefinition {
                    Width = new Microsoft.UI.Xaml.GridLength(
                        1, Microsoft.UI.Xaml.GridUnitType.Star),
                });
                opacityGrid.ColumnDefinitions.Add(new ColumnDefinition {
                    Width = Microsoft.UI.Xaml.GridLength.Auto,
                });
                opacityGrid.Children.Add(PluginSettingText(opacityField));
                var opacityControl = CreatePluginSettingControl(opacityField);
                Grid.SetColumn(opacityControl, 1);
                opacityGrid.Children.Add(opacityControl);
                colorControls.Children.Add(opacityGrid);
            }
            var colorBody = new Border {
                Child = colorControls,
                Padding = new Microsoft.UI.Xaml.Thickness(16, 0, 16, 16),
                HorizontalAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Stretch,
            };
            var expander = new Expander {
                Header = text,
                Content = colorBody,
                IsExpanded = firstColorField,
                HorizontalAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Stretch,
                HorizontalContentAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Stretch,
            };
            cardContent = expander;
            cardPadding = new Microsoft.UI.Xaml.Thickness(0);
            _pluginConfigExpanders[field.Key] = expander;
            if (opacityField is not null)
                _pluginConfigExpanders[opacityField.Key] = expander;
        } else {
            grid.Children.Add(text);
            Grid.SetColumn(control, 1);
            grid.Children.Add(control);
            cardContent = grid;
        }
        var card = new Border { Child = cardContent, Padding = cardPadding,
            CornerRadius = new Microsoft.UI.Xaml.CornerRadius(8), BorderThickness = new Microsoft.UI.Xaml.Thickness(1),
            Background = (Microsoft.UI.Xaml.Media.Brush)Microsoft.UI.Xaml.Application.Current.Resources[
                "CardBackgroundFillColorDefaultBrush"],
            BorderBrush = (Microsoft.UI.Xaml.Media.Brush)Microsoft.UI.Xaml.Application.Current.Resources[
                "CardStrokeColorDefaultBrush"] };
        PluginConfigFields.Children.Add(card);
    }

    private Dictionary<string, object?> CapturePluginValues()
    {
            var values = new Dictionary<string, object?>(StringComparer.Ordinal);
            foreach (var field in _pluginSettings!.Fields) {
                var control = _pluginConfigControls[field.Key];
                values[field.Key] = control switch {
                    ToggleSwitch toggle => toggle.IsOn,
                    NumberBox number when field.Type == "integer" => checked((long)Math.Round(number.Value)),
                    NumberBox number => number.Value,
                    ComboBox { SelectedItem: PluginSettingOption option } =>
                        JsonSerializer.Deserialize<object>(option.Value.GetRawText()),
                    ColorPicker picker =>
                        $"#{picker.Color.R:X2}{picker.Color.G:X2}{picker.Color.B:X2}",
                    TextBox input => input.Text,
                    _ => null,
                };
            }
            return values;
    }

    private void PluginConfigSaveButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (_pluginSettings is null) return;
        try {
            var values = CapturePluginValues();
            _pluginSettings.Save(values);
            _pendingPluginSaves.Remove(_pluginSettings);
            NotifyCandidateThemeChanged();
            ShowStatus($"{_pluginSettings.Plugin.Name} 的设置已保存。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus($"插件设置保存失败：{error.Message}", InfoBarSeverity.Error);
        }
    }

    private void PluginConfigResetButton_Click(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (_pluginSettings is null) return;
        _pluginSettings.Values.Clear();
        foreach (var value in _pluginSettings.DefaultValues())
            _pluginSettings.Values[value.Key] = value.Value;
        PluginConfigFields.Children.Clear();
        _pluginConfigControls.Clear();
        _pluginConfigExpanders.Clear();
        _buildingPluginSettings = true;
        try { AddPluginSettingFields(); }
        finally { _buildingPluginSettings = false; }
        QueuePluginAutoSave();
        ShowStatus(AutoSaveSettings.IsOn ? $"{_pluginSettings.Plugin.Name} 已恢复默认值，正在自动保存。" : $"{_pluginSettings.Plugin.Name} 已恢复默认值；点击“保存插件设置”后应用。",
                   InfoBarSeverity.Success);
    }

    private static Windows.UI.Color ParsePluginColor(string? text)
    {
        if (text is null || text.Length != 7 || text[0] != '#' ||
            !byte.TryParse(text.AsSpan(1, 2), System.Globalization.NumberStyles.HexNumber,
                           null, out var red) ||
            !byte.TryParse(text.AsSpan(3, 2), System.Globalization.NumberStyles.HexNumber,
                           null, out var green) ||
            !byte.TryParse(text.AsSpan(5, 2), System.Globalization.NumberStyles.HexNumber,
                           null, out var blue))
            return Windows.UI.Color.FromArgb(255, 0, 0, 0);
        return Windows.UI.Color.FromArgb(255, red, green, blue);
    }

    private async void PluginReloadButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e) =>
        await LoadPluginsAsync();

    private async void PluginInstallButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        var picker = new FileOpenPicker {
            SuggestedStartLocation = PickerLocationId.Downloads,
            ViewMode = PickerViewMode.List,
        };
        picker.FileTypeFilter.Add(".owopkg");
        picker.FileTypeFilter.Add(".zip");
        var window = ((App)Microsoft.UI.Xaml.Application.Current).MainWindow;
        WinRT.Interop.InitializeWithWindow.Initialize(
            picker, WinRT.Interop.WindowNative.GetWindowHandle(window));
        var package = await picker.PickSingleFileAsync();
        if (package is null) return;
        await InstallPluginSourceAsync(package.Name, package.Path);
    }

    private async void PluginInstallFolderButton_Click(
        object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        var picker = new FolderPicker {
            SuggestedStartLocation = PickerLocationId.Downloads,
            ViewMode = PickerViewMode.List,
        };
        picker.FileTypeFilter.Add("*");
        var window = ((App)Microsoft.UI.Xaml.Application.Current).MainWindow;
        WinRT.Interop.InitializeWithWindow.Initialize(
            picker, WinRT.Interop.WindowNative.GetWindowHandle(window));
        var folder = await picker.PickSingleFolderAsync();
        if (folder is null) return;
        await InstallPluginSourceAsync(folder.Name, folder.Path);
    }

    private async Task InstallPluginSourceAsync(string sourceName, string sourcePath)
    {
        SetBusy(true);
        try {
            var preview = await _pluginClient.InspectInstallAsync(sourcePath);
            PluginInstallSnapshot result;
            if (preview.RequiresRiskConsent) {
                SetBusy(false);
                if (!await ConfirmRiskInstallAsync(sourceName, sourcePath, preview)) return;
                SetBusy(true);
                result = await _pluginClient.InstallRiskAsync(sourcePath, preview.InventorySha256);
            } else {
                SetBusy(false);
                var message = $"{sourceName}\n{sourcePath}\n\n"
                    + $"插件：{preview.Name} {preview.Version}（{preview.PluginId}）\n"
                    + $"发布者：{PluginUiText.Trust(preview.TrustTier)}\n"
                    + "系统将重新核对精确包摘要，随后原子安装并立即启用。";
                if (!await ConfirmAsync("安装插件包", message, "验证并安装")) return;
                SetBusy(true);
                result = await _pluginClient.InstallAsync(sourcePath);
            }
            await LoadPluginsAsync();
            if (result.Activated) NotifyCandidateThemeChanged();
            var publisher = string.IsNullOrWhiteSpace(result.PublisherDisplayName)
                ? PluginUiText.Trust(result.TrustTier) : result.PublisherDisplayName;
            var state = result.Activated ? "已安装并启用" : "已安装但保持停用";
            ShowStatus($"{state} {result.Name} {result.Version}（{result.PluginId}）；发布者：{publisher}。",
                       InfoBarSeverity.Success);
        } catch (Exception error) {
            await LoadPluginsAsync();
            ShowStatus($"插件安装失败：{error.Message}", InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private async Task<bool> ConfirmRiskInstallAsync(
        string packageName, string packagePath, PluginInstallPreviewSnapshot preview)
    {
        var permissions = preview.Permissions.Count == 0 ? "（未申请具名权限）"
            : string.Join("\n", preview.Permissions.Select(
                permission => $"• {PluginUiText.Permission(permission)}（{permission}）"));
        var trustDetail = string.IsNullOrWhiteSpace(preview.TrustDiagnostic)
            ? "未能建立 Windows 发布者信任。" : preview.TrustDiagnostic;
        var acknowledgement = new CheckBox {
            Content = "我已阅读并理解：该插件可能造成隐私泄露、文件或账户损失，风险由我承担。",
        };
        var content = new StackPanel { Spacing = 12 };
        content.Children.Add(new TextBlock {
            Text = $"{packageName}\n{packagePath}\n\n"
                + $"插件：{preview.Name} {preview.Version}（{preview.PluginId}）\n"
                + $"风险：{PluginUiText.Risk(preview.RiskLevel)}\n"
                + $"信任：{PluginUiText.Trust(preview.TrustTier)}\n"
                + $"包摘要：{preview.InventorySha256}\n\n"
                + $"申请的能力：\n{permissions}\n\n"
                + $"验证说明：{trustDetail}\n\n"
                + "风险与免责：第三方插件可能读取、修改、删除或上传你的数据，捕获屏幕/音频、"
                + "覆盖界面、启动程序或以当前 Windows 用户权限执行操作。OwO 项目不代表已审计"
                + "第三方代码，也不保证其安全性、可用性或数据可恢复性。继续安装表示你理解并自行承担后果。"
                + "本授权只绑定上方精确版本与包摘要，更新或新增权限必须重新授权。",
            TextWrapping = Microsoft.UI.Xaml.TextWrapping.Wrap,
        });
        content.Children.Add(acknowledgement);
        var disclosure = new ContentDialog {
            XamlRoot = XamlRoot,
            Title = "高风险插件安全告知",
            Content = new ScrollViewer {
                Content = content,
                MaxHeight = 520,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            },
            PrimaryButtonText = "我已理解风险",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
            IsPrimaryButtonEnabled = false,
        };
        acknowledgement.Checked += (_, _) => disclosure.IsPrimaryButtonEnabled = true;
        acknowledgement.Unchecked += (_, _) => disclosure.IsPrimaryButtonEnabled = false;
        if (await disclosure.ShowAsync() != ContentDialogResult.Primary) return false;
        return await ConfirmAsync("单独授权安装",
            $"只授权安装此包：\n{preview.Name} {preview.Version}\n{preview.PluginId}\n"
                + $"SHA-256 清单摘要：{preview.InventorySha256}\n\n"
                + "安装后将保持停用；启用前仍可撤销权限或卸载。",
            "授权并安装");
    }

    private async Task<bool> ConfirmAsync(string title, string message, string action)
    {
        var dialog = new ContentDialog {
            XamlRoot = XamlRoot,
            Title = title,
            Content = message,
            PrimaryButtonText = action,
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
        };
        return await dialog.ShowAsync() == ContentDialogResult.Primary;
    }

    private async void PluginVersionAction_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginVersionSnapshot plugin }) return;
        var action = plugin.Active ? "停用" : "启用";
        var warning = plugin.Active ? "" : $"\n\n{PluginUiText.Risk(plugin.RiskLevel)} · "
            + $"{PluginUiText.Trust(plugin.TrustTier)}\n"
            + (plugin.Permissions.Count == 0 ? "未申请具名权限" :
               $"已授权：{string.Join("、", plugin.Permissions.Select(PluginUiText.Permission))}");
        var agentPrivacyConsent = false;
        if (!plugin.Active && plugin.Permissions.Contains("agent.ipc", StringComparer.Ordinal)) {
            if (!await ConfirmAgentActivationAsync(plugin)) return;
            agentPrivacyConsent = true;
        } else if (!await ConfirmAsync($"{action}插件",
                       $"{plugin.Name} {plugin.Version}\n{plugin.Id}{warning}", action)) return;
        SetBusy(true);
        try {
            if (plugin.Active) await _pluginClient.DeactivateAsync(plugin.Id, plugin.Version);
            else await _pluginClient.ActivateAsync(
                plugin.Id, plugin.Version, agentPrivacyConsent);
            await LoadPluginsAsync();
            NotifyCandidateThemeChanged();
            ShowStatus($"插件已{action}。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private async Task<bool> ConfirmAgentActivationAsync(PluginVersionSnapshot plugin)
    {
        var acknowledgement = new CheckBox {
            Content = "我明确同意启用，并理解第三方 Agent 可能执行系统操作。",
        };
        var content = new StackPanel { Spacing = 12 };
        content.Children.Add(new TextBlock {
            Text = $"{plugin.Name} {plugin.Version}\n{plugin.Id}\n\n"
                + "启用后，在你以 v 开头输入并按 Enter 或空格确认时，插件可能读取：去除 v 后的输入、"
                + "焦点应用的进程 ID/程序名/窗口类，以及必要的短局部上下文。它会先过滤常见凭据、令牌"
                + "和支付卡号，再把最少数据发送给你配置的本机第三方 Agent。\n\n"
                + "密码框和安全输入框不会采集上下文；不会发送窗口标题、完整文档或剪贴板。第三方 Agent "
                + "仍可能保存请求、关联应用使用习惯，或根据你选择的指令执行文件、程序、系统设置、账户等操作。"
                + "OwO 不审计第三方 Agent，也不保证其操作可恢复。高风险指令不能通过候选框直接确认，必须回到"
                + "第三方 Agent 自身的高风险确认界面。\n\n"
                + "此同意只绑定当前精确插件版本与已授权权限；更新或新增权限需要重新确认。",
            TextWrapping = Microsoft.UI.Xaml.TextWrapping.Wrap,
        });
        content.Children.Add(acknowledgement);
        var dialog = new ContentDialog {
            XamlRoot = XamlRoot,
            Title = "启用 Agent IPC 连接器",
            Content = new ScrollViewer {
                Content = content,
                MaxHeight = 520,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            },
            PrimaryButtonText = "明确同意并启用",
            CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close,
            IsPrimaryButtonEnabled = false,
        };
        acknowledgement.Checked += (_, _) => dialog.IsPrimaryButtonEnabled = true;
        acknowledgement.Unchecked += (_, _) => dialog.IsPrimaryButtonEnabled = false;
        return await dialog.ShowAsync() == ContentDialogResult.Primary;
    }

    private async void PluginVersionRevoke_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginVersionSnapshot plugin } || !plugin.CanRevoke) return;
        var message = $"{plugin.Name} {plugin.Version}\n{plugin.Id}\n\n"
            + "将撤销此精确版本的全部运行权限。插件文件与数据保留；已运行的插件应先停用，"
            + "再次授权需要重新安装该精确包。";
        if (!await ConfirmAsync("撤销插件权限", message, "撤销权限")) return;
        SetBusy(true);
        try {
            if (plugin.Active) await _pluginClient.DeactivateAsync(plugin.Id, plugin.Version);
            await _pluginClient.RevokeAsync(plugin.Id, plugin.Version);
            await LoadPluginsAsync();
            NotifyCandidateThemeChanged();
            ShowStatus("插件权限已撤销，插件已停用。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private async void PluginVersionUninstall_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginVersionSnapshot plugin } || plugin.Active) return;
        var message = $"{plugin.Name} {plugin.Version}\n{plugin.Id}\n\n"
            + "此操作会删除该版本及其精确授权，无法撤销。插件用户数据会保留。";
        if (!await ConfirmAsync("卸载插件版本", message, "卸载")) return;
        SetBusy(true);
        try {
            await _pluginClient.UninstallAsync(plugin.Id, plugin.Version);
            await LoadPluginsAsync();
            ShowStatus("插件版本已卸载；用户数据已保留。", InfoBarSeverity.Success);
        } catch (Exception error) {
            await LoadPluginsAsync();
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }

    private async void RecoveryAction_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: PluginRecoverySnapshot item } || !item.CanApply) return;
        var activating = item.Action == "activate";
        var action = activating ? "切换版本" : "清理";
        if (!await ConfirmAsync(action, $"{item.Title}\n{item.Detail}", action)) return;
        SetBusy(true);
        try {
            if (activating) await _pluginClient.ActivateAsync(item.PluginId, item.Version);
            else await _pluginClient.CleanupAsync(item);
            await LoadPluginsAsync();
            if (activating) NotifyCandidateThemeChanged();
            ShowStatus($"恢复操作“{action}”已完成。", InfoBarSeverity.Success);
        } catch (Exception error) {
            ShowStatus(error.Message, InfoBarSeverity.Error);
        } finally {
            SetBusy(false);
        }
    }
}
