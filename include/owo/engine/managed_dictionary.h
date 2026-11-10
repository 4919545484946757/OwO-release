#pragma once
#include "owo/engine/user_frequency.h"
#include "owo/engine/indexed_lexicon.h"
#include "owo/engine/binary_lexicon.h"
#include <chrono>
#include <future>
#include <memory>

namespace owo::engine {
// Compiles a private, unpublished snapshot. Original source/catalog files are
// retained. The ready directory is published atomically, including aliases.
LexiconIoResult prepare_managed_dictionary_snapshot(const std::filesystem::path& directory);
struct ManagedDictionarySnapshot {
    std::shared_ptr<BinaryLexicon> phonetic;
    std::vector<UserLexiconEntry> aliases;
    bool base_enabled{true}, user_enabled{true};
    bool migrated{};
    std::int64_t prepare_us{};
    std::string error;
};
// Read-only external dictionaries are never recorded/flushed as learning.
// Phonetic words enter Lexicon; non-phonetic shortcuts stay explicit aliases.
class ManagedDictionaryModel final : public UserFrequencyModel, private Lexicon {
public:
    ManagedDictionaryModel(const UserFrequencyModel* learned, std::filesystem::path root)
        : learned_(learned), root_(std::move(root)) {}
    bool refresh(bool force = false);
    void set_enabled(bool enabled) noexcept { enabled_ = enabled; }
    bool base_enabled() const noexcept { return base_enabled_; }
    bool user_enabled() const noexcept { return user_enabled_; }
    const Lexicon& phonetic_lexicon() const noexcept { return *this; }
    bool has_phonetic_entries() const noexcept { return enabled_ && phonetic_ && phonetic_->size(); }
    std::int64_t score(std::string_view text) const override;
    bool preferred(std::string_view input, std::string_view text) const override;
    std::int64_t contextual_score(std::string_view input, std::string_view text) const override;
    std::int64_t language_context_score(std::string_view context, std::string_view input, std::string_view text) const override;
    std::vector<UserLexiconEntry> lookup(std::string_view input, std::size_t limit) const override;
    std::vector<UserLexiconEntry> lookup_prefix(std::string_view input, std::size_t limit) const override;
    std::vector<UserLexiconEntry> lookup_related(std::string_view input, std::size_t limit) const override;
    std::vector<UserLexiconEntry> lookup_explicit(std::string_view input, std::size_t limit) const override;
    const std::string& diagnostic() const noexcept { return diagnostic_; }
private:
    // Stable Lexicon facade: CombinedLexicon/Generator retain this reference
    // while immutable mappings are replaced on the Core thread only.
    std::vector<LexiconEntry> lookup(std::span<const std::string_view> s) const override { return phonetic_ ? phonetic_->lookup(s) : std::vector<LexiconEntry>{}; }
    std::vector<LexiconTextEntry> lookup_texts(std::span<const std::string_view> s) const override { return phonetic_ ? phonetic_->lookup_texts(s) : std::vector<LexiconTextEntry>{}; }
    std::uint32_t lookup_max_frequency(std::span<const std::string_view> s) const override { return phonetic_ ? phonetic_->lookup_max_frequency(s) : 0; }
    std::vector<LexiconEntry> lookup_initial(char c, std::size_t n) const override { return phonetic_ ? phonetic_->lookup_initial(c, n) : std::vector<LexiconEntry>{}; }
    std::vector<InitialLexiconTextEntry> lookup_initial_texts(char c, std::size_t n) const override { return phonetic_ ? phonetic_->lookup_initial_texts(c, n) : std::vector<InitialLexiconTextEntry>{}; }
    std::vector<AbbreviatedLexiconMatch> lookup_mixed_abbreviation(std::string_view s, std::size_t n) const override { return phonetic_ ? phonetic_->lookup_mixed_abbreviation(s, n) : std::vector<AbbreviatedLexiconMatch>{}; }
    std::vector<AbbreviatedLexiconTextMatch> lookup_mixed_abbreviation_texts(std::string_view s, std::size_t n) const override { return phonetic_ ? phonetic_->lookup_mixed_abbreviation_texts(s, n) : std::vector<AbbreviatedLexiconTextMatch>{}; }
    std::size_t maximum_reading_length() const noexcept override { return phonetic_ ? phonetic_->maximum_reading_length() : 0; }
    bool apply(ManagedDictionarySnapshot snapshot);
    std::vector<UserLexiconEntry> combined(std::string_view input, std::size_t limit, bool prefix) const;
    const UserFrequencyModel* learned_{};
    std::filesystem::path root_;
    std::string snapshot_, diagnostic_;
    std::vector<UserLexiconEntry> entries_;
    std::shared_ptr<BinaryLexicon> phonetic_;
    std::future<ManagedDictionarySnapshot> pending_;
    std::string pending_id_;
    std::chrono::steady_clock::time_point next_check_{};
    bool enabled_{true};
    bool base_enabled_{true}, user_enabled_{true};
};
}
