#include "owo/engine/candidate_partition.h"
#include "owo/engine/candidate_scoring.h"

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

void merge_similar_length_partitions(std::vector<Candidate> &candidates,
                                     std::size_t full_input_bytes) {
    const auto anchor = std::find_if(candidates.begin(), candidates.end(), [&](const auto &value) {
        return !value.model_only && sentence(value, full_input_bytes);
    });
    if (anchor == candidates.end())
        return;
    // Only equal character counts share a band. Even a one-character gap
    // retains the original word/sentence partition order.
    const auto anchor_length = characters(anchor->text);
    const auto anchor_kind = anchor->match_kind;
    const auto eligible = [&](const Candidate &value) {
        const auto length = characters(value.text);
        return length >= 2 && length == anchor_length && value.match_kind == anchor_kind;
    };
    if (std::none_of(candidates.begin(), candidates.end(), [&](const auto &value) {
            return !value.model_only && !sentence(value, full_input_bytes) && eligible(value);
        }))
        return;

    std::vector<std::size_t> slots;
    std::vector<Candidate> joint;
    std::size_t sentence_slots = 0;
    for (std::size_t index = 0; index < candidates.size(); ++index) {
        auto &value = candidates[index];
        if (!eligible(value))
            continue;
        value.joint_phrase_partition = true;
        if (value.model_only)
            continue; // Never mix the hidden model tail into visible pages.
        sentence_slots += sentence(value, full_input_bytes);
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
        joint[index].partition_slot = index < sentence_slots
            ? CandidatePartitionSlot::sentence : CandidatePartitionSlot::dictionary;
        candidates[slots[index]] = std::move(joint[index]);
    }
}
} // namespace owo::engine
