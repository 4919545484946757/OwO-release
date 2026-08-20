#include "owo/engine/lexicon_candidate_provider.h"
#include "owo/engine/candidate_scoring.h"

#include <algorithm>

namespace owo::engine {
namespace {

bool is_single_utf8_character(const std::string_view text) {
    if (text.empty()) return false;
    const auto lead = static_cast<unsigned char>(text.front());
    const std::size_t bytes = lead < 0x80 ? 1 : lead < 0xE0 ? 2 :
                              lead < 0xF0 ? 3 : lead < 0xF8 ? 4 : 0;
    if (bytes == 0 || text.size() != bytes) return false;
    for (std::size_t index = 1; index < bytes; ++index) {
        if ((static_cast<unsigned char>(text[index]) & 0xC0) != 0x80) return false;
    }
    return true;
}

}  // namespace

CandidateBatch InitialLexiconCandidateProvider::request(
    const CandidateRequestContext& context, const GenerationBudget budget,
    const ProviderContinuation* continuation) {
    CandidateBatch batch;
    if (context.input.empty() || budget.max_results == 0 ||
        budget.max_work_units == 0) {
        batch.exhausted = context.input.empty();
        return batch;
    }
    std::size_t start_index = 0;
    if (continuation != nullptr) {
        const bool valid = continuation->provider_id == id() &&
                           continuation->input_fingerprint == context.input_fingerprint &&
                           continuation->lexicon_generation == context.lexicon_generation &&
                           continuation->parse_generation == context.parse_generation &&
                           continuation->opaque_state.size() == 1;
        if (!valid) {
            batch.exhausted = true;
            return batch;
        }
        start_index = static_cast<std::size_t>(continuation->opaque_state.front());
    }
    batch.candidates.reserve(budget.max_results);
    auto next_index = start_index;
    std::uint32_t next_frequency = 0;
    while (batch.candidates.size() < budget.max_results &&
           batch.work_units_consumed < budget.max_work_units) {
        const auto lookup = lexicon_.lookup_initial_bounded(
            context.input.front(), next_index, 1, 1);
        batch.work_units_consumed += lookup.work_units_consumed;
        next_index = lookup.next_index;
        next_frequency = lookup.next_frequency;
        if (lookup.entries.empty()) {
            batch.exhausted = lookup.exhausted;
            break;
        }
        const auto& entry = lookup.entries.front();
        if (filter_ == InitialCandidateFilter::single_character &&
            (entry.syllables.size() != 1 || !is_single_utf8_character(entry.text))) {
            batch.exhausted = lookup.exhausted;
            if (batch.exhausted) break;
            continue;
        }
        Candidate candidate;
        candidate.text = entry.text;
        candidate.syllables = entry.syllables;
        candidate.score = entry.frequency;
        candidate.match_kind = InputMatchKind::incomplete_completion;
        candidate.source_segments = {context.input.substr(0, 1)};
        candidate.consumed_input_bytes = 1;
        batch.candidates.push_back(std::move(candidate));
        batch.exhausted = lookup.exhausted;
        if (batch.exhausted) break;
    }
    batch.next_score_upper_bound = batch.exhausted
        ? (std::numeric_limits<std::int64_t>::min)()
        : static_cast<std::int64_t>(next_frequency);
    if (!batch.exhausted) {
        batch.continuation = ProviderContinuation{
            std::string(id()), context.input_fingerprint,
            context.lexicon_generation, context.parse_generation,
            {next_index}};
    }
    return batch;
}

CandidateBatch ExactLexiconCandidateProvider::request(
    const CandidateRequestContext& context, const GenerationBudget budget,
    const ProviderContinuation* continuation) {
    CandidateBatch batch;
    if (!lexicon_.exact_lookup_frequency_ordered() || provider_id_.empty() ||
        syllables_.empty() || budget.max_results == 0 || budget.max_work_units == 0) {
        batch.exhausted = true;
        return batch;
    }
    std::size_t start_index = 0;
    if (continuation != nullptr) {
        const bool valid = continuation->provider_id == id() &&
                           continuation->input_fingerprint == context.input_fingerprint &&
                           continuation->lexicon_generation == context.lexicon_generation &&
                           continuation->parse_generation == context.parse_generation &&
                           continuation->opaque_state.size() == 1;
        if (!valid) {
            batch.exhausted = true;
            return batch;
        }
        start_index = static_cast<std::size_t>(continuation->opaque_state.front());
    }
    std::vector<std::string_view> reading;
    reading.reserve(syllables_.size());
    for (const auto& syllable : syllables_) reading.push_back(syllable);

    auto next_index = start_index;
    std::uint32_t next_frequency = 0;
    batch.candidates.reserve(budget.max_results);
    while (batch.candidates.size() < budget.max_results &&
           batch.work_units_consumed < budget.max_work_units) {
        const auto lookup = lexicon_.lookup_bounded(reading, next_index, 1, 1);
        batch.work_units_consumed += lookup.work_units_consumed;
        next_index = lookup.next_index;
        next_frequency = lookup.next_frequency;
        if (lookup.entries.empty()) {
            batch.exhausted = lookup.exhausted;
            break;
        }
        const auto& entry = lookup.entries.front();
        const bool single_character = is_single_utf8_character(entry.text);
        const bool filtered =
            (filter_ == ExactCandidateFilter::single_character && !single_character) ||
            (filter_ == ExactCandidateFilter::multiple_characters && single_character);
        batch.exhausted = lookup.exhausted;
        if (!filtered) {
            Candidate candidate;
            candidate.text = entry.text;
            candidate.syllables = entry.syllables;
            candidate.score = candidate_unigram_score(entry.frequency) +
                              score_adjustment_;
            candidate.match_kind = match_kind_;
            candidate.source_segments = source_segments_;
            candidate.consumed_input_bytes = consumed_input_bytes_;
            batch.candidates.push_back(std::move(candidate));
        }
        if (batch.exhausted) break;
    }
    batch.next_score_upper_bound = batch.exhausted
        ? (std::numeric_limits<std::int64_t>::min)()
        : candidate_unigram_score(next_frequency) + score_adjustment_;
    if (!batch.exhausted) {
        batch.continuation = ProviderContinuation{
            provider_id_, context.input_fingerprint, context.lexicon_generation,
            context.parse_generation, {next_index}};
    }
    return batch;
}

std::int64_t ExactLexiconCandidateProvider::initial_score_upper_bound(
    const CandidateRequestContext&) const noexcept {
    std::vector<std::string_view> reading;
    reading.reserve(syllables_.size());
    for (const auto& syllable : syllables_) reading.push_back(syllable);
    const auto frequency = lexicon_.exact_frequency_upper_bound(reading);
    return frequency
        ? candidate_unigram_score(*frequency) + score_adjustment_
        : (std::numeric_limits<std::int64_t>::min)();
}

}  // namespace owo::engine
