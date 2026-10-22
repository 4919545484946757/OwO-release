using System.Diagnostics;

namespace OwO_Settings;

internal sealed record SettingsSnapshot(string InputScheme,
                                        uint CandidatePageSize, uint CandidateWrapLength,
                                        bool CandidatePartitionEnabled,
                                        bool CandidateImmediateCommitEnabled,
                                        bool FocusLossCommitEnabled,
                                        bool ChinesePunctuationEnabled,
                                        bool AltCodePreviewEnabled,
                                        uint AltCodePreviewPageSize,
                                        bool AltCodePreviewWheelEnabled,
                                        uint UserLearningSensitivity,
                                        bool UserLearningEnabled,
                                        bool UserCompletionEnabled,
                                        uint UserCompletionMaxSyllables,
                                        bool ModelRankingEnabled, uint ModelTimeoutMs,
                                        bool CorrectionShortcutEnabled, IReadOnlyList<string> CorrectionShortcuts,
                                        bool LanguageShortcutEnabled, IReadOnlyList<string> LanguageShortcuts,
                                        bool RawInputShortcutEnabled, IReadOnlyList<string> RawInputShortcuts,
                                        bool CursorLeftShortcutEnabled, IReadOnlyList<string> CursorLeftShortcuts,
                                        bool CursorRightShortcutEnabled, IReadOnlyList<string> CursorRightShortcuts,
                                        bool PreviousPageShortcutEnabled, IReadOnlyList<string> PreviousPageShortcuts,
                                        bool NextPageShortcutEnabled, IReadOnlyList<string> NextPageShortcuts,
                                        bool PunctuationShortcutEnabled, IReadOnlyList<string> PunctuationShortcuts,
                                        bool TextNavigationShortcutEnabled, IReadOnlyList<string> TextNavigationShortcuts,
                                        bool TextNavigationSelectEnabled,
                                        uint ChinesePunctuationMask,
                                        bool CorpusLearningEnabled,
                                        bool CorpusLearningPaused,
                                        IReadOnlyList<string> CorpusAllowedApps,
                                        IReadOnlyList<string> CorpusExcludedApps,
                                        uint CorpusRetentionDays,
                                        bool CorpusPredictionEnabled,
                                        uint StyleSceneMask,
                                        string StylePreferences,
                                        bool ToneReplyEnabled,
                                        bool ToneContinuationEnabled,
                                        bool ToneCloudModelEnabled);

