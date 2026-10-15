#pragma once

#include "owo/model/model_assets.h"
#include "owo/model/model_inference.h"

#include <filesystem>
#include <memory>
#include <string>

namespace owo::model {

struct InferenceSessionCreateResult {
    std::shared_ptr<IInferenceSession> session;
    OnnxModelMetadata metadata;
    std::string diagnostic;
    [[nodiscard]] explicit operator bool() const noexcept { return session != nullptr; }
};

/// 创建仅使用 CPU Execution Provider 的 ORT session，并核对模型实际元数据。
[[nodiscard]] InferenceSessionCreateResult create_onnxruntime_cpu_session(
    const ModelManifest& manifest, const std::filesystem::path& model_path);

}  // namespace owo::model
