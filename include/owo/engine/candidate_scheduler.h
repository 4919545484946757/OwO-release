#pragma once

#include "owo/engine/candidate_generator.h"

#include <cstddef>
#include <cstdint>
#include <limits>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {

struct GenerationBudget {
    std::size_t max_results{};
    std::size_t max_work_units{};
};

struct CandidateRequestContext {
    std::string input;
    std::uint64_t input_fingerprint{};
    std::uint64_t lexicon_generation{};
    std::uint64_t parse_generation{};
};

// Provider-owned state for unvisited search work. The identity fields make a
// continuation unusable after the input, lexicon, parse graph, or provider has
// changed. opaque_state is deliberately not defined as an offset: a provider
// may store a trie traversal stack, beam frontier, heap cursor, or model state.
struct ProviderContinuation {
    std::string provider_id;
    std::uint64_t input_fingerprint{};
    std::uint64_t lexicon_generation{};
    std::uint64_t parse_generation{};
    std::vector<std::uint64_t> opaque_state;
};

struct CandidateBatch {
    std::vector<Candidate> candidates;
    std::size_t work_units_consumed{};
    bool exhausted{};
    std::int64_t next_score_upper_bound{
        (std::numeric_limits<std::int64_t>::min)()};
    std::optional<ProviderContinuation> continuation;
};

class CandidateProvider {
public:
    virtual ~CandidateProvider() = default;
    [[nodiscard]] virtual std::string_view id() const noexcept = 0;
    [[nodiscard]] virtual std::int64_t initial_score_upper_bound(
        const CandidateRequestContext&) const noexcept {
        return (std::numeric_limits<std::int64_t>::max)();
    }
    [[nodiscard]] virtual CandidateBatch request(
        const CandidateRequestContext& context,
        GenerationBudget budget,
        const ProviderContinuation* continuation) = 0;
};

struct ProviderAllocation {
    CandidateProvider* provider{};
    std::size_t weight{1};
};

struct SchedulerMetrics {
    std::size_t provider_requests{};
    std::size_t continuation_requests{};
    std::size_t result_slots_reclaimed{};
    std::size_t reallocated_result_budget{};
    std::size_t candidates_generated_after_reallocation{};
    std::size_t work_units_consumed{};
    std::size_t work_units_reclaimed{};
    std::size_t reallocated_work_budget{};
    std::size_t work_units_consumed_after_reallocation{};
    std::size_t duplicate_candidates_discarded{};
};

struct ScheduledProviderState {
    std::string provider_id;
    std::size_t weight{1};
    bool started{};
    bool exhausted{};
    std::int64_t next_score_upper_bound{
        (std::numeric_limits<std::int64_t>::min)()};
    std::optional<ProviderContinuation> continuation;
};

// Search frontier returned to the caller for the next page. It is deliberately
// tied to every source of candidate identity so stale page state cannot be
// reused after an edit, lexicon reload, or parse-graph replacement.
struct CandidateScheduleContinuation {
    std::string input;
    std::uint64_t input_fingerprint{};
    std::uint64_t lexicon_generation{};
    std::uint64_t parse_generation{};
    std::vector<ScheduledProviderState> providers;
    std::vector<std::string> seen_candidate_texts;
};

struct ScheduledCandidates {
    std::vector<Candidate> candidates;
    SchedulerMetrics metrics;
    std::optional<CandidateScheduleContinuation> continuation;
};

class CandidateScheduler {
public:
    [[nodiscard]] ScheduledCandidates generate(
        const CandidateRequestContext& context,
        std::span<const ProviderAllocation> providers,
        GenerationBudget total_budget) const;
    [[nodiscard]] ScheduledCandidates generate(
        const CandidateRequestContext& context,
        std::span<const ProviderAllocation> providers,
        GenerationBudget total_budget,
        const CandidateScheduleContinuation* continuation) const;
};

}  // namespace owo::engine
