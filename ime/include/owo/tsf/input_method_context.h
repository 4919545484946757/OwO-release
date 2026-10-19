#pragma once
#include <Windows.h>
#include <msctf.h>
#include <cstdint>

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
// InputScope numeric metadata only; never inspect the contents of a PIN field.
[[nodiscard]] bool sensitive_input_scope(LONG scope) noexcept;
[[nodiscard]] InputMethodContextSafety evaluate_input_method_context(const InputMethodContextFacts&) noexcept;
// Call only from a trusted TSF read/edit session with its own cookie. Reads metadata,
// never text, phrases, selection contents, clipboard or surrounding context.
[[nodiscard]] InputMethodContextSafety inspect_input_method_context(ITfContext*, TfEditCookie, InputMethodContextFacts* observed = nullptr);

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
