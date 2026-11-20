using System.Diagnostics;
using System.Globalization;
using System.Text;

namespace OwO_Settings;

internal sealed record InstallerProcessLine(string Text, bool IsError);
internal sealed record InstallerProcessResult(int ExitCode, string Output, string Error);

internal static class InstallerProcessRunner
{
    internal static bool TryProgress(string line, out int percent, out string stage)
    {
        percent = 0; stage = "";
        var parts = line.Split('|');
        if (parts.Length != 3 || parts[0] != "OWO_SETUP_PROGRESS" ||
            !int.TryParse(parts[1], NumberStyles.None, CultureInfo.InvariantCulture, out percent) ||
            percent is < 0 or > 99) return false;
        stage = parts[2] switch {
            "prepare" => "正在检查安装环境和目标目录…",
            "copy-bin" => "正在复制输入法组件…",
            "copy-data" => "正在复制词典…",
            "copy-model" => "正在复制智能排序模型…",
            "copy-settings" => "正在复制设置中心和运行时…",
            "copy-scripts" => "正在复制安装和卸载脚本…",
            "copy-LICENSES" => "正在复制许可文件…",
            "verify" => "正在校验词典和准备模型…",
            "retire" => "正在切换已有服务和输入法注册…",
            "register" => "正在注册输入法组件…",
            "preferences" => "正在配置语言、启动项和快捷方式…",
            "start" => "正在启动候选服务…",
            "complete" => "安装后端已完成，正在确认结果…",
            _ => ""
        };
        return stage.Length != 0;
    }

    internal static async Task<InstallerProcessResult> RunAsync(string executable,
        IEnumerable<string> arguments, TimeSpan? timeout = null,
        IProgress<InstallerProcessLine>? lines = null, TimeSpan? drainTimeout = null)
    {
        var start = new ProcessStartInfo(executable) {
            UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8
        };
        foreach (var argument in arguments) start.ArgumentList.Add(argument);
        using var process = Process.Start(start) ?? throw new InvalidOperationException($"无法启动 {executable}。");
        using var reads = new CancellationTokenSource();
        var output = new StringBuilder();
        var error = new StringBuilder();
        var gate = new object();
        async Task PumpAsync(StreamReader reader, StringBuilder capture, bool isError)
        {
            try {
                while (await reader.ReadLineAsync(reads.Token).ConfigureAwait(false) is { } line) {
                    lock (gate) {
                        if (capture.Length < 2 * 1024 * 1024) capture.AppendLine(line);
                    }
                    lines?.Report(new(line, isError));
                }
            } catch (OperationCanceledException) when (reads.IsCancellationRequested) { }
              catch (ObjectDisposedException) when (reads.IsCancellationRequested) { }
              catch (IOException) when (reads.IsCancellationRequested) { }
        }
        var pumps = Task.WhenAll(PumpAsync(process.StandardOutput, output, false), PumpAsync(process.StandardError, error, true));
        using var cancellation = new CancellationTokenSource();
        if (timeout is { } duration) cancellation.CancelAfter(duration);
        try {
            await process.WaitForExitAsync(cancellation.Token).ConfigureAwait(false);
        } catch (OperationCanceledException) {
            try { process.Kill(entireProcessTree: true); } catch (InvalidOperationException) { }
            reads.Cancel();
            throw new TimeoutException("语言检测超时");
        }
        try { await pumps.WaitAsync(drainTimeout ?? TimeSpan.FromSeconds(2)).ConfigureAwait(false); }
        catch (TimeoutException) {
            // A detached descendant must not keep an exited installer alive.
            reads.Cancel();
            process.StandardOutput.Dispose();
            process.StandardError.Dispose();
            lock (gate) error.AppendLine("Output drain timed out after backend exit; detached child may still own a pipe.");
        }
        lock (gate) return new(process.ExitCode, output.ToString(), error.ToString());
    }
}
