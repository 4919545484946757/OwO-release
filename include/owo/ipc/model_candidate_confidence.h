#pragma once

#include "owo/engine/candidate_generator.h"

#include <algorithm>
#include <string_view>

namespace owo::ipc {

// Only already-generated, short whole-input abbreviation alternatives. Each
// output syllable keeps a nonempty source prefix; no omitted syllables or
// Cartesian synthesis is introduced by model recall.
[[nodiscard]] inline bool abbreviated_model_alternative(
    const engine::Candidate& candidate, const std::size_t full_input_bytes) {
    if (full_input_bytes < 3 || full_input_bytes > 12 ||
        candidate.consumed_input_bytes != full_input_bytes ||
        candidate.match_kind != engine::InputMatchKind::abbreviated_completion ||
        candidate.syllables.size() < 3 || candidate.syllables.size() > 8 ||
        candidate.source_segments.size() != candidate.syllables.size()) return false;
    if (static_cast<std::size_t>(std::count_if(candidate.text.begin(), candidate.text.end(),
            [](const unsigned char byte) { return (byte & 0xc0U) != 0x80U; })) != candidate.syllables.size() ||
        (candidate.segment_count > 1 && std::none_of(
            candidate.segment_lengths.begin(), candidate.segment_lengths.end(),
            [](const auto length) { return length >= 2; }))) return false;
    std::size_t initials = 0;
    for (std::size_t i = 0; i < candidate.syllables.size(); ++i) {
        const auto& source = candidate.source_segments[i];
        if (source.empty() || !std::string_view(candidate.syllables[i]).starts_with(source)) return false;
        initials += source.size() == 1 && candidate.syllables[i].size() > 1 &&
            std::string_view("aeiouv").find(source.front()) == std::string_view::npos;
    }
    return initials >= 2;
}

// Coherence may credit an incidental dictionary word across a standalone
// sentence head (e.g. a noun instead of a preposition). Identical, independently
// tokenized suffixes do not justify locking those heads into different model
// confidence tiers. This only permits a model comparison, never promotes a
// particular character, changes base scores or overrides an explicit learning.
[[nodiscard]] inline bool standalone_head_alternative(
    const engine::Candidate& reference, const engine::Candidate& candidate,
    const std::size_t full_input_bytes) {
    if (reference.user_preferred() || candidate.user_preferred() ||
        reference.match_kind != engine::InputMatchKind::exact ||
        candidate.match_kind != engine::InputMatchKind::exact ||
        reference.consumed_input_bytes != full_input_bytes ||
        candidate.consumed_input_bytes != full_input_bytes ||
        reference.syllables.size() < 3 || candidate.syllables != reference.syllables ||
        candidate.source_segments != reference.source_segments ||
        reference.segment_count < 2 || candidate.segment_count != reference.segment_count ||
        reference.segment_lengths.empty() || reference.segment_lengths.front() != 1 ||
        candidate.segment_lengths != reference.segment_lengths ||
        std::none_of(reference.segment_lengths.begin(), reference.segment_lengths.end(),
                     [](const auto length) { return length >= 2; }) ||
        reference.text == candidate.text) return false;
    const auto count = [](const std::string_view text) {
        return static_cast<std::size_t>(std::count_if(text.begin(), text.end(),
            [](const unsigned char byte) { return (byte & 0xc0U) != 0x80U; }));
    };
    if (count(reference.text) != reference.syllables.size() ||
        count(candidate.text) != reference.syllables.size()) return false;
    const auto suffix = [](const std::string_view text) {
        auto end = std::size_t{1};
        while (end < text.size() && (static_cast<unsigned char>(text[end]) & 0xc0U) == 0x80U) ++end;
        return text.substr(end);
    };
    return suffix(reference.text) == suffix(candidate.text);
}

}  // namespace owo::ipc
