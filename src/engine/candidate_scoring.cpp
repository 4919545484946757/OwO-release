#include "owo/engine/candidate_scoring.h"
#include "owo/engine/candidate_generator.h"

#include <algorithm>
#include <cmath>
#include <limits>

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

void apply_shortest_segmentation_bonus(const std::span<Candidate> candidates,
                                       const std::size_t full_input_bytes) {
    const auto eligible = [full_input_bytes](const Candidate& candidate) {
        return full_input_bytes != 0 && candidate.consumed_input_bytes == full_input_bytes &&
            candidate.match_kind == InputMatchKind::exact && !candidate.syllables.empty() &&
            candidate.source_segments.size() == candidate.syllables.size() &&
            std::all_of(candidate.source_segments.begin(), candidate.source_segments.end(),
                [](const auto& segment) { return !segment.empty(); }) &&
            static_cast<std::size_t>(std::count_if(candidate.text.begin(), candidate.text.end(),
                [](unsigned char byte) { return (byte & 0xc0U) != 0x80U; })) == candidate.syllables.size();
    };
    std::size_t minimum = (std::numeric_limits<std::size_t>::max)(), maximum = 0;
    for (auto& candidate : candidates) {
        candidate.score -= candidate.shortest_segmentation_bonus;
        candidate.shortest_segmentation_bonus = 0;
        if (!eligible(candidate)) continue;
        minimum = std::min(minimum, candidate.syllables.size());
        maximum = std::max(maximum, candidate.syllables.size());
    }
    if (minimum >= maximum) return;
    for (auto& candidate : candidates) {
        if (!eligible(candidate) || candidate.syllables.size() != minimum) continue;
        candidate.shortest_segmentation_bonus = kCandidateShortestSegmentationBonus;
        candidate.score += candidate.shortest_segmentation_bonus;
    }
}

}  // namespace owo::engine
