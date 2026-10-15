#include "owo/engine/lexicon.h"

#include "lexicon_match.h"

#include <algorithm>
#include <utility>

namespace owo::engine {

MemoryLexicon::MemoryLexicon(std::vector<LexiconEntry> entries) : entries_(std::move(entries)) {
    for (const auto& entry : entries_)
        maximum_reading_length_ = std::max(maximum_reading_length_, entry.syllables.size());
}

std::vector<LexiconEntry> MemoryLexicon::lookup(
    const std::span<const std::string_view> syllables) const {
    std::vector<LexiconEntry> matches;
    for (const auto& entry : entries_) {
        if (entry.syllables.size() != syllables.size()) continue;
        if (std::equal(entry.syllables.begin(), entry.syllables.end(), syllables.begin(),
                       [](const std::string& left, const std::string_view right) {
                           return left == right;
                       })) {
            matches.push_back(entry);
        }
    }
    return matches;
}

std::vector<LexiconTextEntry> MemoryLexicon::lookup_texts(
    const std::span<const std::string_view> syllables) const {
    std::vector<LexiconTextEntry> matches;
    for (const auto& entry : entries_) {
        if (entry.syllables.size() != syllables.size()) continue;
        if (std::equal(entry.syllables.begin(), entry.syllables.end(), syllables.begin(),
                       [](const std::string& left, const std::string_view right) {
                           return left == right;
                       })) {
            matches.push_back({entry.text, entry.frequency});
        }
    }
    return matches;
}

std::uint32_t MemoryLexicon::lookup_max_frequency(
    const std::span<const std::string_view> syllables) const {
    std::uint32_t maximum = 0;
    for (const auto& entry : entries_) {
        if (entry.syllables.size() != syllables.size()) continue;
        if (std::equal(entry.syllables.begin(), entry.syllables.end(), syllables.begin(),
                       [](const std::string& left, const std::string_view right) {
                           return left == right;
                       })) {
            maximum = std::max(maximum, entry.frequency);
        }
    }
    return maximum;
}

std::vector<LexiconEntry> MemoryLexicon::lookup_initial(const char initial,
                                                        const std::size_t limit) const {
    std::vector<LexiconEntry> matches;
    for (const auto& entry : entries_) {
        if (entry.syllables.size() == 1 && !entry.syllables.front().empty() &&
            entry.syllables.front().front() == initial)
            matches.push_back(entry);
    }
    std::sort(matches.begin(), matches.end(), [](const auto& left, const auto& right) {
        if (left.frequency != right.frequency) return left.frequency > right.frequency;
        if (left.syllables.front().size() != right.syllables.front().size())
            return left.syllables.front().size() < right.syllables.front().size();
        return left.text < right.text;
    });
    if (matches.size() > limit) matches.resize(limit);
    return matches;
}

std::vector<InitialLexiconTextEntry> MemoryLexicon::lookup_initial_texts(
    const char initial, const std::size_t limit) const {
    std::vector<InitialLexiconTextEntry> matches;
    for (const auto& entry : entries_) {
        if (entry.syllables.size() == 1 && !entry.syllables.front().empty() &&
            entry.syllables.front().front() == initial) {
            matches.push_back(
                {entry.syllables.front(), entry.text, entry.frequency});
        }
    }
    std::sort(matches.begin(), matches.end(), [](const auto& left, const auto& right) {
        if (left.frequency != right.frequency) return left.frequency > right.frequency;
        if (left.syllable.size() != right.syllable.size())
            return left.syllable.size() < right.syllable.size();
        return left.text < right.text;
    });
    if (matches.size() > limit) matches.resize(limit);
    return matches;
}

std::vector<AbbreviatedLexiconMatch> MemoryLexicon::lookup_mixed_abbreviation(
    const std::string_view input, const std::size_t limit) const {
    if (limit == 0) return {};
    std::vector<AbbreviatedLexiconMatch> matches;
    for (const auto& entry : entries_) {
        if (input.size() == 2 && input.find('\'') == std::string_view::npos) {
            if (entry.syllables.size() != 2 || entry.syllables[0].empty() ||
                entry.syllables[1].empty() || entry.syllables[0].front() != input[0] ||
                entry.syllables[1].front() != input[1])
                continue;
            matches.push_back({entry, {std::string(1, input[0]),
                                       std::string(1, input[1])}});
        } else {
            const auto segments = detail::mixed_abbreviation_segments(entry.syllables, input);
            if (!segments) continue;
            matches.push_back({entry, *segments});
        }
    }
    detail::retain_best_mixed_matches(matches, limit);
    return matches;
}

std::vector<AbbreviatedLexiconTextMatch>
MemoryLexicon::lookup_mixed_abbreviation_texts(
    const std::string_view input, const std::size_t limit) const {
    if (limit == 0) return {};
    std::vector<AbbreviatedLexiconTextMatch> matches;
    for (const auto& entry : entries_) {
        std::vector<std::string> segments;
        if (input.size() == 2 && input.find('\'') == std::string_view::npos) {
            if (entry.syllables.size() != 2 || entry.syllables[0].empty() ||
                entry.syllables[1].empty() || entry.syllables[0].front() != input[0] ||
                entry.syllables[1].front() != input[1])
                continue;
            segments = {std::string(1, input[0]), std::string(1, input[1])};
        } else {
            const auto matched = detail::mixed_abbreviation_segments(entry.syllables, input);
            if (!matched) continue;
            segments = *matched;
        }
        std::vector<std::string_view> syllables;
        syllables.reserve(entry.syllables.size());
        for (const auto& syllable : entry.syllables) syllables.push_back(syllable);
        matches.push_back({std::move(syllables), entry.text, entry.frequency,
                           std::move(segments)});
    }
    detail::retain_best_mixed_matches(matches, limit);
    return matches;
}

}  // namespace owo::engine
