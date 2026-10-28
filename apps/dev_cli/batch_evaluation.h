#pragma once

#include "owo/protocol/messages.h"
#include <filesystem>
#include <functional>
#include <optional>
#include <ostream>
#include <string>
#include <vector>

namespace owo::dev {
struct EvaluationCase {
    std::string id, input, segmentation, context, previous;
    std::vector<std::string> expected, tags;
    std::optional<bool> correction;
    std::optional<std::size_t> consumed;
    bool valid{true};
};
struct EvaluationOptions {
    std::string mode{"both"}, typing{"whole"}, model{"base"};
    unsigned paths{32}, top_k{10}, max_pages{4}, repeat{1};
    bool correction{true}, check_expanded{}, allow_misses{};
    std::optional<double> min_top1, min_first_page, min_topk, max_p95_ms;
};
struct EvaluationReply {
    protocol::Message base, final;
    double base_ms{}, settled_ms{};
    bool model_requested{}, model_applied{}, model_failed{};
};
using EvaluationQuery = std::function<EvaluationReply(protocol::Message)>;
// Strict UTF-8 JSONL or headered TSV, validated in full before starting Core.
std::vector<EvaluationCase> read_evaluation_cases(const std::filesystem::path&, std::string format);
// Emits JSONL records and a final summary; never commits or changes learning.
// Returns 0, 1 (quality gate), or 4 (runtime/protocol failure).
int evaluate_cases(const std::vector<EvaluationCase>&, const EvaluationOptions&,
                   const EvaluationQuery&, const std::filesystem::path& report_directory,
                   std::string_view manifest_fields, std::ostream& output);
} // namespace owo::dev
