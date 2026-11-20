using System.Text.Json;
using OwO_Settings;

if (args.Length != 1 || !File.Exists(args[0])) throw new ArgumentException("Pass owo_config_shell.exe.");
var root = Path.Combine(Path.GetTempPath(), "OwO-settings-modules-" + Guid.NewGuid().ToString("N"));
var checks = 0;
void Check(bool value, string message) {
    if (!value) throw new Exception(message);
    ++checks;
}
void Reject(Action action, string message) {
    try { action(); } catch (InvalidOperationException) { ++checks; return; }
    throw new Exception("Expected rejection: " + message);
}
string Json(object? value) => JsonSerializer.Serialize(value);
try {
    var client = new ConfigShellClient(Path.Combine(root, "owo.conf"), Path.GetFullPath(args[0]));
    var defaults = await client.LoadDefaultsAsync();
    await client.SaveAsync(defaults);
    var before = File.ReadAllBytes(client.ConfigPath);
    var modules = new SettingsModules();
    var fields = modules.Modules.SelectMany(m => m.Fields).ToArray();
    var properties = typeof(SettingsSnapshot).GetProperties();
    Check(modules.Modules.Select(m => m.Id).Distinct().Count() == modules.Modules.Count, "Unique module identity");
    Check(fields.Select(f => f.Key).Distinct().Count() == properties.Length, "Every field has exactly one owner");
    var editors = new MemoryEditors();
    modules.Load(defaults, editors);
    Check(Json(modules.Capture(defaults, editors)) == Json(defaults), "All fields load/capture without loss");
    foreach (var property in properties) {
        var previous = property.GetValue(defaults)!;
        object changed = previous switch {
            bool boolean => !boolean,
            uint number => number + 1,
            string => "plugin:test.input/table",
            IReadOnlyList<string> => new[] { "Ctrl+F12" },
            _ => throw new Exception("Uncovered field type")
        };
        editors.Set(property.Name, changed);
        var captured = modules.Capture(defaults, editors);
        foreach (var other in properties)
            Check(Json(other.GetValue(captured)) == Json(other.Name == property.Name ? changed : other.GetValue(defaults)),
                "Field isolation: " + property.Name + " must not change " + other.Name);
        modules.Load(defaults, editors);
    }
    var full = InputSchemeSettingsPolicy.Resolve("full_pinyin");
    var exact = InputSchemeSettingsPolicy.Resolve("exact_code");
    var plugin = InputSchemeSettingsPolicy.Resolve("plugin:example.input/cangjie");
    var unknown = InputSchemeSettingsPolicy.Resolve("future-scheme");
    var future = new InputSchemeOption("future.phonetic", "未来音码", "", "",
        SettingsCapability.CandidateDisplay | SettingsCapability.Correction);
    var allCapabilities = Enum.GetValues<SettingsCapability>().Aggregate(SettingsCapability.None, (a, b) => a | b);
    Check(full.Supports(allCapabilities), "Full pinyin supports all currently connected settings");
    Check(string.IsNullOrEmpty(full.Capabilities), "Full pinyin does not show the passive capability banner");
    Check(!string.IsNullOrEmpty(exact.Capabilities) && !string.IsNullOrEmpty(plugin.Capabilities),
        "Other scheme explanations are not removed by the full pinyin UI change");
    Check(exact.Supports(SettingsCapability.UserLearning | SettingsCapability.ModelRanking | SettingsCapability.CandidatePartition), "Exact code keeps scheme-independent core features");
    Check(!exact.Supports(SettingsCapability.Correction) && !exact.Supports(SettingsCapability.UserCompletion) &&
        !exact.Supports(SettingsCapability.SingleCharacterFirst), "Exact code excludes pinyin assistance");
    Check(plugin.Supports(SettingsCapability.CandidateDisplay | SettingsCapability.EncodingCursor |
        SettingsCapability.CandidatePaging | SettingsCapability.LanguageSwitch | SettingsCapability.RawCommit), "Plugin host controls remain configurable");
    Check(!plugin.Supports(SettingsCapability.UserLearning) && !plugin.Supports(SettingsCapability.ChinesePunctuation), "Plugin must not inherit unconnected core features");
    Check(plugin.OwnerPluginId == "example.input" && plugin.Id == "plugin:example.input/cangjie", "Plugin settings identity is retained");
    Check(unknown.SettingsCapabilities == SettingsCapability.None, "Unknown schemes fail closed");
    Check(future.Supports(SettingsCapability.Correction) && !future.Supports(SettingsCapability.UserCompletion), "Feature gating does not require a full_pinyin ID");

    var changedAll = defaults with { CandidatePageSize = 9, CandidateWrapLength = 40,
        CandidatePartitionEnabled = !defaults.CandidatePartitionEnabled,
        CandidateSingleCharacterFirstEnabled = !defaults.CandidateSingleCharacterFirstEnabled,
        ChinesePunctuationEnabled = !defaults.ChinesePunctuationEnabled, ChinesePunctuationMask = 3,
        UserLearningEnabled = !defaults.UserLearningEnabled, UserLearningSensitivity = 2,
        ModelRankingEnabled = !defaults.ModelRankingEnabled, ModelTimeoutMs = 19,
        UserCompletionEnabled = !defaults.UserCompletionEnabled, UserCompletionMaxSyllables = 7,
        CorrectionShortcuts = new[] { "Ctrl+F12" } };
    foreach (var scheme in new[] { full, exact, plugin, unknown, future }) {
        foreach (var page in modules.Modules.Select(m => m.Page).Distinct().Append("unrelated-page")) {
            modules.Load(changedAll, editors);
            modules.ResetPage(page, defaults, editors, scheme);
            var result = modules.Capture(changedAll, editors);
            foreach (var property in properties) {
                var reset = modules.PageFor(property.Name) == page && scheme.Supports(modules.Field(property.Name).Capability);
                Check(Json(property.GetValue(result)) == Json(property.GetValue(reset ? defaults : changedAll)),
                    "Scoped reset: " + scheme.Id + "/" + page + "/" + property.Name);
            }
        }
    }
    modules.Load(changedAll, editors);
    foreach (var scheme in new[] { exact, plugin, full }) {
        foreach (var field in fields) _ = scheme.Supports(field.Capability);
        Check(Json(modules.Capture(changedAll, editors)) == Json(changedAll), "Switching capability policy retains dormant configuration");
    }
    editors.Remove(nameof(SettingsSnapshot.InputScheme));
    Reject(() => modules.ValidateEditors(editors), "Missing editor");
    editors.Set(nameof(SettingsSnapshot.InputScheme), "full_pinyin");
    editors.Set("unknown", true);
    Reject(() => modules.ValidateEditors(editors), "Unknown editor");
    editors.Remove("unknown");
    foreach (var invalid in new[] { double.NaN, double.PositiveInfinity, -1.0, 1.5, 10.0, (double)uint.MaxValue })
        Reject(() => SettingsModules.UnsignedNumber(invalid, 1, 9, "页数"), "Invalid numeric editor value");
    Check(SettingsModules.UnsignedNumber(1, 1, 9, "页数") == 1 &&
        SettingsModules.UnsignedNumber(9, 1, 9, "页数") == 9 &&
        SettingsModules.UnsignedNumber(0, 0, 8, "补全") == 0, "Numeric boundaries");
    ShortcutSettingsValidation.Validate(defaults);
    Reject(() => ShortcutSettingsValidation.Validate(defaults with { LanguageShortcuts = defaults.RawInputShortcuts,
        LanguageShortcutEnabled = true, RawInputShortcutEnabled = true }), "Shared shortcut conflict validation");
    Reject(() => ShortcutSettingsValidation.Validate(defaults with { PunctuationShortcutEnabled = true,
        PunctuationShortcuts = Array.Empty<string>() }), "Empty enabled punctuation shortcut");
    Check(before.SequenceEqual(File.ReadAllBytes(client.ConfigPath)), "Module edits/tests do not write runtime settings");
    Console.WriteLine($"PASS: {checks} settings module/policy checks, {modules.Modules.Count} modules, {fields.Length} fields.");
} finally { if (Directory.Exists(root)) Directory.Delete(root, recursive: true); }

internal sealed class MemoryEditors : ISettingsEditors
{
    private readonly Dictionary<string, object?> _values = typeof(SettingsSnapshot).GetProperties()
        .ToDictionary(p => p.Name, _ => (object?)null);
    public IReadOnlyCollection<string> Keys => _values.Keys;
    public T Read<T>(string key) => (T)_values[key]!;
    public void Write<T>(string key, T value) => _values[key] = value;
    internal void Set(string key, object value) => _values[key] = value;
    internal void Remove(string key) => _values.Remove(key);
}
