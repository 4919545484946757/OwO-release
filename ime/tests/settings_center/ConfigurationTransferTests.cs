using System.Text;
using System.Text.Json.Nodes;
using OwO_Settings;

if (args.Length != 1 || !File.Exists(args[0]))
    throw new ArgumentException("Pass the path of owo_config_shell.exe; tests use only isolated files.");
var root = Path.Combine(Path.GetTempPath(), "OwO-transfer-tests-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(root);
var checks = 0;
void Check(bool value, string message) {
    if (!value) throw new Exception(message);
    ++checks;
}
async Task Reject(Func<Task> action, string name) {
    try { await action(); }
    catch (Exception error) when (error is System.Text.Json.JsonException or InvalidDataException or
                                   DecoderFallbackException or InvalidOperationException or IOException) {
        ++checks;
        return;
    }
    throw new Exception("Expected rejection: " + name);
}
try {
    var client = new ConfigShellClient(Path.Combine(root, "config", "owo.conf"), Path.GetFullPath(args[0]));
    var defaults = await client.LoadDefaultsAsync();
    await client.SaveAsync(defaults);
    var prefs = Path.Combine(root, "config", "settings-ui.json");
    new SettingsUiPreferences(true).Save(prefs);
    var original = File.ReadAllBytes(client.ConfigPath);
    var originalPrefs = File.ReadAllBytes(prefs);
    var changed = defaults with { CandidatePageSize = defaults.CandidatePageSize == 9 ? 8u : 9u,
        CandidatePartitionEnabled = !defaults.CandidatePartitionEnabled,
        UserLearningSensitivity = 3, UserCompletionMaxSyllables = 2,
        LanguageShortcuts = ["Ctrl+Space", "Shift+A"],
        CandidateSingleCharacterFirstEnabled = false,
        CorpusLearningEnabled = true, CorpusLearningPaused = true,
        CorpusAllowedApps = ["Code.exe", "WeChat.exe"], CorpusExcludedApps = ["KeePass.exe"],
        CorpusRetentionDays = 37, CorpusPredictionEnabled = false,
        StyleSceneMask = 2, StylePreferences = "short~~precise",
        ToneReplyEnabled = true, ToneContinuationEnabled = true, ToneCloudModelEnabled = false,
        ExpandedPreviousPageShortcuts = ["Ctrl+F7"], ExpandedNextPageShortcuts = ["Ctrl+F8"] };
    var exported = Path.Combine(root, "设置.json");
    await SettingsTransfer.WriteAsync(exported, SettingsTransfer.Create(changed, false));
    var roundtrip = await SettingsTransfer.ReadAsync(exported);
    var encoded = File.ReadAllText(exported);
    Check(roundtrip.Settings.CandidatePageSize == changed.CandidatePageSize &&
        !roundtrip.UiPreferences.AutoSaveEnabled &&
        roundtrip.Settings.LanguageShortcuts.SequenceEqual(changed.LanguageShortcuts), "JSON roundtrip");
    Check(!encoded.Contains(root) && !encoded.Contains("user-frequency"), "No runtime paths or learning data");
    await client.ValidateAsync(roundtrip.Settings);
    Check(original.SequenceEqual(File.ReadAllBytes(client.ConfigPath)), "Validation must not mutate target");

    var invalid = Path.Combine(root, "invalid.json");
    async Task InvalidJson(string text, string name) {
        await File.WriteAllTextAsync(invalid, text);
        await Reject(() => SettingsTransfer.ReadAsync(invalid), name);
    }
    await InvalidJson("{", "malformed");
    await InvalidJson(encoded.Replace("\"formatVersion\": 3", "\"formatVersion\": 4"), "future version");
    await InvalidJson(encoded.Replace("\"formatVersion\": 3", "\"formatVersion\": 3, \"formatVersion\": 3"), "duplicate");
    await InvalidJson(encoded.Replace("\"candidatePageSize\": 9", "\"candidatePageSize\": 9, \"candidatePageSize\": 7"), "nested duplicate");
    await InvalidJson(encoded.Replace("\"autoSaveEnabled\": false", "\"unknown\": false"), "unknown/missing preference");
    var node = JsonNode.Parse(encoded)!.AsObject();
    node["settings"]!.AsObject().Remove("candidateWrapLength");
    await InvalidJson(node.ToJsonString(), "missing setting");
    node = JsonNode.Parse(encoded)!.AsObject(); node["settings"] = null;
    await InvalidJson(node.ToJsonString(), "null settings");
    node = JsonNode.Parse(encoded)!.AsObject(); node["settings"]!["languageShortcuts"] = null;
    await InvalidJson(node.ToJsonString(), "null shortcut list");
    node = JsonNode.Parse(encoded)!.AsObject(); node["settings"]!["languageShortcuts"] = new JsonArray((JsonNode?)null);
    await InvalidJson(node.ToJsonString(), "null shortcut element");
    node = JsonNode.Parse(encoded)!.AsObject(); node["settings"]!["languageShortcuts"] = new JsonArray("Ctrl+Space;Shift+A");
    await InvalidJson(node.ToJsonString(), "shortcut separator injection");
    await File.WriteAllBytesAsync(invalid, [0xff, 0xfe, 0x00]);
    await Reject(() => SettingsTransfer.ReadAsync(invalid), "invalid UTF8");
    await File.WriteAllTextAsync(invalid, new string(' ', SettingsTransfer.MaximumBytes + 1));
    await Reject(() => SettingsTransfer.ReadAsync(invalid), "oversized");
    await Reject(() => client.ValidateAsync(changed with { CandidatePageSize = 0 }), "native range validation");
    await Reject(() => client.ValidateAsync(changed with { LanguageShortcuts = ["not a key"] }), "native key validation");
    await Reject(() => client.ValidateAsync(changed with { LanguageShortcuts = defaults.RawInputShortcuts }), "key conflict");
    Check(original.SequenceEqual(File.ReadAllBytes(client.ConfigPath)), "Invalid imports leave config unchanged");
    var versionTwo = JsonNode.Parse(encoded)!.AsObject();
    versionTwo["formatVersion"] = 2;
    foreach (var field in new[] { "corpusLearningEnabled", "corpusLearningPaused", "corpusAllowedApps",
        "corpusExcludedApps", "corpusRetentionDays", "corpusPredictionEnabled", "styleSceneMask",
        "stylePreferences", "toneReplyEnabled", "toneContinuationEnabled", "toneCloudModelEnabled" })
        versionTwo["settings"]!.AsObject().Remove(field);
    await File.WriteAllTextAsync(invalid, versionTwo.ToJsonString());
    var migratedTwo = await SettingsTransfer.ReadAsync(invalid);
    Check(migratedTwo.FormatVersion == 3 && !migratedTwo.Settings.CorpusLearningEnabled &&
        !migratedTwo.Settings.ToneCloudModelEnabled &&
        !migratedTwo.Settings.CandidateSingleCharacterFirstEnabled &&
        migratedTwo.Settings.ExpandedPreviousPageShortcuts.SequenceEqual(new[] { "Ctrl+F7" }),
        "Version 2 retains release settings and defaults new opt-ins off");
    await client.ValidateAsync(migratedTwo.Settings);
    versionTwo["settings"]!["corpusLearningEnabled"] = true;
    await InvalidJson(versionTwo.ToJsonString(), "Legacy format cannot enable new corpus fields");
    var legacy = JsonNode.Parse(encoded)!.AsObject();
    legacy["formatVersion"] = 1;
    foreach (var field in new[] { "corpusLearningEnabled","corpusLearningPaused","corpusAllowedApps","corpusExcludedApps","corpusRetentionDays","corpusPredictionEnabled","styleSceneMask","stylePreferences","toneReplyEnabled","toneContinuationEnabled","toneCloudModelEnabled" }) legacy["settings"]!.AsObject().Remove(field);
    foreach (var field in new[] { "expandedPreviousPageShortcutEnabled", "expandedPreviousPageShortcuts",
        "expandedNextPageShortcutEnabled", "expandedNextPageShortcuts" }) legacy["settings"]!.AsObject().Remove(field);
    legacy["settings"]!["previousPageShortcuts"] = new JsonArray("[", "Up");
    legacy["settings"]!["nextPageShortcuts"] = new JsonArray("]", "Down");
    await File.WriteAllTextAsync(invalid, legacy.ToJsonString());
    var migrated = await SettingsTransfer.ReadAsync(invalid);
    Check(migrated.FormatVersion == 3 && migrated.Settings.PreviousPageShortcuts.SequenceEqual(new[] { "[" }) &&
        migrated.Settings.ExpandedPreviousPageShortcuts.SequenceEqual(new[] { "Up" }) &&
        migrated.Settings.ExpandedNextPageShortcutEnabled, "Version 1 migrates legacy arrow bindings");
    await client.ValidateAsync(migrated.Settings);
    legacy["settings"]!["previousPageShortcutEnabled"] = false;
    await File.WriteAllTextAsync(invalid, legacy.ToJsonString());
    Check(!(await SettingsTransfer.ReadAsync(invalid)).Settings.ExpandedPreviousPageShortcutEnabled,
        "Legacy disabled paging remains disabled after migration");
    legacy["settings"]!["previousPageShortcuts"] = new JsonArray("[");
    legacy["settings"]!["rawInputShortcuts"] = new JsonArray("Up");
    legacy["settings"]!["rawInputShortcutEnabled"] = true;
    await File.WriteAllTextAsync(invalid, legacy.ToJsonString());
    var conflict = await SettingsTransfer.ReadAsync(invalid);
    Check(!conflict.Settings.ExpandedPreviousPageShortcutEnabled && conflict.Settings.RawInputShortcuts[0] == "Up",
        "Legacy custom key wins over newly added shortcut");
    await client.ValidateAsync(conflict.Settings);

    var learning = Path.Combine(root, "user-frequency.owuf");
    var plugin = Path.Combine(root, "plugin-permissions.json");
    await File.WriteAllTextAsync(learning, "learning sentinel");
    await File.WriteAllTextAsync(plugin, "plugin sentinel");
    var backup = await SettingsTransfer.ApplyAsync(client, prefs, roundtrip);
    var applied = await client.LoadAsync();
    Check(applied.CandidatePageSize == changed.CandidatePageSize &&
        applied.LanguageShortcuts.SequenceEqual(changed.LanguageShortcuts), "Native import persisted");
    Check(System.Text.Json.JsonSerializer.Serialize(applied) ==
        System.Text.Json.JsonSerializer.Serialize(changed), "Every setting survives native import");
    Check(!SettingsUiPreferences.Load(prefs).AutoSaveEnabled, "UI preference applied even with auto-save off");
    Check(original.SequenceEqual(File.ReadAllBytes(Path.Combine(backup, "owo.conf"))) &&
        originalPrefs.SequenceEqual(File.ReadAllBytes(Path.Combine(backup, "settings-ui.json"))), "Exact backup");
    Check(File.ReadAllText(learning) == "learning sentinel" && File.ReadAllText(plugin) == "plugin sentinel", "Excluded data unchanged");

    var blockedPrefs = Path.Combine(root, "blocked-preferences"); Directory.CreateDirectory(blockedPrefs);
    var beforeRollback = File.ReadAllBytes(client.ConfigPath);
    await Reject(() => SettingsTransfer.ApplyAsync(client, blockedPrefs,
        SettingsTransfer.Create(defaults, true)), "preference write failure");
    Check((await client.LoadAsync()).CandidatePageSize == changed.CandidatePageSize, "Rollback after UI preference failure");
    Check(beforeRollback.SequenceEqual(File.ReadAllBytes(client.ConfigPath)), "Rollback preserves exact original file");
    var emptyClient = new ConfigShellClient(Path.Combine(root, "empty", "owo.conf"), Path.GetFullPath(args[0]));
    await Reject(() => SettingsTransfer.ApplyAsync(emptyClient, blockedPrefs, roundtrip), "fresh install rollback");
    Check(!File.Exists(emptyClient.ConfigPath), "Failed import leaves absent config absent");
    Check(!Directory.EnumerateFiles(root, "*.tmp", SearchOption.AllDirectories).Any(), "No abandoned export/preference temps");

    // 替换已有导出文件与 UTF-8 BOM 文件均能正常使用。
    await SettingsTransfer.WriteAsync(exported, SettingsTransfer.Create(defaults, true));
    await File.WriteAllTextAsync(exported, File.ReadAllText(exported), new UTF8Encoding(true));
    Check((await SettingsTransfer.ReadAsync(exported)).UiPreferences.AutoSaveEnabled, "Overwrite/BOM roundtrip");
    Console.WriteLine($"PASS: {checks} configuration transfer checks (isolated data only).");
} finally { Directory.Delete(root, recursive: true); }
