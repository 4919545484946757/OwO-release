#pragma once

#include <cstddef>
#include <cstdint>

namespace owo::tsf {

// What a digit key does while the Chinese composition is active. Candidate
// selection always wins; the remaining question is whether a digit the user
// pressed should join the composition (tone digit or mixed input) or be left to
// the host application.
enum class DigitKeyAction : std::uint8_t {
    select_candidate,  // the digit maps to a visible candidate, or a request is pending
    join_composition,  // tone digit / mixed input; appended at the pinyin cursor
    pass_through,      // not eaten; the host receives the key
};

struct DigitKeyState {
    bool request_pending{};
    bool has_composition{};
    std::size_t candidate_count{};
    // Wide character immediately before the pinyin cursor (0 at the start).
    wchar_t previous_character{};
};

// The composition alphabet is ASCII letters; everything else ends a reading.
[[nodiscard]] bool is_composition_letter(wchar_t character) noexcept;

// Routing rule: `1-9` selects a candidate when one exists (or a request is
// pending, which defers the selection). Otherwise `1-5` directly after a
// composition letter joins the composition, so tone digits (`ni3hao3`) and
// mixed input stay inside the preedit instead of leaking to the document.
[[nodiscard]] DigitKeyAction route_digit_key(const DigitKeyState& state,
                                             wchar_t key) noexcept;

}  // namespace owo::tsf
