#include "owo/engine/candidate_partition.h"
#include "owo/engine/candidate_scoring.h"
#include "owo/ipc/contextual_candidate_order.h"

#include <algorithm>
#include <iostream>

namespace {
owo::engine::Candidate word(std::string text, std::size_t consumed, std::int64_t score,
                            std::size_t segments = 1) {
    owo::engine::Candidate value;
    value.text = std::move(text);
    value.syllables.resize(owo::ipc::utf8_character_count(value.text), "a");
    value.consumed_input_bytes = consumed;
    value.score = score;
    value.segment_count = segments;
    return value;
}
} // namespace

int main() {
    using namespace owo::engine;
    auto split = word("不提安家", 9, 22814, 2);
    split.source_segments = {"bu", "ti", "an", "jia"};
    auto compact = word("不添加", 9, 38615);
    compact.source_segments = {"bu", "tian", "jia"};
    auto partial = word("不提", 4, 90000); partial.source_segments = {"bu", "ti"};
    auto hidden_reading = compact; hidden_reading.model_only = true; hidden_reading.score = 99000;
    for (const bool joint : {false, true}) {
        std::vector<Candidate> ranked{split, partial, compact, hidden_reading};
        if (joint) merge_similar_length_partitions(ranked, 9);
        else rank_full_input_segmentations(ranked, 9);
        if (ranked[0].text != compact.text || ranked[2].text != split.text ||
            ranked[1] != partial || ranked[3].text != hidden_reading.text ||
            !ranked[3].model_only || ranked[3].joint_phrase_partition ||
            ranked[0].partition_slot != CandidatePartitionSlot::sentence ||
            ranked[2].partition_slot != CandidatePartitionSlot::dictionary) return 27;
        auto stronger_long = split; stronger_long.score = compact.score + 1;
        ranked = {compact, stronger_long};
        rank_full_input_segmentations(ranked, 9);
        if (ranked.front().text != split.text) return 28;
        auto learned_compact = compact; learned_compact.user_lexicon = true; learned_compact.score = -1;
        ranked = {stronger_long, learned_compact};
        rank_full_input_segmentations(ranked, 9);
        if (ranked.front().text != compact.text || !ranked.front().user_preferred()) return 29;
    }
    auto reading_a = word("西安街道", 10, 100, 2);
    reading_a.source_segments = {"xi", "an", "jie", "dao"}; reading_a.qualified_sentence_reading = true;
    auto reading_b = word("衔接到", 10, 100, 2);
    reading_b.source_segments = {"xian", "jie", "dao"}; reading_b.qualified_sentence_reading = true;
    auto unqualified = word("低质分割句子", 10, 100, 3);
    unqualified.source_segments = {"x", "i", "a", "n", "jie", "dao"};
    if (sentence_candidate_limit(std::vector<Candidate>{reading_a, reading_a, reading_b, unqualified}, 10) != 4) return 18;
    std::vector<std::string> ranked_sources{"甲句一", "甲句二", "甲句三", "乙句一", "乙句二", "乙句三"};
    std::vector<std::uint64_t> ranked_consumed(6, 10);
    const auto source_candidates = ranked_sources;
    owo::ipc::limit_model_sentence_candidates(source_candidates, ranked_consumed,
        {true, true, true, true, true, true}, 4, ranked_sources, ranked_consumed,
        {2, 2, 2, 3, 3, 3}, {"a", "a", "a", "b", "b", "b"});
    if (ranked_sources != std::vector<std::string>{"甲句一", "甲句二", "乙句一", "乙句二"}) return 19;
    auto paired_a = reading_a; paired_a.text = "西安接到"; paired_a.score = 1900;
    auto paired_b = reading_b; paired_b.text = "先接到"; paired_b.score = 50;
    reading_a.score = 2000; reading_b.score = 100;
    std::vector<Candidate> page_readings{reading_a, paired_a, reading_b, paired_b};
    arrange_sentence_reading_pages(page_readings);
    if (page_readings != std::vector<Candidate>{reading_a, reading_b, paired_a, paired_b}) return 20;
    auto reading_c = reading_a; reading_c.text = "先鸡饿到";
    reading_c.source_segments = {"xian", "ji", "e", "dao"};
    auto paired_c = reading_c; paired_c.text = "先寄饿到";
    page_readings = {reading_a, paired_a, reading_b, reading_c, paired_b, paired_c};
    arrange_sentence_reading_pages(page_readings);
    if (page_readings != std::vector<Candidate>{reading_a, reading_b, paired_a, paired_b, reading_c, paired_c}) return 21;
    page_readings = {reading_a, reading_b, paired_b, reading_c, paired_c};
    arrange_sentence_reading_pages(page_readings);
    if (page_readings != std::vector<Candidate>{reading_a, reading_b, paired_b, reading_c, paired_c}) return 22;
    ranked_sources = source_candidates; ranked_consumed.assign(6, 10);
    owo::ipc::limit_model_sentence_candidates(source_candidates, std::vector<std::uint64_t>(6, 10),
        {true, true, true, true, true, true}, 4, ranked_sources, ranked_consumed,
        {2, 2, 2, 3, 3, 3}, {"a", "a", "a", "b", "b", "b"}, true);
    if (ranked_sources != std::vector<std::string>{"甲句一", "乙句一", "甲句二", "乙句二"}) return 25;
    ranked_sources = source_candidates; ranked_consumed.assign(6, 10);
    owo::ipc::limit_model_sentence_candidates(source_candidates, std::vector<std::uint64_t>(6, 10),
        {true, true, true, true, true, true}, 2, ranked_sources, ranked_consumed,
        {2, 2, 2, 3, 3, 3}, {"a", "a", "a", "b", "b", "b"}, true);
    if (ranked_sources != std::vector<std::string>{"甲句一", "乙句一"}) return 26;
    std::vector<Candidate> page_slots{
        word("甲句子一", 13, 600, 2), word("甲句子二", 13, 500, 2), word("完整词甲", 13, 1000),
        word("甲", 3, 1), word("乙", 3, 1),
        word("乙句子一", 13, 400, 2), word("乙句子二", 13, 300, 2), word("完整词乙", 13, 900)};
    merge_similar_length_partitions(page_slots, 13);
    for (const auto index : {0U, 1U, 5U, 6U})
        if (page_slots[index].partition_slot != CandidatePartitionSlot::sentence) return 23;
    for (const auto index : {2U, 7U})
        if (page_slots[index].partition_slot != CandidatePartitionSlot::dictionary) return 24;
    ParseResult incomplete;
    incomplete.normalized_input = "rgs";
    incomplete.has_incomplete_syllable = true;
    auto abbreviation = word("如果说", 3, 100);
    abbreviation.match_kind = InputMatchKind::abbreviated_completion;
    auto exact_prefix = word("如果", 2, 100);
    auto learned_word = word("学习词", 3, 100); learned_word.user_lexicon = true;
    std::vector<Candidate> detection{abbreviation, exact_prefix, learned_word};
    if (!requires_incomplete_candidate_limit(incomplete, detection)) return 15;
    detection.push_back(word("完整输入", 3, 100));
    if (requires_incomplete_candidate_limit(incomplete, detection)) return 16;
    detection.pop_back(); incomplete.phonetic_assistance = false;
    if (requires_incomplete_candidate_limit(incomplete, detection)) return 17;
    for (const auto page : {4U, 5U, 7U, 9U}) {
        std::vector<Candidate> limited;
        for (std::size_t index = 0; index < 50; ++index) {
            auto composition = word("组合词", 3, 100, 2);
            composition.match_kind = index % 2 == 0 ? InputMatchKind::abbreviated_completion
                                                    : InputMatchKind::incomplete_completion;
            composition.model_only = index >= 20;
            limited.push_back(composition);
            auto dictionary = word("真词条", 3, 100);
            dictionary.match_kind = composition.match_kind;
            limited.push_back(dictionary);
            limited.push_back(word("单", 1, 100));
        }
        auto exact = word("完整句子", 12, 100, 2);
        auto corrected = exact; corrected.match_kind = InputMatchKind::corrected;
        auto learned = exact; learned.match_kind = InputMatchKind::abbreviated_completion; learned.user_lexicon = true;
        limited.insert(limited.begin(), {exact, corrected, learned});
        const auto unchanged_complete = limited;
        limit_incomplete_candidates(limited, page, false);
        if (limited != unchanged_complete) return 13;
        limit_incomplete_candidates(limited, page, true);
        if (std::count_if(limited.begin(), limited.end(), is_non_single_candidate) != page * 2 ||
            std::count_if(limited.begin(), limited.end(), [](const auto& value) { return value.text == "单"; }) != 50 ||
            limited.size() != page * 2 + 50) return 11;
        auto reserve = unchanged_complete;
        limit_incomplete_candidates(reserve, page, true, 2);
        if (std::count_if(reserve.begin(), reserve.end(), is_non_single_candidate) != page * 2 - 2) return 14;
    }
    std::vector<Candidate> candidates{
        word("自动展开", 13, 100, 2), word("自动站", 10, 300),     word("自动", 6, 10000),
        word("自", 2, 10000),         word("词典整词", 13, 200),   word("长度更长了吧", 13, 10000),
        word("五字长词条", 12, 50),   word("纠错词条", 13, 10000), word("隐含词", 10, 10000)};
    candidates[7].match_kind = InputMatchKind::corrected;
    candidates.back().model_only = true;
    merge_similar_length_partitions(candidates, 13);
    if (candidates[0].text != "词典整词" || candidates[1].text != "自动站" ||
        candidates[4].text != "自动展开" || candidates[2].text != "自动" ||
        candidates[3].text != "自" || candidates[5].text != "长度更长了吧" ||
        candidates[6].text != "五字长词条" || candidates[7].text != "纠错词条" ||
        candidates.back().text != "隐含词" || !candidates.back().model_only ||
        candidates.back().joint_phrase_partition || candidates[1].joint_phrase_partition ||
        candidates[2].joint_phrase_partition || candidates[5].joint_phrase_partition ||
        candidates[6].joint_phrase_partition || candidates[7].joint_phrase_partition ||
        !candidates[0].joint_phrase_partition || !candidates[4].joint_phrase_partition) {
        std::cerr << "Joined scores changed unrelated lengths, singles or hidden slots\n";
        return 1;
    }
    if (candidates[0].partition_slot != CandidatePartitionSlot::sentence ||
        candidates[4].partition_slot != CandidatePartitionSlot::dictionary ||
        candidates[3].partition_slot != CandidatePartitionSlot::automatic ||
        candidates.back().partition_slot != CandidatePartitionSlot::automatic) return 12;
    auto learned = word("已学词条", 10, -100);
    learned.user_lexicon = true;
    auto hidden = word("隐藏词条", 10, 10000);
    hidden.model_only = true;
    std::vector<Candidate> preferred{word("四字句子", 13, 1000, 2), learned};
    merge_similar_length_partitions(preferred, 13);
    if (preferred.front().text != "已学词条" || !preferred.front().user_preferred())
        return 2;
    std::vector<Candidate> no_near_word{word("四字句子", 13, 1000, 2), word("短词", 6, 9000),
                                        hidden};
    const auto unchanged = no_near_word;
    merge_similar_length_partitions(no_near_word, 13);
    if (no_near_word != unchanged)
        return 3;
    std::vector<std::string> order;
    std::vector<std::uint64_t> consumed;
    owo::ipc::apply_contextual_candidate_order(
        {"自动站点", "自动展开", "自动", "自"}, 13, {"自动展开", "自动站点", "自动", "自"},
        {13, 10, 6, 2}, order, consumed, {0, 0, 0, 0}, {true, true, false, false});
    if (order != std::vector<std::string>{"自动站点", "自动展开", "自动", "自"} ||
        consumed != std::vector<std::uint64_t>{10, 13, 6, 2})
        return 4;
    order = {"自动展开", "自动站点", "自动", "自"};
    consumed = {13, 10, 6, 2};
    owo::ipc::restore_preferred_candidate_positions(
        {"自动站点"}, {"自动站点", "自动展开", "自动", "自"}, {10, 13, 6, 2}, order, consumed,
        {true, true, false, false});
    if (order.front() != "自动站点" || consumed.front() != 10)
        return 5;
    // Confidence still wins over an adversarial model inside the joined band.
    owo::ipc::apply_contextual_candidate_order({"自动站点", "自动展开"}, 13, {"自动展开", "自动站点"},
                                               {13, 10}, order, consumed, {0, 1}, {true, true});
    if (order.front() != "自动展开")
        return 6;
    order = {"长句一", "长句二", "长句三", "字典词", "前缀"};
    consumed = {13, 13, 13, 13, 10};
    const auto original = order;
    const auto original_consumed = consumed;
    owo::ipc::limit_model_sentence_candidates(original, original_consumed,
                                              {true, true, true, false, false}, 2, order, consumed);
    if (order != std::vector<std::string>{"长句一", "长句二", "字典词", "前缀"} ||
        consumed != std::vector<std::uint64_t>{13, 13, 13, 10})
        return 7;
    // A small, one-off score adjustment, not a hard dictionary-first tier.
    std::vector<Candidate> soft{
        word("组合句子", 13, 10900, 2), word("精确整词", 13, 10000),
        word("更强句子", 13, 14000, 2), word("另一整词", 13, 9000),
        word("短句子", 13, 10900, 2), word("更长的句子", 13, 10900, 2),
        word("单", 13, 10900), word("部分句子", 10, 10900, 2),
        word("学习句子", 13, 5000, 2), word("纠错句子", 13, 10900, 2)};
    soft[8].user_lexicon = true;
    soft[9].match_kind = InputMatchKind::corrected;
    auto expected_soft = soft;
    expected_soft[0].score -= kCandidateExactDictionarySentencePenalty;
    expected_soft[2].score -= kCandidateExactDictionarySentencePenalty;
    apply_exact_dictionary_sentence_penalty(soft, 13);
    if (soft != expected_soft)
        return 8; // Includes no accumulation for multiple exact dictionary anchors.
    merge_similar_length_partitions(soft, 13);
    const auto position = [&soft](std::string_view text) {
        return std::find_if(soft.begin(), soft.end(), [text](const Candidate &value) { return value.text == text; });
    };
    if (position("精确整词") >= position("组合句子") ||
        position("更强句子") >= position("精确整词") || !soft.front().user_preferred())
        return 9;
    // Partial, repaired, provisional, hidden and different-length words must
    // not act as exact whole-word evidence against a sentence.
    std::vector<Candidate> no_exact{
        word("组合句子", 13, 10000, 2), word("纠错整词", 13, 10000),
        word("部分整词", 10, 10000), word("临时整词", 13, 10000),
        word("隐藏整词", 13, 10000), word("短整词", 13, 10000)};
    no_exact[1].match_kind = InputMatchKind::corrected;
    no_exact[3].provisional_user_word = true;
    no_exact[4].model_only = true;
    const auto unchanged_no_exact = no_exact;
    apply_exact_dictionary_sentence_penalty(no_exact, 13);
    if (no_exact != unchanged_no_exact)
        return 10;
    return 0;
}
