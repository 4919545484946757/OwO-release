#pragma once
#include <Windows.h>

namespace owo::compatibility {
enum class Decision { unknown, denied, allowed };
enum class Reason { missing_metadata, focus_mismatch, unsupported_control, password, read_only, disabled, verified };
struct Resolution { Decision decision{Decision::unknown}; Reason reason{Reason::missing_metadata}; };
struct EditEvidence {
    bool identity_known{}, focus_matches{}, editable{}, properties_known{}, enabled{}, password{}, read_only{};
};
[[nodiscard]] constexpr Resolution evaluate_edit(const EditEvidence& e) noexcept {
    if (!e.identity_known) return {};
    if (!e.focus_matches) return {Decision::denied, Reason::focus_mismatch};
    if (!e.editable) return {Decision::unknown, Reason::unsupported_control};
    if (!e.properties_known) return {};
    if (e.password) return {Decision::denied, Reason::password};
    if (!e.enabled) return {Decision::denied, Reason::disabled};
    if (e.read_only) return {Decision::denied, Reason::read_only};
    return {Decision::allowed, Reason::verified};
}
// Metadata only. The caller must obtain focus from GetGUIThreadInfo and recheck
// it before using the proof. No text, password contents, or user app names.
[[nodiscard]] inline Resolution inspect_native_edit(HWND view, HWND focus, DWORD pid, DWORD thread) noexcept {
    if (!view || !focus || !IsWindow(view) || !IsWindow(focus)) return {};
    if (view != focus && !IsChild(view, focus)) return {Decision::denied, Reason::focus_mismatch};
    DWORD actual{};
    if (!pid || !thread || !GetWindowThreadProcessId(view, &actual) || actual != pid ||
        GetWindowThreadProcessId(focus, &actual) != thread || actual != pid) return {};
    wchar_t name[128]{};
    // RealGetWindowClass preserves the OS control identity through standard
    // superclassing (including WinForms), without framework/app allowlists.
    if (!RealGetWindowClassW(focus, name, 128)) return {};
    const bool edit = _wcsicmp(name, L"Edit") == 0 || _wcsnicmp(name, L"RichEdit", 8) == 0;
    bool enabled = true;
    // IsWindowEnabled does not include disabled ancestors. Bound traversal even
    // if a host changes its hierarchy while collecting evidence.
    HWND current = focus;
    for (unsigned n = 0; current && n < 128; ++n) {
        if (!IsWindowEnabled(current)) enabled = false;
        if (current == view) break;
        current = GetParent(current);
    }
    if (current != view) return {};
    const auto style = GetWindowLongPtrW(focus, GWL_STYLE);
    return evaluate_edit({true, true, edit, true, enabled, (style & ES_PASSWORD) != 0, (style & ES_READONLY) != 0});
}
} // namespace owo::compatibility
