#pragma once

#include "owo/engine/lexicon.h"

#include <array>
#include <cstdint>
#include <filesystem>
#include <limits>
#include <optional>
#include <string>
#include <unordered_map>
#include <vector>

namespace owo::engine {

inline constexpr std::uint32_t kBinaryLexiconVersion = 3;

struct LexiconIoResult {
    bool success{};
    std::string error;
};

struct BoundedInitialLookup {
    std::vector<LexiconEntry> entries;
    std::size_t work_units_consumed{};
    std::size_t next_index{};
    std::uint32_t next_frequency{};
    bool exhausted{};
};

struct BoundedLexiconLookup {
    std::vector<LexiconEntry> entries;
    std::size_t work_units_consumed{};
    std::size_t next_index{};
    std::uint32_t next_frequency{};
    bool exhausted{};
};

[[nodiscard]] LexiconIoResult write_binary_lexicon(
    const std::filesystem::path& path, std::vector<LexiconEntry> entries);

class BinaryLexicon final : public Lexicon {
public:
    BinaryLexicon() = default;
    ~BinaryLexicon();
    BinaryLexicon(const BinaryLexicon&) = delete;
    BinaryLexicon& operator=(const BinaryLexicon&) = delete;
    BinaryLexicon(BinaryLexicon&&) = delete;
    BinaryLexicon& operator=(BinaryLexicon&&) = delete;

    [[nodiscard]] LexiconIoResult load(const std::filesystem::path& path);
    [[nodiscard]] std::vector<LexiconEntry> lookup(
        std::span<const std::string_view> syllables) const override;
    [[nodiscard]] std::vector<LexiconTextEntry> lookup_texts(
        std::span<const std::string_view> syllables) const override;
    [[nodiscard]] std::uint32_t lookup_max_frequency(
        std::span<const std::string_view> syllables) const override;
    [[nodiscard]] BoundedLexiconLookup lookup_bounded(
        std::span<const std::string_view> syllables, std::size_t start_index,
        std::size_t result_limit, std::size_t work_unit_limit) const;
    [[nodiscard]] std::optional<std::uint32_t> exact_frequency_upper_bound(
        std::span<const std::string_view> syllables,
        std::size_t start_index = 0) const;
    [[nodiscard]] std::vector<LexiconEntry> lookup_initial(
        char initial,
        std::size_t limit = (std::numeric_limits<std::size_t>::max)()) const override;
    [[nodiscard]] std::vector<InitialLexiconTextEntry> lookup_initial_texts(
        char initial,
        std::size_t limit = (std::numeric_limits<std::size_t>::max)()) const override;
    [[nodiscard]] BoundedInitialLookup lookup_initial_bounded(
        char initial, std::size_t start_index, std::size_t result_limit,
        std::size_t work_unit_limit) const;
    [[nodiscard]] std::vector<AbbreviatedLexiconMatch> lookup_mixed_abbreviation(
        std::string_view input, std::size_t limit) const override;
    [[nodiscard]] std::vector<AbbreviatedLexiconTextMatch>
        lookup_mixed_abbreviation_texts(
            std::string_view input, std::size_t limit) const override;
    [[nodiscard]] std::size_t maximum_reading_length() const noexcept override {
        return maximum_reading_length_;
    }
    [[nodiscard]] std::size_t size() const noexcept {
        return mapped_entries_ != nullptr ? mapped_entry_count_ : entries_.size();
    }
    [[nodiscard]] bool exact_lookup_frequency_ordered() const noexcept {
        return lexicon_version_ >= 3;
    }
    [[nodiscard]] std::vector<LexiconEntry> materialize_entries() const;

private:
    struct CompactEntry {
        std::uint32_t syllable_offset{};
        std::uint32_t text_offset{};
        std::uint32_t frequency{};
        std::uint16_t syllable_count{};
        std::uint16_t text_size{};
    };

    struct SyllableRecord {
        std::uint32_t offset{};
        std::uint16_t size{};
        std::uint16_t reserved{};
    };

    struct IndexRange {
        std::uint32_t offset{};
        std::uint32_t count{};
    };

    struct MixedBucket {
        std::uint32_t key{};
        std::uint32_t offset{};
        std::uint32_t count{};
    };

    [[nodiscard]] LexiconEntry materialize(const CompactEntry& entry) const;
    [[nodiscard]] std::string_view syllable_at(std::uint16_t id) const;
    void reset_mapping() noexcept;

    std::vector<CompactEntry> entries_;
    std::vector<std::uint16_t> syllable_ids_;
    std::vector<std::string> syllables_;
    std::unordered_map<std::string, std::uint16_t> syllable_to_id_;
    std::string text_pool_;
    std::array<std::vector<std::uint32_t>, 26> initial_entries_;
    std::unordered_map<std::uint32_t, std::vector<std::uint32_t>> mixed_entries_;
    std::size_t maximum_reading_length_{};
    std::uint32_t lexicon_version_{};

    void* mapped_file_handle_{reinterpret_cast<void*>(-1)};
    void* mapped_mapping_handle_{};
    const unsigned char* mapped_bytes_{};
    std::size_t mapped_size_{};
    const CompactEntry* mapped_entries_{};
    std::size_t mapped_entry_count_{};
    const std::uint16_t* mapped_syllable_ids_{};
    std::size_t mapped_syllable_id_count_{};
    const SyllableRecord* mapped_syllable_records_{};
    std::size_t mapped_syllable_count_{};
    const char* mapped_syllable_pool_{};
    std::size_t mapped_syllable_pool_size_{};
    const char* mapped_text_pool_{};
    std::size_t mapped_text_pool_size_{};
    const IndexRange* mapped_initial_ranges_{};
    const std::uint32_t* mapped_initial_indices_{};
    std::size_t mapped_initial_index_count_{};
    const MixedBucket* mapped_mixed_buckets_{};
    std::size_t mapped_mixed_bucket_count_{};
    const std::uint32_t* mapped_mixed_indices_{};
    std::size_t mapped_mixed_index_count_{};
};

}  // namespace owo::engine
