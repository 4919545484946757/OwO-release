#pragma once

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace owo::engine {

struct UserLexiconEntry {
    std::string text;
    std::uint32_t count{};
    std::string input;
};

class UserFrequencyModel {
public:
    virtual ~UserFrequencyModel() = default;
    [[nodiscard]] virtual std::int64_t score(std::string_view text) const = 0;
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

class UserFrequencyStore final : public UserFrequencyModel {
public:
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

private:
    void rebuild_user_lexicon();

    std::filesystem::path path_;
    std::unordered_map<std::string, std::uint32_t> counts_;
    std::unordered_map<std::string, std::uint32_t> contextual_counts_;
    std::unordered_map<std::string, std::uint32_t> language_context_counts_;
    std::unordered_map<std::string,
                       std::unordered_map<std::string, std::uint32_t>> user_lexicon_;
    std::uint32_t sensitivity_{7};
};

}  // namespace owo::engine
