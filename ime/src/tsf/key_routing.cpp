#include "owo/tsf/key_routing.h"

namespace owo::tsf {

bool is_composition_letter(const wchar_t character) noexcept {
    return (character >= L'a' && character <= L'z') ||
           (character >= L'A' && character <= L'Z');
}

DigitKeyAction route_digit_key(const DigitKeyState& state,
                               const wchar_t key) noexcept {
    if (key < L'0' || key > L'9') return DigitKeyAction::pass_through;
    // Virtual digit keys also represent symbols with Shift and application
    // shortcuts with Ctrl/Alt/Win. Only plain digits belong to this router.
    if (state.shift_down || state.command_modifier_down)
        return DigitKeyAction::pass_through;
    // `0` is not a candidate selection key, but it is still input text.
    const auto index = key == L'0' ? static_cast<std::size_t>(-1)
                                   : static_cast<std::size_t>(key - L'1');
    // A visible candidate (or an in-flight request that will answer with one)
    // owns the digit. This keeps the long-standing selection interaction
    // untouched. `0` never selects a candidate.
    if (key != L'0' &&
        (state.request_pending || index < state.candidate_count))
        return DigitKeyAction::select_candidate;
    // Any digit that does not select a candidate joins the composition when it
    // directly follows a reading character: `1-5` become tone digits in the
    // engine, while other digits make up mixed input (`ni6`, `woaini123`). A
    // digit after a digit or an apostrophe keeps the previous behaviour.
    if (state.has_composition &&
        is_composition_letter(state.previous_character))
        return DigitKeyAction::join_composition;
    return DigitKeyAction::pass_through;
}

}  // namespace owo::tsf
