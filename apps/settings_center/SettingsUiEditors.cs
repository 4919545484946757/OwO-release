using Microsoft.UI.Xaml.Controls;

namespace OwO_Settings;

// WinUI 只提供字段编辑器；模块不持有控件，也不依赖 MainPage。
internal sealed class SettingsUiEditors : ISettingsEditors
{
    private sealed record Editor(Type Type, Func<object?> Read, Action<object?> Write);
    private readonly Dictionary<string, Editor> _editors = [];
    private readonly List<Control> _controls = [];
    public IReadOnlyCollection<string> Keys => _editors.Keys;
    internal IEnumerable<Control> ObservedControls => _controls.Distinct();

    internal void Bind<T>(string key, Func<T> read, Action<T> write, params Control[] controls)
    {
        _editors.Add(key, new(typeof(T), () => read(), value => write((T)value!)));
        _controls.AddRange(controls);
    }

    public T Read<T>(string key) => _editors[key].Type == typeof(T)
        ? (T)_editors[key].Read()! : throw new InvalidOperationException($"设置字段类型不匹配：{key}");
    public void Write<T>(string key, T value)
    {
        if (_editors[key].Type != typeof(T)) throw new InvalidOperationException($"设置字段类型不匹配：{key}");
        _editors[key].Write(value);
    }
}
