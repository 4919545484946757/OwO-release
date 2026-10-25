#include "owo/engine/candidate_partition.h"

#include <algorithm>

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
    for (std::size_t index = 0; index < candidates.size(); ++index) {
        auto &value = candidates[index];
        if (!eligible(value))
            continue;
        value.joint_phrase_partition = true;
        if (value.model_only)
            continue; // Never mix the hidden model tail into visible pages.
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
    for (std::size_t index = 0; index < slots.size(); ++index)
        candidates[slots[index]] = std::move(joint[index]);
}
} // namespace owo::engine
