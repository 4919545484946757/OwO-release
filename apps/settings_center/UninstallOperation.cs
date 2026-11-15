using System.ComponentModel;
using System.Diagnostics;
using System.Security.Principal;
using System.Text.Json;
using System.Text.RegularExpressions;
using Microsoft.Win32;

namespace OwO_Settings;

internal sealed record UninstallProgress(int Percent, string Stage, string Detail = "")
{
    internal string Label => Stage switch {
        "prepare" => "检查安装位置",
        "stop" => "关闭已安装的 OwO 进程",
        "unregister" => "注销 Windows 输入法组件",
        "shortcuts" => "清理快捷方式和开机启动项",
        "keep-data" => "保留用户数据",
        "remove-data" => "删除用户数据",
        "data-removed" => "用户数据已移除",
        "program-files" => "清理程序文件",
        "complete" => "卸载完成",
        "failed" => "卸载未完成",
        _ => ""
    };

    internal static bool TryParse(string line, out UninstallProgress? progress)
    {
        progress = null;
        if (line.Length is 0 or > 16000) return false;
        try {
            using var json = JsonDocument.Parse(line);
            var root = json.RootElement;
            var candidate = new UninstallProgress(root.GetProperty("percent").GetInt32(),
                root.GetProperty("stage").GetString() ?? "", root.GetProperty("detail").GetString() ?? "");
            if (candidate.Label.Length == 0 || candidate.Percent is < 0 or > 100 ||
                (candidate.Percent == 100) != (candidate.Stage == "complete")) return false;
            progress = candidate;
            return true;
        } catch (Exception error) when (error is JsonException or KeyNotFoundException or InvalidOperationException or FormatException) {
            return false;
        }
    }
}

internal static partial class UninstallOperation
{
    internal static string UserDataDirectory => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OwO", "InputMethod");

    [GeneratedRegex("^OwO-UninstallUi-[a-f0-9]{32}$", RegexOptions.CultureInvariant)]
    private static partial Regex WorkspaceName();

    internal static async Task RunAsync(string installRoot, string workspace, bool removeUserData, Action<UninstallProgress> report)
    {
        using var key = Registry.CurrentUser.OpenSubKey(
            @"Software\Microsoft\Windows\CurrentVersion\Uninstall\OwO Input Method");
        var current = key?.GetValue("InstallLocation") as string;
        if (string.IsNullOrWhiteSpace(current))
            current = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                "Programs", "OwO", "InputMethod", typeof(UninstallOperation).Assembly.GetName().Version!.ToString(3));
        static string Full(string value) => Path.TrimEndingDirectorySeparator(Path.GetFullPath(value));
        if (!Full(installRoot).Equals(Full(current), StringComparison.OrdinalIgnoreCase) ||
            !Full(Path.GetDirectoryName(workspace) ?? "").Equals(Full(Path.GetTempPath()), StringComparison.OrdinalIgnoreCase) ||
            !WorkspaceName().IsMatch(Path.GetFileName(workspace)) ||
            !Full(AppContext.BaseDirectory).Equals(Full(Path.Combine(workspace, "settings")), StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("卸载位置或临时工作目录无效，请从已安装应用或开始菜单重新打开。");
        var backend = Path.Combine(current, "scripts", "Uninstall-OwO.ps1");
        if (!File.Exists(backend) || !File.Exists(Path.Combine(current, "scripts", "Uninstall-Common.ps1")))
            throw new FileNotFoundException("找不到已安装版本的卸载后端。");

        using var identity = WindowsIdentity.GetCurrent();
        var user = identity.User?.Value ?? throw new InvalidOperationException("无法识别当前 Windows 用户。");
        var elevated = new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator);
        var output = Path.Combine(workspace, $"progress-{Guid.NewGuid():N}.jsonl");
        var powershell = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System),
            "WindowsPowerShell", "v1.0", "powershell.exe");
        var options = new ProcessStartInfo(powershell) {
            UseShellExecute = true,
            WorkingDirectory = workspace,
            WindowStyle = ProcessWindowStyle.Hidden,
            Verb = elevated ? "" : "runas"
        };
        foreach (var argument in new[] { "-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
            "-File", backend, "-Worker", "-InstallRoot", current, "-ExpectedUserSid", user, "-ProgressPath", output })
            options.ArgumentList.Add(argument);
        if (removeUserData) options.ArgumentList.Add("-RemoveUserData");

        Process process;
        try {
            process = await Task.Run(() => Process.Start(options)) ??
                throw new InvalidOperationException("无法启动卸载后端。");
        } catch (Win32Exception error) when (error.NativeErrorCode == 1223) {
            throw new OperationCanceledException("已取消管理员授权，未开始卸载。", error);
        }
        using (process) {
            var seen = 0;
            UninstallProgress? complete = null;
            string? failure = null;
            void ReadProgress()
            {
                string[] lines;
                try {
                    if (!File.Exists(output) || new FileInfo(output).Length > 64 * 1024) return;
                    lines = File.ReadAllLines(output);
                } catch (IOException) { return; } // The producer may be appending this line.
                for (; seen < lines.Length; ++seen) {
                    if (!UninstallProgress.TryParse(lines[seen], out var entry)) {
                        if (seen == lines.Length - 1) break;
                        continue;
                    }
                    if (entry!.Stage == "complete") complete = entry;
                    else if (entry.Stage == "failed") failure = entry.Detail;
                    else report(entry);
                }
            }
            var exit = process.WaitForExitAsync();
            while (!exit.IsCompleted) {
                await Task.WhenAny(exit, Task.Delay(140));
                ReadProgress();
            }
            await exit;
            ReadProgress();
            if (process.ExitCode != 0 || complete is null ||
                (Directory.Exists(current) && (complete.Detail != "empty-directory-retained" ||
                 Directory.EnumerateFileSystemEntries(current).Any())))
                throw new InvalidOperationException(!string.IsNullOrWhiteSpace(failure) ? failure :
                    $"卸载后端未确认完成（退出码 {process.ExitCode}）。请使用当前用户批准授权，并关闭可能占用 OwO 文件的应用。");
            report(complete);
        }
    }
}