internal sealed class ConfigShellClient
{
    private readonly string _shellPath = Environment.GetEnvironmentVariable("OWO_CONFIG_SHELL_PATH")
        ?? Path.Combine(AppContext.BaseDirectory, "owo_config_shell.exe");
    private readonly string _configPath = Environment.GetEnvironmentVariable("OWO_CONFIG_PATH")
        ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                        "OwO", "InputMethod", "config", "owo.conf");

    internal string ConfigPath => _configPath;

    internal async Task<SettingsSnapshot> LoadAsync(CancellationToken cancellationToken = default)
    {
        var result = await RunAsync([_configPath, "show"], cancellationToken);
        return Parse(result);
    }

    internal async Task<SettingsSnapshot> LoadDefaultsAsync(
        CancellationToken cancellationToken = default)
    {
        var result = await RunAsync([_configPath, "defaults"], cancellationToken);
        return Parse(result);
    }

    private static SettingsSnapshot Parse(string result)
    {
        var values = result.Split('\n', StringSplitOptions.RemoveEmptyEntries)
            .Select(line => line.TrimEnd('\r').Split('=', 2)).Where(parts => parts.Length == 2)
            .ToDictionary(parts => parts[0], parts => parts[1], StringComparer.Ordinal);
        return new(values["input_scheme"],
                   uint.Parse(values["candidate_page_size"]),
                   uint.Parse(values["candidate_wrap_length"]),
                   bool.Parse(values["candidate_partition_enabled"]),
                   bool.Parse(values["candidate_immediate_commit_enabled"]),
                   bool.Parse(values["focus_loss_commit_enabled"]),
                   bool.Parse(values["chinese_punctuation_enabled"]),
                   bool.Parse(values["alt_code_preview_enabled"]),
                   uint.Parse(values["alt_code_preview_page_size"]),
                   bool.Parse(values["alt_code_preview_wheel_enabled"]),
                   uint.Parse(values["user_learning_sensitivity"]),
                   bool.Parse(values["user_learning_enabled"]),
                   bool.Parse(values["user_lexicon_completion_enabled"]),
                   uint.Parse(values["user_lexicon_completion_max_syllables"]),
                   bool.Parse(values["model_ranking_enabled"]),
                   uint.Parse(values["model_timeout_ms"]),
                   bool.Parse(values["correction_shortcut_enabled"]),
                   SplitShortcuts(values["correction_shortcut"]),
                   bool.Parse(values["language_shortcut_enabled"]),
                   SplitShortcuts(values["language_shortcut"]),
                   bool.Parse(values["raw_input_shortcut_enabled"]),
                   SplitShortcuts(values["raw_input_shortcut"]),
                   bool.Parse(values["cursor_left_shortcut_enabled"]),
                   SplitShortcuts(values["cursor_left_shortcut"]),
                   bool.Parse(values["cursor_right_shortcut_enabled"]),
                   SplitShortcuts(values["cursor_right_shortcut"]),
                   bool.Parse(values["previous_page_shortcut_enabled"]),
                   SplitShortcuts(values["previous_page_shortcut"]),
                   bool.Parse(values["next_page_shortcut_enabled"]),
                   SplitShortcuts(values["next_page_shortcut"]),
                   bool.Parse(values["punctuation_shortcut_enabled"]),
                   SplitShortcuts(values["punctuation_shortcut"]),
                   bool.Parse(values["text_navigation_shortcut_enabled"]),
                   SplitShortcuts(values["text_navigation_shortcut"]),
                   bool.Parse(values["text_navigation_select_enabled"]),
                   uint.Parse(values["chinese_punctuation_mask"]),
                   bool.Parse(values["corpus_learning_enabled"]),
                   bool.Parse(values["corpus_learning_paused"]),
                   SplitShortcuts(values["corpus_learning_allowed_apps"]),
                   SplitShortcuts(values["corpus_learning_excluded_apps"]),
                   uint.Parse(values["corpus_retention_days"]),
                   bool.Parse(values["corpus_prediction_enabled"]),
                   uint.Parse(values["style_scene_mask"]),
                   UnescapeStylePreferences(values["style_preferences"]),
                   bool.Parse(values["tone_reply_enabled"]),
                   bool.Parse(values["tone_continuation_enabled"]),
                   bool.Parse(values["tone_cloud_model_enabled"]));
    }

    internal Task SaveAsync(SettingsSnapshot value, CancellationToken cancellationToken = default) =>
        RunAsync([_configPath, "set-all", value.CandidatePageSize.ToString(),
                  value.UserLearningEnabled.ToString().ToLowerInvariant(),
                  value.ModelRankingEnabled.ToString().ToLowerInvariant(),
                  value.ModelTimeoutMs.ToString(),
                  value.CorrectionShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.CorrectionShortcuts),
                  value.LanguageShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.LanguageShortcuts),
                  value.RawInputShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.RawInputShortcuts),
                  value.CandidateWrapLength.ToString(),
                  value.UserLearningSensitivity.ToString(),
                  value.CursorLeftShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.CursorLeftShortcuts),
                  value.CursorRightShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.CursorRightShortcuts),
                  value.PreviousPageShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.PreviousPageShortcuts),
                  value.NextPageShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.NextPageShortcuts),
                  value.CandidatePartitionEnabled.ToString().ToLowerInvariant(),
                  value.ChinesePunctuationEnabled.ToString().ToLowerInvariant(),
                  value.PunctuationShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.PunctuationShortcuts),
                  value.AltCodePreviewEnabled.ToString().ToLowerInvariant(),
                  value.AltCodePreviewPageSize.ToString(),
                  value.AltCodePreviewWheelEnabled.ToString().ToLowerInvariant(),
                  value.CandidateImmediateCommitEnabled.ToString().ToLowerInvariant(),
                  value.FocusLossCommitEnabled.ToString().ToLowerInvariant(),
                  value.TextNavigationShortcutEnabled.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.TextNavigationShortcuts),
                  value.TextNavigationSelectEnabled.ToString().ToLowerInvariant(),
                  value.InputScheme,
                  value.UserCompletionEnabled.ToString().ToLowerInvariant(),
                  value.UserCompletionMaxSyllables.ToString(),
                  value.ChinesePunctuationMask.ToString(),
                  value.CorpusLearningEnabled.ToString().ToLowerInvariant(),
                  value.CorpusLearningPaused.ToString().ToLowerInvariant(),
                  JoinShortcuts(value.CorpusAllowedApps),
                  JoinShortcuts(value.CorpusExcludedApps),
                  value.CorpusRetentionDays.ToString(),
                  value.CorpusPredictionEnabled.ToString().ToLowerInvariant(),
                  value.StyleSceneMask.ToString(),
                  NormalizeStylePreferences(value.StylePreferences),
                  value.ToneReplyEnabled.ToString().ToLowerInvariant(),
                  value.ToneContinuationEnabled.ToString().ToLowerInvariant(),
                  value.ToneCloudModelEnabled.ToString().ToLowerInvariant()],
            cancellationToken);

    // Preferences are one line in the key=value file. A literal newline cannot
    // be represented, so it is dropped rather than rejected after the user has
    // already typed the rest of the setting.
    internal static string NormalizeStylePreferences(string value) =>
        value.Replace("\r", " ", StringComparison.Ordinal)
             .Replace("\n", " ", StringComparison.Ordinal);

    // The file format escapes `~` when it serializes; the shell reads `show`
    // output, so it must undo exactly that escaping.
    internal static string UnescapeStylePreferences(string value)
    {
        var result = new System.Text.StringBuilder(value.Length);
        for (var index = 0; index < value.Length; ++index) {
            if (value[index] != '~') { result.Append(value[index]); continue; }
            if (index + 1 >= value.Length || value[index + 1] != '~')
                throw new FormatException("style_preferences 含有无效的转义。");
            result.Append('~');
            ++index;
        }
        return result.ToString();
    }

    private static string[] SplitShortcuts(string value) =>
        value.Split(';', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);

    private static string JoinShortcuts(IReadOnlyList<string> value) => string.Join(';', value);

    private async Task<string> RunAsync(IEnumerable<string> arguments,
                                        CancellationToken cancellationToken)
    {
        if (!File.Exists(_shellPath))
            throw new FileNotFoundException("找不到 OwO 配置后端。", _shellPath);
        Directory.CreateDirectory(Path.GetDirectoryName(_configPath)!);
        var start = new ProcessStartInfo(_shellPath) {
            UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardOutput = true, RedirectStandardError = true,
        };
        foreach (var argument in arguments) start.ArgumentList.Add(argument);
        using var process = Process.Start(start) ?? throw new InvalidOperationException("无法启动配置后端。");
        var output = process.StandardOutput.ReadToEndAsync(cancellationToken);
        var error = process.StandardError.ReadToEndAsync(cancellationToken);
        await process.WaitForExitAsync(cancellationToken);
        if (process.ExitCode != 0) {
            var message = (await error).Trim();
            throw new InvalidOperationException(message.Length > 0 ? message :
                $"配置后端退出码：{process.ExitCode}");
        }
        return await output;
    }
}
