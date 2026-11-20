namespace OwO_Settings;

// 这是宿主已接线的设置能力，不是插件权限，也不能由插件自行声明来提权。
[Flags]
internal enum SettingsCapability
{
    None = 0,
    CandidateDisplay = 1 << 0,
    EncodingCursor = 1 << 1,
    CandidatePaging = 1 << 2,
    RawCommit = 1 << 3,
    LanguageSwitch = 1 << 4,
    AltCodePreview = 1 << 5,
    CandidatePartition = 1 << 6,
    StagedCommit = 1 << 7,
    FocusLossCommit = 1 << 8,
    ChinesePunctuation = 1 << 9,
    DocumentNavigation = 1 << 10,
    UserLearning = 1 << 11,
    ModelRanking = 1 << 12,
    Correction = 1 << 13,
    UserCompletion = 1 << 14,
    SingleCharacterFirst = 1 << 15
}

internal sealed record InputSchemeOption(string Id, string Name, string Description,
    string Capabilities, SettingsCapability SettingsCapabilities, string? OwnerPluginId = null)
{
    internal bool Supports(SettingsCapability capability) =>
        (SettingsCapabilities & capability) == capability;
}

internal static class InputSchemeSettingsPolicy
{
    private const SettingsCapability Shared = SettingsCapability.CandidateDisplay |
        SettingsCapability.EncodingCursor | SettingsCapability.CandidatePaging |
        SettingsCapability.RawCommit | SettingsCapability.LanguageSwitch | SettingsCapability.AltCodePreview;
    private const SettingsCapability BuiltIn = Shared | SettingsCapability.CandidatePartition |
        SettingsCapability.StagedCommit | SettingsCapability.FocusLossCommit |
        SettingsCapability.ChinesePunctuation | SettingsCapability.DocumentNavigation |
        SettingsCapability.UserLearning | SettingsCapability.ModelRanking;
    internal static readonly InputSchemeOption[] BuiltIns = [
        new("full_pinyin", "全拼", "使用汉语拼音连续输入，适合日常中文输入。",
            "", // 默认方案不显示常驻能力说明；能力标记不变。
            BuiltIn | SettingsCapability.Correction | SettingsCapability.UserCompletion |
                SettingsCapability.SingleCharacterFirst),
        new("exact_code", "精确编码（实验性）", "按显式撇号分段并精确查询词典，用于验证表码方案接入。",
            "不启用拼音纠错、自动补全或缩写展开；当前仍使用拼音词典数据。", BuiltIn)
    ];

    internal static InputSchemeOption Plugin(string id, string name, string description) =>
        new(id, name, description,
            "使用插件自身的编码与候选规则；宿主提供候选显示、编码光标、分页、原样输出和中英文切换。" +
            "宿主拼音补全、纠错、词频学习和模型排序不参与。", Shared, Owner(id));

    internal static InputSchemeOption Resolve(string id) => BuiltIns.FirstOrDefault(s => s.Id == id)
        ?? (id.StartsWith("plugin:", StringComparison.Ordinal)
            ? Plugin(id, id[7..], "已保存的插件方案；请核对插件安装、授权和可用状态。")
            : new(id, "未知输入方案", "当前设置中心尚未适配此方案，不启用未确认的方案能力。",
                "仅可管理通用配置；请更新设置中心或选择已支持的方案。", SettingsCapability.None));

    private static string? Owner(string id)
    {
        var separator = id.LastIndexOf('/');
        return id.StartsWith("plugin:", StringComparison.Ordinal) && separator > 7
            ? id[7..separator] : null;
    }
}
