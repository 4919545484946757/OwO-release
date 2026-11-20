#include "owo/ipc/contextual_candidate_order.h"

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
    return 0;
}
