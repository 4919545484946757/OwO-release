#pragma once
#include "owo/engine/user_frequency.h"
#include <chrono>

namespace owo::engine {
// Read-only external dictionaries are never recorded/flushed as learning.
// Exact/prefix indices preserve the base binary lexicon's optimized hot paths.
class ManagedDictionaryModel final : public UserFrequencyModel {
public:
    ManagedDictionaryModel(const UserFrequencyModel* learned, std::filesystem::path root)
        : learned_(learned), root_(std::move(root)) {}
    bool refresh(bool force = false);
    void set_enabled(bool enabled) noexcept { enabled_ = enabled; }
    std::int64_t score(std::string_view text) const override;
    bool preferred(std::string_view input, std::string_view text) const override;
    std::int64_t contextual_score(std::string_view input, std::string_view text) const override;
    std::int64_t language_context_score(std::string_view context, std::string_view input, std::string_view text) const override;
    std::vector<UserLexiconEntry> lookup(std::string_view input, std::size_t limit) const override;
    std::vector<UserLexiconEntry> lookup_prefix(std::string_view input, std::size_t limit) const override;
    std::vector<UserLexiconEntry> lookup_explicit(std::string_view input, std::size_t limit) const override;
    const std::string& diagnostic() const noexcept { return diagnostic_; }
private:
    std::vector<UserLexiconEntry> combined(std::string_view input, std::size_t limit, bool prefix) const;
    const UserFrequencyModel* learned_{};
    std::filesystem::path root_;
    std::string snapshot_, diagnostic_;
    std::vector<UserLexiconEntry> entries_;
    std::chrono::steady_clock::time_point next_check_{};
    bool enabled_{true};
};
}
