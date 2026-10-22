using System.Diagnostics;
using System.Text;

namespace OwO_Settings;

// Access to the authorized corpus and to the tone feedback records,
// both through owo_corpus_shell.exe. The shell owns the file formats,
// decryption and locking, so the settings center never parses corpus or
// feedback bytes itself. Mutations are explicit delete/clear operations.
internal sealed class CorpusShellClient
{
    private readonly string _shellPath = Environment.GetEnvironmentVariable("OWO_CORPUS_SHELL_PATH")
        ?? Path.Combine(AppContext.BaseDirectory, "owo_corpus_shell.exe");
    private readonly string _corpusPath = Environment.GetEnvironmentVariable("OWO_CORPUS_PATH")
        ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                        "OwO", "InputMethod", "data", "user-corpus.owuc");
    // Feedback lives next to the corpus in the same data directory.
    private readonly string _feedbackPath = Environment.GetEnvironmentVariable("OWO_CORPUS_PATH") is { } corpus
        ? Path.Combine(Path.GetDirectoryName(Path.GetFullPath(corpus))!, "tone-feedback.owtf")
        : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                       "OwO", "InputMethod", "data", "tone-feedback.owtf");
    private readonly string _configPath = new ConfigShellClient().ConfigPath;

    private string[] CommonArguments() =>
        ["--corpus", _corpusPath, "--config", _configPath, "--feedback", _feedbackPath];

    internal Task<string> DescribeStyleAsync(CancellationToken cancellationToken = default) =>
        RunAsync(["--corpus", _corpusPath, "--config", _configPath, "style"],
                 cancellationToken);

    // Builds the local draft plan for `message` and returns the exact request
    // text the caller may send. This never contacts a model: the settings
    // center has no network client, and the shell only reads local files.
    internal Task<string> DescribeTonePromptAsync(string message, bool continuation,
                                                  CancellationToken cancellationToken = default)
    {
        List<string> arguments = [.. CommonArguments(), "tone"];
        if (continuation) arguments.Add("--continue");
        arguments.Add(message);
        return RunAsync(arguments, cancellationToken);
    }

    // Counts of every feedback kind plus one line per scene. Draft texts are
    // not included, so the overview can be shown without re-reading them.
    internal Task<string> DescribeToneFeedbackAsync(CancellationToken cancellationToken = default) =>
        RunAsync([.. CommonArguments(), "feedback", "summary"], cancellationToken);

    // Records are shown one per line without their text: the user decides
    // whether to keep them, not to re-read every draft.
    internal Task<string> ListToneFeedbackAsync(CancellationToken cancellationToken = default) =>
        RunAsync([.. CommonArguments(), "feedback", "metadata"], cancellationToken);

    internal Task<string> DeleteToneFeedbackAsync(ulong id,
                                                  CancellationToken cancellationToken = default) =>
        RunAsync([.. CommonArguments(), "feedback", "delete", id.ToString()],
                 cancellationToken);

    internal Task<string> ClearToneFeedbackAsync(CancellationToken cancellationToken = default) =>
        RunAsync([.. CommonArguments(), "feedback", "clear"], cancellationToken);

    private async Task<string> RunAsync(IEnumerable<string> arguments,
                                        CancellationToken cancellationToken)
    {
        if (!File.Exists(_shellPath))
            throw new FileNotFoundException("找不到语料查看工具。", _shellPath);
        var start = new ProcessStartInfo(_shellPath) {
            UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8,
        };
        foreach (var argument in arguments) start.ArgumentList.Add(argument);
        using var process = Process.Start(start)
            ?? throw new InvalidOperationException("无法启动语料查看工具。");
        var output = process.StandardOutput.ReadToEndAsync(cancellationToken);
        var error = process.StandardError.ReadToEndAsync(cancellationToken);
        await process.WaitForExitAsync(cancellationToken);
        if (process.ExitCode != 0) {
            var message = (await error).Trim();
            throw new InvalidOperationException(message.Length > 0 ? message
                : $"语料查看工具退出码：{process.ExitCode}");
        }
        return await output;
    }
}
