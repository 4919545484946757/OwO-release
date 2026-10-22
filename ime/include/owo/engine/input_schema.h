#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {

enum class InputSchemeKind : std::uint8_t {
    full_pinyin,
    exact_code,
};

struct Syllable {
    std::string text;
    std::size_t begin{};
    std::size_t end{};
    bool complete{true};

    bool operator==(const Syllable&) const = default;
};

enum class InputMatchKind : std::uint8_t {
    exact,
    incomplete_completion,
    corrected,
    abbreviated_completion,
    // Learned continuation of the text before this composition. It is not
    // parsed from pinyin, so it always consumes the complete preedit.
    input_continuation,
};

struct ParsePath {
    std::vector<Syllable> syllables;
    InputMatchKind match_kind{InputMatchKind::exact};
    std::uint32_t edit_count{};
    std::uint32_t completion_characters{};

    bool operator==(const ParsePath&) const = default;
};

struct ParseResult {
    std::string normalized_input;
    std::vector<ParsePath> paths;
    bool valid{false};
    bool has_incomplete_syllable{false};
    InputSchemeKind scheme{InputSchemeKind::full_pinyin};
    // Candidate generation consumes capabilities rather than scheme identities.
    // A future double-pinyin scheme can opt into phonetic assistance, while a
    // table-code scheme can remain exact without adding another generator branch.
    bool phonetic_assistance{true};
};

class InputSchema {
public:
    virtual ~InputSchema() = default;
    [[nodiscard]] virtual ParseResult parse(std::string_view input,
                                            std::size_t max_paths = 32) const = 0;
};

}  // namespace owo::engine
