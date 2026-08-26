#pragma once

#include "owo/engine/lexicon.h"

#include <algorithm>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine::detail {

inline std::optional<std::vector<std::string>> mixed_abbreviation_segments(
    const std::span<const std::string_view> syllables, const std::string_view input) {
    if (syllables.size() < 2 || input.size() < 2 ||
        input.find('\'') != std::string_view::npos)
        return std::nullopt;

    std::vector<std::string> segments;
    const auto visit = [&](const auto& self, const std::size_t syllable_index,
                           const std::size_t input_offset,
                           const bool abbreviated) -> bool {
        if (syllable_index == syllables.size())
            return input_offset == input.size() && abbreviated;
        const auto syllables_left = syllables.size() - syllable_index;
        const auto input_left = input.size() - input_offset;
        if (input_left < syllables_left) return false;
        const auto& syllable = syllables[syllable_index];
        const auto maximum = std::min(syllable.size(), input_left - (syllables_left - 1));
        for (std::size_t consumed = maximum; consumed > 0; --consumed) {
            const auto source = input.substr(input_offset, consumed);
            if (!syllable.starts_with(source)) continue;
            segments.emplace_back(source);
            if (self(self, syllable_index + 1, input_offset + consumed,
                     abbreviated || consumed < syllable.size()))
                return true;
            segments.pop_back();
        }
        return false;
    };
    if (!visit(visit, 0, 0, false)) return std::nullopt;
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
