namespace OwO_Settings;

internal static class SettingsModuleDefinitions
{
    private static SettingsField<T> F<T>(string name, Func<SettingsSnapshot, T> get,
        Func<SettingsSnapshot, T, SettingsSnapshot> set, SettingsCapability capability = SettingsCapability.None) =>
        new(name, capability, get, set);

    internal static SettingsModule[] Create() => [
        new("input-scheme", "schemes", [
            F(nameof(SettingsSnapshot.InputScheme), s => s.InputScheme, (s, v) => s with { InputScheme = v })]),
        new("candidate-display", "candidates", [
            F(nameof(SettingsSnapshot.CandidatePageSize), s => s.CandidatePageSize, (s, v) => s with { CandidatePageSize = v }, SettingsCapability.CandidateDisplay),
            F(nameof(SettingsSnapshot.CandidateWrapLength), s => s.CandidateWrapLength, (s, v) => s with { CandidateWrapLength = v }, SettingsCapability.CandidateDisplay)]),
        new("candidate-policy", "candidates", [
            F(nameof(SettingsSnapshot.CandidatePartitionEnabled), s => s.CandidatePartitionEnabled, (s, v) => s with { CandidatePartitionEnabled = v }, SettingsCapability.CandidatePartition),
            F(nameof(SettingsSnapshot.CandidateSingleCharacterFirstEnabled), s => s.CandidateSingleCharacterFirstEnabled, (s, v) => s with { CandidateSingleCharacterFirstEnabled = v }, SettingsCapability.SingleCharacterFirst),
            F(nameof(SettingsSnapshot.CandidateImmediateCommitEnabled), s => s.CandidateImmediateCommitEnabled, (s, v) => s with { CandidateImmediateCommitEnabled = v }, SettingsCapability.StagedCommit),
            F(nameof(SettingsSnapshot.FocusLossCommitEnabled), s => s.FocusLossCommitEnabled, (s, v) => s with { FocusLossCommitEnabled = v }, SettingsCapability.FocusLossCommit)]),
        new("punctuation", "punctuation", [
            F(nameof(SettingsSnapshot.ChinesePunctuationEnabled), s => s.ChinesePunctuationEnabled, (s, v) => s with { ChinesePunctuationEnabled = v }, SettingsCapability.ChinesePunctuation),
            F(nameof(SettingsSnapshot.ChinesePunctuationMask), s => s.ChinesePunctuationMask, (s, v) => s with { ChinesePunctuationMask = v }, SettingsCapability.ChinesePunctuation)]),
        new("alt-code-preview", "candidates", [
            F(nameof(SettingsSnapshot.AltCodePreviewEnabled), s => s.AltCodePreviewEnabled, (s, v) => s with { AltCodePreviewEnabled = v }, SettingsCapability.AltCodePreview),
            F(nameof(SettingsSnapshot.AltCodePreviewPageSize), s => s.AltCodePreviewPageSize, (s, v) => s with { AltCodePreviewPageSize = v }, SettingsCapability.AltCodePreview),
            F(nameof(SettingsSnapshot.AltCodePreviewWheelEnabled), s => s.AltCodePreviewWheelEnabled, (s, v) => s with { AltCodePreviewWheelEnabled = v }, SettingsCapability.AltCodePreview)]),
        new("learning", "intelligence", [
            F(nameof(SettingsSnapshot.UserLearningEnabled), s => s.UserLearningEnabled, (s, v) => s with { UserLearningEnabled = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.UserLearningSensitivity), s => s.UserLearningSensitivity, (s, v) => s with { UserLearningSensitivity = v }, SettingsCapability.UserLearning)]),
        new("completion", "intelligence", [
            F(nameof(SettingsSnapshot.UserCompletionEnabled), s => s.UserCompletionEnabled, (s, v) => s with { UserCompletionEnabled = v }, SettingsCapability.UserCompletion),
            F(nameof(SettingsSnapshot.UserCompletionMaxSyllables), s => s.UserCompletionMaxSyllables, (s, v) => s with { UserCompletionMaxSyllables = v }, SettingsCapability.UserCompletion)]),
        new("ranking", "intelligence", [
            F(nameof(SettingsSnapshot.ModelRankingEnabled), s => s.ModelRankingEnabled, (s, v) => s with { ModelRankingEnabled = v }, SettingsCapability.ModelRanking),
            F(nameof(SettingsSnapshot.ModelTimeoutMs), s => s.ModelTimeoutMs, (s, v) => s with { ModelTimeoutMs = v }, SettingsCapability.ModelRanking)]),
        new("corpus-and-tone", "intelligence", [
            F(nameof(SettingsSnapshot.CorpusLearningEnabled), s => s.CorpusLearningEnabled, (s, v) => s with { CorpusLearningEnabled = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.CorpusLearningPaused), s => s.CorpusLearningPaused, (s, v) => s with { CorpusLearningPaused = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.CorpusAllowedApps), s => s.CorpusAllowedApps, (s, v) => s with { CorpusAllowedApps = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.CorpusExcludedApps), s => s.CorpusExcludedApps, (s, v) => s with { CorpusExcludedApps = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.CorpusRetentionDays), s => s.CorpusRetentionDays, (s, v) => s with { CorpusRetentionDays = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.CorpusPredictionEnabled), s => s.CorpusPredictionEnabled, (s, v) => s with { CorpusPredictionEnabled = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.ToneReplyEnabled), s => s.ToneReplyEnabled, (s, v) => s with { ToneReplyEnabled = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.ToneContinuationEnabled), s => s.ToneContinuationEnabled, (s, v) => s with { ToneContinuationEnabled = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.ToneCloudModelEnabled), s => s.ToneCloudModelEnabled, (s, v) => s with { ToneCloudModelEnabled = v }, SettingsCapability.UserLearning)]),
        new("style-profile", "style", [
            F(nameof(SettingsSnapshot.StyleSceneMask), s => s.StyleSceneMask, (s, v) => s with { StyleSceneMask = v }, SettingsCapability.UserLearning),
            F(nameof(SettingsSnapshot.StylePreferences), s => s.StylePreferences, (s, v) => s with { StylePreferences = v }, SettingsCapability.UserLearning)]),
        new("shortcuts", "shortcuts", [
            F(nameof(SettingsSnapshot.CorrectionShortcutEnabled), s => s.CorrectionShortcutEnabled, (s, v) => s with { CorrectionShortcutEnabled = v }, SettingsCapability.Correction),
            F(nameof(SettingsSnapshot.CorrectionShortcuts), s => s.CorrectionShortcuts, (s, v) => s with { CorrectionShortcuts = v }, SettingsCapability.Correction),
            F(nameof(SettingsSnapshot.LanguageShortcutEnabled), s => s.LanguageShortcutEnabled, (s, v) => s with { LanguageShortcutEnabled = v }, SettingsCapability.LanguageSwitch),
            F(nameof(SettingsSnapshot.LanguageShortcuts), s => s.LanguageShortcuts, (s, v) => s with { LanguageShortcuts = v }, SettingsCapability.LanguageSwitch),
            F(nameof(SettingsSnapshot.RawInputShortcutEnabled), s => s.RawInputShortcutEnabled, (s, v) => s with { RawInputShortcutEnabled = v }, SettingsCapability.RawCommit),
            F(nameof(SettingsSnapshot.RawInputShortcuts), s => s.RawInputShortcuts, (s, v) => s with { RawInputShortcuts = v }, SettingsCapability.RawCommit),
            F(nameof(SettingsSnapshot.CursorLeftShortcutEnabled), s => s.CursorLeftShortcutEnabled, (s, v) => s with { CursorLeftShortcutEnabled = v }, SettingsCapability.EncodingCursor),
            F(nameof(SettingsSnapshot.CursorLeftShortcuts), s => s.CursorLeftShortcuts, (s, v) => s with { CursorLeftShortcuts = v }, SettingsCapability.EncodingCursor),
            F(nameof(SettingsSnapshot.CursorRightShortcutEnabled), s => s.CursorRightShortcutEnabled, (s, v) => s with { CursorRightShortcutEnabled = v }, SettingsCapability.EncodingCursor),
            F(nameof(SettingsSnapshot.CursorRightShortcuts), s => s.CursorRightShortcuts, (s, v) => s with { CursorRightShortcuts = v }, SettingsCapability.EncodingCursor),
            F(nameof(SettingsSnapshot.PreviousPageShortcutEnabled), s => s.PreviousPageShortcutEnabled, (s, v) => s with { PreviousPageShortcutEnabled = v }, SettingsCapability.CandidatePaging),
            F(nameof(SettingsSnapshot.PreviousPageShortcuts), s => s.PreviousPageShortcuts, (s, v) => s with { PreviousPageShortcuts = v }, SettingsCapability.CandidatePaging),
            F(nameof(SettingsSnapshot.NextPageShortcutEnabled), s => s.NextPageShortcutEnabled, (s, v) => s with { NextPageShortcutEnabled = v }, SettingsCapability.CandidatePaging),
            F(nameof(SettingsSnapshot.NextPageShortcuts), s => s.NextPageShortcuts, (s, v) => s with { NextPageShortcuts = v }, SettingsCapability.CandidatePaging),
            F(nameof(SettingsSnapshot.ExpandedPreviousPageShortcutEnabled), s => s.ExpandedPreviousPageShortcutEnabled, (s, v) => s with { ExpandedPreviousPageShortcutEnabled = v }, SettingsCapability.CandidatePaging),
            F(nameof(SettingsSnapshot.ExpandedPreviousPageShortcuts), s => s.ExpandedPreviousPageShortcuts, (s, v) => s with { ExpandedPreviousPageShortcuts = v }, SettingsCapability.CandidatePaging),
            F(nameof(SettingsSnapshot.ExpandedNextPageShortcutEnabled), s => s.ExpandedNextPageShortcutEnabled, (s, v) => s with { ExpandedNextPageShortcutEnabled = v }, SettingsCapability.CandidatePaging),
            F(nameof(SettingsSnapshot.ExpandedNextPageShortcuts), s => s.ExpandedNextPageShortcuts, (s, v) => s with { ExpandedNextPageShortcuts = v }, SettingsCapability.CandidatePaging),
            F(nameof(SettingsSnapshot.PunctuationShortcutEnabled), s => s.PunctuationShortcutEnabled, (s, v) => s with { PunctuationShortcutEnabled = v }, SettingsCapability.ChinesePunctuation),
            F(nameof(SettingsSnapshot.PunctuationShortcuts), s => s.PunctuationShortcuts, (s, v) => s with { PunctuationShortcuts = v }, SettingsCapability.ChinesePunctuation),
            F(nameof(SettingsSnapshot.TextNavigationShortcutEnabled), s => s.TextNavigationShortcutEnabled, (s, v) => s with { TextNavigationShortcutEnabled = v }, SettingsCapability.DocumentNavigation),
            F(nameof(SettingsSnapshot.TextNavigationShortcuts), s => s.TextNavigationShortcuts, (s, v) => s with { TextNavigationShortcuts = v }, SettingsCapability.DocumentNavigation),
            F(nameof(SettingsSnapshot.TextNavigationSelectEnabled), s => s.TextNavigationSelectEnabled, (s, v) => s with { TextNavigationSelectEnabled = v }, SettingsCapability.DocumentNavigation)])
    ];
}
