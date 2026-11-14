using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;

namespace OwO_Settings;

internal static class DictionaryEnableDialog
{
    internal static async Task<(DictionaryActivationResult Activation, bool Persisted)> PrepareAsync(XamlRoot root, ManagedDictionary dictionary,
        bool wasEnabled, bool promptBeforeEnablingPending, Func<ManagedDictionary, bool, Task> persist) {
        var keepReminder = promptBeforeEnablingPending;
        var persisted = false;
        var activation = await DictionaryActivation.PrepareAsync(
            dictionary, wasEnabled, promptBeforeEnablingPending, async pending => {
                var suppress = new CheckBox { Content = "不再提醒（所有字典）" };
                AutomationProperties.SetAutomationId(suppress, "DictionaryEnableDoNotRemind");
                var body = new StackPanel { Spacing = 12, MaxWidth = 520 };
                body.Children.Add(new TextBlock { Text = $"“{dictionary.Name}”有 {pending} 条词条尚未补全拼音编码，是否先自动补码？\n\n自动补码会打开预览，能够确定读音的项目会自动选中，多音或未知项目默认跳过，保存后才启用。直接启用时仅已有编码的词条参与候选，其余仍保留待补状态。", TextWrapping = TextWrapping.Wrap });
                body.Children.Add(suppress);
                var dialog = new ContentDialog { XamlRoot = root, Title = "是否自动补全词库编码？", Content = body,
                    PrimaryButtonText = "自动补码", SecondaryButtonText = "直接启用", CloseButtonText = "取消启用",
                    DefaultButton = ContentDialogButton.Close };
                AutomationProperties.SetAutomationId(dialog, "DictionaryEnablePrompt");
                var choice = await dialog.ShowAsync();
                keepReminder = suppress.IsChecked != true;
                return new DictionaryActivationDecision(choice switch {
                    ContentDialogResult.Primary => DictionaryActivationChoice.Supplement,
                    ContentDialogResult.Secondary => DictionaryActivationChoice.EnableWithoutEncoding,
                    _ => DictionaryActivationChoice.Cancel
                }, suppress.IsChecked == true);
            }, async staged => (await DictionaryEncodingDialog.SupplementAsync(root, staged, autoStart: true, persist: async result => {
                await persist(result.Dictionary with { Enabled = true }, keepReminder);
                persisted = true;
            }))?.Dictionary);
        return (activation, persisted);
    }
}
