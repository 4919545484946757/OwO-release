using OwO_Settings;

await InstallerPayloadTests.RunAsync();
if (args.Length == 1) await InstallerPayloadTests.VerifyReleaseAsync(args[0]);

foreach (var architecture in Enum.GetValues<System.Runtime.InteropServices.Architecture>()) {
    var supported = SetupPrerequisites.ArchitectureError(architecture).Length == 0;
    if (supported != (architecture == System.Runtime.InteropServices.Architecture.X64))
        throw new Exception("Installer must accept only x64 OS architecture");
}

if (!SetupPrerequisites.LanguageReady(0, "OWO_LANGUAGE_READY\r\n") ||
    SetupPrerequisites.LanguageReady(0, "") ||
    SetupPrerequisites.LanguageReady(2, "OWO_LANGUAGE_READY") ||
    SetupPrerequisites.LanguageReady(1, "OWO_LANGUAGE_MISSING"))
    throw new Exception("Language probe must require both success marker and exit code");
if (SetupPrerequisites.BlockingReason(false, false, true, true, true, true, "") != "")
    throw new Exception("Trusted ready installer must enable install");
foreach (var reason in new[] {
    SetupPrerequisites.BlockingReason(true, false, true, true, true, true, ""),
    SetupPrerequisites.BlockingReason(false, true, true, true, true, true, ""),
    SetupPrerequisites.BlockingReason(false, false, false, true, true, true, ""),
    SetupPrerequisites.BlockingReason(false, false, true, false, true, true, ""),
    SetupPrerequisites.BlockingReason(false, false, true, true, false, true, ""),
    SetupPrerequisites.BlockingReason(false, false, true, true, true, false, "bad path")
}) {
    if (string.IsNullOrWhiteSpace(reason)) throw new Exception("Blocked install needs an explanation");
}
if (OperatingSystem.IsWindows()) {
    var start = new System.Diagnostics.ProcessStartInfo(Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.System),
        "WindowsPowerShell", "v1.0", "powershell.exe")) {
        UseShellExecute = false, CreateNoWindow = true,
        RedirectStandardOutput = true, RedirectStandardError = true
    };
    foreach (var argument in new[] { "-NoProfile", "-NonInteractive", "-Command", SetupPrerequisites.LanguageProbe })
        start.ArgumentList.Add(argument);
    string? previous = null;
    for (var attempt = 0; attempt < 5; attempt++) {
        using var process = System.Diagnostics.Process.Start(start)!;
        var output = process.StandardOutput.ReadToEndAsync();
        var error = process.StandardError.ReadToEndAsync();
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(20));
        await process.WaitForExitAsync(timeout.Token);
        var marker = (await output).Trim();
        if (!SetupPrerequisites.LanguageReady(process.ExitCode, marker) &&
            !(process.ExitCode == 1 && marker == "OWO_LANGUAGE_MISSING"))
            throw new Exception("Local language check failed: " + await error);
        if (previous is not null && previous != marker)
            throw new Exception("Repeated language checks returned inconsistent results");
        previous = marker;
    }
    Console.WriteLine("Local language result: " + previous);
}
Console.WriteLine("Installer readiness and repeated local language checks passed.");

var root = Path.Combine(Path.GetTempPath(), "owo-ui-preferences-" + Guid.NewGuid().ToString("N"));
var path = Path.Combine(root, "config", "settings-ui.json");
try {
    if (!SettingsUiPreferences.Load(path).AutoSaveEnabled) throw new Exception("Default must enable auto-save");
    new SettingsUiPreferences(false).Save(path);
    if (SettingsUiPreferences.Load(path).AutoSaveEnabled) throw new Exception("Disabled state was not persisted");
    new SettingsUiPreferences(true).Save(path);
    if (!SettingsUiPreferences.Load(path).AutoSaveEnabled) throw new Exception("Enabled state was not persisted");
    if (Directory.EnumerateFiles(Path.GetDirectoryName(path)!, "*.tmp").Any()) throw new Exception("Temporary file leaked");
    File.WriteAllText(path, "broken");
    var rejected = false;
    try { SettingsUiPreferences.Load(path); } catch (System.Text.Json.JsonException) { rejected = true; }
    if (!rejected || File.ReadAllText(path) != "broken") throw new Exception("Malformed preference was silently overwritten");
    Console.WriteLine("Settings UI preference tests passed.");
} finally {
    if (Directory.Exists(root)) Directory.Delete(root, recursive: true);
}
