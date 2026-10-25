#pragma once

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <functional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace owo::engine {

struct UserLexiconEntry {
    std::string text;
    std::uint32_t count{};
    std::string input;
    bool preferred{true};
};

class UserFrequencyModel {
public:
    virtual ~UserFrequencyModel() = default;
    [[nodiscard]] virtual std::int64_t score(std::string_view text) const = 0;
    // Preference is bound to the actually consumed encoding, not just the
    // displayed word. This also applies when it is a prefix of a longer input.
    [[nodiscard]] virtual bool preferred(std::string_view input, std::string_view text) const {
        (void)input;
        (void)text;
        return false;
    }
    [[nodiscard]] virtual std::int64_t contextual_score(std::string_view input,
                                                        std::string_view text) const {
        (void)input;
        (void)text;
        return 0;
    }
    [[nodiscard]] virtual std::int64_t language_context_score(
        std::string_view context, std::string_view input, std::string_view text) const {
        (void)context;
        (void)input;
        (void)text;
        return 0;
    }
    [[nodiscard]] virtual std::vector<UserLexiconEntry> lookup(
        std::string_view input, std::size_t limit) const {
        (void)input;
        (void)limit;
        return {};
    }
    [[nodiscard]] virtual std::vector<UserLexiconEntry> lookup_prefix(
        std::string_view input, std::size_t limit) const {
        (void)input;
        (void)limit;
        return {};
    }
};

struct UserFrequencyIoResult {
    bool success{};
    bool recovered_from_backup{};
    std::string error;
};
struct UserLexiconPage {
    std::vector<UserLexiconEntry> entries;
    std::size_t total{};
};

class UserFrequencyStore final : public UserFrequencyModel {
public:
    using Clock = std::function<std::uint64_t()>;
    explicit UserFrequencyStore(Clock clock = {});
    [[nodiscard]] UserFrequencyIoResult load(const std::filesystem::path& path);
    void record(std::string_view text, std::uint32_t amount = 1);
    void record(std::string_view input, std::string_view text, std::uint32_t amount = 1);
    void record(std::string_view context, std::string_view input, std::string_view text,
                std::uint32_t amount = 1);
    void set_sensitivity(std::uint32_t sensitivity) noexcept;
    [[nodiscard]] std::uint32_t count(std::string_view text) const;
    [[nodiscard]] std::uint32_t contextual_count(std::string_view input,
                                                 std::string_view text) const;
    [[nodiscard]] std::int64_t score(std::string_view text) const override;
    [[nodiscard]] bool preferred(std::string_view input, std::string_view text) const override;
    [[nodiscard]] std::int64_t contextual_score(std::string_view input,
                                                std::string_view text) const override;
    [[nodiscard]] std::int64_t language_context_score(
        std::string_view context, std::string_view input,
        std::string_view text) const override;
    [[nodiscard]] std::vector<UserLexiconEntry> lookup(
        std::string_view input, std::size_t limit) const override;
    [[nodiscard]] std::vector<UserLexiconEntry> lookup_prefix(
        std::string_view input, std::size_t limit) const override;
    [[nodiscard]] UserFrequencyIoResult flush() const;
    [[nodiscard]] std::uint64_t ranking_epoch() const;
    [[nodiscard]] UserLexiconPage list_words(std::string_view query, std::size_t offset,
                                             std::size_t limit) const;
    [[nodiscard]] static std::string validate_word(const UserLexiconEntry& entry);
    // Explicit user edits are confirmed preferences, not simulated selections.
    [[nodiscard]] std::string edit_words(const std::vector<UserLexiconEntry>& entries,
                                         std::string_view old_input = {},
                                         std::string_view old_text = {}, bool replace = false);
    void clear_learning();

private:
    void rebuild_user_lexicon();
    using ObservationTimes = std::unordered_map<std::string, std::uint64_t>;
    [[nodiscard]] std::int64_t decay_singleton(std::int64_t score, std::string_view key,
                                              const ObservationTimes& times) const;
    [[nodiscard]] bool preferred_observation(std::string_view key,
                                            const ObservationTimes& times) const;

    std::filesystem::path path_;
    std::unordered_map<std::string, std::uint32_t> counts_;
    std::unordered_map<std::string, std::uint32_t> contextual_counts_;
    std::unordered_map<std::string, std::uint32_t> language_context_counts_;
    ObservationTimes singleton_times_, contextual_singleton_times_, language_singleton_times_;
    std::unordered_map<std::string, ObservationTimes> user_lexicon_times_;
    Clock clock_;
    std::uint64_t singleton_expires_at_{};
    std::unordered_map<std::string,
                       std::unordered_map<std::string, std::uint32_t>> user_lexicon_;
    std::uint32_t sensitivity_{7};
};

}  // namespace owo::engine
