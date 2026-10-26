using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using System.Runtime.InteropServices;
using Windows.System;

namespace OwO_Settings;

public sealed partial class MainPage
{
    private void ShortcutButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button button) return;
        _shortcutCaptureTarget = button;
        StartShortcutCaptureHook();
        button.Content = "请按新的快捷键…";
        button.Focus(Microsoft.UI.Xaml.FocusState.Programmatic);
    }

    private void Page_KeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (_shortcutCaptureTarget is null) return;
        if (IsModifierKey(e.Key)) {
            var modifiers = ShortcutForKeyEvent(e.Key);
            _shortcutCaptureTarget.Content = string.IsNullOrEmpty(modifiers)
                ? "请按新的快捷键…" : $"{modifiers}+…";
            e.Handled = true;
            return;
        }
        var shortcut = ShortcutForKeyEvent(e.Key);
        if (shortcut is null) {
            ShowStatus("该按键暂不支持，请使用字母、数字、功能键、导航键或常用符号键。",
                       InfoBarSeverity.Warning);
            e.Handled = true;
            return;
        }
        CommitCapturedShortcut(shortcut);
        e.Handled = true;
    }

    private void Page_KeyUp(object sender, KeyRoutedEventArgs e)
    {
        if (_shortcutCaptureTarget is null || !IsModifierKey(e.Key)) return;
        var shortcut = ShortcutForKeyEvent(e.Key);
        if (shortcut is not null &&
            (shortcut == "Alt" || shortcut.Count(character => character == '+') >= 1)) {
            CommitCapturedShortcut(shortcut);
        } else {
            _shortcutCaptureTarget.Content = "请继续按主键…";
            ShowStatus("Ctrl、Shift 或多个修饰键不能单独作为快捷键，请继续按一个主键。",
                       InfoBarSeverity.Warning);
        }
        e.Handled = true;
    }

    private static bool IsModifierKey(VirtualKey key) =>
        (int)key is 0x10 or 0x11 or 0x12 or 0xA0 or 0xA1 or 0xA2 or 0xA3 or 0xA4 or 0xA5;

    private void CommitCapturedShortcut(string shortcut)
    {
        if (_shortcutCaptureTarget is null) return;
        if (_shortcutCaptureTarget.Tag is ShortcutBinding binding) binding.Value = shortcut;
        _shortcutCaptureTarget = null;
        StopShortcutCaptureHook();
        UpdateShortcutButtons();
        ShowStatus($"快捷键已改为 {shortcut}，点击“保存”后生效。", InfoBarSeverity.Informational);
    }

    private static bool IsDown(int key) => (GetKeyState(key) & 0x8000) != 0;

    private static string? ShortcutForKeyEvent(VirtualKey key)
    {
        const int shiftKey = 0x10;
        const int controlKey = 0x11;
        const int altKey = 0x12;
        var code = (int)key;
        var control = code is 0x11 or 0xA2 or 0xA3 || IsDown(controlKey);
        var alt = code is 0x12 or 0xA4 or 0xA5 || IsDown(altKey);
        var shift = code is 0x10 or 0xA0 or 0xA1 || IsDown(shiftKey);
        return ShortcutForKeyCode(code, control, alt, shift);
    }

    private static string? ShortcutForKeyCode(int code, bool control, bool alt, bool shift)
    {
        var parts = new List<string>();
        if (control) parts.Add("Ctrl");
        if (alt) parts.Add("Alt");
        if (shift) parts.Add("Shift");
        if (code is not (0x10 or 0x11 or 0x12 or 0xA0 or 0xA1 or 0xA2 or 0xA3 or 0xA4 or 0xA5)) {
            var primary = PrimaryKeyName(code);
            if (primary is null) return null;
            parts.Add(primary);
        }
        return parts.Count == 0 ? null : string.Join('+', parts);
    }

    private void StartShortcutCaptureHook()
    {
        StopShortcutCaptureHook();
        // A managed callback is represented by a runtime thunk rather than code in the
        // WinUI executable image. Passing the EXE module handle makes Windows reject the
        // low-level hook on some systems; a null module is correct for this in-process hook.
        _shortcutCaptureHook = SetWindowsHookEx(13, _shortcutCaptureHookProc,
                                                IntPtr.Zero, 0);
        if (_shortcutCaptureHook == IntPtr.Zero)
            ShowStatus($"无法启动快捷键捕获（Win32 {Marshal.GetLastWin32Error()}）。",
                       InfoBarSeverity.Error);
    }

    private void StopShortcutCaptureHook()
    {
        if (_shortcutCaptureHook == IntPtr.Zero) return;
        UnhookWindowsHookEx(_shortcutCaptureHook);
        _shortcutCaptureHook = IntPtr.Zero;
    }

    private IntPtr ShortcutCaptureHook(int code, IntPtr message, IntPtr data)
    {
        const int space = 0x20;
        const int keyDown = 0x0100;
        const int systemKeyDown = 0x0104;
        if (code >= 0 && _shortcutCaptureTarget is not null &&
            ((int)message == keyDown || (int)message == systemKeyDown) &&
            Marshal.ReadInt32(data) == space) {
            var shortcut = ShortcutForKeyCode(space,
                (GetAsyncKeyState(0x11) & 0x8000) != 0,
                (GetAsyncKeyState(0x12) & 0x8000) != 0,
                (GetAsyncKeyState(0x10) & 0x8000) != 0);
            if (shortcut is not null)
                DispatcherQueue.TryEnqueue(() => CommitCapturedShortcut(shortcut));
            return new IntPtr(1);
        }
        return CallNextHookEx(_shortcutCaptureHook, code, message, data);
    }

    private static string? PrimaryKeyName(int key)
    {
        if (key is >= 0x41 and <= 0x5A || key is >= 0x30 and <= 0x39)
            return ((char)key).ToString();
        if (key is >= 0x70 and <= 0x87) return $"F{key - 0x6F}";
        return key switch {
            0x20 => "Space", 0x0D => "Enter", 0x09 => "Tab", 0x1B => "Escape",
            0x08 => "Backspace", 0x2E => "Delete", 0x2D => "Insert",
            0x24 => "Home", 0x23 => "End", 0x21 => "PageUp", 0x22 => "PageDown",
            0x25 => "Left", 0x27 => "Right", 0x26 => "Up", 0x28 => "Down",
            0xDB => "[", 0xDD => "]", 0xBD => "Minus", 0xBB => "Plus",
            0xBC => "Comma", 0xBE => "Period", 0xBF => "Slash",
            0xBA => "Semicolon", 0xDE => "Quote", 0xC0 => "Backtick", _ => null,
        };
    }

    private void SetShortcutBindings(string group, IEnumerable<string> values)
    {
        var items = values.Where(value => !string.IsNullOrWhiteSpace(value))
            .Select(value => new ShortcutBinding(group, value)).ToList();
        _shortcutBindings[group] = items;
    }

    private IReadOnlyList<string> ShortcutValues(string group) =>
        _shortcutBindings[group].Select(binding => binding.Value).ToArray();

    private StackPanel ShortcutPanel(string group) => group switch {
        "correction" => CorrectionShortcutBindings,
        "language" => LanguageShortcutBindings,
        "raw" => RawInputShortcutBindings,
        "cursor-left" => CursorLeftShortcutBindings,
        "cursor-right" => CursorRightShortcutBindings,
        "previous-page" => PreviousPageShortcutBindings,
        "next-page" => NextPageShortcutBindings,
        "expanded-previous-page" => ExpandedPreviousPageShortcutBindings,
        "expanded-next-page" => ExpandedNextPageShortcutBindings,
        "text-navigation" => TextNavigationShortcutBindings,
        "punctuation" => PunctuationShortcutBindings,
        _ => throw new ArgumentOutOfRangeException(nameof(group)),
    };

    private void UpdateShortcutButtons()
    {
        QueueSettingsAutoSave();
        foreach (var (group, bindings) in _shortcutBindings) {
            var panel = ShortcutPanel(group);
            panel.Children.Clear();
            foreach (var binding in bindings) {
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                var capture = new Button {
                    Content = binding.Value, Tag = binding,
                    HorizontalAlignment = Microsoft.UI.Xaml.HorizontalAlignment.Left,
                };
                capture.Click += ShortcutButton_Click;
                var remove = new Button { Content = "−", Tag = binding };
                remove.Click += RemoveShortcutButton_Click;
                row.Children.Add(capture);
                row.Children.Add(remove);
                panel.Children.Add(row);
            }
        }
        UpdateDependentSettings();
    }

    private void AddShortcutButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: string group }) return;
        var bindings = _shortcutBindings[group];
        if (bindings.Count >= 16) {
            ShowStatus("每项快捷键最多可添加 16 种方案。", InfoBarSeverity.Warning);
            return;
        }
        var binding = new ShortcutBinding(group,
            bindings.Count == 0 ? "Ctrl+Period" : bindings[^1].Value);
        bindings.Add(binding);
        UpdateShortcutButtons();
        var panel = ShortcutPanel(group);
        var row = (StackPanel)panel.Children[panel.Children.Count - 1];
        ShortcutButton_Click(row.Children[0], e);
    }

    private void RemoveShortcutButton_Click(object sender, Microsoft.UI.Xaml.RoutedEventArgs e)
    {
        if (sender is not Button { Tag: ShortcutBinding binding }) return;
        var bindings = _shortcutBindings[binding.Group];
        if (bindings.Count == 1 && binding.Group != "punctuation") {
            ShowStatus("每项快捷键至少保留一种方案；如不需要可关闭该快捷键项。",
                       InfoBarSeverity.Warning);
            return;
        }
        if (_shortcutCaptureTarget?.Tag == binding) {
            _shortcutCaptureTarget = null;
            StopShortcutCaptureHook();
        }
        bindings.Remove(binding);
        UpdateShortcutButtons();
    }
}
