#pragma once

#include "owo/engine/binary_lexicon.h"
#include "owo/engine/candidate_scheduler.h"

#include <utility>

namespace owo::engine {

enum class InitialCandidateFilter {
    none,
    single_character
};

enum class ExactCandidateFilter {
    none,
    single_character,
    multiple_characters
};

// Resumable provider backed by the pre-sorted single-initial index in a V2
// binary lexicon. One work unit is one index entry visited and materialized.
class InitialLexiconCandidateProvider final : public CandidateProvider {
public:
    explicit InitialLexiconCandidateProvider(
        const BinaryLexicon& lexicon,
        const InitialCandidateFilter filter = InitialCandidateFilter::none)
        : lexicon_(lexicon), filter_(filter) {}

    [[nodiscard]] std::string_view id() const noexcept override {
        return filter_ == InitialCandidateFilter::single_character
            ? "binary-initial-character" : "binary-initial";
    }
    [[nodiscard]] CandidateBatch request(
        const CandidateRequestContext& context,
        GenerationBudget budget,
        const ProviderContinuation* continuation) override;

private:
    const BinaryLexicon& lexicon_;
    InitialCandidateFilter filter_{};
};

// Resumable exact-reading source for V3 lexicons, whose entries are ordered by
// frequency inside each reading range. Different instances can expose a full
// dictionary-word range and the leading-syllable character range as separate
// budget participants.
class ExactLexiconCandidateProvider final : public CandidateProvider {
public:
    ExactLexiconCandidateProvider(
        const BinaryLexicon& lexicon, std::string provider_id,
        std::vector<std::string> syllables,
        std::vector<std::string> source_segments,
        InputMatchKind match_kind, std::size_t consumed_input_bytes,
        ExactCandidateFilter filter = ExactCandidateFilter::none,
        std::int64_t score_adjustment = 0)
        : lexicon_(lexicon), provider_id_(std::move(provider_id)),
          syllables_(std::move(syllables)),
          source_segments_(std::move(source_segments)), match_kind_(match_kind),
          consumed_input_bytes_(consumed_input_bytes), filter_(filter),
          score_adjustment_(score_adjustment) {}

    [[nodiscard]] std::string_view id() const noexcept override {
        return provider_id_;
    }
    [[nodiscard]] std::int64_t initial_score_upper_bound(
        const CandidateRequestContext&) const noexcept override;
    [[nodiscard]] CandidateBatch request(
        const CandidateRequestContext& context,
        GenerationBudget budget,
        const ProviderContinuation* continuation) override;

private:
    const BinaryLexicon& lexicon_;
    std::string provider_id_;
    std::vector<std::string> syllables_;
    std::vector<std::string> source_segments_;
    InputMatchKind match_kind_{};
    std::size_t consumed_input_bytes_{};
    ExactCandidateFilter filter_{};
    std::int64_t score_adjustment_{};
};

}  // namespace owo::engine
