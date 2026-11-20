using System.Collections.Concurrent;
using System.Diagnostics;
using OwO_Settings;

internal static class InstallerProcessRunnerTests
{
    private sealed class Lines(Action<InstallerProcessLine> receive) : IProgress<InstallerProcessLine>
    {
        public void Report(InstallerProcessLine value) => receive(value);
    }
    internal static async Task RunAsync(string fixture)
    {
        foreach (var invalid in new[] { "text", "OWO_SETUP_PROGRESS|100|complete", "OWO_SETUP_PROGRESS|-1|prepare",
                    "OWO_SETUP_PROGRESS|3|unknown", "OWO_SETUP_PROGRESS|3|prepare|injected" })
            if (InstallerProcessRunner.TryProgress(invalid, out _, out _)) throw new Exception("Accepted invalid progress: " + invalid);
        if (!InstallerProcessRunner.TryProgress("OWO_SETUP_PROGRESS|45|copy-settings", out var percent, out var stage) || percent != 45 || stage.Length == 0)
            throw new Exception("Rejected valid stage progress");
        var lines = new ConcurrentQueue<InstallerProcessLine>();
        var early = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var run = InstallerProcessRunner.RunAsync(fixture, ["--emit-progress"], lines: new Lines(line => {
            lines.Enqueue(line);
            if (line.Text.Contains("|3|prepare")) early.TrySetResult();
        }));
        await early.Task.WaitAsync(TimeSpan.FromSeconds(3));
        if (run.IsCompleted) throw new Exception("Output was buffered until backend exit");
        var result = await run;
        if (result.ExitCode != 0 || !result.Output.Contains("|99|complete") || !result.Error.Contains("diagnostic") || lines.Count < 3)
            throw new Exception("Lost streamed output or stderr");
        result = await InstallerProcessRunner.RunAsync(fixture, ["--exit-error"]);
        if (result.ExitCode != 7 || !result.Error.Contains("backend failure")) throw new Exception("Backend failure was hidden");
        var watch = Stopwatch.StartNew();
        result = await InstallerProcessRunner.RunAsync(fixture, ["--hold-output-parent"], drainTimeout: TimeSpan.FromMilliseconds(500));
        if (watch.Elapsed > TimeSpan.FromSeconds(3) || result.ExitCode != 0 || !result.Output.Contains("backend completed") || !result.Error.Contains("drain timed out"))
            throw new Exception("Exited backend was blocked by descendant holding output pipe");
        Console.WriteLine("Installer streaming, progress parsing, failure capture and inherited-pipe drain tests passed.");
    }
}
