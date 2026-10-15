#pragma once

#include <cstdint>
#include <limits>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {

struct LexiconEntry {
    std::vector<std::string> syllables;
    std::string text;
    std::uint32_t frequency{};
};

// Non-owning result for hot exact-lookup paths. The views remain valid while
// the lexicon is alive and unmodified.
struct LexiconTextEntry {
    std::string_view text;
    std::uint32_t frequency{};
};

struct InitialLexiconTextEntry {
    std::string_view syllable;
    std::string_view text;
    std::uint32_t frequency{};
};

struct AbbreviatedLexiconMatch {
    LexiconEntry entry;
    std::vector<std::string> source_segments;
};

struct AbbreviatedLexiconTextMatch {
    std::vector<std::string_view> syllables;
    std::string_view text;
    std::uint32_t frequency{};
    std::vector<std::string> source_segments;
};

class Lexicon {
public:
    virtual ~Lexicon() = default;
    [[nodiscard]] virtual std::vector<LexiconEntry> lookup(
        std::span<const std::string_view> syllables) const = 0;
    [[nodiscard]] virtual std::vector<LexiconTextEntry> lookup_texts(
        std::span<const std::string_view> syllables) const = 0;
    [[nodiscard]] virtual std::uint32_t lookup_max_frequency(
        std::span<const std::string_view> syllables) const = 0;
    [[nodiscard]] virtual std::vector<LexiconEntry> lookup_initial(
        char initial,
        std::size_t limit = (std::numeric_limits<std::size_t>::max)()) const = 0;
    [[nodiscard]] virtual std::vector<InitialLexiconTextEntry> lookup_initial_texts(
        char initial,
        std::size_t limit = (std::numeric_limits<std::size_t>::max)()) const = 0;
    [[nodiscard]] virtual std::vector<AbbreviatedLexiconMatch> lookup_mixed_abbreviation(
        std::string_view input, std::size_t limit) const = 0;
    [[nodiscard]] virtual std::vector<AbbreviatedLexiconTextMatch>
        lookup_mixed_abbreviation_texts(
            std::string_view input, std::size_t limit) const = 0;
    [[nodiscard]] virtual std::size_t maximum_reading_length() const noexcept = 0;
};

class MemoryLexicon final : public Lexicon {
public:
    explicit MemoryLexicon(std::vector<LexiconEntry> entries);
    [[nodiscard]] std::vector<LexiconEntry> lookup(
        std::span<const std::string_view> syllables) const override;
    [[nodiscard]] std::vector<LexiconTextEntry> lookup_texts(
        std::span<const std::string_view> syllables) const override;
    [[nodiscard]] std::uint32_t lookup_max_frequency(
        std::span<const std::string_view> syllables) const override;
    [[nodiscard]] std::vector<LexiconEntry> lookup_initial(
        char initial,
        std::size_t limit = (std::numeric_limits<std::size_t>::max)()) const override;
    [[nodiscard]] std::vector<InitialLexiconTextEntry> lookup_initial_texts(
        char initial,
        std::size_t limit = (std::numeric_limits<std::size_t>::max)()) const override;
    [[nodiscard]] std::vector<AbbreviatedLexiconMatch> lookup_mixed_abbreviation(
        std::string_view input, std::size_t limit) const override;
    [[nodiscard]] std::vector<AbbreviatedLexiconTextMatch>
        lookup_mixed_abbreviation_texts(
            std::string_view input, std::size_t limit) const override;
    [[nodiscard]] std::size_t maximum_reading_length() const noexcept override {
        return maximum_reading_length_;
    }

private:
    std::vector<LexiconEntry> entries_;
    std::size_t maximum_reading_length_{};
};

}  // namespace owo::engine
