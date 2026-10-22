using OwO_Settings;

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
    // The shell prints style preferences with ~ escaped for the one-line
    // key=value file; the client must undo exactly that, and flatten line
    // breaks before sending a single-line field back.
    foreach (var pair in new[] {
        ("", ""), ("简短、少用感叹号", "简短、少用感叹号"),
        ("a~~b", "a~b"), ("~~~~", "~~"), ("对老师说“收到~~”", "对老师说“收到~”") }) {
        if (ConfigShellClient.UnescapeStylePreferences(pair.Item1) != pair.Item2)
            throw new Exception($"Style preference unescape failed: {pair.Item1}");
    }
    var withLineBreak = "a" + new string((char)10, 1) + "b";
    var withCarriageReturn = "a" + new string((char)13, 1) + "b";
    if (ConfigShellClient.NormalizeStylePreferences(withLineBreak) != "a b" ||
        ConfigShellClient.NormalizeStylePreferences(withCarriageReturn) != "a b")
        throw new Exception("Line breaks were not flattened into one field");
    var danglingRejected = false;
    try { ConfigShellClient.UnescapeStylePreferences("bad~escape"); }
    catch (FormatException) { danglingRejected = true; }
    if (!danglingRejected) throw new Exception("Dangling escape was silently accepted");
    // The feedback page formats the shell's summary and list. Empty input must
    // read as "no records", counts and scenes must survive, and a record row
    // must carry only its identifying fields.
    if (ToneFeedbackText.DescribeSummary("") != ToneFeedbackText.Empty)
        throw new Exception("Empty feedback summary was not described as empty");
    var summary = "records=3\nadopted=1\nedited=1\nrejected=1\nundone=0\n"
        + "scene=chat\t2\t2\ton\nscene=work\t1\t1\ton\nscene=general\t0\t0\toff\n";
    var described = ToneFeedbackText.DescribeSummary(summary);
    if (!described.Contains("共 3 条", StringComparison.Ordinal) ||
        !described.Contains("采纳 1", StringComparison.Ordinal) ||
        !described.Contains("拒绝 1", StringComparison.Ordinal) ||
        !described.Contains("聊天 2 项", StringComparison.Ordinal) ||
        !described.Contains("其他（已关闭）", StringComparison.Ordinal))
        throw new Exception($"Feedback summary was not described: {described}");
    var rows = ToneFeedbackText.ParseRows(
        "entries=2\n7\t1700000000000\tedited\tWeChat.exe\t草稿\t提交\n"
        + "8\t1700000000001\trejected\t需要\\t转义\\\\的应用\t草稿\t\n");
    if (rows.Count != 2 || rows[0].Id != 7 || rows[0].Kind != "修改" ||
        rows[0].Application != "WeChat.exe" || rows[0].Detail.Contains("草稿") ||
        rows[1].Kind != "拒绝" || rows[1].Application != "需要\t转义\\的应用")
        throw new Exception("Feedback rows were not parsed correctly");
    if (ToneFeedbackText.ParseRows("entries=0\n").Count != 0)
        throw new Exception("Empty feedback list produced rows");
    var metadata = ToneFeedbackText.ParseRows("9\t18446744073709551615\tundone\tWeChat.exe\n");
    if (metadata.Count != 1 || metadata[0].Created != "时间未知" || metadata[0].Id != 9)
        throw new Exception("Invalid timestamp broke the metadata list");
    Console.WriteLine("Settings UI preference tests passed.");
} finally {
    if (Directory.Exists(root)) Directory.Delete(root, recursive: true);
}
