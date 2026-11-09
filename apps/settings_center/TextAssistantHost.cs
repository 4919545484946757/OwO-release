using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;
using Microsoft.Win32.SafeHandles;

namespace OwO_Settings;

// One-shot, current-user IPC worker. No network or credentials in TSF/Core.
internal static class TextAssistantHost {
    private static readonly UTF8Encoding Utf8 = new(false, true);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool GetNamedPipeClientProcessId(SafePipeHandle pipe, out uint process);
    internal static string Encode(string value) => Convert.ToHexString(Utf8.GetBytes(value));
    internal static string Decode(string value) => Utf8.GetString(Convert.FromHexString(value));
    internal static TextAssistantInput Parse(string wire) {
        var lines = wire.Split('\n');
        if (lines.Length != 6 || lines[0] != "OWOTEXT1" || lines[5] != "" ||
            lines[1] is not ("translate" or "polish" or "code" or "generate"))
            throw new InvalidOperationException("文字助手请求格式错误。");
        return new(lines[1], Decode(lines[2]), Decode(lines[3]), Decode(lines[4]));
    }
    internal static void CheckSensitive(TextAssistantInput input) {
        if (Regex.IsMatch(input.Selection + "\n" + input.Context,
            @"(?i)-----BEGIN [A-Z ]*PRIVATE KEY-----|\bsk-[a-z0-9_-]{20,}|\b(?:api[_-]?key|password|secret|access[_-]?token)\s*[:=]\s*[""']?[^\s""';,]{8,}",
            RegexOptions.CultureInvariant, TimeSpan.FromMilliseconds(50)))
            throw new InvalidOperationException("选区或附近文本疑似包含密钥、口令等敏感信息，请移除后重试。原文未修改。");
    }
    internal static async Task RunAsync(string[] args) {
        if (args.Length != 5 || args[1] != "--text-assistant-host" || args[3] != "--parent-pid" ||
            !uint.TryParse(args[4], out var parent) || parent == 0 ||
            !args[2].StartsWith("OwO.TextAssistant.", StringComparison.Ordinal) ||
            !Guid.TryParseExact(args[2]["OwO.TextAssistant.".Length..], "N", out _)) return;
        using var lifetime = new CancellationTokenSource(TimeSpan.FromSeconds(130));
        await using var pipe = new NamedPipeServerStream(args[2], PipeDirection.InOut, 1,
            PipeTransmissionMode.Byte, PipeOptions.Asynchronous | PipeOptions.CurrentUserOnly);
        await pipe.WaitForConnectionAsync(lifetime.Token);
        if (!GetNamedPipeClientProcessId(pipe.SafePipeHandle, out var peer) || peer != parent) return;
        string response;
        try {
            var prefix = new byte[4];
            await pipe.ReadExactlyAsync(prefix, lifetime.Token);
            var count = System.Buffers.Binary.BinaryPrimitives.ReadUInt32LittleEndian(prefix);
            if (count is 0 or > 128 * 1024) return;
            var data = new byte[count];
            await pipe.ReadExactlyAsync(data, lifetime.Token);
            var request = Parse(Utf8.GetString(data));
            var settings = new ModelManagerStore(ModelManagerStore.DefaultRoot).Load();
            if (!settings.IncludeContext) request = request with { Context = "" };
            CheckSensitive(request);
            var text = await TextAssistantApi.ExecuteAsync(settings, request, lifetime.Token);
            response = "OWOTEXT1\nOK\n" + Encode(text) + "\n";
        } catch (OperationCanceledException) { response = Error("请求已取消或超时，原文未修改。"); }
          catch (HttpRequestException) { response = Error("无法连接模型服务，请检查网络及 API 地址。原文未修改。"); }
          catch (InvalidOperationException error) { response = Error(error.Message); }
          catch { response = Error("模型配置、密钥或返回格式无效，原文未修改。"); }
        var bytes = Utf8.GetBytes(response);
        var header = new byte[4];
        System.Buffers.Binary.BinaryPrimitives.WriteUInt32LittleEndian(header, (uint)bytes.Length);
        await pipe.WriteAsync(header, lifetime.Token);
        await pipe.WriteAsync(bytes, lifetime.Token);
        await pipe.FlushAsync(lifetime.Token);
    }
    private static string Error(string text) => "OWOTEXT1\nERROR\n" + Encode(text) + "\n";
}
