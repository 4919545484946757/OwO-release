#pragma once
#include "owo/engine/sentence_quality.h"
#include "owo/engine/sentence_reading_order.h"

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <numeric>
#include <string>
#include <string_view>
#include <vector>
#include <unordered_map>

namespace owo::ipc {

[[nodiscard]] inline std::size_t utf8_character_count(const std::string_view text) {
    return static_cast<std::size_t>(std::count_if(
        text.begin(), text.end(), [](const unsigned char byte) {
            return (byte & 0xc0U) != 0x80U;
        }));
}

// Model ranking is authoritative inside one structural/confidence tier.
// Tier boundaries remain stable:
// whole-input phrases, then other prefixes, then single-character fallbacks.
inline void apply_contextual_candidate_order(
    const std::vector<std::string>& model_order,
    const std::size_t full_input_bytes,
    const std::vector<std::string>& original_candidates,
    const std::vector<std::uint64_t>& original_consumed,
    std::vector<std::string>& ordered_candidates,
    std::vector<std::uint64_t>& ordered_consumed,
    const std::vector<std::uint64_t>& original_confidence = {},
    const std::vector<bool>& joint_phrase_partition = {}) {
    if (original_candidates.size() != original_consumed.size()) return;
    if (!original_confidence.empty() && original_confidence.size() != original_candidates.size())
        return;
    if (!joint_phrase_partition.empty() && joint_phrase_partition.size() != original_candidates.size())
        return;

    const auto model_rank = [&model_order](const std::string& candidate) {
        const auto found = std::find(model_order.begin(), model_order.end(), candidate);
        return found == model_order.end()
                   ? model_order.size()
                   : static_cast<std::size_t>(found - model_order.begin());
    };
    const auto tier = [full_input_bytes](const std::string& candidate,
                                         const std::uint64_t consumed, std::size_t) {
        if (consumed == full_input_bytes) return 0;
        if (utf8_character_count(candidate) >= 2) return 1;
        return 2;
    };

    std::vector<std::size_t> indices(original_candidates.size());
    std::iota(indices.begin(), indices.end(), 0);
    std::stable_sort(indices.begin(), indices.end(), [&](const std::size_t left,
                                                         const std::size_t right) {
        const auto left_tier = tier(original_candidates[left], original_consumed[left], left);
        const auto right_tier = tier(original_candidates[right], original_consumed[right], right);
        if (left_tier != right_tier) return left_tier < right_tier;
        if (left_tier == 1 && original_consumed[left] != original_consumed[right])
            return original_consumed[left] > original_consumed[right];
        if (left_tier == 0 && !original_confidence.empty() &&
            original_confidence[left] != original_confidence[right])
            return original_confidence[left] < original_confidence[right];
        const auto left_rank = model_rank(original_candidates[left]);
        const auto right_rank = model_rank(original_candidates[right]);
        if (left_rank != right_rank) return left_rank < right_rank;
        return left < right;
    });

    ordered_candidates.clear();
    ordered_consumed.clear();
    ordered_candidates.reserve(indices.size());
    ordered_consumed.reserve(indices.size());
    for (const auto index : indices) {
        ordered_candidates.push_back(original_candidates[index]);
        ordered_consumed.push_back(original_consumed[index]);
    }
}

// A model's noun/compound preference must not erase the independently scored
// base ending when only the final character differs. Retain it as the second
// selectable sentence; never replace the model leader or enlarge the quota.
inline void retain_base_sentence_ending(
    const std::vector<std::string>& base, const std::vector<std::uint64_t>& base_consumed,
    const std::vector<bool>& generated, const std::size_t full_input_bytes,
    std::vector<std::string>& candidates, std::vector<std::uint64_t>& consumed) {
    if (base.empty() || base_consumed.size() != base.size() || generated.size() != base.size() ||
        !generated.front() || base_consumed.front() != full_input_bytes || candidates.size() < 2 ||
        consumed.size() != candidates.size() || consumed.front() != full_input_bytes) return;
    const auto prefix = [](std::string_view text) {
        if (text.empty()) return text;
        auto end = text.size() - 1;
        while (end > 0 && (static_cast<unsigned char>(text[end]) & 0xc0U) == 0x80U) --end;
        return text.substr(0, end);
    };
    std::size_t reference = 0;
    if (candidates.front() == base.front()) {
        if (base.size() < 2 || !generated[1] || base_consumed[1] != full_input_bytes ||
            prefix(base[1]) != prefix(base.front())) return;
        reference = 1;
    }
    if (utf8_character_count(base[reference]) < 3 || prefix(candidates.front()) != prefix(base[reference])) return;
    const auto found = std::find(candidates.begin() + 1, candidates.end(), base[reference]);
    if (found == candidates.end()) return;
    const auto index = static_cast<std::size_t>(found - candidates.begin());
    if (consumed[index] != full_input_bytes) return;
    std::rotate(candidates.begin() + 1, candidates.begin() + index, candidates.begin() + index + 1);
    std::rotate(consumed.begin() + 1, consumed.begin() + index, consumed.begin() + index + 1);
}

// A joined area must not turn prefix slots into unlimited generated sentences
// when the model promotes its hidden pool. Whole dictionary words do not count.
inline void limit_model_sentence_candidates(
    const std::vector<std::string>& original_candidates,
    const std::vector<std::uint64_t>& original_consumed,
    const std::vector<bool>& generated_sentences, std::size_t limit,
    std::vector<std::string>& candidates, std::vector<std::uint64_t>& consumed,
    const std::vector<std::uint64_t>& sentence_segments = {},
    const std::vector<std::string>& sentence_readings = {},
    const bool diversify_pages = false,
    const std::vector<std::string>& head_identities = {}) {
    if (original_candidates.size() != original_consumed.size() ||
        generated_sentences.size() != original_candidates.size() ||
        candidates.size() != consumed.size() ||
        (!sentence_segments.empty() && sentence_segments.size() != original_candidates.size()) ||
        (!sentence_readings.empty() && sentence_readings.size() != original_candidates.size()) ||
        (!head_identities.empty() && head_identities.size() != original_candidates.size())) return;
    if (diversify_pages && !sentence_readings.empty()) {
        std::vector<std::size_t> slots;
        std::vector<std::string> readings, texts;
        std::vector<std::uint64_t> ranges;
        for (std::size_t index = 0; index < candidates.size(); ++index) {
            for (std::size_t source = 0; source < original_candidates.size(); ++source) {
                if (candidates[index] != original_candidates[source] || consumed[index] != original_consumed[source]) continue;
                if (generated_sentences[source] && !sentence_readings[source].empty()) {
                    slots.push_back(index); readings.push_back(sentence_readings[source]);
                    texts.push_back(candidates[index]); ranges.push_back(consumed[index]);
                }
                break;
            }
        }
        const auto order = owo::engine::sentence_reading_page_order(readings);
        for (std::size_t index = 0; index < slots.size(); ++index) {
            candidates[slots[index]] = std::move(texts[order[index]]);
            consumed[slots[index]] = ranges[order[index]];
        }
    }
    std::size_t kept_sentences = 0, destination = 0;
    std::uint64_t first_segments = 0;
    std::string first_sentence;
    struct ReadingState { std::size_t count{}; std::uint64_t segments{}; std::string first, head_identity; };
    std::unordered_map<std::string, ReadingState> readings;
    for (std::size_t source = 0; source < candidates.size(); ++source) {
        bool generated = false;
        std::uint64_t segments = 0;
        std::string reading;
        std::string head_identity;
        for (std::size_t index = 0; index < original_candidates.size(); ++index) {
            if (candidates[source] == original_candidates[index] && consumed[source] == original_consumed[index]) {
                generated = generated_sentences[index];
                if (!sentence_segments.empty()) segments = sentence_segments[index];
                if (!sentence_readings.empty()) reading = sentence_readings[index];
                if (!head_identities.empty()) head_identity = head_identities[index];
                break;
            }
        }
        if (generated) {
            if (!sentence_readings.empty()) {
                if (reading.empty()) continue;
                auto& state = readings[reading];
                if (state.count >= 2 || kept_sentences >= limit ||
                    (state.count != 0 && state.segments != 0 && segments > state.segments) ||
                    (state.count != 0 && owo::engine::redundant_sentence_variant(state.first, candidates[source]) &&
                     (head_identity.empty() || head_identity != state.head_identity))) continue;
                if (state.count++ == 0) { state.segments = segments; state.first = candidates[source]; state.head_identity = head_identity; }
                ++kept_sentences;
            } else {
                // Legacy callers without reading metadata retain their old
                // single-source fragmentation and redundancy checks.
                if (kept_sentences != 0 && first_segments != 0 && segments > first_segments) continue;
                if (kept_sentences != 0 && owo::engine::redundant_sentence_variant(first_sentence, candidates[source])) continue;
                if (kept_sentences++ >= limit) continue;
                if (kept_sentences == 1) { first_segments = segments; first_sentence = candidates[source]; }
            }
        }
        if (destination != source) candidates[destination] = std::move(candidates[source]);
        consumed[destination++] = consumed[source];
    }
    candidates.resize(destination);
    consumed.resize(destination);
}

inline void promote_preferred_candidates(
    const std::vector<std::string>& preferences,
    std::vector<std::string>& candidates,
    std::vector<std::uint64_t>& consumed) {
    if (candidates.size() != consumed.size()) return;
    for (auto preference = preferences.rbegin(); preference != preferences.rend();
         ++preference) {
        const auto found = std::find(candidates.begin(), candidates.end(), *preference);
        if (found == candidates.end()) continue;
        const auto index = static_cast<std::size_t>(found - candidates.begin());
        const auto tier = consumed[index];
        const auto tier_begin = std::find(consumed.begin(), consumed.begin() + index, tier);
        if (tier_begin == consumed.begin() + index) continue;
        const auto destination = static_cast<std::size_t>(tier_begin - consumed.begin());
        auto text = std::move(candidates[index]);
        candidates.erase(candidates.begin() + static_cast<std::ptrdiff_t>(index));
        consumed.erase(consumed.begin() + static_cast<std::ptrdiff_t>(index));
        candidates.insert(candidates.begin() + static_cast<std::ptrdiff_t>(destination),
                          std::move(text));
        consumed.insert(consumed.begin() + static_cast<std::ptrdiff_t>(destination), tier);
    }
}

inline void restore_preferred_candidate_positions(
    const std::vector<std::string>& preferences,
    const std::vector<std::string>& base_candidates,
    const std::vector<std::uint64_t>& base_consumed,
    std::vector<std::string>& candidates,
    std::vector<std::uint64_t>& consumed,
    const std::vector<bool>& joint_phrase_partition = {}) {
    if (candidates.size() != consumed.size() ||
        candidates.size() != base_candidates.size() ||
        base_candidates.size() != base_consumed.size()) return;
    if (!joint_phrase_partition.empty() && joint_phrase_partition.size() != base_candidates.size())
        return;
    for (const auto& preference : preferences) {
        const auto base = std::find(base_candidates.begin(), base_candidates.end(),
                                    preference);
        const auto current = std::find(candidates.begin(), candidates.end(), preference);
        if (base == base_candidates.end() || current == candidates.end()) continue;
        const auto base_index = static_cast<std::size_t>(base - base_candidates.begin());
        const auto source = static_cast<std::size_t>(current - candidates.begin());
        const auto consumed_bytes = consumed[source];
        if (base_consumed[base_index] != consumed_bytes) continue;
        // A joint display band must not restore a learned prefix across the
        // whole-input coverage boundary. Preserve learning within its tier.
        const auto tier_rank = static_cast<std::size_t>(std::count(
            base_consumed.begin(),
            base_consumed.begin() + static_cast<std::ptrdiff_t>(base_index),
            consumed_bytes));
        std::size_t destination = candidates.size();
        std::size_t seen = 0;
        for (std::size_t index = 0; index < consumed.size(); ++index) {
            if (consumed[index] != consumed_bytes) continue;
            if (seen++ == tier_rank) {
                destination = index;
                break;
            }
        }
        if (destination == candidates.size()) continue;
        if (source == destination) continue;
        auto text = std::move(candidates[source]);
        candidates.erase(candidates.begin() + static_cast<std::ptrdiff_t>(source));
        consumed.erase(consumed.begin() + static_cast<std::ptrdiff_t>(source));
        candidates.insert(candidates.begin() + static_cast<std::ptrdiff_t>(destination),
                          std::move(text));
        consumed.insert(consumed.begin() + static_cast<std::ptrdiff_t>(destination),
                        consumed_bytes);
    }
}

}  // namespace owo::ipc
