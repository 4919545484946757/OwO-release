using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Windows.ApplicationModel.DataTransfer;

namespace OwO_Settings;

internal sealed partial class ModelManagerPage {
    private async Task ShowLocalDebugAsync(LocalModel model) {
        var localStore = StoreFor(model);
        bool installed = localStore.Installed(model), closed = false;
        using var lifetime = new CancellationTokenSource();
        CancellationTokenSource? current = null;
        var history = new List<LocalChatTurn>();
        var display = new List<LocalChatTurn>();
        var mode = new ToggleSwitch { Header = "复现 0 号候选（只输入拼音）", IsOn = false };
        var system = new TextBox { Header = "系统提示（可选）", MaxLength = LocalModelChat.MaxSystemLength,
            PlaceholderText = "自由对话时使用；留空则不附加系统提示" };
        var input = new TextBox { Header = "消息", PlaceholderText = "输入调试消息…", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
            MaxLength = LocalModelChat.MaxUserLength, MinHeight = 68, MaxHeight = 100 };
        var conversation = new TextBlock { Text = "暂无对话", IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap };
        var scroll = new ScrollViewer { Content = conversation, MaxHeight = 180, MinHeight = 75,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        var state = Note(installed ? "文字只交给此本地模型；关闭即清除记录。"
            : "模型尚未下载，或此设备不支持。可以查看提示词；完成下载后再打开弹窗试跑。");
        var send = new Button { Content = "发送", IsEnabled = false, Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
        var stop = new Button { Content = "停止", IsEnabled = false };
        var clear = new Button { Content = "清空" };
        var copy = new Button { Content = "复制提示词" };
        var prompt = new Expander { Header = "查看 0 号候选提示词", Content = new ScrollViewer {
            MaxHeight = 120, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Content = new TextBlock { Text = TextAssistantApi.LocalCandidatePromptPreview, IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap }
        } };
        AutomationProperties.SetAutomationId(send, "LocalModelDebugSend");
        AutomationProperties.SetAutomationId(input, "LocalModelDebugInput");
        AutomationProperties.SetAutomationId(mode, "LocalModelDebugCandidateMode");
        AutomationProperties.SetAutomationId(copy, "LocalModelDebugCopyPrompt");
        var buttons = Stack(6, send, stop, clear, copy);
        buttons.Orientation = Orientation.Horizontal;
        var body = Stack(8, state, mode, prompt, system, scroll, input, buttons);
        var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = $"本地模型调试 · {model.Name}",
            CloseButtonText = "关闭", Content = new ScrollViewer { Content = body, VerticalScrollBarVisibility = ScrollBarVisibility.Auto } };
        AutomationProperties.SetAutomationId(dialog, "LocalModelDebugDialog");
        dialog.Closing += (_, _) => { closed = true; lifetime.Cancel(); };

        void UpdateControls() {
            bool busy = current is not null;
            send.IsEnabled = installed && !busy && !string.IsNullOrWhiteSpace(input.Text);
            clear.IsEnabled = !busy;
            stop.IsEnabled = busy;
            mode.IsEnabled = !busy;
            system.IsEnabled = !busy && !mode.IsOn;
            system.Visibility = mode.IsOn ? Visibility.Collapsed : Visibility.Visible;
        }
        void AddTurn(string user, string assistant) {
            display.Add(new(user, assistant));
            if (display.Count > LocalModelChat.MaxTurns) display.RemoveAt(0);
            conversation.Text = string.Join("\n\n", display.Select(turn => $"你：{turn.User}\n模型：{turn.Assistant}"));
            scroll.UpdateLayout();
            scroll.ChangeView(null, scroll.ScrollableHeight, null, true);
        }
        void ResetHistory() {
            history.Clear(); display.Clear(); conversation.Text = "暂无对话";
        }
        input.TextChanged += (_, _) => UpdateControls();
        mode.Toggled += (_, _) => {
            ResetHistory();
            input.Text = "";
            input.MaxLength = mode.IsOn ? 96 : LocalModelChat.MaxUserLength;
            input.PlaceholderText = mode.IsOn ? "例如：xianzai" : "输入调试消息…";
            if (installed) state.Text = mode.IsOn
                ? "使用实际候选提示、语法约束及读音验证。不读取前文，不使用普通候选补位。"
                : "文字只交给此本地模型；关闭即清除记录。";
            UpdateControls();
        };
        clear.Click += (_, _) => { ResetHistory(); state.Text = "对话已清空。"; input.Focus(FocusState.Programmatic); };
        stop.Click += (_, _) => current?.Cancel();
        copy.Click += (_, _) => {
            try {
                var data = new DataPackage(); data.SetText(TextAssistantApi.LocalCandidatePromptPreview);
                Clipboard.SetContent(data); Clipboard.Flush(); state.Text = "已复制 0 号候选提示词。";
            } catch { state.Text = "剪贴板暂不可用，可展开上方提示词后手动复制。"; }
        };
        send.Click += async (_, _) => {
            var message = input.Text.Trim();
            if (!installed || current is not null || message.Length == 0) return;
            using var run = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
            current = run;
            UpdateControls();
            state.Text = "正在启动本地模型并生成…";
            string? raw = null;
            try {
                if (mode.IsOn) {
                    var approval = Guid.NewGuid().ToString("N");
                    var settings = new TextAssistantSettings { ZeroCandidateEnabled = true, ZeroCandidateLocalModel = model.Id,
                        ZeroCandidateConsentProfile = ModelManagerStore.LocalConsent(model.Id), ZeroCandidateApproval = approval,
                        LocalModelStorageParent = _value.LocalModelStorageParent,
                        ZeroCandidatePerformance = _value.ZeroCandidatePerformance };
                    var candidate = await TextAssistantApi.ExecuteAsync(settings,
                        new(settings.ZeroCandidatePerformance ? "zero-performance" : "zero-eco", message, "", "", approval),
                        run.Token, modelRoot: _store.Root, localDebugRawOutput: value => raw = value);
                    if (!closed) AddTurn(message, $"原始回复：{raw ?? candidate}\n校验结果：{candidate}");
                } else {
                    var reply = await LocalModelChat.SendAsync(localStore, model, system.Text, history.ToArray(), message, run.Token);
                    if (!closed) {
                        history.Add(new(message, reply.Text));
                        if (history.Count > LocalModelChat.MaxTurns) history.RemoveAt(0);
                        AddTurn(message, reply.Text + (reply.Truncated ? "\n[已达到输出长度上限]" : ""));
                    }
                }
                if (!closed) { state.Text = "本地推理完成。"; if (input.Text.Trim() == message) input.Text = ""; }
            } catch (OperationCanceledException) {
                if (!closed) state.Text = run.IsCancellationRequested ? "已停止。" : "本地推理超时。";
            } catch (Exception error) {
                if (!closed) {
                    var diagnostic = error is ZeroCandidateFailure issue ? $"{issue.Message}（{issue.Code}）"
                        : error is InvalidOperationException or ArgumentException or InvalidDataException ? error.Message
                        : "本地推理失败，请检查模型及运行时。";
                    if (mode.IsOn && raw is not null) AddTurn(message, $"原始回复：{raw}\n校验未通过：{diagnostic}");
                    state.Text = diagnostic;
                }
            } finally {
                current = null;
                if (!closed) UpdateControls();
            }
        };
        await ShowDialogAsync(dialog);
    }
}
