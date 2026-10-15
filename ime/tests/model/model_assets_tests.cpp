#include "owo/model/model_assets.h"

#include <string>
#include <vector>

namespace {

owo::model::ModelManifest valid_manifest() {
    owo::model::ModelManifest value;
    value.model_id = "uer.chinese-roberta-mini.rank.v1";
    value.architecture = "bert";
    value.task = "candidate-ranking";
    value.format = "onnx";
    value.source_revision = std::string(40, 'a');
    value.model_sha256 = std::string(64, 'b');
    value.vocabulary_sha256 = std::string(64, 'c');
    value.license = "license-ref-local-evaluation";
    value.model_file = "model.onnx";
    value.vocabulary_file = "vocab.txt";
    return value;
}

}  // namespace

int main(const int argc, char** argv) {
    if (!owo::model::validate_manifest(valid_manifest()).ok) return 1;
    auto manifest = valid_manifest();
    manifest.license = "unknown";
    if (owo::model::validate_manifest(manifest).ok) return 2;
    manifest = valid_manifest();
    manifest.source_revision = "main";
    if (owo::model::validate_manifest(manifest).ok) return 3;
    manifest = valid_manifest();
    manifest.maximum_candidates = 0;
    if (owo::model::validate_manifest(manifest).ok) return 4;
    manifest = valid_manifest();
    manifest.model_file = "../model.onnx";
    if (owo::model::validate_manifest(manifest).ok) return 12;
    manifest = valid_manifest();
    manifest.task = "masked-lm";
    if (owo::model::validate_manifest(manifest).ok) return 17;
    manifest = valid_manifest();
    manifest.onnx_opset = 21;
    if (owo::model::validate_manifest(manifest).ok) return 18;
    manifest = valid_manifest();
    manifest.output_name = "input_ids";
    if (owo::model::validate_manifest(manifest).ok) return 19;
    manifest = valid_manifest();
    manifest.output_element_type = "float16";
    if (owo::model::validate_manifest(manifest).ok) return 20;
    const auto contract_manifest = valid_manifest();
    owo::model::OnnxModelMetadata metadata{
        17,
        {{"attention_mask", "int64", {-1, 64}},
         {"input_ids", "int64", {-1, 64}},
         {"token_type_ids", "int64", {-1, 64}}},
        {{"logits", "float32", {-1, 1}}}};
    if (!owo::model::validate_onnx_metadata(contract_manifest, metadata).ok) return 22;
    metadata.inputs[0].dimensions = {-1, -1};
    if (owo::model::validate_onnx_metadata(contract_manifest, metadata).ok) return 23;
    metadata.inputs[0].dimensions = {-1, 64};
    metadata.outputs[0].element_type = "float16";
    if (owo::model::validate_onnx_metadata(contract_manifest, metadata).ok) return 24;

    const std::vector<std::string> vocabulary = {
        "[PAD]", "[UNK]", "[CLS]", "[SEP]", "你", "好", "hello", "##s", "!"};
    const owo::model::WordPieceTokenizer tokenizer(vocabulary, true);
    if (!tokenizer.validation().ok) return 5;
    const auto chinese = tokenizer.encode("你好", 64);
    if (!chinese.ok || chinese.value.tokens !=
                           std::vector<std::string>{"[CLS]", "你", "好", "[SEP]"} ||
        chinese.value.input_ids != std::vector<std::int64_t>{2, 4, 5, 3}) return 6;
    const auto mixed = tokenizer.encode("HELLOs!", 64);
    if (!mixed.ok || mixed.value.tokens !=
                         std::vector<std::string>{"[CLS]", "hello", "##s", "!", "[SEP]"}) return 7;
    const auto unknown = tokenizer.encode("不存在", 64);
    if (!unknown.ok || unknown.value.tokens !=
                           std::vector<std::string>{"[CLS]", "[UNK]", "[UNK]", "[UNK]", "[SEP]"})
        return 8;
    if (tokenizer.encode("你好", 3).ok) return 9;
    const auto pair = tokenizer.encode_pair("你", "好", 5);
    if (!pair.ok || pair.value.tokens !=
                        std::vector<std::string>{"[CLS]", "你", "[SEP]", "好", "[SEP]"} ||
        pair.value.token_type_ids != std::vector<std::int64_t>{0, 0, 0, 1, 1}) return 15;
    if (tokenizer.encode_pair("你好", "你好", 6).ok) return 16;
    const std::string invalid_utf8{"\xE4\xB8", 2};
    if (tokenizer.encode(invalid_utf8, 64).ok) return 10;

    const owo::model::WordPieceTokenizer missing_special({"[UNK]", "[CLS]", "[SEP]"});
    if (missing_special.validation().ok || missing_special.encode("你", 64).ok) return 11;
    if (argc != 2) return 13;
    const auto loaded = owo::model::load_model_assets(argv[1]);
    if (!loaded.ok || loaded.value.vocabulary.size() != 8 ||
        loaded.value.manifest.model_id != "owo.synthetic.bert.v1") return 14;
    if (loaded.value.manifest.onnx_opset != 17 ||
        loaded.value.manifest.input_ids_name != "input_ids" ||
        loaded.value.manifest.output_name != "logits" ||
        loaded.value.manifest.output_columns != 1) return 21;
    const auto opset = owo::model::read_onnx_default_opset(loaded.value.model_path);
    if (!opset.ok || opset.opset != 17) return 25;
    return 0;
}
