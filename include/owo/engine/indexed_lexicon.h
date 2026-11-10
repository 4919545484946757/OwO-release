#pragma once
#include "owo/engine/lexicon.h"
#include <array>
#include <unordered_map>

namespace owo::engine {
// Compact, immutable-after-build index for imported phonetic dictionaries.
// Rows share integer syllables and a text pool; no per-key full-table scan.
class IndexedLexicon final : public Lexicon {
public:
    // Offline snapshot compilation only; never used by candidate queries.
    std::vector<LexiconEntry> materialize_entries() const;
    void add(std::span<const std::string> reading, std::string_view text, std::uint32_t frequency);
    void finish();
    bool empty() const noexcept { return rows_.empty(); }
    std::vector<LexiconEntry> lookup(std::span<const std::string_view>) const override;
    std::vector<LexiconTextEntry> lookup_texts(std::span<const std::string_view>) const override;
    std::uint32_t lookup_max_frequency(std::span<const std::string_view>) const override;
    std::vector<LexiconEntry> lookup_initial(char, std::size_t limit) const override;
    std::vector<InitialLexiconTextEntry> lookup_initial_texts(char, std::size_t limit) const override;
    std::vector<AbbreviatedLexiconMatch> lookup_mixed_abbreviation(std::string_view, std::size_t) const override;
    std::vector<AbbreviatedLexiconTextMatch> lookup_mixed_abbreviation_texts(std::string_view, std::size_t) const override;
    std::size_t maximum_reading_length() const noexcept override { return maximum_; }
private:
    struct Row { std::uint32_t reading, text, frequency; std::uint16_t count, bytes; };
    std::span<const std::uint16_t> reading(const Row&) const;
    std::string_view text(const Row&) const;
    LexiconEntry materialize(const Row&) const;
    std::pair<std::size_t, std::size_t> exact_range(std::span<const std::string_view>) const;
    std::vector<Row> rows_;
    std::vector<std::uint16_t> ids_;
    std::vector<std::string> syllables_;
    std::unordered_map<std::string, std::uint16_t> syllable_ids_;
    std::string pool_;
    std::array<std::vector<std::uint32_t>, 26> initials_;
    std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> mixed_;
    std::size_t maximum_{};
};

// Non-owning view. The mapped base is not copied or re-indexed.
class CombinedLexicon final : public Lexicon {
public:
    CombinedLexicon(const Lexicon& base, const Lexicon& imported) : base_(base), imported_(imported) {}
    std::vector<LexiconEntry> lookup(std::span<const std::string_view>) const override;
    std::vector<LexiconTextEntry> lookup_texts(std::span<const std::string_view>) const override;
    std::uint32_t lookup_max_frequency(std::span<const std::string_view>) const override;
    std::vector<LexiconEntry> lookup_initial(char, std::size_t limit) const override;
    std::vector<InitialLexiconTextEntry> lookup_initial_texts(char, std::size_t limit) const override;
    std::vector<AbbreviatedLexiconMatch> lookup_mixed_abbreviation(std::string_view, std::size_t) const override;
    std::vector<AbbreviatedLexiconTextMatch> lookup_mixed_abbreviation_texts(std::string_view, std::size_t) const override;
    std::size_t maximum_reading_length() const noexcept override;
private:
    const Lexicon &base_, &imported_;
};
}
