#pragma once

#include <cstddef>
#include <cstdint>

namespace owo::tsf {

// What a digit key does while the Chinese composition is active. Candidate
// selection wins for unmodified digits; other digits either join the
// composition (tone digit or mixed input) or go to the host application.
enum class DigitKeyAction : std::uint8_t {
    select_candidate,  // the digit maps to a visible candidate, or a request is pending
    join_composition,  // tone digit / mixed input; appended at the pinyin cursor
    pass_through,      // not eaten; the host receives the key
};

struct DigitKeyState {
    bool request_pending{};
    bool has_composition{};
    bool shift_down{};
    // Ctrl, Alt or either Windows key: leave application shortcuts alone.
    bool command_modifier_down{};
    std::size_t candidate_count{};
    // Wide character immediately before the pinyin cursor (0 at the start).
    wchar_t previous_character{};
};

// The composition alphabet is ASCII letters; everything else ends a reading.
[[nodiscard]] bool is_composition_letter(wchar_t character) noexcept;

// Modified digit keys pass through to punctuation handling or the host.
// Routing rule: plain `1-9` selects a candidate when one exists (or a request is
// pending, which defers the selection). Otherwise `0-9` directly after a
// composition letter joins the composition, so tone digits (`ni3hao3`) and
// mixed input stay inside the preedit instead of leaking to the document.
[[nodiscard]] DigitKeyAction route_digit_key(const DigitKeyState& state,
                                             wchar_t key) noexcept;

}  // namespace owo::tsf
