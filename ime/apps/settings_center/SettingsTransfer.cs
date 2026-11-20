using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Text.Json.Nodes;

namespace OwO_Settings;

internal sealed record TransferUiPreferences(bool AutoSaveEnabled);
internal sealed record SettingsTransferDocument(int FormatVersion, string Kind,
    SettingsSnapshot Settings, TransferUiPreferences UiPreferences);

internal static class SettingsTransfer
{
    internal const int MaximumBytes = 64 * 1024;
    private static readonly JsonSerializerOptions Options = new() {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
        WriteIndented = true,
        RespectRequiredConstructorParameters = true,
        RespectNullableAnnotations = true,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
        MaxDepth = 8
    };

    internal static SettingsTransferDocument Create(SettingsSnapshot settings, bool autoSave) =>
        new(3, "OwO.Settings", settings, new(autoSave));

    internal static async Task<SettingsTransferDocument> ReadAsync(string path)
    {
        await using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read,
                                                4096, useAsync: true);
        if (stream.Length > MaximumBytes) throw new InvalidDataException("配置文件不能超过 64 KiB。");
        var bytes = new byte[MaximumBytes + 1];
        var count = await stream.ReadAtLeastAsync(bytes, bytes.Length, throwOnEndOfStream: false);
        if (count > MaximumBytes) throw new InvalidDataException("配置文件不能超过 64 KiB。");
        // UTF-8 BOM 可接受；非法字节不允许被替换后继续导入。
        var offset = count >= 3 && bytes[0] == 0xef && bytes[1] == 0xbb && bytes[2] == 0xbf ? 3 : 0;
        var json = new UTF8Encoding(false, true).GetString(bytes, offset, count - offset);
        using var parsed = JsonDocument.Parse(json, new() { MaxDepth = 8 });
        RejectDuplicateProperties(parsed.RootElement);
        var legacy = parsed.RootElement.ValueKind == JsonValueKind.Object &&
            parsed.RootElement.TryGetProperty("formatVersion", out var version) &&
            version.ValueKind == JsonValueKind.Number && version.TryGetInt32(out var number) && number == 1;
        if (legacy) {
            var root = JsonNode.Parse(json)!.AsObject();
            var settings = root["settings"] as JsonObject ?? throw new InvalidDataException("旧版设置快照无效。");
            string[] newFields = ["expandedPreviousPageShortcutEnabled", "expandedPreviousPageShortcuts",
                "expandedNextPageShortcutEnabled", "expandedNextPageShortcuts"];
            if (newFields.Any(settings.ContainsKey)) throw new InvalidDataException("旧版设置包含未知字段。");
            settings[newFields[0]] = false;
            settings[newFields[1]] = new JsonArray("Up");
            settings[newFields[2]] = false;
            settings[newFields[3]] = new JsonArray("Down");
            root["formatVersion"] = 2;
            json = root.ToJsonString();
        }
        var upgraded = JsonNode.Parse(json)!.AsObject();
        if (upgraded["formatVersion"]?.GetValue<int>() == 2) {
            var settings = upgraded["settings"] as JsonObject
                ?? throw new InvalidDataException("Invalid legacy settings snapshot.");
            var defaults = new Dictionary<string, JsonNode?> {
                ["corpusLearningEnabled"] = JsonValue.Create(false),
                ["corpusLearningPaused"] = JsonValue.Create(false),
                ["corpusAllowedApps"] = new JsonArray(),
                ["corpusExcludedApps"] = new JsonArray("KeePass.exe", "KeePassXC.exe", "1Password.exe", "Bitwarden.exe"),
                ["corpusRetentionDays"] = JsonValue.Create(180u),
                ["corpusPredictionEnabled"] = JsonValue.Create(true),
                ["styleSceneMask"] = JsonValue.Create(7u),
                ["stylePreferences"] = JsonValue.Create(""),
                ["toneReplyEnabled"] = JsonValue.Create(false),
                ["toneContinuationEnabled"] = JsonValue.Create(false),
                ["toneCloudModelEnabled"] = JsonValue.Create(false)
            };
            if (defaults.Keys.Any(settings.ContainsKey))
                throw new InvalidDataException("Legacy settings contain unknown corpus or tone fields.");
            foreach (var entry in defaults) settings[entry.Key] = entry.Value;
            upgraded["formatVersion"] = 3;
            json = upgraded.ToJsonString();
        }
        var document = JsonSerializer.Deserialize<SettingsTransferDocument>(json, Options)
            ?? throw new InvalidDataException("配置文件内容为空。");
        ValidateStructure(document);
        if (legacy) document = document with { Settings = MigrateLegacyShortcuts(document.Settings) };
        return document;
    }

    private static SettingsSnapshot MigrateLegacyShortcuts(SettingsSnapshot value)
    {
        // 与原生 v16→v17 迁移保持一致；不覆盖其他动作已占用的方向键。
        ShortcutSettingsValidation.Validate(value);
        var up = value.PreviousPageShortcuts.Contains("Up");
        var down = value.NextPageShortcuts.Contains("Down");
        var previous = value.PreviousPageShortcuts.Where(key => key != "Up").ToArray();
        var next = value.NextPageShortcuts.Where(key => key != "Down").ToArray();
        var migrated = value with {
            PreviousPageShortcuts = previous, NextPageShortcuts = next,
            PreviousPageShortcutEnabled = value.PreviousPageShortcutEnabled && (!up || previous.Length != 0),
            NextPageShortcutEnabled = value.NextPageShortcutEnabled && (!down || next.Length != 0),
            ExpandedPreviousPageShortcuts = new[] { "Up" }, ExpandedNextPageShortcuts = new[] { "Down" },
            ExpandedPreviousPageShortcutEnabled = !up || value.PreviousPageShortcutEnabled,
            ExpandedNextPageShortcutEnabled = !down || value.NextPageShortcutEnabled
        };
        var used = new List<string>();
        if (migrated.CorrectionShortcutEnabled) used.AddRange(migrated.CorrectionShortcuts);
        if (migrated.LanguageShortcutEnabled) used.AddRange(migrated.LanguageShortcuts);
        if (migrated.RawInputShortcutEnabled) used.AddRange(migrated.RawInputShortcuts);
        if (migrated.CursorLeftShortcutEnabled) used.AddRange(migrated.CursorLeftShortcuts);
        if (migrated.CursorRightShortcutEnabled) used.AddRange(migrated.CursorRightShortcuts);
        if (migrated.PreviousPageShortcutEnabled) used.AddRange(previous);
        if (migrated.NextPageShortcutEnabled) used.AddRange(next);
        if (migrated.TextNavigationShortcutEnabled) used.AddRange(migrated.TextNavigationShortcuts);
        if (migrated.PunctuationShortcutEnabled) used.AddRange(migrated.PunctuationShortcuts);
        return migrated with {
            ExpandedPreviousPageShortcutEnabled = migrated.ExpandedPreviousPageShortcutEnabled && !used.Contains("Up"),
            ExpandedNextPageShortcutEnabled = migrated.ExpandedNextPageShortcutEnabled && !used.Contains("Down")
        };
    }

    private static void RejectDuplicateProperties(JsonElement element)
    {
        if (element.ValueKind == JsonValueKind.Object) {
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in element.EnumerateObject()) {
                if (!names.Add(property.Name)) throw new InvalidDataException("配置文件包含重复字段。");
                RejectDuplicateProperties(property.Value);
            }
        } else if (element.ValueKind == JsonValueKind.Array) {
            foreach (var child in element.EnumerateArray()) RejectDuplicateProperties(child);
        }
    }

    private static void ValidateStructure(SettingsTransferDocument document)
    {
        if (document.FormatVersion != 3 || document.Kind != "OwO.Settings")
            throw new InvalidDataException("不支持此配置文件格式或版本。");
        var value = document.Settings;
        IReadOnlyList<string>[] groups = [value.CorrectionShortcuts, value.LanguageShortcuts,
            value.RawInputShortcuts, value.CursorLeftShortcuts, value.CursorRightShortcuts,
            value.PreviousPageShortcuts, value.NextPageShortcuts, value.PunctuationShortcuts,
            value.TextNavigationShortcuts, value.ExpandedPreviousPageShortcuts, value.ExpandedNextPageShortcuts];
        foreach (var group in groups) {
            if (group.Count > 16 || group.Any(key => string.IsNullOrWhiteSpace(key) ||
                key.Length > 128 || key.Contains(';') || key.Any(char.IsControl)))
                throw new InvalidDataException("快捷键列表包含无效项；每项只能包含一个按键组合。");
        }
    }

    internal static async Task WriteAsync(string path, SettingsTransferDocument document)
    {
        ValidateStructure(document);
        var bytes = JsonSerializer.SerializeToUtf8Bytes(document, Options);
        if (bytes.Length > MaximumBytes) throw new InvalidDataException("配置文件不能超过 64 KiB。");
        var temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try {
            await File.WriteAllBytesAsync(temporary, bytes);
            File.Move(temporary, path, overwrite: true);
        } finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }

    internal static async Task<string> ApplyAsync(ConfigShellClient client, string preferencesPath,
                                                 SettingsTransferDocument document)
    {
        ValidateStructure(document);
        var validated = await client.ValidateAsync(document.Settings);
        var hadConfig = File.Exists(client.ConfigPath);
        var backup = Path.Combine(Path.GetDirectoryName(client.ConfigPath)!, "backups",
            "import-" + DateTime.Now.ToString("yyyyMMdd-HHmmss") + "-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(backup);
        if (hadConfig)
            File.Copy(client.ConfigPath, Path.Combine(backup, "owo.conf"));
        if (File.Exists(preferencesPath))
            File.Copy(preferencesPath, Path.Combine(backup, "settings-ui.json"));
        try {
            await client.SaveAsync(validated);
            new SettingsUiPreferences(document.UiPreferences.AutoSaveEnabled).Save(preferencesPath);
        }
        catch (Exception error) {
            // 直接原子恢复原文件，保留注释、编码及 EFS 属性；原先不存在时不留下默认配置。
            var temporary = client.ConfigPath + "." + Guid.NewGuid().ToString("N") + ".rollback.tmp";
            try {
                if (hadConfig) {
                    File.Copy(Path.Combine(backup, "owo.conf"), temporary);
                    File.Move(temporary, client.ConfigPath, overwrite: true);
                } else { File.Delete(client.ConfigPath); }
            }
            catch (Exception rollback) {
                throw new IOException($"导入失败且原配置回滚失败，请从备份恢复：{backup}",
                    new AggregateException(error, rollback));
            } finally { if (File.Exists(temporary)) File.Delete(temporary); }
            throw new IOException($"导入失败，已恢复原配置。备份：{backup}；{error.Message}", error);
        }
        return backup;
    }
}
