#pragma once

#include "owo/engine/input_schema.h"

#include <algorithm>
#include <string_view>

namespace owo::engine {

// Phonetic paths describe one output character per source syllable. A
// completion may contain more characters, but selecting it must not turn its
// short source into a new exact user-word spelling. Table-code schemes do not
// have this constraint (one code can map to a whole phrase).
[[nodiscard]] inline bool user_word_matches_source_path(
    const ParseResult& parsed, const ParsePath& path, const std::string_view text) {
    if (text.empty() || path.syllables.empty()) return false;
    if (!parsed.phonetic_assistance) return true;
    const auto characters = static_cast<std::size_t>(std::count_if(
        text.begin(), text.end(), [](const unsigned char value) {
            return (value & 0xc0U) != 0x80U;
        }));
    return characters == path.syllables.size();
}

}  // namespace owo::engine
