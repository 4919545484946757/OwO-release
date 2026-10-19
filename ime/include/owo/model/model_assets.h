#pragma once

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace owo::model {

inline constexpr std::uint32_t kModelManifestVersion = 1;

struct ModelManifest {
    std::uint32_t manifest_version{kModelManifestVersion};
    std::string model_id;
    std::string architecture;
    std::string task;
    std::string format;
    std::string source_revision;
    std::string model_sha256;
    std::string vocabulary_sha256;
    std::string license;
    std::string model_file;
    std::string vocabulary_file;
    std::uint32_t onnx_opset{17};
    std::string input_ids_name{"input_ids"};
    std::string attention_mask_name{"attention_mask"};
    std::string token_type_ids_name{"token_type_ids"};
    std::string output_name{"logits"};
    std::string input_element_type{"int64"};
    std::string output_element_type{"float32"};
    std::size_t output_columns{1};
    std::size_t maximum_sequence_length{64};
    std::size_t maximum_candidates{8};
};

struct ValidationResult {
    bool ok{};
    std::string diagnostic;
};

/// 校验 ModelHost 内部模型资产契约；不代表接受模型许可证或授权分发。
[[nodiscard]] ValidationResult validate_manifest(const ModelManifest& manifest);

struct TensorMetadata {
    std::string name;
    std::string element_type;
    std::vector<std::int64_t> dimensions;
};

struct OnnxModelMetadata {
    std::uint32_t opset{};
    std::vector<TensorMetadata> inputs;
    std::vector<TensorMetadata> outputs;
};

/// 比较运行时读取的 ONNX 元数据与 manifest；-1 仅允许表示动态 batch。
[[nodiscard]] ValidationResult validate_onnx_metadata(const ModelManifest& manifest,
                                                      const OnnxModelMetadata& metadata);

struct ModelAssetBundle {
    ModelManifest manifest;
    std::filesystem::path model_path;
    std::filesystem::path vocabulary_path;
    std::vector<std::string> vocabulary;
};

struct ModelAssetLoadResult {
    bool ok{};
    ModelAssetBundle value;
    std::string diagnostic;
};

struct OnnxOpsetResult {
    bool ok{};
    std::uint32_t opset{};
    std::string diagnostic;
};

/// 流式读取 ONNX ModelProto 的默认域 opset，不解析或分配模型权重。
[[nodiscard]] OnnxOpsetResult read_onnx_default_opset(const std::filesystem::path& model_path);

/// 从严格 key=value 文件加载并核验模型资产；文件名只能指向 manifest 同目录。
[[nodiscard]] ModelAssetLoadResult load_model_assets(const std::filesystem::path& manifest_path);

struct TokenizedInput {
    std::vector<std::int64_t> input_ids;
    std::vector<std::int64_t> attention_mask;
    std::vector<std::int64_t> token_type_ids;
    std::vector<std::string> tokens;
};

struct TokenizeResult {
    bool ok{};
    TokenizedInput value;
    std::string diagnostic;
};

class WordPieceTokenizer final {
public:
    /// 词表顺序即模型 token ID；构造时要求包含四个 BERT 特殊 token。
    explicit WordPieceTokenizer(std::vector<std::string> vocabulary, bool lowercase_ascii = false);

    [[nodiscard]] ValidationResult validation() const;
    [[nodiscard]] std::int64_t pad_token_id() const;
    /// 严格 UTF-8；输出自动包含 [CLS]/[SEP]，超限时失败而非截断。
    [[nodiscard]] TokenizeResult encode(std::string_view text,
                                        std::size_t maximum_sequence_length) const;
    /// 编码 BERT 文本对：[CLS] first [SEP] second [SEP]，第二段 token type 为 1。
    [[nodiscard]] TokenizeResult encode_pair(std::string_view first, std::string_view second,
                                             std::size_t maximum_sequence_length) const;

private:
    std::vector<std::string> vocabulary_;
    std::unordered_map<std::string, std::int64_t> token_ids_;
    bool lowercase_ascii_{};
    ValidationResult validation_;
};

}  // namespace owo::model
