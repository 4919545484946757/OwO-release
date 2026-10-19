#include "owo/engine/composition_candidate_provider.h"

#include "owo/engine/candidate_scoring.h"

#include <algorithm>

namespace owo::engine {

CompositionCandidateProvider::CompositionCandidateProvider(
    const BinaryLexicon& lexicon, std::string provider_id,
    std::vector<std::string> syllables,
    std::vector<std::string> source_segments, const InputMatchKind match_kind,
    const std::size_t consumed_input_bytes, const std::int64_t base_score)
    : lexicon_(lexicon), provider_id_(std::move(provider_id)),
      syllables_(std::move(syllables)),
      source_segments_(std::move(source_segments)), match_kind_(match_kind),
      consumed_input_bytes_(consumed_input_bytes), base_score_(base_score) {}

std::int64_t CompositionCandidateProvider::initial_score_upper_bound(
    const CandidateRequestContext&) const noexcept {
    constexpr auto kMaximumEntryScore = std::int64_t{23'000};
    return base_score_ + kCandidateComposedSentenceSchedulingBonus +
           static_cast<std::int64_t>(syllables_.size()) * kMaximumEntryScore;
}

CandidateBatch CompositionCandidateProvider::request(
    const CandidateRequestContext& context, const GenerationBudget budget,
    const ProviderContinuation* continuation) {
    CandidateBatch batch;
    if (emitted_ || syllables_.size() < 2 || budget.max_results == 0 ||
        budget.max_work_units == 0) {
        batch.exhausted = true;
        return batch;
    }
    if (!initialized_) {
        best_.resize(syllables_.size() + 1);
        State initial;
        initial.score = base_score_;
        best_[0] = std::move(initial);
        initialized_ = true;
    }
    if (continuation != nullptr) {
        const bool valid = continuation->provider_id == id() &&
                           continuation->input_fingerprint == context.input_fingerprint &&
                           continuation->lexicon_generation == context.lexicon_generation &&
                           continuation->parse_generation == context.parse_generation &&
                           continuation->opaque_state.size() == 2 &&
                           continuation->opaque_state[0] == begin_ &&
                           continuation->opaque_state[1] == end_;
        if (!valid) {
            batch.exhausted = true;
            return batch;
        }
    }

    while (begin_ < syllables_.size() &&
           batch.work_units_consumed < budget.max_work_units) {
        if (end_ <= begin_) end_ = begin_ + 1;
        if (end_ > syllables_.size()) {
            ++begin_;
            end_ = begin_ + 1;
            continue;
        }
        if (best_[begin_]) {
            if (begin_ == 0 && end_ == syllables_.size()) {
                ++end_;
                continue;
            }
            std::vector<std::string_view> reading;
            reading.reserve(end_ - begin_);
            for (std::size_t index = begin_; index < end_; ++index)
                reading.push_back(syllables_[index]);
            const auto lookup = lexicon_.lookup_bounded(reading, 0, 1, 1);
            ++batch.work_units_consumed;
            if (!lookup.entries.empty()) {
                const auto& entry = lookup.entries.front();
                State next = *best_[begin_];
                next.text += entry.text;
                next.syllables.insert(next.syllables.end(), entry.syllables.begin(),
                                      entry.syllables.end());
                next.segment_lengths.push_back(entry.syllables.size());
                next.score += candidate_unigram_score(entry.frequency);
                if (next.segment_count != 0)
                    next.score -= kCandidateAdditionalSegmentPenalty;
                ++next.segment_count;
                if (!best_[end_] || next.score > best_[end_]->score)
                    best_[end_] = std::move(next);
            }
        }
        ++end_;
    }

    if (begin_ < syllables_.size()) {
        batch.next_score_upper_bound = initial_score_upper_bound(context);
        batch.continuation = ProviderContinuation{
            provider_id_, context.input_fingerprint, context.lexicon_generation,
            context.parse_generation, {begin_, end_}};
        return batch;
    }
    batch.exhausted = true;
    emitted_ = true;
    if (!best_.back() || best_.back()->segment_count < 2) return batch;
    auto& state = *best_.back();
    Candidate candidate;
    candidate.text = std::move(state.text);
    candidate.syllables = std::move(state.syllables);
    candidate.score = state.score + kCandidateComposedSentenceSchedulingBonus;
    candidate.match_kind = match_kind_;
    candidate.source_segments = source_segments_;
    candidate.consumed_input_bytes = consumed_input_bytes_;
    candidate.segment_count = state.segment_count;
    candidate.segment_lengths = std::move(state.segment_lengths);
    batch.candidates.push_back(std::move(candidate));
    return batch;
}

}  // namespace owo::engine
