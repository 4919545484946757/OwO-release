using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using System.Text.Json;
using Windows.Storage.Pickers;

namespace OwO_Settings;

internal sealed class CandidateDiagnosticsPage : UserControl {
    private readonly CandidateTimingStore _store = new();
    private readonly Action<string, InfoBarSeverity> _notify;
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromSeconds(2) };
    private readonly ScrollNeutralComboBox _duration = new() { Header = "采样时长", MinWidth = 120 };
    private readonly Button _start = new() { Content = "开始计时" };
    private readonly Button _stop = new() { Content = "停止计时", IsEnabled = false };
    private readonly Button _export = new() { Content = "导出报告", IsEnabled = false };
    private readonly TextBlock _state = Note("尚未开始。计时默认关闭，不影响日常输入。"), _summary = Note("暂无样本。");
    private readonly StackPanel _statistics = new() { Spacing = 4 };
    private readonly ListView _requests = new() { Height = 240, SelectionMode = ListViewSelectionMode.Single };
    private readonly TextBox _details = new() { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
        MinHeight = 160, MaxHeight = 360, FontFamily = new FontFamily("Consolas"), PlaceholderText = "选择一条请求，查看阶段耗时与事件时间线。" };
    private CandidateTimingSnapshot _snapshot = CandidateTimingStore.Build([]);
    private bool _busy, _closed, _pageActive, _windowActive = true;
    private string _lastSnapshot = "";
    private sealed record RowItem(CandidateTimingRow Row) {
        public override string ToString() => $"#{Row.RequestId} · PID {Row.Pid} · 第 {Row.Page + 1} 页 · {Row.InputUnits} 字符\n" +
            $"{Row.Status}　首批 {Ms(Row.FirstMs)}　精化 {Ms(Row.SettledMs)}　模型 {Ms(Row.ModelMs)}　缓存 {Row.CacheHits}/{Row.Rounds}";
    }
    internal CandidateDiagnosticsPage(Action<string, InfoBarSeverity> notify) {
        _notify = notify;
        var body = new StackPanel { Spacing = 12, Margin = new Thickness(24, 0, 24, 24), MaxWidth = 1040 };
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        Content = new ScrollViewer { Content = body, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        body.Children.Add(Note("用于安装版跨设备排查：按“开始计时”后等待 1 秒，切到记事本等应用输入、翻页或展开，再返回查看。已有程序若加载了旧版 TSF，请关闭并重新打开测试程序。当前覆盖内置拼音/精确编码主链路；Agent、选区 API 和插件独立输入方案不计入该统计。"));
        body.Children.Add(Note("仅记录请求 ID、进程 ID、输入长度、配置状态和耗时；不记录输入/候选文字、窗口标题、上下文或密钥，不联网、不自动上传。关闭设置中心或到期停止；每进程最多 8 MiB，界面最多读取 50,000 条事件。原始记录仅保存在本机。"));
        foreach (var minutes in new[] { 1, 5, 10, 30 }) _duration.Items.Add($"{minutes} 分钟");
        _duration.SelectedIndex = 2;
        _start.Style = (Style)Application.Current.Resources["AccentButtonStyle"];
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Bottom };
        actions.Children.Add(_start); actions.Children.Add(_stop);
        var bar = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        bar.Children.Add(_duration); bar.Children.Add(actions); body.Children.Add(bar);
        var reports = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        var refresh = new Button { Content = "刷新结果" };
        var clear = new Button { Content = "清除此轮记录" };
        reports.Children.Add(refresh); reports.Children.Add(_export); reports.Children.Add(clear); body.Children.Add(reports);
        foreach (var (element, id) in new (DependencyObject, string)[] { (_start, "CandidateTimingStart"), (_stop, "CandidateTimingStop"),
            (_export, "CandidateTimingExport"), (refresh, "CandidateTimingRefresh"), (_state, "CandidateTimingState"),
            (_summary, "CandidateTimingSummary"), (_requests, "CandidateTimingRequests"), (_details, "CandidateTimingDetails") })
            AutomationProperties.SetAutomationId(element, id);
        body.Children.Add(_state); body.Children.Add(_summary);
        body.Children.Add(Note("口径：从 TSF 按键回调（鼠标操作从请求入队）到候选快照的界面更新提交；不是物理按键延迟或屏幕实际显示时间。首批、精化和模型应用分开计时；失败、淘汰和缺失记录不计入成功延迟分位数。"));
        body.Children.Add(new Expander { Header = "阶段耗时（P50 / P95 / 最大值）", Content = _statistics,
            HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        body.Children.Add(Note("阶段统计包含已记录的成功、失败和淘汰阶段，以每个请求累计耗时为样本；解析/查询/排序等属于嵌套指标，不能直接相加。缓存命中显示实际零工作量，缺失阶段显示“—”。“传输/等待残差”还包含连接、调度及串行服务排队，并非纯管道传输时间。"));
        body.Children.Add(new TextBlock { Text = "最近请求（最多显示 100 条）", FontSize = 18 });
        body.Children.Add(_requests); body.Children.Add(_details);
        body.Children.Add(Note($"本地目录：{_store.Root}"));
        _timer.Tick += async (_, _) => await RefreshAsync();
        _start.Click += async (_, _) => {
            try {
                _store.Start(new[] { 1, 5, 10, 30 }[_duration.SelectedIndex]);
                _lastSnapshot = ""; UpdateState();
                _notify("本地候选计时已开始；请切到目标应用输入。", InfoBarSeverity.Informational);
                await RefreshAsync();
            } catch (Exception error) { _notify("无法开始计时：" + error.Message, InfoBarSeverity.Error); }
        };
        _stop.Click += async (_, _) => { Stop(); await RefreshAsync(); };
        refresh.Click += async (_, _) => await RefreshAsync();
        _export.Click += async (_, _) => await ExportAsync();
        clear.Click += async (_, _) => {
            if (_busy) return;
            if (_store.Running) { _notify("请先停止计时，再清除此轮记录。", InfoBarSeverity.Warning); return; }
            if (_store.Session is null) return;
            var consent = new ContentDialog { XamlRoot = XamlRoot, Title = "清除此轮计时记录？",
                Content = "只删除本轮本地计时文件，不影响词库、词频、插件和配置。已导出的报告不会删除。",
                PrimaryButtonText = "清除", CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close };
            try {
                if (await consent.ShowAsync() != ContentDialogResult.Primary) return;
                if (_busy) { _notify("正在刷新，请稍后重试清除。", InfoBarSeverity.Informational); return; }
                _store.Clear();
                _snapshot = CandidateTimingStore.Build([]); _lastSnapshot = ""; Render();
                _notify("本轮计时文件已清除；无法从应用恢复。", InfoBarSeverity.Success);
            } catch (Exception error) { _notify("清除失败：" + error.Message, InfoBarSeverity.Error); }
        };
        _requests.SelectionChanged += (_, _) => RenderDetails();
    }

    internal void SetPageActive(bool active) {
        _pageActive = active;
        UpdatePolling();
    }
    internal void SetWindowActive(bool active) {
        _windowActive = active;
        UpdatePolling();
    }
    private void UpdatePolling() {
        if (_closed) return;
        if (_pageActive && _windowActive) { _timer.Start(); _ = RefreshAsync(); } else _timer.Stop();
    }
    internal void Close() {
        if (_closed) return;
        _closed = true; _timer.Stop();
        try { _store.Stop(); } catch { /* Controller liveness/expiry still fails closed. */ }
    }
    private void Stop() {
        try { _store.Stop(); }
        catch (Exception error) { _notify("停止标记写入失败；采样将在到期后自动停止：" + error.Message, InfoBarSeverity.Warning); }
        UpdateState();
    }
    private static TextBlock Note(string text) => new() { Text = text, TextWrapping = TextWrapping.Wrap };
    private static string Ms(double? value) => value is null ? "—" : $"{value:0.00} ms";
    private void UpdateState() {
        _start.IsEnabled = !_store.Running && !_busy;
        _stop.IsEnabled = _store.Running;
        _duration.IsEnabled = !_store.Running;
        _export.IsEnabled = _snapshot.Events.Count > 0;
        _state.Text = _store.Running ? $"正在计时 · 剩余 {Math.Max(0, (_store.Deadline - Environment.TickCount64) / 1000)} 秒" :
            _store.Session is null ? "尚未开始。" : "已停止采样；结果保留在本机。";
    }
    private async Task RefreshAsync() {
        if (_busy || _closed) return;
        _busy = true; UpdateState();
        try {
            _snapshot = await Task.Run(_store.Scan);
            if (_closed) return;
            var fingerprint = $"{_snapshot.Events.Count}/{_snapshot.Dropped}/{_snapshot.Warning}";
            if (_lastSnapshot != fingerprint) { Render(); _lastSnapshot = fingerprint; }
        } catch (Exception error) { _notify("计时记录读取失败：" + error.Message, InfoBarSeverity.Warning); }
        finally { _busy = false; if (!_closed) UpdateState(); }
    }

    private static readonly (string Key, string Title)[] StageNames = [
        ("input", "按键处理至入队"), ("preedit", "预编辑同步/即时预览更新"), ("queue", "TSF 工作者排队"), ("ipc", "基础/精化 IPC 往返"),
        ("service_wait", "Core 接收前等待（连接/排队/传输）"), ("return_wait", "Core 准备完成至 TSF 收到（含编码/写出）"),
        ("transport_wait", "传输/服务等待残差"), ("core", "Core 处理总计（含后续符号行）"),
        ("prepare", "配置/词库刷新与准备"), ("parse", "解析总计"), ("normalize", "　输入规范化"), ("segment", "　拼音分割"),
        ("correction", "　纠错解析"), ("assistance", "　不完整拼音/辅助解析"), ("generation", "候选召回/组合"), ("lookup", "　词典查询"),
        ("sort", "　生成器排序"), ("layout", "Core 候选布局/分页"),
        ("model_prepare", "　模型候选元数据准备"), ("encode", "Core 响应编码"),
        ("write", "Core 响应写出"), ("convert", "TSF 解码/转换"), ("dispatch", "UI 消息调度"),
        ("render", "界面更新/绘制提交"), ("model", "ModelHost 模型评分"),
        ("model_queue", "Core 模型工作队列等待"),
        ("model_ipc", "Core→ModelHost 往返"), ("model_poll", "TSF 等待模型更新") ];

    private void Render() {
        var rows = _snapshot.Rows;
        var measured = rows.Where(r => r.Status != "失败/超时" && r.Status != "已淘汰").ToArray();
        var success = measured.Where(r => r.FirstMs is not null).ToArray();
        string Distribution(IEnumerable<double> values) {
            var items = values.ToArray();
            return items.Length == 0 ? "—（0 样本）" :
                $"P50 {CandidateTimingStore.Percentile(items, .5):0.00} ms · P95 {CandidateTimingStore.Percentile(items, .95):0.00} ms · 最大 {items.Max():0.00} ms（{items.Length} 样本）";
        }
        _summary.Text = $"请求 {rows.Count} · 已有可用候选 {success.Length} · 失败/超时 {rows.Count(r => r.Status == "失败/超时")} · 淘汰 {rows.Count(r => r.Status == "已淘汰")}\n" +
            $"首批可用：{Distribution(success.Select(r => r.FirstMs!.Value))}\n" +
            $"精化完成：{Distribution(measured.Where(r => r.SettledMs is not null).Select(r => r.SettledMs!.Value))}\n" +
            $"模型应用：{Distribution(measured.Where(r => r.ModelMs is not null).Select(r => r.ModelMs!.Value))}\n" +
            $"记录覆盖：{string.Join(" / ", _snapshot.Components)} · 丢弃事件 {_snapshot.Dropped}\n" +
            (_snapshot.Components.Contains("tsf") && _snapshot.Components.Contains("core") ? "" :
             "缺少 TSF 或 Core 记录时无法给出完整链路结论。请确认内核和测试应用均已使用新版，并输入后等待 1 秒。\n") + _snapshot.Warning;
        _statistics.Children.Clear();
        foreach (var (key, title) in StageNames) {
            var values = rows.Where(r => r.Stages.ContainsKey(key)).Select(r => r.Stages[key]);
            _statistics.Children.Add(Note(title + "：" + Distribution(values)));
        }
        var selectedId = (_requests.SelectedItem as RowItem)?.Row.RequestId;
        var items = rows.Take(100).Select(r => new RowItem(r)).ToArray();
        _requests.ItemsSource = items;
        _requests.SelectedItem = items.FirstOrDefault(r => r.Row.RequestId == selectedId) ?? items.FirstOrDefault();
        RenderDetails(); UpdateState();
    }
    private void RenderDetails() {
        if (_requests.SelectedItem is not RowItem selected) { _details.Text = ""; return; }
        var row = selected.Row;
        var lines = StageNames.Select(s => s.Title.Trim() + "：" + Ms(row.Stages.TryGetValue(s.Key, out var value) ? value : null));
        var events = _snapshot.Events.Where(e => e.RequestId == row.RequestId).OrderBy(e => e.TimestampUs)
            .Select(e => $"+{(e.TimestampUs - row.StartUs) / 1000.0:0.000} ms　{e.Component}/{e.Event}　PID {e.Pid}" +
                string.Join("", e.Values.Where(v => v.Key is "candidate_count" or "cache_hit" or "refinement_pending" or "model_update" or "auxiliary" or "ok")
                    .Select(v => $"　{v.Key}={v.Value}")));
        var config = _snapshot.Events.FirstOrDefault(e => e.RequestId == row.RequestId && e.Component == "core" &&
            e.Event == "prepare" && e.Get("auxiliary") == 0 && e.Get("update") == 0);
        var configuration = config is null ? "采样配置：未知\n" :
            $"采样配置：页长 {config.Get("page_size")} · 分区 {config.Get("partition")} · 智能排序 {config.Get("model_enabled")} · 模型预算 {config.Get("model_timeout_ms")} ms\n" +
            $"本体词库 {config.Get("base_dictionary")} · 导入拼音词库 {config.Get("imported_dictionary")}（1=开，0=关）\n";
        _details.Text = configuration + string.Join("\n", lines) + "\n\n事件时间线\n" + string.Join("\n", events);
    }
    private async Task ExportAsync() {
        if (_busy) return;
        try {
            await RefreshAsync();
            var picker = new FileSavePicker { SuggestedFileName = "OwO-候选计时-" + DateTime.Now.ToString("yyyyMMdd-HHmmss") };
            picker.FileTypeChoices.Add("本地候选计时报告", [".json"]);
            WinRT.Interop.InitializeWithWindow.Initialize(picker,
                WinRT.Interop.WindowNative.GetWindowHandle(((App)Application.Current).MainWindow));
            var file = await picker.PickSaveFileAsync();
            if (file is null) return;
            var payload = _store.Export(_snapshot);
            await File.WriteAllTextAsync(file.Path, JsonSerializer.Serialize(payload, new JsonSerializerOptions { WriteIndented = true }));
            _notify("计时报告已导出，未上传任何数据。", InfoBarSeverity.Success);
        } catch (Exception error) { _notify("报告导出失败：" + error.Message, InfoBarSeverity.Error); }
    }
}
