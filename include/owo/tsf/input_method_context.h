#pragma once
#include <Windows.h>
#include <msctf.h>
#include <cstdint>
#include "owo/compatibility/input_context.h"

namespace owo::tsf {
[[nodiscard]] inline bool native_scope_fallback_allowed(HRESULT result, VARTYPE type) noexcept {
    return type==VT_EMPTY && (result==S_OK || result==E_FAIL || result==E_NOTIMPL);
}
enum class InputMethodContextSafety { unknown, disabled, read_only, sensitive, allowed };
struct InputMethodContextFacts {
    bool status_known{};
    bool read_only{};
    bool loading{};
    bool compartments_known{};
    bool disabled{};
    bool empty_context{};
    bool scope_known{};
    bool sensitive_scope{};
};
[[nodiscard]] InputMethodContextSafety evaluate_input_method_context(const InputMethodContextFacts&) noexcept;
// Call only from a trusted TSF read/edit session with its own cookie. Reads metadata,
// never text, phrases, selection contents, clipboard or surrounding context.
[[nodiscard]] InputMethodContextSafety inspect_input_method_context(ITfContext*, TfEditCookie);
struct InputContextResolution {
    compatibility::Resolution access;
    InputMethodContextSafety safety{InputMethodContextSafety::unknown};
    HWND view{}, focus{};
    DWORD process{}, thread{};
    bool range_known{}, native_scope{};
    bool pending_verification{};
    HRESULT scope_result{E_PENDING};
};
// Returned handles/identity describe this edit session only; re-resolve before
// writing. No range COM pointer or application text escapes the read lock.
[[nodiscard]] InputContextResolution resolve_input_context(ITfContext*, TfEditCookie);
// Independent OS evidence for native Edit/RichEdit hosts that do not expose InputScope.
[[nodiscard]] InputMethodContextSafety inspect_native_edit_scope(HWND context_view, HWND focused_window);

struct InputMethodWriteFacts {
    std::uint64_t expected_generation{}, current_generation{};
    std::uint64_t expected_deadline{}, current_deadline{}, now{};
    bool scheme_selected{}, write_pending{}, same_context{}, same_selection{};
    InputMethodContextSafety safety{InputMethodContextSafety::unknown};
};
[[nodiscard]] inline bool input_method_write_allowed(const InputMethodWriteFacts& f) noexcept {
    return f.expected_generation && f.expected_generation == f.current_generation &&
        f.expected_deadline && f.expected_deadline == f.current_deadline && f.now < f.expected_deadline &&
        f.scheme_selected && f.write_pending && f.same_context && f.same_selection &&
        f.safety == InputMethodContextSafety::allowed;
}
} // namespace owo::tsf
