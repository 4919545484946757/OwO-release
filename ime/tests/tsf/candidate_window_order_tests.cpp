#include "owo/tsf/candidate_window_order.h"
#include "owo/tsf/candidate_anchor.h"
#include <iostream>

int main() {
    const HWND foreground = GetForegroundWindow();
    const auto create = [] {
        return CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                              L"STATIC", L"OwO Z-order test", WS_POPUP,
                              -30000, -30000, 1, 1, nullptr, nullptr,
                              GetModuleHandleW(nullptr), nullptr);
    };
    const HWND host = create();
    const HWND candidate = create();
    const HWND shadow = create();
    // WPS returned this S_OK zero-height rectangle; do not anchor to it.
    bool passed = !owo::tsf::candidate_text_extent_usable(RECT{2559,1503,2560,1503}) &&
        !owo::tsf::candidate_text_extent_usable(RECT{0,0,0,0}) &&
        !owo::tsf::candidate_text_extent_usable(RECT{12,10,11,30}) &&
        !owo::tsf::candidate_text_extent_usable(RECT{10,30,12,10}) &&
        owo::tsf::candidate_text_extent_usable(RECT{-100,-200,-100,-180}) &&
        owo::tsf::candidate_text_extent_usable(RECT{100,200,120,220}) &&
        host && candidate && shadow;
    if (passed) {
        const HWND editor = CreateWindowExW(0, L"STATIC", L"", WS_CHILD,
            0, 0, 1, 1, host, nullptr, GetModuleHandleW(nullptr), nullptr);
        const HWND unrelated = create();
        const DWORD process = GetCurrentProcessId();
        passed = editor && unrelated &&
            owo::tsf::candidate_host_contains_process_focus(host, editor, process) &&
            !owo::tsf::candidate_host_contains_process_focus(unrelated, editor, process) &&
            !owo::tsf::candidate_host_contains_process_focus(host, candidate, process) &&
            !owo::tsf::candidate_host_contains_process_focus(host, editor, process + 1) &&
            !owo::tsf::candidate_host_contains_process_focus(host, nullptr, process) &&
            !owo::tsf::candidate_host_contains_process_focus(nullptr, editor, process) &&
            !owo::tsf::candidate_host_has_process_focus(nullptr) &&
            owo::tsf::candidate_host_has_process_focus(host);
        if (unrelated) DestroyWindow(unrelated);
        if (editor) DestroyWindow(editor);
        ShowWindow(host, SW_SHOWNOACTIVATE);
        ShowWindow(candidate, SW_SHOWNOACTIVATE);
        ShowWindow(shadow, SW_SHOWNOACTIVATE);
        // Reproduce a host reordering after the candidate's last frame.
        SetWindowPos(host, HWND_TOPMOST, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        passed = owo::tsf::candidate_needs_raise(candidate, host) && passed;
        passed = owo::tsf::raise_candidate_windows(candidate, shadow) && passed;
        passed = !owo::tsf::candidate_needs_raise(candidate, host) && passed;
        passed = GetWindow(shadow, GW_HWNDPREV) == candidate && passed;
        owo::tsf::bind_candidate_owner(candidate, shadow, host);
        passed = GetWindow(candidate, GW_OWNER) == host &&
                 GetWindow(shadow, GW_OWNER) == host && passed;
        // A demoted surface must return to the topmost band without activation.
        SetWindowPos(candidate, HWND_NOTOPMOST, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        passed = owo::tsf::candidate_needs_raise(candidate, host) && passed;
        passed = owo::tsf::raise_candidate_windows(candidate, shadow) && passed;
        passed = !owo::tsf::candidate_needs_raise(candidate, host) && passed;
        passed = GetForegroundWindow() == foreground && passed;
    }
    if (candidate) DestroyWindow(candidate);
    if (shadow) DestroyWindow(shadow);
    if (host) DestroyWindow(host);
    std::cout << (passed ? "Candidate Z-order checks passed\n" : "Candidate Z-order checks failed\n");
    return passed ? 0 : 1;
}
