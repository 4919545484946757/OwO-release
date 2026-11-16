using System.Buffers.Binary;
using System.IO.Pipes;
using System.Security.Principal;
using System.Text.Json;

namespace OwO_Settings;

// Internal Core pipe contract (include/owo/protocol/envelope.h and
// src/protocol/messages.cpp). Only manually entered ASCII goes through here.
internal static class ZeroCandidateDebug {
    private const int ProtocolVersion = 15;
    private static int _sequence;
    internal sealed record Result(string Initial, string Final, string[] Paths, bool QwenApplied);

    internal static async Task<Result> ProbeAsync(string input, bool performance, ModelManagerStore manager,
        TextAssistantSettings settings, CancellationToken cancellation) {
        if (input.Length > 96 || input.Count(char.IsAsciiLetter) < 2 ||
            !input.All(c => char.IsAsciiLetter(c) || c == '\''))
            throw new ArgumentException("请输入 2–96 个拼音字母，可包含分隔符 '。");
        var id = ((ulong)(uint)Environment.ProcessId << 32) | 0x80000000U | (uint)Interlocked.Increment(ref _sequence);
        JsonElement response;
        using (var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellation)) {
            deadline.CancelAfter(TimeSpan.FromSeconds(2));
            response = await ExchangeAsync(id, input, performance, deadline.Token);
        }
        var initial = response.GetProperty("text").GetString() ?? "";
        var paths = Paths(response);
        if (paths.Length <= 1) return new(initial, initial, paths, false);
        try {
            var winner = await ZeroModelReranker.RankAsync(manager, settings,
                new(performance ? "performance" : "eco", input, "", paths), cancellation);
            return new(initial, winner, paths, true);
        } catch (Exception error) when (!cancellation.IsCancellationRequested &&
            (error is OperationCanceledException or IOException or InvalidOperationException or HttpRequestException or JsonException or KeyNotFoundException)) {
            return new(initial, initial, paths, false);
        }
    }

    private static string[] Paths(JsonElement root) => root.GetProperty("candidates").EnumerateArray()
        .Select(item => item.GetString() ?? "").ToArray();

    private static async Task<JsonElement> ExchangeAsync(ulong id, string input, bool performance, CancellationToken cancellation) {
        await using var pipe = new NamedPipeClientStream(".", "OwO.InputMethod.Core.P1", PipeDirection.InOut,
            PipeOptions.Asynchronous, TokenImpersonationLevel.Identification);
        await pipe.ConnectAsync(cancellation);
        var payload = JsonSerializer.SerializeToUtf8Bytes(new {
            protocol_version = ProtocolVersion,
            type = "zero_candidate_request",
            request_id = id, context_generation = 1, text = input, candidates = Array.Empty<string>(),
            page = 0, has_more = false, model_pending = false, defer_expensive_candidates = false,
            candidate_refinement_pending = false, syllables = Array.Empty<string>(),
            candidate_consumed = Array.Empty<int>(), candidate_groups = Array.Empty<int>(), expanded = false, page_size = 0,
            correction_enabled = true, input = performance ? "performance" : "eco", context = "",
            application_context_id = "", application_process_id = 0, application_thread_id = 0, application_executable = "",
            application_window_class = "", application_focus_window_class = "", application_sensitive_input = false,
            symbol_composition = false, manual_breaks = Array.Empty<int>(), symbol_preview = "",
            following_previews = Array.Empty<string[]>(), following_groups = Array.Empty<int[]>(), input_scheme = "full_pinyin"
        }, new JsonSerializerOptions { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping });
        var prefix = new byte[4];
        BinaryPrimitives.WriteUInt32LittleEndian(prefix, (uint)payload.Length);
        await pipe.WriteAsync(prefix, cancellation);
        await pipe.WriteAsync(payload, cancellation);
        await pipe.ReadExactlyAsync(prefix, cancellation);
        var length = BinaryPrimitives.ReadUInt32LittleEndian(prefix);
        if (length is 0 or > 64 * 1024) throw new InvalidDataException("Core 响应超出长度限制。");
        var bytes = new byte[length];
        await pipe.ReadExactlyAsync(bytes, cancellation);
        using var json = JsonDocument.Parse(bytes);
        var root = json.RootElement;
        if (root.GetProperty("protocol_version").GetInt32() != ProtocolVersion ||
            root.GetProperty("type").GetString() != "zero_candidate_response" ||
            root.GetProperty("request_id").GetUInt64() != id || root.GetProperty("context_generation").GetUInt64() != 1)
            throw new InvalidDataException("Core 版本不同或响应不匹配，请启动新版输入法。");
        return root.Clone();
    }
}
