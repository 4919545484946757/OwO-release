#include "owo/engine/candidate_partition.h"
#include "owo/engine/candidate_scoring.h"
#include "owo/engine/sentence_reading_order.h"

#include <algorithm>
#include <limits>
#include <unordered_set>

namespace owo::engine {
namespace {
std::size_t characters(std::string_view text) {
    return static_cast<std::size_t>(std::count_if(
        text.begin(), text.end(), [](unsigned char byte) { return (byte & 0xc0U) != 0x80U; }));
}
bool sentence(const Candidate &candidate, std::size_t full_input_bytes) {
    return candidate.consumed_input_bytes == full_input_bytes && candidate.syllables.size() >= 3 &&
           candidate.segment_count > 1 && characters(candidate.text) == candidate.syllables.size();
}
} // namespace

bool is_non_single_candidate(const Candidate& candidate) noexcept {
    return characters(candidate.text) >= 2;
}

std::string sentence_reading_key(const Candidate& candidate) {
    std::string key;
    for (const auto& source : candidate.source_segments) {
        key += std::to_string(source.size()); key += ':'; key += source; key += '|';
    }
    return key;
}

std::size_t sentence_candidate_limit(const std::span<const Candidate> candidates,
                                     const std::size_t full_input_bytes) {
    std::unordered_set<std::string> readings;
    for (const auto& candidate : candidates)
        if (candidate.qualified_sentence_reading && candidate.consumed_input_bytes == full_input_bytes &&
            candidate.syllables.size() >= 3 && characters(candidate.text) == candidate.syllables.size())
            readings.insert(sentence_reading_key(candidate));
    return std::max<std::size_t>(1, readings.size()) * 2;
}

void arrange_sentence_reading_pages(const std::span<Candidate> candidates) {
    if (candidates.size() < 3) return;
    std::vector<std::string> readings;
    readings.reserve(candidates.size());
    for (const auto& candidate : candidates)
        readings.push_back(candidate.qualified_sentence_reading ? sentence_reading_key(candidate) : std::string{});
    const auto order = sentence_reading_page_order(readings);
    std::vector<Candidate> arranged;
    arranged.reserve(candidates.size());
    for (const auto index : order) arranged.push_back(std::move(candidates[index]));
    std::move(arranged.begin(), arranged.end(), candidates.begin());
}

bool requires_incomplete_candidate_limit(const ParseResult& parsed,
                                         const std::span<const Candidate> candidates) noexcept {
    if (!parsed.phonetic_assistance) return false;
    if (std::any_of(candidates.begin(), candidates.end(), [&](const Candidate& candidate) {
        return candidate.match_kind == InputMatchKind::exact && !candidate.user_lexicon &&
            candidate.consumed_input_bytes == parsed.normalized_input.size();
    })) return false;
    return parsed.has_incomplete_syllable || std::any_of(candidates.begin(), candidates.end(), [](const Candidate& candidate) {
        return candidate.match_kind == InputMatchKind::incomplete_completion ||
            candidate.match_kind == InputMatchKind::abbreviated_completion;
    });
}

void limit_incomplete_candidates(std::vector<Candidate>& candidates, const std::size_t page_size,
                                const bool incomplete_input, const std::size_t reserved_slots) {
    if (page_size == 0 || !incomplete_input) return;
    const auto capacity = page_size > std::numeric_limits<std::size_t>::max() / 2
        ? std::numeric_limits<std::size_t>::max() : page_size * 2;
    const auto limit = capacity - std::min(capacity, reserved_slots);
    std::size_t kept = 0;
    std::erase_if(candidates, [&](const Candidate& candidate) {
        return is_non_single_candidate(candidate) && ++kept > limit;
    });
}

void prioritize_complete_characters(std::vector<Candidate>& candidates,
                                    const std::size_t full_input_bytes) {
    std::stable_partition(candidates.begin(), candidates.end(),
        [full_input_bytes](const Candidate& value) {
            return value.match_kind == InputMatchKind::exact &&
                   value.consumed_input_bytes == full_input_bytes &&
                   value.syllables.size() == 1 && characters(value.text) == 1;
        });
}

void apply_exact_dictionary_sentence_penalty(std::span<Candidate> candidates,
                                              std::size_t full_input_bytes) {
    std::unordered_set<std::size_t> dictionary_lengths;
    for (const auto &value : candidates) {
        if (value.match_kind != InputMatchKind::exact || value.segment_count != 1 ||
            value.consumed_input_bytes != full_input_bytes || value.model_only ||
            value.provisional_user_word)
            continue;
        const auto length = characters(value.text);
        if (length >= 3 && length == value.syllables.size())
            dictionary_lengths.insert(length);
    }
    if (dictionary_lengths.empty())
        return;
    for (auto &value : candidates) {
        if (!value.user_preferred() && value.match_kind == InputMatchKind::exact &&
            sentence(value, full_input_bytes) && dictionary_lengths.contains(characters(value.text)))
            value.score -= kCandidateExactDictionarySentencePenalty;
    }
}

static void merge_scoring_band(std::vector<Candidate> &candidates,
                               std::size_t full_input_bytes, bool equal_length_band) {
    const auto anchor = std::find_if(candidates.begin(), candidates.end(), [&](const auto &value) {
        return !value.model_only && sentence(value, full_input_bytes);
    });
    if (anchor == candidates.end())
        return;
    // Length is not evidence for an interpretation of the same encoding.
    // Keep the existing equal-length band, but compare complete alternative
    // exact readings by score as well. Partial commits never enter that band.
    const auto anchor_length = characters(anchor->text);
    const auto anchor_kind = anchor->match_kind;
    const auto anchor_segments = anchor->source_segments;
    const auto encoding = [](const Candidate& value) {
        std::string result;
        for (const auto& segment : value.source_segments) result += segment;
        return result;
    };
    const auto anchor_encoding = encoding(*anchor);
    const auto full_reading = [&](const Candidate& value) {
        return anchor_kind == InputMatchKind::exact && value.match_kind == anchor_kind &&
            !anchor_encoding.empty() && value.consumed_input_bytes == full_input_bytes &&
            value.source_segments.size() == value.syllables.size() &&
            characters(value.text) == value.syllables.size() && characters(value.text) >= 2 &&
            encoding(value) == anchor_encoding;
    };
    const bool alternative_reading = std::any_of(candidates.begin(), candidates.end(), [&](const auto& value) {
        return !value.model_only && full_reading(value) && value.source_segments != anchor_segments;
    });
    const auto eligible = [&](const Candidate &value) {
        const auto length = characters(value.text);
        return value.match_kind == anchor_kind &&
            ((equal_length_band && length >= 2 && length == anchor_length) ||
             (alternative_reading && full_reading(value)));
    };
    if (!alternative_reading && std::none_of(candidates.begin(), candidates.end(), [&](const auto &value) {
            return !value.model_only && !sentence(value, full_input_bytes) && eligible(value);
        }))
        return;

    std::vector<std::size_t> slots;
    std::vector<Candidate> joint;
    std::vector<CandidatePartitionSlot> display_slots;
    for (std::size_t index = 0; index < candidates.size(); ++index) {
        auto &value = candidates[index];
        if (!eligible(value))
            continue;
        if (value.model_only && !(equal_length_band && characters(value.text) == anchor_length))
            continue;
        value.joint_phrase_partition = true;
        if (value.model_only)
            continue; // Never mix the hidden model tail into visible pages.
        display_slots.push_back(value.partition_slot != CandidatePartitionSlot::automatic
            ? value.partition_slot : sentence(value, full_input_bytes)
                ? CandidatePartitionSlot::sentence : CandidatePartitionSlot::dictionary);
        slots.push_back(index);
        joint.push_back(std::move(value));
    }
    std::stable_sort(joint.begin(), joint.end(), [](const auto &left, const auto &right) {
        if (left.user_preferred() != right.user_preferred())
            return left.user_preferred();
        // Scores already include matching, lexical, coherence and user/context
        // evidence. Do not add another source-dependent word/sentence bonus.
        if (left.score != right.score)
            return left.score > right.score;
        if (left.consumed_input_bytes != right.consumed_input_bytes)
            return left.consumed_input_bytes > right.consumed_input_bytes;
        return false;
    });
    for (std::size_t index = 0; index < slots.size(); ++index) {
        // Layout slots belong to their original page positions. Packing all
        // sentence flags at the front would undo the per-page sentence cap.
        joint[index].partition_slot = display_slots[index];
        candidates[slots[index]] = std::move(joint[index]);
    }
}
void merge_similar_length_partitions(std::vector<Candidate>& candidates,
                                     std::size_t full_input_bytes) {
    merge_scoring_band(candidates, full_input_bytes, true);
}

void rank_full_input_segmentations(std::vector<Candidate>& candidates,
                                    std::size_t full_input_bytes) {
    merge_scoring_band(candidates, full_input_bytes, false);
}
} // namespace owo::engine
