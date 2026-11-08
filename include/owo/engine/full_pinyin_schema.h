#pragma once

#include "owo/engine/input_schema.h"

#include <functional>

namespace owo::engine {

struct FullPinyinParseMetrics {
    std::uint64_t normalization_us{};
    std::uint64_t segmentation_us{};
    std::uint64_t correction_us{};
    std::uint64_t assistance_us{};
};

struct FullPinyinIncrementalState {
    std::string input;
    ParseResult result;
    std::size_t max_paths{};
};

class FullPinyinSchema final : public InputSchema {
public:
    FullPinyinSchema();
    // Bounded preflight for normalized, unseparated input. This does not
    // generate correction paths or consult the dictionary.
    [[nodiscard]] bool is_single_syllable_transposition(
        std::string_view normalized_input) const;
    // One-edit repair of an invalid local token, preserving its typed initial.
    // Used only to distinguish fragmented vowel tokens from strong abbreviations.
    [[nodiscard]] bool has_initial_preserving_syllable_repair(
        std::string_view normalized_input) const;
    [[nodiscard]] ParseResult parse(std::string_view input,
                                    std::size_t max_paths = 32) const override;
    [[nodiscard]] ParseResult parse(std::string_view input, std::size_t max_paths,
                                    bool correction_enabled) const;
    [[nodiscard]] ParseResult parse(std::string_view input, std::size_t max_paths,
                                    bool correction_enabled,
                                    FullPinyinParseMetrics* metrics,
                                    const std::function<bool()>& cancelled = {}) const;
    // Accept complete spellings and a final syllable prefix without correction or initials.
    [[nodiscard]] ParseResult parse_spelling(std::string_view input, std::size_t max_paths,
        const std::function<bool()>& cancelled = {}) const;
    [[nodiscard]] ParseResult parse_incremental(
        std::string_view input, std::size_t max_paths,
        FullPinyinIncrementalState& state,
        FullPinyinParseMetrics* metrics = nullptr,
        const std::function<bool()>& cancelled = {},
        bool* reused = nullptr) const;

private:
    [[nodiscard]] ParseResult parse_impl(std::string_view input, std::size_t max_paths,
        bool correction_enabled, FullPinyinParseMetrics*, const std::function<bool()>& cancelled,
        bool spelling_only) const;
};

}  // namespace owo::engine
