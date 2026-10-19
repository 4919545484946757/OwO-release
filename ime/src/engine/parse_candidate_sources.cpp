#include "owo/engine/parse_candidate_sources.h"
#include "owo/engine/candidate_scoring.h"
#include "owo/engine/composition_candidate_provider.h"

#include <algorithm>
#include <unordered_set>

namespace owo::engine {
namespace {

std::vector<std::string> source_segments(const ParseResult& parsed,
                                         const ParsePath& path) {
    std::vector<std::string> result;
    result.reserve(path.syllables.size());
    for (const auto& syllable : path.syllables) {
        if (syllable.begin >= syllable.end ||
            syllable.end > parsed.normalized_input.size()) return {};
        result.push_back(parsed.normalized_input.substr(
            syllable.begin, syllable.end - syllable.begin));
    }
    return result;
}

std::int64_t match_adjustment(const ParsePath& path) {
    return -candidate_input_match_penalty(path);
}

std::size_t match_weight(const InputMatchKind kind) {
    switch (kind) {
    case InputMatchKind::exact: return 4;
    case InputMatchKind::incomplete_completion: return 2;
    case InputMatchKind::corrected: return 1;
    case InputMatchKind::abbreviated_completion: return 1;
    }
    return 1;
}

}  // namespace

ParseCandidateSourcePlan make_parse_candidate_source_plan(
    const BinaryLexicon& lexicon, const ParseResult& parsed,
    const std::uint64_t input_fingerprint,
    const std::uint64_t lexicon_generation,
    const std::uint64_t parse_generation,
    const std::size_t maximum_paths,
    const bool include_experimental_composition) {
    ParseCandidateSourcePlan plan;
    plan.context = {parsed.normalized_input, input_fingerprint,
                    lexicon_generation, parse_generation};
    if (!parsed.valid || !lexicon.exact_lookup_frequency_ordered() ||
        maximum_paths == 0) return plan;

    std::unordered_set<std::string> readings;
    std::size_t path_index = 0;
    std::size_t composition_sources = 0;
    for (const auto& path : parsed.paths) {
        if (path_index >= maximum_paths) break;
        if (path.syllables.empty() ||
            std::any_of(path.syllables.begin(), path.syllables.end(),
                        [](const Syllable& syllable) { return !syllable.complete; }))
            continue;
        auto segments = source_segments(parsed, path);
        if (segments.size() != path.syllables.size()) continue;
        std::vector<std::string> syllables;
        syllables.reserve(path.syllables.size());
        std::string reading_key;
        for (const auto& syllable : path.syllables) {
            syllables.push_back(syllable.text);
            reading_key += syllable.text;
            reading_key.push_back('\0');
        }
        reading_key.push_back(static_cast<char>(path.match_kind));
        if (!readings.insert(reading_key).second) continue;

        const auto current_path_index = path_index++;
        const auto id = "parse-path-" + std::to_string(current_path_index);
        auto provider = std::make_unique<ExactLexiconCandidateProvider>(
            lexicon, id, std::move(syllables), std::move(segments),
            path.match_kind, path.syllables.back().end,
            ExactCandidateFilter::multiple_characters,
            match_adjustment(path) + kCandidateDictionaryWordSchedulingBonus);
        plan.allocations.push_back({provider.get(), match_weight(path.match_kind)});
        plan.owned_providers.push_back(std::move(provider));

        if (include_experimental_composition && path.syllables.size() >= 2 &&
            composition_sources < 4) {
            std::vector<std::string> composition_syllables;
            std::vector<std::string> composition_segments;
            composition_syllables.reserve(path.syllables.size());
            composition_segments.reserve(path.syllables.size());
            for (std::size_t index = 0; index < path.syllables.size(); ++index) {
                composition_syllables.push_back(path.syllables[index].text);
                composition_segments.push_back(parsed.normalized_input.substr(
                    path.syllables[index].begin,
                    path.syllables[index].end - path.syllables[index].begin));
            }
            auto composition = std::make_unique<CompositionCandidateProvider>(
                lexicon,
                "parse-composition-" + std::to_string(current_path_index),
                std::move(composition_syllables), std::move(composition_segments),
                path.match_kind, path.syllables.back().end,
                match_adjustment(path));
            plan.allocations.push_back({composition.get(), 5});
            plan.owned_providers.push_back(std::move(composition));
            ++composition_sources;
        }

        // Leading dictionary ranges are independently consumable candidates.
        // Limit them to the three longest proper prefixes per parse path so a
        // long sentence cannot create an unbounded number of providers.
        constexpr std::size_t kMaximumPrefixSourcesPerPath = 3;
        std::size_t prefix_sources = 0;
        for (std::size_t end = path.syllables.size();
             end > 2 && prefix_sources < kMaximumPrefixSourcesPerPath;) {
            --end;
            std::vector<std::string> prefix_syllables;
            std::vector<std::string> prefix_segments;
            prefix_syllables.reserve(end);
            prefix_segments.reserve(end);
            for (std::size_t index = 0; index < end; ++index) {
                prefix_syllables.push_back(path.syllables[index].text);
                prefix_segments.push_back(parsed.normalized_input.substr(
                    path.syllables[index].begin,
                    path.syllables[index].end - path.syllables[index].begin));
            }
            auto prefix = std::make_unique<ExactLexiconCandidateProvider>(
                lexicon, "parse-prefix-" + std::to_string(current_path_index) +
                             "-" + std::to_string(end),
                std::move(prefix_syllables), std::move(prefix_segments),
                path.match_kind, path.syllables[end - 1].end,
                ExactCandidateFilter::multiple_characters,
                match_adjustment(path) + kCandidateDictionaryWordSchedulingBonus);
            plan.allocations.push_back({prefix.get(), 2});
            plan.owned_providers.push_back(std::move(prefix));
            ++prefix_sources;
        }
    }

    const auto preferred = std::find_if(
        parsed.paths.begin(), parsed.paths.end(), [](const ParsePath& path) {
            return !path.syllables.empty() && path.syllables.front().complete &&
                   path.match_kind == InputMatchKind::exact;
        });
    if (preferred != parsed.paths.end()) {
        const auto segments = source_segments(parsed, *preferred);
        if (!segments.empty()) {
            auto provider = std::make_unique<ExactLexiconCandidateProvider>(
                lexicon, "leading-character", std::vector<std::string>{
                    preferred->syllables.front().text},
                std::vector<std::string>{segments.front()}, InputMatchKind::exact,
                preferred->syllables.front().end,
                ExactCandidateFilter::single_character, 0);
            plan.allocations.push_back({provider.get(), 5});
            plan.owned_providers.push_back(std::move(provider));
        }
    }
    return plan;
}

ShadowCandidateMetrics compare_candidate_results(
    const std::span<const Candidate> legacy,
    const std::span<const Candidate> scheduled,
    const std::size_t visible_limit) {
    ShadowCandidateMetrics metrics;
    metrics.legacy_count = std::min(legacy.size(), visible_limit);
    metrics.scheduled_count = std::min(scheduled.size(), visible_limit);
    metrics.scheduled_complete = metrics.scheduled_count == visible_limit;
    metrics.first_candidate_equal = metrics.legacy_count != 0 &&
                                    metrics.scheduled_count != 0 &&
                                    legacy.front().text == scheduled.front().text;
    const auto shared_prefix = std::min(metrics.legacy_count, metrics.scheduled_count);
    while (metrics.equal_prefix_count < shared_prefix &&
           legacy[metrics.equal_prefix_count].text ==
               scheduled[metrics.equal_prefix_count].text)
        ++metrics.equal_prefix_count;
    std::unordered_set<std::string_view> legacy_texts;
    for (std::size_t index = 0; index < metrics.legacy_count; ++index)
        legacy_texts.insert(legacy[index].text);
    for (std::size_t index = 0; index < metrics.scheduled_count; ++index)
        if (legacy_texts.contains(scheduled[index].text)) ++metrics.shared_count;
    return metrics;
}

}  // namespace owo::engine
