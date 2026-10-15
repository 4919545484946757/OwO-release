#pragma once

#include "owo/engine/input_schema.h"

#include <cstdint>

namespace owo::engine {

inline constexpr std::int64_t kCandidateAdditionalSegmentPenalty = 22'000;
inline constexpr std::int64_t kCandidatePreferredExactSegmentationBonus = 16'000;
inline constexpr std::int64_t kCandidateIncompleteCharacterPenalty = 750;
inline constexpr std::int64_t kCandidateAbbreviationBasePenalty = 8'000;
// CandidateGenerator promotes dictionary words ahead of single characters as
// a final ordering tier. Bounded providers encode that same tier in their
// comparable score so the scheduler can reason about it before materializing.
inline constexpr std::int64_t kCandidateDictionaryWordSchedulingBonus = 2'000;
inline constexpr std::int64_t kCandidateComposedSentenceSchedulingBonus = 4'000;

[[nodiscard]] std::int64_t candidate_unigram_score(std::uint32_t frequency);
[[nodiscard]] std::int64_t candidate_input_match_penalty(const ParsePath& path);

}  // namespace owo::engine
