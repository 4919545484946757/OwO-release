#pragma once

#include "owo/engine/lexicon.h"

#include <algorithm>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <unordered_set>
#include <vector>

namespace owo::engine::detail {

struct MixedMatchMetrics {
    std::size_t visits{}, memo_hits{};
};

inline std::optional<std::vector<std::string>> mixed_abbreviation_segments(
    const std::span<const std::string_view> syllables, const std::string_view input,
    MixedMatchMetrics* metrics = nullptr) {
    if (syllables.size() < 2 || input.size() < 2 ||
        syllables.size() > input.size() ||
        input.find('\'') != std::string_view::npos)
        return std::nullopt;

    std::size_t maximum_input = 0;
    for (const auto syllable : syllables) {
        if (syllable.empty()) return std::nullopt;
        maximum_input += syllable.size();
    }
    if (input.size() > maximum_input || !syllables.front().starts_with(input.substr(0, 1)))
        return std::nullopt;
    // All initials have exactly one possible source mapping. Avoid allocating
    // a search chart for the most common abbreviation-index lookup.
    if (syllables.size() == input.size()) {
        bool abbreviated = false;
        for (std::size_t index = 0; index < syllables.size(); ++index) {
            if (syllables[index].front() != input[index]) return std::nullopt;
            abbreviated = abbreviated || syllables[index].size() > 1;
        }
        if (!abbreviated) return std::nullopt;
        std::vector<std::string> segments;
        for (const char value : input) segments.emplace_back(1, value);
        return segments;
    }

    std::vector<std::size_t> suffix_maximum(syllables.size() + 1);
    for (std::size_t index = syllables.size(); index-- > 0;)
        suffix_maximum[index] = suffix_maximum[index + 1] + syllables[index].size();
    std::vector<std::size_t> lengths(syllables.size());
    // Memoize failures, not successes: retain the original longest-prefix-first
    // traversal and therefore the exact same successful source segmentation.
    const auto stride = (input.size() + 1) * 2;
    constexpr std::size_t kMaximumMemoCells = 256 * 1024;
    const bool dense = syllables.size() <= kMaximumMemoCells / stride;
    std::vector<unsigned char> failed(dense ? syllables.size() * stride : 0);
    std::unordered_set<std::uint64_t> sparse_failed;
    const auto visit = [&](const auto& self, const std::size_t syllable_index,
                           const std::size_t input_offset,
                           const bool abbreviated) -> bool {
        if (metrics) ++metrics->visits;
        if (syllable_index == syllables.size())
            return input_offset == input.size() && abbreviated;
        const auto syllables_left = syllables.size() - syllable_index;
        const auto input_left = input.size() - input_offset;
        if (input_left < syllables_left || input_left > suffix_maximum[syllable_index])
            return false;
        const auto state_key = static_cast<std::uint64_t>(syllable_index) * stride +
            input_offset * 2 + abbreviated;
        if (dense ? failed[static_cast<std::size_t>(state_key)] != 0 : sparse_failed.contains(state_key)) {
            if (metrics) ++metrics->memo_hits;
            return false;
        }
        const auto& syllable = syllables[syllable_index];
        const auto maximum = std::min(syllable.size(), input_left - (syllables_left - 1));
        std::size_t matching = 0;
        while (matching < maximum && syllable[matching] == input[input_offset + matching])
            ++matching;
        for (std::size_t consumed = matching; consumed > 0; --consumed) {
            lengths[syllable_index] = consumed;
            if (self(self, syllable_index + 1, input_offset + consumed,
                     abbreviated || consumed < syllable.size()))
                return true;
        }
        if (dense) failed[static_cast<std::size_t>(state_key)] = 1;
        else if (sparse_failed.size() < 4096) sparse_failed.insert(state_key);
        return false;
    };
    if (!visit(visit, 0, 0, false)) return std::nullopt;
    std::vector<std::string> segments;
    segments.reserve(lengths.size());
    std::size_t offset = 0;
    for (const auto length : lengths) {
        segments.emplace_back(input.substr(offset, length));
        offset += length;
    }
    return segments;
}

inline std::optional<std::vector<std::string>> mixed_abbreviation_segments(
    const std::span<const std::string> syllables, const std::string_view input) {
    std::vector<std::string_view> views;
    views.reserve(syllables.size());
    for (const auto& syllable : syllables) views.push_back(syllable);
    return mixed_abbreviation_segments(std::span<const std::string_view>(views), input);
}

inline void retain_best_mixed_matches(std::vector<AbbreviatedLexiconMatch>& matches,
                                      const std::size_t limit) {
    std::sort(matches.begin(), matches.end(), [](const auto& left, const auto& right) {
        const auto all_initials = [](const auto& match) {
            return std::all_of(match.source_segments.begin(),
                               match.source_segments.end(),
                               [](const std::string& segment) {
                                   return segment.size() == 1;
                               });
        };
        const auto left_initials = all_initials(left);
        const auto right_initials = all_initials(right);
        const auto exact_segments = [](const auto& match) {
            std::size_t value = 0;
            const auto shared = std::min(match.entry.syllables.size(),
                                         match.source_segments.size());
            for (std::size_t index = 0; index < shared; ++index)
                value += match.entry.syllables[index].size() ==
                         match.source_segments[index].size();
            return value;
        };
        const auto left_exact = exact_segments(left);
        const auto right_exact = exact_segments(right);
        if (!left_initials && !right_initials && left_exact != right_exact)
            return left_exact > right_exact;
        const auto completion = [](const auto& match) {
            std::size_t value = 0;
            const auto shared = std::min(match.entry.syllables.size(),
                                         match.source_segments.size());
            for (std::size_t index = 0; index < shared; ++index)
                value += match.entry.syllables[index].size() -
                         match.source_segments[index].size();
            return value;
        };
        const auto left_completion = completion(left);
        const auto right_completion = completion(right);
        if (left_completion != right_completion)
            return left_completion < right_completion;
        if (left.entry.syllables.size() != right.entry.syllables.size())
            return left.entry.syllables.size() < right.entry.syllables.size();
        if (left.entry.frequency != right.entry.frequency)
            return left.entry.frequency > right.entry.frequency;
        return left.entry.text < right.entry.text;
    });
    if (matches.size() > limit) matches.resize(limit);
}

inline void retain_best_mixed_matches(
    std::vector<AbbreviatedLexiconTextMatch>& matches,
    const std::size_t limit) {
    std::sort(matches.begin(), matches.end(), [](const auto& left, const auto& right) {
        const auto all_initials = [](const auto& match) {
            return std::all_of(match.source_segments.begin(),
                               match.source_segments.end(),
                               [](const std::string& segment) {
                                   return segment.size() == 1;
                               });
        };
        const auto left_initials = all_initials(left);
        const auto right_initials = all_initials(right);
        const auto exact_segments = [](const auto& match) {
            std::size_t value = 0;
            const auto shared = std::min(match.syllables.size(),
                                         match.source_segments.size());
            for (std::size_t index = 0; index < shared; ++index)
                value += match.syllables[index].size() ==
                         match.source_segments[index].size();
            return value;
        };
        const auto left_exact = exact_segments(left);
        const auto right_exact = exact_segments(right);
        if (!left_initials && !right_initials && left_exact != right_exact)
            return left_exact > right_exact;
        const auto completion = [](const auto& match) {
            std::size_t value = 0;
            const auto shared = std::min(match.syllables.size(),
                                         match.source_segments.size());
            for (std::size_t index = 0; index < shared; ++index)
                value += match.syllables[index].size() -
                         match.source_segments[index].size();
            return value;
        };
        const auto left_completion = completion(left);
        const auto right_completion = completion(right);
        if (left_completion != right_completion)
            return left_completion < right_completion;
        if (left.syllables.size() != right.syllables.size())
            return left.syllables.size() < right.syllables.size();
        if (left.frequency != right.frequency)
            return left.frequency > right.frequency;
        return left.text < right.text;
    });
    if (matches.size() > limit) matches.resize(limit);
}

}  // namespace owo::engine::detail
