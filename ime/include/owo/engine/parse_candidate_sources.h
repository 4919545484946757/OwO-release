#pragma once

#include "owo/engine/candidate_scheduler.h"
#include "owo/engine/input_schema.h"
#include "owo/engine/lexicon_candidate_provider.h"

#include <memory>

namespace owo::engine {

struct ParseCandidateSourcePlan {
    CandidateRequestContext context;
    std::vector<std::unique_ptr<CandidateProvider>> owned_providers;
    std::vector<ProviderAllocation> allocations;
};

// Builds bounded providers exclusively from parser-produced paths. Surface
// input is used only for source ranges and continuation identity; syllable
// boundaries always come from ParsePath.
[[nodiscard]] ParseCandidateSourcePlan make_parse_candidate_source_plan(
    const BinaryLexicon& lexicon, const ParseResult& parsed,
    std::uint64_t input_fingerprint, std::uint64_t lexicon_generation,
    std::uint64_t parse_generation, std::size_t maximum_paths = 8,
    bool include_experimental_composition = false);

struct ShadowCandidateMetrics {
    std::size_t legacy_count{};
    std::size_t scheduled_count{};
    std::size_t shared_count{};
    std::size_t equal_prefix_count{};
    bool first_candidate_equal{};
    bool scheduled_complete{};
};

// Produces aggregate comparison values only. It does not retain candidate text
// or user input, making it suitable for an opt-in local shadow evaluator.
[[nodiscard]] ShadowCandidateMetrics compare_candidate_results(
    std::span<const Candidate> legacy, std::span<const Candidate> scheduled,
    std::size_t visible_limit);

}  // namespace owo::engine
