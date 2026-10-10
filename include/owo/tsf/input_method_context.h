#pragma once
#include <Windows.h>
#include <msctf.h>

namespace owo::tsf {
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
} // namespace owo::tsf
