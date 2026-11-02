namespace OwO_Settings;

internal enum DictionaryActivationChoice { Cancel, Supplement, EnableWithoutEncoding }
internal sealed record DictionaryActivationDecision(DictionaryActivationChoice Choice, bool DoNotRemind = false);
internal sealed record DictionaryActivationResult(ManagedDictionary? Dictionary, bool PromptBeforeEnablingPending);

// No UI, persistence or input-thread work here. The caller publishes the
// accepted dictionary and preference together, using the catalog revision.
internal static class DictionaryActivation
{
    internal static async Task<DictionaryActivationResult> PrepareAsync(ManagedDictionary dictionary,
        bool wasEnabled, bool promptBeforeEnablingPending,
        Func<int, Task<DictionaryActivationDecision>> choose,
        Func<ManagedDictionary, Task<ManagedDictionary?>> supplement) {
        var pending = dictionary.Entries.Count(word => !DictionaryStore.Usable(word));
        if (!dictionary.Enabled || wasEnabled || !promptBeforeEnablingPending ||
            dictionary.Scheme != "full_pinyin" || pending == 0)
            return new(dictionary, promptBeforeEnablingPending);
        var decision = await choose(pending);
        if (decision.Choice == DictionaryActivationChoice.Cancel)
            return new(null, promptBeforeEnablingPending);
        if (decision.Choice == DictionaryActivationChoice.Supplement) {
            var prepared = await supplement(dictionary with { Enabled = false });
            if (prepared is null) return new(null, promptBeforeEnablingPending);
            if (prepared.Id != dictionary.Id || prepared.Scheme != dictionary.Scheme)
                throw new InvalidDataException("补码结果与当前字典不匹配，未启用字典。");
            dictionary = prepared with { Enabled = true };
        } else if (decision.Choice != DictionaryActivationChoice.EnableWithoutEncoding) {
            throw new InvalidDataException("未知的词库启用操作。");
        }
        return new(dictionary, !decision.DoNotRemind);
    }
}
