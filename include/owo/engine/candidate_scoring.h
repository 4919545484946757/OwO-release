#pragma once

#include "owo/engine/input_schema.h"

#include <cstddef>
#include <cstdint>
#include <span>

namespace owo::engine {

struct Candidate;

inline constexpr std::int64_t kCandidateAdditionalSegmentPenalty = 22'000;
inline constexpr std::int64_t kCandidatePreferredExactSegmentationBonus = 16'000;
inline constexpr std::int64_t kCandidateIncompleteCharacterPenalty = 750;
inline constexpr std::int64_t kCandidateAbbreviationBasePenalty = 8'000;
// Soft evidence for a complete exact dictionary word, not a forced source tier.
inline constexpr std::int64_t kCandidateExactDictionarySentencePenalty = 1'500;
inline constexpr std::int64_t kCandidateShortestSegmentationBonus = 1'000;
// CandidateGenerator promotes dictionary words ahead of single characters as
// a final ordering tier. Bounded providers encode that same tier in their
// comparable score so the scheduler can reason about it before materializing.
inline constexpr std::int64_t kCandidateDictionaryWordSchedulingBonus = 2'000;
inline constexpr std::int64_t kCandidateComposedSentenceSchedulingBonus = 4'000;

[[nodiscard]] std::int64_t candidate_unigram_score(std::uint32_t frequency);
[[nodiscard]] std::int64_t candidate_input_match_penalty(const ParsePath& path);

// Candidates must belong to one input. Among generated full exact readings
// with different syllable counts, give the shortest count one fixed reward.
// Idempotent: replacing a candidate pool never compounds an earlier reward.
void apply_shortest_segmentation_bonus(std::span<Candidate> candidates,
                                       std::size_t full_input_bytes);

}  // namespace owo::engine
