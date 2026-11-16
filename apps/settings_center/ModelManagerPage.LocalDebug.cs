using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Windowing;
using Windows.Graphics;

namespace OwO_Settings;

internal sealed partial class ModelManagerPage {
    private Window? _debugWindow;
    private static InputScope DebugTextScope() {
        var scope = new InputScope();
        scope.Names.Add(new InputScopeName { NameValue = InputScopeNameValue.Default });
        return scope;
    }

    private async Task ShowLocalDebugAsync(LocalModel model) {
        if (_debugWindow is { } existing) { existing.Activate(); return; }
        var localStore = StoreFor(model);
        bool installed = localStore.Installed(model), closed = false;
        using var lifetime = new CancellationTokenSource();
        CancellationTokenSource? current = null;
        var history = new List<LocalChatTurn>();
        var display = new List<LocalChatTurn>();
        var mode = new ToggleSwitch { Header = "复现 0 号候选（只输入拼音）", IsOn = false };
        var system = new TextBox { Header = "系统提示（可选）", MaxLength = LocalModelChat.MaxSystemLength, InputScope = DebugTextScope(),
            PlaceholderText = "自由对话时使用；留空则不附加系统提示" };
        var input = new TextBox { Header = "消息", PlaceholderText = "输入调试消息…", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
            MaxLength = LocalModelChat.MaxUserLength, MinHeight = 68, MaxHeight = 100, InputScope = DebugTextScope() };
        var conversation = new TextBlock { Text = "暂无对话", IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap };
        var scroll = new ScrollViewer { Content = conversation, MaxHeight = 180, MinHeight = 75,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        var state = Note(installed ? "自由对话只交给此本地模型；关闭即清除记录。"
            : "此模型未安装或设备不支持；仍可复现 Core 的 0 号候选。");
        var send = new Button { Content = "发送", IsEnabled = false, Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
        var stop = new Button { Content = "停止", IsEnabled = false };
        var clear = new Button { Content = "清空" };
        var close = new Button { Content = "关闭调试", HorizontalAlignment = HorizontalAlignment.Right };
        AutomationProperties.SetAutomationId(send, "LocalModelDebugSend");
        AutomationProperties.SetAutomationId(input, "LocalModelDebugInput");
        AutomationProperties.SetAutomationId(system, "LocalModelDebugSystemPrompt");
        AutomationProperties.SetAutomationId(mode, "LocalModelDebugCandidateMode");
        AutomationProperties.SetAutomationId(close, "LocalModelDebugClose");
        var buttons = Stack(6, send, stop, clear);
        buttons.Orientation = Orientation.Horizontal;
        var body = Stack(8, Heading(model.Name, 20), state, mode, system, scroll, input, buttons, close);
        body.Margin = new Thickness(24);
        var view = new ScrollViewer { Content = body, RequestedTheme = ActualTheme, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        AutomationProperties.SetAutomationId(view, "LocalModelDebugDialog");
        // WinUI's modal popup reports an invisible accessibility wrapper to
        // TSF. A dedicated WinUI window gives the editor its own input view.
        var owner = ((App)Application.Current).MainWindow;
        var window = new Window { Title = "本地模型调试", Content = view, SystemBackdrop = new MicaBackdrop() };
        _debugWindow = window;
        var done = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        void CloseWithSettings(object sender, WindowEventArgs args) => window.Close();
        owner.Closed += CloseWithSettings;
        window.Closed += (_, _) => {
            closed = true; lifetime.Cancel(); _debugWindow = null;
            owner.Closed -= CloseWithSettings;
            done.TrySetResult();
        };
        input.Loaded += (_, _) => input.Focus(FocusState.Programmatic);
        close.Click += (_, _) => window.Close();

        void UpdateControls() {
            bool busy = current is not null;
            send.IsEnabled = (installed || mode.IsOn) && !busy && !string.IsNullOrWhiteSpace(input.Text);
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
            input.PlaceholderText = mode.IsOn ? "例如：xianzai（OwO 中按 Enter 保留原拼音）" : "输入调试消息…";
            state.Text = mode.IsOn
                ? "请求实际 Core 候选格，再由本机 Qwen3-4B 选优；不读取其他窗口或前文。"
                : installed ? "文字只交给此本地模型；关闭即清除记录。" : "此模型未安装或设备不支持。";
            UpdateControls();
        };
        clear.Click += (_, _) => { ResetHistory(); state.Text = "对话已清空。"; input.Focus(FocusState.Programmatic); };
        stop.Click += (_, _) => current?.Cancel();
        send.Click += async (_, _) => {
            var message = input.Text.Trim();
            if ((!installed && !mode.IsOn) || current is not null || message.Length == 0) return;
            using var run = CancellationTokenSource.CreateLinkedTokenSource(lifetime.Token);
            current = run;
            UpdateControls();
            state.Text = mode.IsOn ? "正在请求 Core 候选格…" : "正在启动本地模型并生成…";
            try {
                if (mode.IsOn) {
                    var result = await ZeroCandidateDebug.ProbeAsync(message, _value.ZeroCandidatePerformance, _store, _value, run.Token);
                    if (!closed) AddTurn(message, result.Paths.Length == 0 ? "没有整串候选。" :
                        $"重排前：{result.Initial}\n最终 0 号：{result.Final}\n来源：{(result.QwenApplied ? "本地 Qwen3-4B" : "Core 初排")}\n候选格：{string.Join("、", result.Paths)}");
                } else {
                    var reply = await LocalModelChat.SendAsync(localStore, model, system.Text, history.ToArray(), message, run.Token);
                    if (!closed) {
                        history.Add(new(message, reply.Text));
                        if (history.Count > LocalModelChat.MaxTurns) history.RemoveAt(0);
                        AddTurn(message, reply.Text + (reply.Truncated ? "\n[已达到输出长度上限]" : ""));
                    }
                }
                if (!closed) { state.Text = mode.IsOn ? "候选格检查完成。" : "本地推理完成。"; if (input.Text.Trim() == message) input.Text = ""; }
            } catch (OperationCanceledException) {
                if (!closed) state.Text = run.IsCancellationRequested ? "已停止。" : "本地推理超时。";
            } catch (Exception error) {
                if (!closed) {
                    var diagnostic = error is LocalModelFailure issue ? $"{issue.Message}（{issue.Code}）"
                        : error is InvalidOperationException or ArgumentException or InvalidDataException ? error.Message
                        : mode.IsOn ? "无法读取候选格，请确认新版 Core 正在运行。" : "本地推理失败，请检查模型及运行时。";
                    state.Text = diagnostic;
                }
            } finally {
                current = null;
                if (!closed) UpdateControls();
            }
        };
        var area = DisplayArea.GetFromWindowId(owner.AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        var dpi = XamlRoot?.RasterizationScale ?? 1.0;
        var width = Math.Min((int)(660 * dpi), area.Width);
        var height = Math.Min((int)(620 * dpi), area.Height);
        window.AppWindow.MoveAndResize(new RectInt32(area.X + (area.Width - width) / 2,
            area.Y + (area.Height - height) / 2, width, height));
        window.Activate();
        await done.Task;
    }
}
