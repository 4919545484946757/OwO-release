using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using System.Text;
using System.Text.Json;
using Windows.Storage.Pickers;

namespace OwO_Settings;

// Shared by card menus and detail pages; neither path bypasses confirmation,
// revision checking, private Core IPC or atomic file replacement.
internal static class DictionaryActions
{
    internal static async Task<string?> RenameAsync(XamlRoot root, string current) {
        var name = new TextBox { Text = current, MaxLength = 128, Header = "字典名称" };
        AutomationProperties.SetAutomationId(name, "DictionaryRenameInput");
        var dialog = Dialog(root, "重命名字典", name, "保存");
        AutomationProperties.SetAutomationId(dialog, "DictionaryRenameDialog");
        dialog.IsPrimaryButtonEnabled = !string.IsNullOrWhiteSpace(name.Text);
        name.TextChanged += (_, _) => dialog.IsPrimaryButtonEnabled = !string.IsNullOrWhiteSpace(name.Text) && !name.Text.Any(char.IsControl);
        return await dialog.ShowAsync() == ContentDialogResult.Primary ? name.Text.Trim() : null;
    }
    internal static async Task<bool> ConfirmDeleteAsync(XamlRoot root, string name, bool builtin = false) =>
        await Dialog(root, "删除字典", builtin
            ? $"移除“{name}”并停止使用该来源？不会擦除安装文件或个人学习数据；可通过“恢复内置词库”重新加入，恢复后默认停用。清空学习数据请进入用户词库单独确认。"
            : $"删除“{name}”及其候选来源？不会清除已学习的同名词。旧快照仍保留在本机，可用于恢复，不属于安全抹除。", "删除").ShowAsync() == ContentDialogResult.Primary;
    private static ContentDialog Dialog(XamlRoot root, string title, object content, string primary) => new() {
        XamlRoot = root, Title = title, Content = content, PrimaryButtonText = primary,
        CloseButtonText = "取消", DefaultButton = ContentDialogButton.Close
    };
    private static FileSavePicker Picker(string name) => new() {
        SuggestedFileName = string.Concat(name.Where(c => !Path.GetInvalidFileNameChars().Contains(c))) is { Length: > 0 } safe ? safe : "OwO-字典"
    };
    private static async Task WriteAsync(string path, string content) {
        var temporary = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try { await File.WriteAllTextAsync(temporary, content, new UTF8Encoding(false)); File.Move(temporary, path, true); }
        finally { if (File.Exists(temporary)) File.Delete(temporary); }
    }
    internal static async Task<bool> ExportAsync(ManagedDictionary dictionary) {
        var picker = Picker(dictionary.Name);
        picker.FileTypeChoices.Add("OwO 字典 JSON", [".json"]); picker.FileTypeChoices.Add("制表符文本 TSV", [".tsv"]);
        DictionaryManager.InitializePicker(picker); var file = await picker.PickSaveFileAsync(); if (file is null) return false;
        await WriteAsync(file.Path, DictionaryFormats.Export(dictionary, Path.GetExtension(file.Path).Equals(".tsv", StringComparison.OrdinalIgnoreCase)));
        return true;
    }
    internal static async Task<bool> ExportUserAsync(string name) {
        var picker = Picker(name);
        picker.FileTypeChoices.Add("OwO 用户词库 JSON", [".json"]); picker.FileTypeChoices.Add("制表符文本 TSV", [".tsv"]);
        DictionaryManager.InitializePicker(picker); var file = await picker.PickSaveFileAsync(); if (file is null) return false;
        var client = new UserLexiconClient();
        var words = new List<LexiconWord>(); ulong? revision = null;
        for (var offset = 0;; offset += 200) {
            var page = await client.RequestAsync("list", revision, offset: offset, limit: 200);
            revision = page.Revision; words.AddRange(page.Words);
            if (words.Count >= page.Total) break;
            if (page.Words.Count == 0) throw new IOException("导出分页异常，请刷新重试。");
        }
        var output = Path.GetExtension(file.Path).Equals(".tsv", StringComparison.OrdinalIgnoreCase)
            ? "input\ttext\tcount\n" + string.Concat(words.Select(word => $"{word.Input}\t{word.Text}\t{word.Count}\n"))
            : JsonSerializer.Serialize(new LexiconFile(1, "OwO.UserLexicon", words), new JsonSerializerOptions { WriteIndented = true });
        await WriteAsync(file.Path, output); return true;
    }
    internal static async Task<bool> ExportBaseAsync(string name) {
        var source = await new UserLexiconClient().BaseFileAsync();
        var picker = Picker(name); picker.FileTypeChoices.Add("OwO 只读词典原始文件", [".owolx"]);
        DictionaryManager.InitializePicker(picker); var file = await picker.PickSaveFileAsync(); if (file is null) return false;
        if (Path.GetFullPath(file.Path).Equals(Path.GetFullPath(source), StringComparison.OrdinalIgnoreCase))
            throw new IOException("不能覆盖正在使用的基础词典。");
        var temporary = file.Path + "." + Guid.NewGuid().ToString("N") + ".tmp";
        try {
            await using (var input = new FileStream(source, FileMode.Open, FileAccess.Read, FileShare.Read, 65536, FileOptions.Asynchronous | FileOptions.SequentialScan))
            await using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None, 65536, FileOptions.Asynchronous))
                await input.CopyToAsync(output);
            File.Move(temporary, file.Path, true);
        } finally { if (File.Exists(temporary)) File.Delete(temporary); }
        return true;
    }
}
