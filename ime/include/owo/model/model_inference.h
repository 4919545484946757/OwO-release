#pragma once

#include "owo/model/model_assets.h"
#include "owo/model/model_backend.h"

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <memory>
#include <stop_token>
#include <string>
#include <string_view>
#include <vector>

namespace owo::model {

struct InferenceBatch {
    std::size_t batch_size{};
    std::size_t sequence_length{};
    std::vector<std::int64_t> input_ids;
    std::vector<std::int64_t> attention_mask;
    std::vector<std::int64_t> token_type_ids;
};

struct InferenceResult {
    ModelStatus status{ModelStatus::backend_error};
    std::vector<float> scores;
    std::string diagnostic;
};

class IInferenceSession {
public:
    virtual ~IInferenceSession() = default;
    [[nodiscard]] virtual InferenceResult run(const InferenceBatch&, std::stop_token,
                                              std::chrono::milliseconds timeout) = 0;
};

struct SyntheticSessionOptions {
    std::chrono::milliseconds latency{};
    bool fail{};
};

/// 仅用于契约测试；按 token ID 生成确定性分数，不读取或解释 ONNX 文件。
class SyntheticInferenceSession final : public IInferenceSession {
public:
    explicit SyntheticInferenceSession(SyntheticSessionOptions options = {}) : options_(options) {}
    [[nodiscard]] InferenceResult run(const InferenceBatch&, std::stop_token,
                                      std::chrono::milliseconds timeout) override;

private:
    SyntheticSessionOptions options_;
};

class AssetCandidateRanker final : public IModelBackend {
public:
    AssetCandidateRanker(ModelManifest manifest, std::vector<std::string> vocabulary,
                         std::shared_ptr<IInferenceSession> session);
    [[nodiscard]] std::string_view id() const noexcept override;
    [[nodiscard]] ModelResult rank(const ModelRequest&, std::stop_token) override;

private:
    ModelManifest manifest_;
    WordPieceTokenizer tokenizer_;
    std::shared_ptr<IInferenceSession> session_;
};

}  // namespace owo::model
