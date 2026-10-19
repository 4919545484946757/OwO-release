#include "owo/engine/candidate_scoring.h"

#include <cmath>

namespace owo::engine {

std::int64_t candidate_unigram_score(const std::uint32_t frequency) {
    return static_cast<std::int64_t>(
        std::log1p(static_cast<double>(frequency)) * 1000.0);
}

std::int64_t candidate_input_match_penalty(const ParsePath& path) {
    constexpr std::int64_t kIncompleteBasePenalty = 1'500;
    constexpr std::int64_t kIncompleteCharacterPenalty =
        kCandidateIncompleteCharacterPenalty;
    constexpr std::int64_t kCorrectionPenalty = 6'000;
    constexpr std::int64_t kAbbreviationBasePenalty =
        kCandidateAbbreviationBasePenalty;
    if (path.match_kind == InputMatchKind::incomplete_completion) {
        return kIncompleteBasePenalty +
               static_cast<std::int64_t>(path.completion_characters) *
                   kIncompleteCharacterPenalty;
    }
    if (path.match_kind == InputMatchKind::corrected)
        return static_cast<std::int64_t>(path.edit_count) * kCorrectionPenalty;
    if (path.match_kind == InputMatchKind::abbreviated_completion) {
        return kAbbreviationBasePenalty +
               static_cast<std::int64_t>(path.completion_characters) *
                   kIncompleteCharacterPenalty;
    }
    return 0;
}

}  // namespace owo::engine
