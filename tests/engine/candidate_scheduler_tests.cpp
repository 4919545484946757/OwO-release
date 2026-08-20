#include "owo/engine/candidate_scheduler.h"
#include "owo/engine/composition_candidate_provider.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/engine/lexicon_candidate_provider.h"
#include "owo/engine/parse_candidate_sources.h"

#include <algorithm>
#include <filesystem>
#include <iostream>
#include <string_view>
#include <utility>
#include <vector>

namespace {

class BoundedProvider final : public owo::engine::CandidateProvider {
public:
    BoundedProvider(std::string id, std::vector<owo::engine::Candidate> candidates,
                    const std::size_t work_per_candidate)
        : id_(std::move(id)), candidates_(std::move(candidates)),
          work_per_candidate_(work_per_candidate) {}

    std::string_view id() const noexcept override { return id_; }

    std::int64_t initial_score_upper_bound(
        const owo::engine::CandidateRequestContext&) const noexcept override {
        return candidates_.empty()
            ? (std::numeric_limits<std::int64_t>::min)()
            : candidates_.front().score;
    }

    owo::engine::CandidateBatch request(
        const owo::engine::CandidateRequestContext& context,
        const owo::engine::GenerationBudget budget,
        const owo::engine::ProviderContinuation* continuation) override {
        std::size_t offset = 0;
        if (continuation != nullptr && !continuation->opaque_state.empty())
            offset = continuation->opaque_state.front();
        owo::engine::CandidateBatch batch;
        while (offset < candidates_.size() &&
               batch.candidates.size() < budget.max_results &&
               batch.work_units_consumed + work_per_candidate_ <=
                   budget.max_work_units) {
            batch.candidates.push_back(candidates_[offset++]);
            batch.work_units_consumed += work_per_candidate_;
        }
        visited_ += batch.work_units_consumed;
        batch.exhausted = offset == candidates_.size();
        if (!batch.exhausted) {
            batch.next_score_upper_bound = candidates_[offset].score;
            batch.continuation = owo::engine::ProviderContinuation{
                id_, context.input_fingerprint, context.lexicon_generation,
                context.parse_generation, {offset}};
        }
        return batch;
    }

    std::size_t visited() const noexcept { return visited_; }

private:
    std::string id_;
    std::vector<owo::engine::Candidate> candidates_;
    std::size_t work_per_candidate_{};
    std::size_t visited_{};
};

owo::engine::Candidate candidate(std::string text, const std::int64_t score) {
    owo::engine::Candidate value;
    value.text = std::move(text);
    value.score = score;
    return value;
}

}  // namespace

