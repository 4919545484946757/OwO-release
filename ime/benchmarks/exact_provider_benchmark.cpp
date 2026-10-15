#include "owo/engine/binary_lexicon.h"
#include "owo/engine/candidate_scheduler.h"
#include "owo/engine/lexicon_candidate_provider.h"

#include <algorithm>
#include <array>
#include <filesystem>
#include <iostream>
#include <string>
#include <vector>

namespace {

struct Query {
    std::string input;
    std::vector<std::string> reading;
    std::string expected_dictionary_word;
};

}  // namespace

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(std::filesystem::path(argv[1])).success ||
        !lexicon.exact_lookup_frequency_ordered())
        return 2;

    const std::array<Query, 6> queries{{
        {"nihao", {"ni", "hao"}, "你好"},
        {"wode", {"wo", "de"}, "我的"},
        {"keyi", {"ke", "yi"}, "可以"},
        {"mingan", {"min", "gan"}, "敏感"},
        {"culue", {"cu", "lve"}, "粗略"},
        {"shijie", {"shi", "jie"}, "世界"},
    }};
    const owo::engine::CandidateScheduler scheduler;
    std::size_t expected_hits = 0;
    std::size_t complete_pages = 0;
    std::size_t dictionary_candidates = 0;
    std::size_t character_candidates = 0;
    std::size_t work_consumed = 0;
    std::size_t work_reclaimed = 0;
    std::size_t result_slots_reclaimed = 0;
    std::size_t reallocated_result_budget = 0;
    std::size_t candidates_after_reallocation = 0;
    std::size_t continuation_requests = 0;
    for (std::size_t index = 0; index < queries.size(); ++index) {
        const auto& query = queries[index];
        const auto first_syllable_bytes = query.reading.front().size();
        owo::engine::ExactLexiconCandidateProvider dictionary(
            lexicon, "exact-dictionary", query.reading, query.reading,
            owo::engine::InputMatchKind::exact, query.input.size(),
            owo::engine::ExactCandidateFilter::multiple_characters);
        owo::engine::ExactLexiconCandidateProvider characters(
            lexicon, "leading-characters", {query.reading.front()},
            {query.reading.front()}, owo::engine::InputMatchKind::exact,
            first_syllable_bytes,
            owo::engine::ExactCandidateFilter::single_character);
        const std::vector<owo::engine::ProviderAllocation> providers{
            {&dictionary, 3}, {&characters, 4}};
        const owo::engine::CandidateRequestContext context{
            query.input, static_cast<std::uint64_t>(index + 1), 1, 1};
        const auto generated = scheduler.generate(context, providers, {7, 256});
        if (generated.candidates.size() == 7) ++complete_pages;
        if (std::any_of(generated.candidates.begin(), generated.candidates.end(),
                        [&query](const auto& candidate) {
                            return candidate.text == query.expected_dictionary_word;
                        }))
            ++expected_hits;
        else
            std::cerr << "missing expected dictionary word for " << query.input << '\n';
        for (const auto& candidate : generated.candidates) {
            if (candidate.consumed_input_bytes == query.input.size())
                ++dictionary_candidates;
            else
                ++character_candidates;
        }
        work_consumed += generated.metrics.work_units_consumed;
        work_reclaimed += generated.metrics.work_units_reclaimed;
        result_slots_reclaimed += generated.metrics.result_slots_reclaimed;
        reallocated_result_budget += generated.metrics.reallocated_result_budget;
        candidates_after_reallocation +=
            generated.metrics.candidates_generated_after_reallocation;
        continuation_requests += generated.metrics.continuation_requests;
    }
    std::cout << "{\"benchmark\":\"owo.engine.exact_dual_provider\""
              << ",\"queries\":" << queries.size()
              << ",\"expected_dictionary_hits\":" << expected_hits
              << ",\"complete_pages\":" << complete_pages
              << ",\"dictionary_candidates\":" << dictionary_candidates
              << ",\"character_candidates\":" << character_candidates
              << ",\"work_consumed\":" << work_consumed
              << ",\"work_reclaimed\":" << work_reclaimed
              << ",\"result_slots_reclaimed\":" << result_slots_reclaimed
              << ",\"reallocated_result_budget\":"
              << reallocated_result_budget
              << ",\"candidates_after_reallocation\":"
              << candidates_after_reallocation
              << ",\"continuation_requests\":" << continuation_requests << "}\n";
    return expected_hits == queries.size() && complete_pages == queries.size() ? 0 : 3;
}
