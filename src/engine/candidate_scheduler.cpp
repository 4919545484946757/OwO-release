#include "owo/engine/candidate_scheduler.h"

#include <algorithm>
#include <numeric>
#include <unordered_map>
#include <unordered_set>

namespace owo::engine {
namespace {

struct ProviderState {
    CandidateProvider* provider{};
    std::size_t weight{};
    bool started{};
    bool exhausted{};
    std::int64_t next_score_upper_bound{
        (std::numeric_limits<std::int64_t>::min)()};
    std::optional<ProviderContinuation> continuation;
};

bool continuation_matches(const CandidateRequestContext& context,
                          const CandidateProvider& provider,
                          const ProviderContinuation& continuation) {
    return continuation.provider_id == provider.id() &&
           continuation.input_fingerprint == context.input_fingerprint &&
           continuation.lexicon_generation == context.lexicon_generation &&
           continuation.parse_generation == context.parse_generation;
}

bool schedule_matches(const CandidateRequestContext& context,
                      const CandidateScheduleContinuation& continuation) {
    return continuation.input == context.input &&
           continuation.input_fingerprint == context.input_fingerprint &&
           continuation.lexicon_generation == context.lexicon_generation &&
           continuation.parse_generation == context.parse_generation;
}

}  // namespace

ScheduledCandidates CandidateScheduler::generate(
    const CandidateRequestContext& context,
    const std::span<const ProviderAllocation> providers,
    const GenerationBudget total_budget) const {
    return generate(context, providers, total_budget, nullptr);
}

ScheduledCandidates CandidateScheduler::generate(
    const CandidateRequestContext& context,
    const std::span<const ProviderAllocation> providers,
    const GenerationBudget total_budget,
    const CandidateScheduleContinuation* const previous) const {
    ScheduledCandidates output;
    if (providers.empty() || total_budget.max_results == 0 ||
        total_budget.max_work_units == 0) return output;

    std::vector<ProviderState> states;
    states.reserve(providers.size());
    std::unordered_set<std::string> provider_ids;
    for (const auto& allocation : providers) {
        if (allocation.provider == nullptr) continue;
        if (!provider_ids.insert(std::string(allocation.provider->id())).second)
            return output;
        states.push_back({allocation.provider, std::max<std::size_t>(1, allocation.weight)});
    }
    if (states.empty()) return output;

    std::unordered_set<std::string> seen;
    std::unordered_map<std::string, std::size_t> current_candidates;
    seen.reserve(total_budget.max_results * 2);
    bool restored = previous != nullptr && schedule_matches(context, *previous) &&
                    previous->providers.size() == states.size();
    if (restored) {
        for (auto& state : states) {
            const auto found = std::find_if(
                previous->providers.begin(), previous->providers.end(),
                [&state](const ScheduledProviderState& saved) {
                    return saved.provider_id == state.provider->id();
                });
            if (found == previous->providers.end() || found->weight != state.weight) {
                restored = false;
                break;
            }
            state.started = found->started;
            state.exhausted = found->exhausted;
            state.next_score_upper_bound = found->next_score_upper_bound;
            state.continuation = found->continuation;
            if (state.started && !state.exhausted && !state.continuation) {
                restored = false;
                break;
            }
        }
    }
    if (!restored) {
        for (auto& state : states) {
            state.started = false;
            state.exhausted = false;
            state.next_score_upper_bound =
                state.provider->initial_score_upper_bound(context);
            if (state.next_score_upper_bound ==
                (std::numeric_limits<std::int64_t>::min)())
                state.exhausted = true;
            state.continuation.reset();
        }
    } else {
        seen.insert(previous->seen_candidate_texts.begin(),
                    previous->seen_candidate_texts.end());
    }
    std::size_t remaining_work = total_budget.max_work_units;

    const auto distribute = [&](const std::size_t total) {
        std::vector<std::size_t> shares(states.size());
        const auto total_weight = std::accumulate(
            states.begin(), states.end(), std::size_t{},
            [](const std::size_t value, const ProviderState& state) {
                return value + (state.exhausted ? 0 : state.weight);
            });
        if (total_weight == 0) return shares;
        std::size_t distributed = 0;
        for (std::size_t index = 0; index < states.size(); ++index) {
            if (states[index].exhausted) continue;
            shares[index] = total * states[index].weight / total_weight;
            distributed += shares[index];
        }
        for (std::size_t index = 0; distributed < total; ++index) {
            auto& state = states[index % states.size()];
            if (state.exhausted) continue;
            ++shares[index % shares.size()];
            ++distributed;
        }
        return shares;
    };

    const auto consume = [&](ProviderState& state, const GenerationBudget budget,
                             const bool continuation_request,
                             const bool reclaimable_allocation) {
        const auto* continuation = continuation_request && state.continuation
                                       ? &*state.continuation : nullptr;
        if (continuation != nullptr &&
            !continuation_matches(context, *state.provider, *continuation)) {
            state.exhausted = true;
            state.continuation.reset();
            return std::size_t{};
        }
        auto batch = state.provider->request(context, budget, continuation);
        state.started = true;
        ++output.metrics.provider_requests;
        if (continuation != nullptr) ++output.metrics.continuation_requests;
        const auto consumed_work = std::min(batch.work_units_consumed, budget.max_work_units);
        output.metrics.work_units_consumed += consumed_work;
        remaining_work -= std::min(remaining_work, consumed_work);
        // Initial allocations are disjoint shares of the total budget, so
        // their unused part is genuinely reclaimed. A continuation request is
        // offered the already-shared remaining pool; counting its unused part
        // again would multiply the same reclaimed work across providers.
        if (reclaimable_allocation && batch.exhausted &&
            consumed_work < budget.max_work_units)
            output.metrics.work_units_reclaimed += budget.max_work_units - consumed_work;
        if (reclaimable_allocation && batch.exhausted &&
            batch.candidates.size() < budget.max_results)
            output.metrics.result_slots_reclaimed +=
                budget.max_results - batch.candidates.size();
        for (auto& candidate : batch.candidates) {
            if (seen.contains(candidate.text)) {
                ++output.metrics.duplicate_candidates_discarded;
                const auto current = current_candidates.find(candidate.text);
                if (current != current_candidates.end() &&
                    candidate.score > output.candidates[current->second].score)
                    output.candidates[current->second] = std::move(candidate);
                continue;
            }
            if (output.candidates.size() >= total_budget.max_results) break;
            seen.insert(candidate.text);
            current_candidates.emplace(candidate.text, output.candidates.size());
            output.candidates.push_back(std::move(candidate));
        }
        state.exhausted = batch.exhausted;
        state.next_score_upper_bound = batch.next_score_upper_bound;
        state.continuation = std::move(batch.continuation);
        if (!state.exhausted && !state.continuation) state.exhausted = true;
        return consumed_work;
    };

    const auto best_receiver = [&]() {
        return std::max_element(
            states.begin(), states.end(), [](const ProviderState& left,
                                             const ProviderState& right) {
                if (left.exhausted != right.exhausted) return left.exhausted;
                if (left.next_score_upper_bound != right.next_score_upper_bound)
                    return left.next_score_upper_bound < right.next_score_upper_bound;
                return left.weight < right.weight;
            });
    };

    if (states.size() > total_budget.max_results) {
        // When sources outnumber visible slots, allocating by registration
        // order hides late high-quality sources. V3 providers expose a cheap
        // score upper bound, allowing a bounded best-first merge that visits
        // only the source currently capable of producing the strongest head.
        while (output.candidates.size() < total_budget.max_results &&
               remaining_work != 0) {
            const auto receiver = best_receiver();
            if (receiver == states.end() || receiver->exhausted) break;
            const auto before_work = output.metrics.work_units_consumed;
            const auto before_results = output.candidates.size();
            consume(*receiver, {1, remaining_work}, receiver->started, false);
            if (before_work == output.metrics.work_units_consumed &&
                before_results == output.candidates.size() && !receiver->exhausted)
                receiver->exhausted = true;
        }
    } else {
        const auto result_shares = distribute(total_budget.max_results);
        const auto work_shares = distribute(total_budget.max_work_units);
        for (std::size_t index = 0; index < states.size(); ++index) {
            if (result_shares[index] == 0 || work_shares[index] == 0) continue;
            consume(states[index], {result_shares[index], work_shares[index]},
                    states[index].started, true);
        }
    }

    while (output.candidates.size() < total_budget.max_results && remaining_work != 0) {
        const auto receiver = best_receiver();
        if (receiver == states.end() || receiver->exhausted) break;
        const auto missing = total_budget.max_results - output.candidates.size();
        output.metrics.reallocated_result_budget += missing;
        output.metrics.reallocated_work_budget += remaining_work;
        const auto before = output.metrics.work_units_consumed;
        const auto before_results = output.candidates.size();
        consume(*receiver, {missing, remaining_work}, true, false);
        output.metrics.work_units_consumed_after_reallocation +=
            output.metrics.work_units_consumed - before;
        output.metrics.candidates_generated_after_reallocation +=
            output.candidates.size() - before_results;
    }

    std::stable_sort(output.candidates.begin(), output.candidates.end(),
                     [](const Candidate& left, const Candidate& right) {
                         return left.score > right.score;
                     });
    if (std::any_of(states.begin(), states.end(), [](const ProviderState& state) {
            return !state.exhausted;
        })) {
        CandidateScheduleContinuation next;
        next.input = context.input;
        next.input_fingerprint = context.input_fingerprint;
        next.lexicon_generation = context.lexicon_generation;
        next.parse_generation = context.parse_generation;
        next.seen_candidate_texts.assign(seen.begin(), seen.end());
        next.providers.reserve(states.size());
        for (auto& state : states) {
            next.providers.push_back({std::string(state.provider->id()), state.weight,
                                      state.started, state.exhausted,
                                      state.next_score_upper_bound,
                                      std::move(state.continuation)});
        }
        output.continuation = std::move(next);
    }
    return output;
}

}  // namespace owo::engine
