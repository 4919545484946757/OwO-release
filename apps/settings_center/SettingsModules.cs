namespace OwO_Settings;

internal interface ISettingsEditors
{
    IReadOnlyCollection<string> Keys { get; }
    T Read<T>(string key);
    void Write<T>(string key, T value);
}

internal abstract record SettingsField(string Key, SettingsCapability Capability)
{
    internal abstract void Load(SettingsSnapshot value, ISettingsEditors editors);
    internal abstract SettingsSnapshot Capture(SettingsSnapshot value, ISettingsEditors editors);
}

internal sealed record SettingsField<T>(string Name, SettingsCapability Requirement,
    Func<SettingsSnapshot, T> Get, Func<SettingsSnapshot, T, SettingsSnapshot> Set)
    : SettingsField(Name, Requirement)
{
    internal override void Load(SettingsSnapshot value, ISettingsEditors editors) => editors.Write(Key, Get(value));
    internal override SettingsSnapshot Capture(SettingsSnapshot value, ISettingsEditors editors) =>
        Set(value, editors.Read<T>(Key));
}

internal sealed record SettingsModule(string Id, string Page, IReadOnlyList<SettingsField> Fields);

// 独立于 WinUI 的字段所有权与读写管线；旧 SettingsSnapshot/原生配置格式仅作兼容边界。
internal sealed class SettingsModules
{
    internal IReadOnlyList<SettingsModule> Modules { get; }
    private readonly Dictionary<string, SettingsField> _fields;

    internal SettingsModules()
    {
        Modules = SettingsModuleDefinitions.Create();
        _fields = Modules.SelectMany(module => module.Fields).ToDictionary(field => field.Key);
        var properties = typeof(SettingsSnapshot).GetProperties().Select(property => property.Name);
        if (!_fields.Keys.ToHashSet().SetEquals(properties))
            throw new InvalidOperationException("配置模块未完整覆盖设置快照，或包含未知字段。");
    }

    internal SettingsField Field(string key) => _fields[key];
    internal string PageFor(string key) => Modules.Single(module => module.Fields.Any(f => f.Key == key)).Page;

    internal void ValidateEditors(ISettingsEditors editors)
    {
        if (!_fields.Keys.ToHashSet().SetEquals(editors.Keys))
            throw new InvalidOperationException("配置模块与界面绑定不匹配。");
    }

    internal void Load(SettingsSnapshot value, ISettingsEditors editors)
    {
        ValidateEditors(editors);
        foreach (var field in _fields.Values) field.Load(value, editors);
    }

    internal SettingsSnapshot Capture(SettingsSnapshot baseline, ISettingsEditors editors)
    {
        ValidateEditors(editors);
        return _fields.Values.Aggregate(baseline, (value, field) => field.Capture(value, editors));
    }

    internal void ResetPage(string page, SettingsSnapshot defaults, ISettingsEditors editors,
                            InputSchemeOption scheme)
    {
        foreach (var field in Modules.Where(module => module.Page == page).SelectMany(module => module.Fields))
            if (scheme.Supports(field.Capability)) field.Load(defaults, editors);
    }

    internal static uint UnsignedNumber(double value, uint minimum, uint maximum, string name)
    {
        if (!double.IsFinite(value) || value != Math.Truncate(value) || value < minimum || value > maximum)
            throw new InvalidOperationException($"{name}须为 {minimum}–{maximum} 范围内的整数。");
        return (uint)value;
    }
}
