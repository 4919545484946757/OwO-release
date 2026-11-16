using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;

namespace OwO_Settings;

internal sealed record LocalChatMessage(string role, string content);
internal sealed record LocalChatTurn(string User, string Assistant);
internal sealed record LocalChatReply(string Text, bool Truncated);

internal static class LocalModelChat {
    internal const int MaxUserLength = 1200, MaxSystemLength = 800, MaxTurns = 6;
    private const int TotalBudget = 3000, MaxResponseBytes = 128 * 1024;

    internal static IReadOnlyList<LocalChatMessage> Messages(string system, IReadOnlyList<LocalChatTurn> history, string input) {
        if (string.IsNullOrWhiteSpace(input) || input.Length > MaxUserLength || input.Contains('\0') ||
            system.Length > MaxSystemLength || system.Contains('\0'))
            throw new ArgumentException("请填写调试消息，并确认消息与系统提示没有超过长度限制。");
        var messages = new List<LocalChatMessage>();
        if (!string.IsNullOrWhiteSpace(system)) messages.Add(new("system", system));
        var remaining = TotalBudget - system.Length - input.Length;
        int first = history.Count;
        while (first > Math.Max(0, history.Count - MaxTurns)) {
            var turn = history[first - 1];
            remaining -= turn.User.Length + turn.Assistant.Length;
            if (remaining < 0) break;
            --first;
        }
        for (int i = first; i < history.Count; ++i) {
            messages.Add(new("user", history[i].User));
            messages.Add(new("assistant", history[i].Assistant));
        }
        messages.Add(new("user", input));
        return messages;
    }

    internal static async Task<LocalChatReply> SendAsync(LocalModelStore store, LocalModel model, string system,
        IReadOnlyList<LocalChatTurn> history, string input, CancellationToken cancellation, HttpMessageHandler? testHandler = null) {
        var messages = Messages(system, history, input);
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellation);
        timeout.CancelAfter(TimeSpan.FromSeconds(75));
        timeout.Token.ThrowIfCancellationRequested();
        using var session = testHandler is null
            ? await LocalModelInference.StartAsync(store, model, true, timeout.Token, contextTokens: 4096).ConfigureAwait(false) : null;
        using var fixture = testHandler is null ? null : new HttpClient(testHandler) { Timeout = Timeout.InfiniteTimeSpan };
        var client = session?.Client ?? fixture!;
        using var request = new HttpRequestMessage(HttpMethod.Post, session?.Endpoint ?? new Uri("http://127.0.0.1/v1/chat/completions"));
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", session?.SessionKey ?? "fixture-only");
        request.Content = new StringContent(JsonSerializer.Serialize(new {
            model = model.Id, messages, stream = false, temperature = 0.3, max_tokens = 256,
            chat_template_kwargs = new { enable_thinking = false }
        }), Encoding.UTF8, "application/json");
        using var response = await client.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, timeout.Token).ConfigureAwait(false);
        if (!response.IsSuccessStatusCode) throw new InvalidOperationException($"本地模型返回 HTTP {(int)response.StatusCode}。");
        if (response.Content.Headers.ContentLength > MaxResponseBytes) throw new InvalidDataException("本地模型回复过大。");
        await using var inputStream = await response.Content.ReadAsStreamAsync(timeout.Token).ConfigureAwait(false);
        using var output = new MemoryStream();
        var chunk = new byte[4096];
        for (;;) {
            var n = await inputStream.ReadAsync(chunk, timeout.Token).ConfigureAwait(false);
            if (n == 0) break;
            if (output.Length + n > MaxResponseBytes) throw new InvalidDataException("本地模型回复过大。");
            output.Write(chunk, 0, n);
        }
        using var json = JsonDocument.Parse(output.ToArray());
        var options = json.RootElement.GetProperty("choices");
        if (options.ValueKind != JsonValueKind.Array || options.GetArrayLength() == 0)
            throw new InvalidDataException("本地模型未返回可用回复。");
        var choice = options[0];
        string? finish = choice.TryGetProperty("finish_reason", out var value) ? value.GetString() : null;
        if (finish is not ("stop" or "length" or null))
            throw new InvalidDataException("本地模型返回了非文本操作，已忽略。");
        var message = choice.GetProperty("message");
        if (message.TryGetProperty("tool_calls", out var tools) && tools.ValueKind != JsonValueKind.Null &&
            (tools.ValueKind != JsonValueKind.Array || tools.GetArrayLength() != 0))
            throw new InvalidDataException("本地模型请求调用工具，已忽略。");
        var text = message.GetProperty("content").GetString()?.Trim();
        if (string.IsNullOrEmpty(text) || text.Length > 6000 || text.Contains('\0'))
            throw new InvalidDataException("本地模型未返回可用的文本。");
        return new(text, finish == "length");
    }
}
