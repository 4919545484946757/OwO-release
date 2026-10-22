#pragma once

#include "owo/engine/user_corpus.h"

#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>

namespace owo::engine {

// Continuation text beyond this length is rarely a useful completion and
// costs the user more to undo than to type.
inline constexpr std::size_t kMaximumCollocationContinuationBytes = 48;
// Context keys are 1-3 characters. The bound keeps the in-memory index small
// even for a corpus at the entry limit.
inline constexpr std::size_t kMaximumCollocationContexts = 200'000;
inline constexpr std::uint32_t kMaximumCollocationFollowerCount = 4096;
inline constexpr std::size_t kMaximumCollocationFollowers = 48;
// Only the newest corpus sentences feed the statistics. Recent phrasing is the
// better predictor, and the bound keeps a rebuild after a corpus change short
// enough not to become a visible pause between keystrokes.
inline constexpr std::size_t kMaximumCollocationCorpusEntries = 6'000;

/// Character-level continuation statistics derived from the authorized corpus
/// (learning layer 2). The model lives in memory only: it is rebuilt from the
/// corpus file, so deleting corpus entries also removes their statistics.
class UserCollocationModel final {
public:
    /// Counts the followers of every 1-3 character context in one sentence.
    void observe(std::string_view sentence);
    void clear() noexcept;
    [[nodiscard]] bool empty() const noexcept { return contexts_.empty(); }
    [[nodiscard]] std::size_t sentence_count() const noexcept { return sentences_; }
    [[nodiscard]] std::size_t context_count() const noexcept { return contexts_.size(); }

    /// Best continuation after `history`, at most `maximum_bytes` of UTF-8.
    /// Returns an empty string when the model has no evidence, when the history
    /// ends a sentence, or when the best continuation is a single character.
    [[nodiscard]] std::string continuation(
        std::string_view history,
        std::size_t maximum_bytes = kMaximumCollocationContinuationBytes) const;

private:
    struct Followers {
        std::unordered_map<std::string, std::uint32_t> next;
        std::uint32_t total{};
    };
    // Most frequent follower of the last one to three characters of `history`.
    [[nodiscard]] bool best_follower(std::string_view history,
                                     std::string& follower) const;

    std::unordered_map<std::string, Followers> contexts_;
    std::size_t sentences_{};
};

[[nodiscard]] UserCollocationModel build_user_collocation_model(
    std::span<const UserCorpusEntry> entries);

}  // namespace owo::engine
