#pragma once

#include "owo/engine/binary_lexicon.h"
#include "owo/engine/candidate_scheduler.h"

#include <optional>

namespace owo::engine {

// Session-lifetime resumable dynamic program. One work unit evaluates one
// reading edge and materializes at most its highest-frequency dictionary entry.
// The first implementation deliberately returns only the strongest composed
// sentence; alternative enumeration is a separate later frontier.
class CompositionCandidateProvider final : public CandidateProvider {
public:
    CompositionCandidateProvider(
        const BinaryLexicon& lexicon, std::string provider_id,
        std::vector<std::string> syllables,
        std::vector<std::string> source_segments,
        InputMatchKind match_kind, std::size_t consumed_input_bytes,
        std::int64_t base_score);

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
    struct State {
        std::string text;
        std::vector<std::string> syllables;
        std::vector<std::size_t> segment_lengths;
        std::int64_t score{(std::numeric_limits<std::int64_t>::min)()};
        std::size_t segment_count{};
    };

    const BinaryLexicon& lexicon_;
    std::string provider_id_;
    std::vector<std::string> syllables_;
    std::vector<std::string> source_segments_;
    InputMatchKind match_kind_{};
    std::size_t consumed_input_bytes_{};
    std::int64_t base_score_{};
    std::vector<std::optional<State>> best_;
    std::size_t begin_{};
    std::size_t end_{1};
    bool initialized_{};
    bool emitted_{};
};

}  // namespace owo::engine
