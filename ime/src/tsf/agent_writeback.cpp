#include "owo/tsf/agent_writeback.h"

#include <richedit.h>
#include <limits>
#include <iostream>
#include <string>
#include <vector>

namespace owo::tsf {

NativeWriteResult replace_richedit_selection(
    ITfContext* context, TfEditCookie cookie, ITfRange* expected,
    std::wstring_view original, std::wstring_view replacement) {
    const auto rejected = [](const char* reason) {
        // Reason codes only: never log document text or model output.
        std::clog << "[agent-writeback] rejected: " << reason << '\n';
        return NativeWriteResult::rejected;
    };
    if (!context || !expected) return rejected("missing_context");
    ITfContextView* view = nullptr;
    HWND window = nullptr;
    if (SUCCEEDED(context->GetActiveView(&view)) && view) {
        (void)view->GetWnd(&window);
        view->Release();
    }
    wchar_t name[64]{};
    if (!window || !GetClassNameW(window, name, static_cast<int>(std::size(name))) ||
        _wcsicmp(name, L"RICHEDIT50W") != 0)
        return NativeWriteResult::not_applicable;

    DWORD process = 0;
    const auto thread = GetWindowThreadProcessId(window, &process);
    const auto style = GetWindowLongPtrW(window, GWL_STYLE);
    if (process != GetCurrentProcessId() || thread != GetCurrentThreadId() ||
        GetFocus() != window || GetAncestor(window, GA_ROOT) != GetForegroundWindow() ||
        !IsWindowEnabled(window) || (style & (ES_READONLY | ES_PASSWORD)) != 0 ||
        replacement.empty() || replacement.find(L'\0') != std::wstring_view::npos)
        return rejected("target_window");

    ITfRangeACP* acp = nullptr;
    LONG start = 0, length = 0;
    const auto queried = expected->QueryInterface(IID_PPV_ARGS(&acp));
    const auto extent = acp ? acp->GetExtent(&start, &length) : E_NOINTERFACE;
    if (acp) acp->Release();
    if (FAILED(queried) || FAILED(extent) || start < 0 || length < 0 ||
        replacement.size() > static_cast<std::size_t>((std::numeric_limits<LONG>::max)() - start))
        return rejected("range_extent");

    CHARRANGE selected{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selected));
    if (selected.cpMin != start || selected.cpMax - selected.cpMin != length)
        return rejected("selection_extent");

    // Ctrl+A can include RichEdit's implicit final paragraph position, which
    // neither TSF GetText nor EM_GETTEXTRANGE returns as document text. Accept
    // that one position only at the independently measured end of this control.
    const auto end = static_cast<long long>(start) + length;
    if (end > (std::numeric_limits<LONG>::max)()) return rejected("range_overflow");
    GETTEXTLENGTHEX specification{GTL_PRECISE | GTL_NUMCHARS, 1200};
    const auto document_length = SendMessageW(
        window, EM_GETTEXTLENGTHEX, reinterpret_cast<WPARAM>(&specification), 0);
    if (document_length < 0 || original.size() > static_cast<std::size_t>(document_length))
        return rejected("document_extent");
    if (static_cast<std::size_t>(length) != original.size()) {
        if (original.empty() || static_cast<std::size_t>(length) != original.size() + 1 ||
            document_length < 0 || end != static_cast<long long>(document_length) + 1)
            return rejected("text_extent");
    }

    // Allocate for the entire native range plus its terminator, including the
    // possible implicit final paragraph position. Compare only returned text.
    std::vector<wchar_t> before(static_cast<std::size_t>(length) + 1);
    TEXTRANGEW range{{selected.cpMin, selected.cpMax}, before.data()};
    const auto read = SendMessageW(window, EM_GETTEXTRANGE, 0, reinterpret_cast<LPARAM>(&range));
    if (read != static_cast<LRESULT>(original.size()) ||
        std::wstring_view(before.data(), original.size()) != original)
        return rejected("selection_text");

    const auto prepared = richedit_native_text(replacement);
    if (!prepared) {
        std::clog << "[agent-writeback] rejected: unsupported_native_text\n";
        return NativeWriteResult::unsupported;
    }
    const auto& value = *prepared;
    const auto expected_length = static_cast<long long>(document_length) -
        static_cast<long long>(original.size()) + static_cast<long long>(value.size());
    if (expected_length > (std::numeric_limits<LONG>::max)())
        return rejected("document_overflow");
    SendMessageW(window, EM_STOPGROUPTYPING, 0, 0);
    // Last focus/selection check after any reentrant host processing above.
    CHARRANGE current{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&current));
    if (GetFocus() != window || current.cpMin != selected.cpMin || current.cpMax != selected.cpMax)
        return rejected("final_focus_selection");
    SendMessageW(window, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(value.c_str()));
    // EM_REPLACESEL has no return value: once issued, never retry through SetText.
    SendMessageW(window, EM_STOPGROUPTYPING, 0, 0);
    const bool verified = verify_richedit_write(
        window, start, value, static_cast<LRESULT>(expected_length));
    (void)cookie; // The caller holds the read/write cookie throughout this adapter.
    std::clog << "[agent-writeback] " << (verified ? "completed" : "uncertain") << '\n';
    return verified ? NativeWriteResult::completed : NativeWriteResult::uncertain;
}

}  // namespace owo::tsf
