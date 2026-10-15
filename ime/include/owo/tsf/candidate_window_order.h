#pragma once

#include <windows.h>
#include <initializer_list>

namespace owo::tsf {

// An owned popup follows its input host, but must never activate that host.
inline void bind_candidate_owner(HWND candidate, HWND shadow, HWND host) noexcept {
    if (!host || host == candidate || host == shadow) return;
    DWORD process = 0;
    GetWindowThreadProcessId(host, &process);
    if (process != GetCurrentProcessId()) return;
    for (const HWND window : {candidate, shadow}) {
        if (window && GetWindow(window, GW_OWNER) != host) {
            SetWindowLongPtrW(window, GWLP_HWNDPARENT, reinterpret_cast<LONG_PTR>(host));
        }
    }
}

inline bool candidate_needs_raise(HWND candidate, HWND host) noexcept {
    if (!candidate || !host) return false;
    if (!(GetWindowLongPtrW(candidate, GWL_EXSTYLE) & WS_EX_TOPMOST)) return true;
    // The foreground host can reorder itself after the last rendered frame.
    for (HWND above = GetWindow(candidate, GW_HWNDPREV); above;
         above = GetWindow(above, GW_HWNDPREV)) {
        if (above == host) return true;
    }
    return false;
}

inline bool raise_candidate_windows(HWND candidate, HWND shadow) noexcept {
    constexpr UINT flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
    if (!SetWindowPos(candidate, HWND_TOPMOST, 0, 0, 0, 0, flags)) return false;
    if (shadow && IsWindowVisible(shadow)) {
        SetWindowPos(shadow, candidate, 0, 0, 0, 0, flags);
    }
    return true;
}

} // namespace owo::tsf
