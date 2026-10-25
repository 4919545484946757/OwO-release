#include "owo/engine/candidate_partition.h"
#include "owo/ipc/contextual_candidate_order.h"

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
    return 0;
}
