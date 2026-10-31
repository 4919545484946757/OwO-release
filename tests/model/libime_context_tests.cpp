#include "owo/model/model_backend.h"

#include <chrono>
#include <iostream>
#include <string>
#include <vector>

namespace {

bool first_is(owo::model::IModelBackend& backend,
              const std::string& context,
              const std::vector<std::string>& candidates,
              const std::string& expected) {
    owo::model::ModelRequest request;
    request.request_id = 1;
    request.model_id = "owo.libime.ngram.v1";
    request.input = "context-test";
    request.context = context;
    request.candidates = candidates;
    request.timeout = std::chrono::milliseconds(500);
    const auto result = backend.rank(request, {});
    if (result.status != owo::model::ModelStatus::success || result.candidates.empty()) {
        std::cerr << "rank failed for context: " << context << ": " << result.diagnostic << '\n';
        return false;
    }
    if (result.candidates.front() != expected) {
        std::cerr << "unexpected first candidate for context: " << context << '\n';
        return false;
    }
    return true;
}

}  // namespace

int main(int argc, char** argv) {
    if (argc != 3) return 2;
    auto loaded = owo::model::load_libime_backend(argv[1], argv[2]);
    if (!loaded) {
        std::cerr << loaded.diagnostic << '\n';
        return 3;
    }

    const std::vector<std::string> work_candidates{"工作", "世界", "问题"};
    const std::vector<std::string> learning_candidates{"计划", "方法", "中文"};
    if (!first_is(*loaded.backend, "解决", work_candidates, "问题") ||
        !first_is(*loaded.backend, "开始", work_candidates, "工作") ||
        !first_is(*loaded.backend, "制定", learning_candidates, "计划") ||
        !first_is(*loaded.backend, "学习", learning_candidates, "方法"))
        return 4;
    if (!first_is(*loaded.backend, "", {"价值大妈", "价值大吗"}, "价值大吗") ||
        !first_is(*loaded.backend, "", {"联系大吗", "联系大妈"}, "联系大妈") ||
        !first_is(*loaded.backend, "", {"购买芝吗", "购买芝麻"}, "购买芝麻") ||
        !first_is(*loaded.backend, "", {"这是一串代吗", "这是一串代码"}, "这是一串代码")) return 5;
    return 0;
}

