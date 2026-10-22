#include "text_service.h"
#include <iostream>
namespace owo::tsf {
struct ToneInputTest {
    static int run() {
        TextService service;
        int failures = 0;
        const auto expect = [&](bool ok, const char* label) {
            if (!ok) { std::cerr << label << '\n'; ++failures; }
        };
        service.tone_active_ = service.tone_editing_ = service.agent_origin_valid_ = true;
        expect(!service.tone_preview_key(VK_MENU) && !service.tone_preview_key(VK_LMENU) &&
            !service.tone_preview_key(VK_RMENU) && !service.tone_preview_key(VK_LWIN) &&
            !service.tone_preview_key(VK_RWIN), "system window-switch keys pass through");
        service.tone_edited_ = L"好的明天见";
        service.tone_edit_cursor_ = 2;
        // Own the preview with a hidden message window on an isolated desktop.
        service.message_window_ = CreateWindowExW(0, L"STATIC", L"", 0, 0, 0, 0, 0,
            HWND_MESSAGE, nullptr, GetModuleHandleW(nullptr), nullptr);
        // update_tone_preview creates a top-level window; test on an isolated
        // desktop so it never appears in the users desktop or steals focus.
        expect(service.message_window_ != nullptr, "hidden message window");
        expect(SUCCEEDED(service.append_tone_edit(L"呀")) && service.tone_edited_ == L"好的呀明天见" &&
            service.tone_edit_cursor_ == 3, "insert at preview caret");
        const auto preview = FindWindowW(L"OwO.Tone.Preview.v1", nullptr);
        expect(preview && (GetWindowLongPtrW(preview, GWL_EXSTYLE) & WS_EX_NOACTIVATE) != 0,
            "preview cannot activate original target");
        auto before = service.tone_edited_;
        expect(FAILED(service.append_tone_edit(L"\n")) && service.tone_edited_ == before,
            "multiline edit rejected");
        expect(FAILED(service.append_tone_edit(std::wstring_view(L"\0", 1))) && service.tone_edited_ == before,
            "embedded NUL edit rejected");
        expect(FAILED(service.append_tone_edit(std::wstring(1025, L'a'))) && service.tone_edited_ == before,
            "oversized edit rejected");
        service.input_buffer_ = L"hao";
        service.input_cursor_ = 3;
        service.candidates_ = {L"好"}; service.candidate_consumed_ = {3};
        service.candidate_groups_ = {0}; service.candidate_request_pending_ = false;
        // A null document would reject/crash an accidental ordinary write. This
        // path succeeds by editing only the in-memory draft, never the corpus.
        expect(SUCCEEDED(service.commit_candidate(nullptr, 0)) &&
            service.tone_edited_ == L"好的呀好明天见" && service.input_buffer_.empty(),
            "candidate goes only into preview");
        expect(service.recent_language_context_.empty() && service.feedback_requests_.empty(),
            "preview is neither committed context nor corpus feedback");
        service.agent_origin_valid_ = false;
        before = service.tone_edited_;
        expect(FAILED(service.append_tone_edit(L"错")) && service.tone_edited_ == before,
            "stale origin cannot edit preview");
        service.confirm_tone();
        expect(!service.tone_active_ && service.tone_adoption_id_ == 0,
            "invalid origin cannot confirm or learn adoption");
        expect(!service.undo_tone(), "no receipt cannot undo");
        service.Deactivate();
        return failures ? 1 : 0;
    }
};
}
int main() {
    // Isolated desktop verifies the real window style without a visible GUI.
    const auto desktop = CreateDesktopW(L"OwO-Tone-Test", nullptr, nullptr, 0,
        DESKTOP_CREATEWINDOW | DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS, nullptr);
    if (!desktop || !SetThreadDesktop(desktop)) return 2;
    const int result = owo::tsf::ToneInputTest::run();
    CloseDesktop(desktop);
    return result;
}
