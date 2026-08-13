#include "owo/engine/binary_lexicon.h"
#include "owo/engine/candidate_generator.h"
#include "owo/engine/candidate_scheduler.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/parse_candidate_sources.h"

#include <array>
#include <filesystem>
#include <iostream>
#include <string_view>

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(std::filesystem::path(argv[1])).success ||
        !lexicon.exact_lookup_frequency_ordered()) return 2;

    constexpr std::array<std::string_view, 18> inputs{
        "nihao", "wode", "keyi", "mingan", "culue", "shijie",
        "wanan", "nanan", "niaho", "gongnneg", "bugd", "xingbaf",
        "shqiyishi", "shizhegeyangzi", "shijikeshengcheng",
        "yikuangaoduzidingyi", "zhekuangaoduzidingyide",
        "wodekuangaojiemingjiyuci"};
    constexpr std::size_t kVisible = 7;
    const owo::engine::FullPinyinSchema schema;
    const owo::engine::CandidateGenerator legacy_generator(lexicon);
    const owo::engine::CandidateScheduler scheduler;
    std::size_t valid_inputs = 0;
    std::size_t complete_scheduled_pages = 0;
    std::size_t first_equal = 0;
    std::size_t shared_candidates = 0;
    std::size_t equal_prefix_candidates = 0;
    std::size_t legacy_candidates = 0;
    std::size_t scheduled_candidates = 0;
    std::size_t work_consumed = 0;
    std::size_t work_reclaimed = 0;
    std::size_t created_sources = 0;
    for (std::size_t index = 0; index < inputs.size(); ++index) {
        const auto parsed = schema.parse(inputs[index], 32, true);
        if (!parsed.valid) continue;
        ++valid_inputs;
        const auto legacy = legacy_generator.generate(parsed, kVisible);
        auto plan = owo::engine::make_parse_candidate_source_plan(
            lexicon, parsed, static_cast<std::uint64_t>(index + 1), 1, 1, 12);
        created_sources += plan.allocations.size();
        const auto scheduled = scheduler.generate(
            plan.context, plan.allocations, {kVisible, 256});
        const auto shadow = owo::engine::compare_candidate_results(
            legacy, scheduled.candidates, kVisible);
        legacy_candidates += shadow.legacy_count;
        scheduled_candidates += shadow.scheduled_count;
        shared_candidates += shadow.shared_count;
        equal_prefix_candidates += shadow.equal_prefix_count;
        if (shadow.first_candidate_equal) ++first_equal;
        if (shadow.scheduled_complete) ++complete_scheduled_pages;
        work_consumed += scheduled.metrics.work_units_consumed;
        work_reclaimed += scheduled.metrics.work_units_reclaimed;
    }
    std::cout << "{\"benchmark\":\"owo.engine.parse_shadow\""
              << ",\"inputs\":" << inputs.size()
              << ",\"valid_inputs\":" << valid_inputs
              << ",\"created_sources\":" << created_sources
              << ",\"legacy_candidates\":" << legacy_candidates
              << ",\"scheduled_candidates\":" << scheduled_candidates
              << ",\"complete_scheduled_pages\":" << complete_scheduled_pages
              << ",\"first_candidate_equal\":" << first_equal
              << ",\"shared_candidates\":" << shared_candidates
              << ",\"equal_prefix_candidates\":" << equal_prefix_candidates
              << ",\"work_consumed\":" << work_consumed
              << ",\"work_reclaimed\":" << work_reclaimed << "}\n";
    return valid_inputs == inputs.size() && created_sources != 0 ? 0 : 3;
}