int main() {
    BoundedProvider dictionary("dictionary", {
        candidate("你好", 100), candidate("你们", 90), candidate("你看", 80),
        candidate("你说", 70), candidate("你想", 60)}, 3);
    BoundedProvider characters("character", {candidate("你", 95)}, 2);
    const owo::engine::CandidateRequestContext context{"ni", 11, 7, 3};
    const std::vector<owo::engine::ProviderAllocation> providers{
        {&dictionary, 1}, {&characters, 1}};
    const owo::engine::CandidateScheduler scheduler;
    const auto result = scheduler.generate(context, providers, {5, 20});
    const std::vector<std::string> expected{"你好", "你", "你们", "你看", "你说"};
    std::vector<std::string> actual;
    for (const auto& value : result.candidates) actual.push_back(value.text);
    if (actual != expected || result.metrics.provider_requests != 3 ||
        result.metrics.continuation_requests != 1 ||
        result.metrics.result_slots_reclaimed == 0 ||
        result.metrics.reallocated_result_budget == 0 ||
        result.metrics.candidates_generated_after_reallocation == 0 ||
        result.metrics.work_units_reclaimed == 0 ||
        result.metrics.reallocated_work_budget == 0 ||
        result.metrics.work_units_consumed_after_reallocation == 0 ||
        dictionary.visited() != 12 ||
        characters.visited() != 2) {
        std::cerr << "bounded provider scheduling failed\n";
        return 1;
    }

    BoundedProvider paged_dictionary("paged-dictionary", {
        candidate("d1", 100), candidate("d2", 90), candidate("d3", 80),
        candidate("d4", 70), candidate("d5", 60)}, 1);
    BoundedProvider paged_characters("paged-characters", {
        candidate("c1", 95)}, 1);
    const std::vector<owo::engine::ProviderAllocation> paged_providers{
        {&paged_dictionary, 1}, {&paged_characters, 1}};
    const auto page_one = scheduler.generate(context, paged_providers, {3, 12});
    if (!page_one.continuation || page_one.candidates.size() != 3 ||
        page_one.candidates[0].text != "d1" || page_one.candidates[1].text != "c1" ||
        page_one.candidates[2].text != "d2") {
        std::cerr << "first paged scheduling failed\n";
        return 1;
    }
    const auto page_two = scheduler.generate(
        context, paged_providers, {3, 12}, &*page_one.continuation);
    if (page_two.continuation || page_two.candidates.size() != 3 ||
        page_two.candidates[0].text != "d3" || page_two.candidates[1].text != "d4" ||
        page_two.candidates[2].text != "d5") {
        std::cerr << "continued paged scheduling failed\n";
        return 1;
    }
    auto edited_context = context;
    edited_context.input = "nih";
    ++edited_context.input_fingerprint;
    const auto restarted = scheduler.generate(
        edited_context, paged_providers, {3, 12}, &*page_one.continuation);
    if (restarted.candidates.size() != 3 || restarted.candidates[0].text != "d1" ||
        restarted.candidates[1].text != "c1" || restarted.candidates[2].text != "d2") {
        std::cerr << "stale page restart failed\n";
        return 1;
    }

    BoundedProvider duplicate_dictionary("dictionary", {
        candidate("你好", 100), candidate("你们", 90)}, 1);
    BoundedProvider duplicate_model("model", {
        candidate("你好", 110), candidate("您好", 85)}, 1);
    const std::vector<owo::engine::ProviderAllocation> duplicate_providers{
        {&duplicate_dictionary, 1}, {&duplicate_model, 1}};
    const auto deduplicated = scheduler.generate(context, duplicate_providers, {3, 8});
    if (deduplicated.candidates.size() != 3 ||
        deduplicated.metrics.duplicate_candidates_discarded != 1)
        return 2;

    BoundedProvider tiny_first("tiny-first", {candidate("first", 10)}, 1);
    BoundedProvider tiny_second("tiny-second", {candidate("second", 9)}, 1);
    const std::vector<owo::engine::ProviderAllocation> tiny_providers{
        {&tiny_first, 1}, {&tiny_second, 1}};
    const auto tiny = scheduler.generate(context, tiny_providers, {1, 1});
    if (tiny.candidates.size() != 1 || tiny.metrics.work_units_consumed != 1 ||
        tiny.metrics.provider_requests != 1)
        return 2;
    const std::vector<owo::engine::ProviderAllocation> duplicate_ids{
        {&tiny_first, 1}, {&tiny_first, 1}};
    const auto ambiguous = scheduler.generate(context, duplicate_ids, {2, 2});
    if (!ambiguous.candidates.empty() || ambiguous.continuation) return 2;

    BoundedProvider low("front-low", {candidate("low", 10)}, 1);
    BoundedProvider middle("front-middle", {candidate("middle", 20)}, 1);
    BoundedProvider late_high("late-high", {candidate("high", 100)}, 1);
    const std::vector<owo::engine::ProviderAllocation> frontier_providers{
        {&low, 1}, {&middle, 1}, {&late_high, 1}};
    const auto frontier = scheduler.generate(context, frontier_providers, {1, 3});
    if (frontier.candidates.size() != 1 ||
        frontier.candidates.front().text != "high" || late_high.visited() != 1 ||
        low.visited() != 0 || middle.visited() != 0)
        return 2;

    const auto path = std::filesystem::temp_directory_path() /
                      "owo-candidate-provider-test.owolx";
    if (!owo::engine::write_binary_lexicon(path, {
            {{"de"}, "的", 1000}, {{"dou"}, "都", 900},
            {{"dui"}, "对", 800}, {{"deng"}, "等", 700},
            {{"dao"}, "到", 600}, {{"ni"}, "你", 1200},
            {{"hao"}, "好", 1100},
            {{"ni", "hao"}, "你好", 1000},
            {{"ni", "hao"}, "你号", 50}}).success)
        return 3;
    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(path).success) return 3;
    owo::engine::InitialLexiconCandidateProvider initial_provider(lexicon);
    const owo::engine::CandidateRequestContext initial_context{"d", 12, 8, 4};
    const auto first = initial_provider.request(initial_context, {2, 2}, nullptr);
    if (first.candidates.size() != 2 || first.exhausted || !first.continuation ||
        first.work_units_consumed != 2 || first.candidates.front().text != "的")
        return 3;
    const auto second = initial_provider.request(
        initial_context, {2, 2}, &*first.continuation);
    if (second.candidates.size() != 2 || second.exhausted ||
        second.work_units_consumed != 2 || second.candidates.front().text != "对")
        return 3;
    auto stale_context = initial_context;
    ++stale_context.parse_generation;
    const auto stale = initial_provider.request(
        stale_context, {2, 2}, &*first.continuation);
    if (!stale.exhausted || !stale.candidates.empty()) return 3;
    stale_context = initial_context;
    ++stale_context.lexicon_generation;
    const auto stale_lexicon = initial_provider.request(
        stale_context, {2, 2}, &*first.continuation);
    if (!stale_lexicon.exhausted || !stale_lexicon.candidates.empty()) return 3;
    stale_context = initial_context;
    ++stale_context.input_fingerprint;
    const auto stale_input = initial_provider.request(
        stale_context, {2, 2}, &*first.continuation);
    if (!stale_input.exhausted || !stale_input.candidates.empty()) return 3;

    owo::engine::ExactLexiconCandidateProvider exact_dictionary(
        lexicon, "exact-dictionary", {"ni", "hao"}, {"ni", "hao"},
        owo::engine::InputMatchKind::exact, 5,
        owo::engine::ExactCandidateFilter::multiple_characters);
    owo::engine::ExactLexiconCandidateProvider leading_characters(
        lexicon, "leading-characters", {"ni"}, {"ni"},
        owo::engine::InputMatchKind::exact, 2,
        owo::engine::ExactCandidateFilter::single_character);
    const std::vector<owo::engine::ProviderAllocation> exact_providers{
        {&exact_dictionary, 1}, {&leading_characters, 2}};
    const owo::engine::CandidateRequestContext exact_context{"nihao", 13, 8, 4};
    const auto exact = scheduler.generate(exact_context, exact_providers, {3, 16});
    if (exact.candidates.size() != 3 || exact.continuation ||
        exact.candidates[0].text != "你" || exact.candidates[1].text != "你好" ||
        exact.candidates[2].text != "你号" ||
        exact.metrics.continuation_requests != 1 ||
        exact.metrics.work_units_reclaimed == 0) {
        std::cerr << "exact provider scheduling failed\n";
        return 3;
    }

    owo::engine::ParseResult parsed;
    parsed.normalized_input = "nihao";
    parsed.valid = true;
    parsed.paths.push_back({{{"ni", 0, 2, true}, {"hao", 2, 5, true}},
                            owo::engine::InputMatchKind::exact, 0, 0});
    parsed.paths.push_back({{{"ni", 0, 2, true}, {"hao", 2, 5, true}},
                            owo::engine::InputMatchKind::corrected, 1, 0});
    auto parse_plan = owo::engine::make_parse_candidate_source_plan(
        lexicon, parsed, 14, 8, 5);
    if (parse_plan.allocations.size() != 3 ||
        parse_plan.owned_providers.size() != 3) {
        std::cerr << "parse source plan failed\n";
        return 3;
    }
    const auto parsed_candidates = scheduler.generate(
        parse_plan.context, parse_plan.allocations, {3, 32});
    if (parsed_candidates.candidates.size() != 3 ||
        parsed_candidates.candidates.front().text != "你好") {
        std::cerr << "parsed candidate scheduling failed: size="
                  << parsed_candidates.candidates.size();
        if (!parsed_candidates.candidates.empty())
            std::cerr << " first=" << parsed_candidates.candidates.front().text;
        std::cerr << '\n';
        return 3;
    }
    const auto shadow = owo::engine::compare_candidate_results(
        parsed_candidates.candidates, parsed_candidates.candidates, 3);
    if (!shadow.first_candidate_equal || shadow.shared_count != 3 ||
        shadow.equal_prefix_count != 3 || !shadow.scheduled_complete) {
        std::cerr << "shadow comparison failed\n";
        return 3;
    }

    owo::engine::CompositionCandidateProvider composition(
        lexicon, "composition-test", {"ni", "hao"}, {"ni", "hao"},
        owo::engine::InputMatchKind::exact, 5, 0);
    std::optional<owo::engine::ProviderContinuation> composition_state;
    owo::engine::CandidateBatch composed;
    for (std::size_t attempt = 0; attempt < 8; ++attempt) {
        composed = composition.request(
            exact_context, {1, 1},
            composition_state ? &*composition_state : nullptr);
        composition_state = composed.continuation;
        if (composed.exhausted) break;
    }
    if (!composed.exhausted || composed.candidates.size() != 1 ||
        composed.candidates.front().text != "你好" ||
        composed.candidates.front().segment_count != 2) {
        std::cerr << "composition continuation failed\n";
        return 3;
    }
    std::error_code ignored;
    std::filesystem::remove(path, ignored);

    return 0;
}
