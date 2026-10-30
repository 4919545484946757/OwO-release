#pragma once

#include "owo/engine/input_schema.h"

#include <algorithm>
#include <string_view>
#include <optional>

namespace owo::engine {

// Match every known syllable to a nonempty prefix in the current input.
// No skipped syllables, arbitrary subsequences, or crossing manual breaks.
[[nodiscard]] inline std::optional<std::vector<std::string>> abbreviated_user_reading(
    const ParsePath& reading, const std::string_view input) {
    const auto columns = input.size() + 1;
    std::vector<int> choices((reading.syllables.size() + 1) * columns, -1);
    const auto match = [&](auto&& self, const std::size_t syllable, const std::size_t offset) -> bool {
        if (syllable == reading.syllables.size()) return offset == input.size();
        auto& choice = choices[syllable * columns + offset];
        if (choice != -1) return choice > 0;
        choice = 0;
        const auto& text = reading.syllables[syllable].text;
        for (auto length = std::min(text.size(), input.size() - offset); length > 0; --length) {
            if (input.substr(offset, length) != std::string_view(text).substr(0, length)) continue;
            auto next = offset + length;
            if (next < input.size() && input[next] == '\'') ++next;
            if (self(self, syllable + 1, next)) { choice = static_cast<int>(length); return true; }
        }
        return false;
    };
    if (!match(match, 0, 0)) return {};
    std::vector<std::string> segments;
    std::size_t offset = 0;
    for (std::size_t syllable = 0; syllable < reading.syllables.size(); ++syllable) {
        const auto length = static_cast<std::size_t>(choices[syllable * columns + offset]);
        segments.emplace_back(input.substr(offset, length));
        offset += length;
        if (offset < input.size() && input[offset] == '\'') ++offset;
    }
    return segments;
}

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
