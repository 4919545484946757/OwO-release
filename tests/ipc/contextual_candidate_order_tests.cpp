#include "owo/ipc/contextual_candidate_order.h"
#include "owo/ipc/model_candidate_confidence.h"

#include <cstdint>
#include <string>
#include <vector>

int main() {
    std::vector<std::string> ordered;
    std::vector<std::uint64_t> consumed;
    owo::ipc::apply_contextual_candidate_order(
        {"问题", "工作", "世界", "问"}, 8,
        {"工作", "世界", "问题", "问"}, {8, 8, 8, 2}, ordered, consumed);
    if (ordered != std::vector<std::string>{"问题", "工作", "世界", "问"} ||
        consumed != std::vector<std::uint64_t>{8, 8, 8, 2}) return 1;

    owo::ipc::apply_contextual_candidate_order(
        {"字", "较长词", "另一整句", "整句"}, 10,
        {"整句", "较长词", "字", "另一整句"}, {10, 6, 2, 10}, ordered, consumed);
    if (ordered != std::vector<std::string>{"另一整句", "整句", "较长词", "字"} ||
        consumed != std::vector<std::uint64_t>{10, 10, 6, 2}) return 2;

    owo::ipc::apply_contextual_candidate_order(
        {"已知"}, 8, {"未知一", "未知二", "已知"}, {8, 8, 8}, ordered, consumed);
    if (ordered != std::vector<std::string>{"已知", "未知一", "未知二"}) return 3;

    owo::ipc::apply_contextual_candidate_order(
        {"第一页一", "第二页二", "第一页二", "第二页一"}, 8,
        {"第二页一", "第二页二"}, {8, 8}, ordered, consumed);
    if (ordered != std::vector<std::string>{"第二页二", "第二页一"} ||
        consumed != std::vector<std::uint64_t>{8, 8}) return 4;

    ordered = {"可以", "刻意", "可", "克", "刻"};
    consumed = {4, 4, 2, 2, 2};
    owo::ipc::promote_preferred_candidates({"刻意"}, ordered, consumed);
    if (ordered != std::vector<std::string>{"刻意", "可以", "可", "克", "刻"} ||
        consumed != std::vector<std::uint64_t>{4, 4, 2, 2, 2}) return 5;

    ordered = {"刻意", "可以", "可", "克", "刻"};
    consumed = {4, 4, 2, 2, 2};
    owo::ipc::restore_preferred_candidate_positions(
        {"刻意"}, {"可以", "刻意", "可", "刻", "克"},
        {4, 4, 2, 2, 2}, ordered, consumed);
    if (ordered != std::vector<std::string>{"可以", "刻意", "可", "克", "刻"} ||
        consumed != std::vector<std::uint64_t>{4, 4, 2, 2, 2}) return 6;

    ordered = {"可以", "刻意", "可", "克", "刻"};
    owo::ipc::restore_preferred_candidate_positions(
        {"刻意"}, {"刻意", "可以", "可", "刻", "克"},
        {4, 4, 2, 2, 2}, ordered, consumed);
    if (ordered.front() != "刻意" || consumed.front() != 4) return 7;

    ordered = {"进保留", "仅保留", "仅", "今"};
    consumed = {10, 10, 3, 3};
    owo::ipc::restore_preferred_candidate_positions(
        {"进保留", "仅"}, {"进保留", "仅", "今", "仅保留"},
        {10, 3, 3, 10}, ordered, consumed);
    if (ordered != std::vector<std::string>{"进保留", "仅保留", "仅", "今"} ||
        consumed != std::vector<std::uint64_t>{10, 10, 3, 3}) return 8;
    // Strong dictionary evidence survives an adversarial rank-only model;
    // equally credible alternatives still follow the model's contextual order.
    owo::ipc::apply_contextual_candidate_order(
        {"朽腐类似问题", "修复类似文体", "修复类似问题", "修复", "修"}, 15,
        {"修复类似问题", "朽腐类似问题", "修复", "修", "修复类似文体"},
        {15, 15, 5, 3, 15}, ordered, consumed, {0, 2, 0, 0, 0});
    if (ordered != std::vector<std::string>{"修复类似文体", "修复类似问题",
                                          "朽腐类似问题", "修复", "修"}) return 9;
    owo::ipc::apply_contextual_candidate_order(
        {"朽腐类似问题", "修复类似问题"}, 15,
        {"修复类似问题", "朽腐类似问题"}, {15, 15}, ordered, consumed, {0, 2});
    if (ordered.front() != "修复类似问题" || consumed != std::vector<std::uint64_t>{15, 15}) return 10;
    ordered = {"家里的牛奶已经喝完了", "家里的牛奶已经合完了", "家里"};
    consumed = {26, 26, 5};
    owo::ipc::limit_model_sentence_candidates(ordered, consumed, {true, true, false}, 2,
        ordered, consumed, {5, 6, 1});
    if (ordered != std::vector<std::string>{"家里的牛奶已经喝完了", "家里"} ||
        consumed != std::vector<std::uint64_t>{26, 5}) return 11;
    ordered = {"这款高度自定", "这款高度自订", "这款"};
    consumed = {17, 17, 7};
    owo::ipc::limit_model_sentence_candidates(ordered, consumed, {true, true, false}, 2,
        ordered, consumed, {3, 3, 1});
    if (ordered.size() != 3) return 12;
    if (!owo::engine::redundant_sentence_variant("家里的牛奶已经喝完了", "佳丽的牛奶已经喝完了") ||
        !owo::engine::redundant_sentence_variant("家里的牛奶已经喝完了", "家里的牛奶意境喝完了") ||
        owo::engine::redundant_sentence_variant("这款高度自定", "这款高度自订") ||
        owo::engine::redundant_sentence_variant("将与概率", "降雨概率")) return 13;
    ordered = {"价值大妈", "价值大码", "价值大吗", "价值"};
    consumed = {10, 10, 10, 6};
    owo::ipc::retain_base_sentence_ending({"价值大妈", "价值大吗", "价值"}, {10, 10, 6},
        {true, true, false}, 10, ordered, consumed);
    if (ordered != std::vector<std::string>{"价值大妈", "价值大吗", "价值大码", "价值"}) return 14;
    ordered = {"价值大妈", "价值大码", "价值大吗", "价值"};
    owo::ipc::retain_base_sentence_ending({"价值大吗", "价值打吗", "价值"}, {10, 10, 6},
        {true, true, false}, 10, ordered, consumed);
    if (ordered[0] != "价值大妈" || ordered[1] != "价值大吗" || consumed[1] != 10) return 15;
    const auto unchanged = ordered;
    owo::ipc::retain_base_sentence_ending({"换个前文吗"}, {10}, {true}, 10, ordered, consumed);
    if (ordered != unchanged) return 16;
    owo::engine::Candidate head{
        "易货物运输的速度", {"yi", "huo", "wu", "yun", "shu", "de", "su", "du"},
        60000, owo::engine::InputMatchKind::exact,
        {"yi", "huo", "wu", "yun", "shu", "de", "su", "du"}, 18, 4};
    head.segment_lengths = {1, 4, 1, 2};
    auto alternative = head;
    alternative.text = "以货物运输的速度";
    alternative.score = 37000;
    if (!owo::ipc::standalone_head_alternative(head, alternative, 18)) return 17;
    const auto rejected = [&](auto change) {
        auto changed = alternative;
        change(changed);
        return !owo::ipc::standalone_head_alternative(head, changed, 18);
    };
    if (!rejected([](auto& value) { value.text = "以货物运输得速度"; }) ||
        !rejected([](auto& value) { value.text = "以货物运输的速"; }) ||
        !rejected([](auto& value) { value.segment_lengths = {2, 3, 1, 2}; }) ||
        !rejected([](auto& value) { value.consumed_input_bytes = 17; }) ||
        !rejected([](auto& value) { value.match_kind = owo::engine::InputMatchKind::corrected; }) ||
        !rejected([](auto& value) { value.source_segments.front() = "y"; }) ||
        !rejected([](auto& value) { value.user_lexicon = true; })) return 18;
    auto learned_head = head;
    learned_head.user_lexicon = true;
    if (owo::ipc::standalone_head_alternative(learned_head, alternative, 18)) return 19;
    head.segment_lengths = alternative.segment_lengths = {1, 1, 1, 1, 1, 1, 1, 1};
    head.segment_count = alternative.segment_count = 8;
    if (owo::ipc::standalone_head_alternative(head, alternative, 18)) return 20;
    return 0;
}
