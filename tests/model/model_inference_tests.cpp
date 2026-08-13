#include "owo/model/model_inference.h"

#include <chrono>
#include <memory>
#include <stop_token>
#include <vector>

using namespace std::chrono_literals;

namespace {

owo::model::ModelManifest manifest() {
    owo::model::ModelManifest value;
    value.model_id = "owo.synthetic.bert.v1";
    value.architecture = "bert";
    value.task = "candidate-ranking";
    value.format = "onnx";
    value.maximum_sequence_length = 8;
    value.maximum_candidates = 2;
    return value;
}

std::vector<std::string> vocabulary() {
    return {"[PAD]", "[UNK]", "[CLS]", "[SEP]", "你", "好", "泥", "号"};
}

}  // namespace

int main(const int argc, char** argv) {
    auto session = std::make_shared<owo::model::SyntheticInferenceSession>();
    owo::model::AssetCandidateRanker ranker(manifest(), vocabulary(), session);
    if (ranker.id() != "owo.synthetic.bert.v1") return 1;
    const auto ranked = ranker.rank({1, std::string(ranker.id()), "你", {"你好", "泥号"}, 100ms}, {});
    if (ranked.status != owo::model::ModelStatus::success || ranked.candidates.size() != 2 ||
        ranked.candidates[0] != "泥号") return 2;

    const auto too_many = ranker.rank(
        {2, std::string(ranker.id()), "你", {"你", "好", "泥"}, 100ms}, {});
    if (too_many.status != owo::model::ModelStatus::backend_error) return 3;
    if (ranker.rank({7, std::string(ranker.id()), "你", {""}, 100ms}, {}).status !=
        owo::model::ModelStatus::backend_error) return 13;
    const auto too_long = ranker.rank(
        {3, std::string(ranker.id()), "你你你你你你", {"你好"}, 100ms}, {});
    if (too_long.status != owo::model::ModelStatus::backend_error) return 4;

    auto slow_session = std::make_shared<owo::model::SyntheticInferenceSession>(
        owo::model::SyntheticSessionOptions{100ms, false});
    owo::model::AssetCandidateRanker slow(manifest(), vocabulary(), slow_session);
    if (slow.rank({4, std::string(slow.id()), "你", {"你好"}, 20ms}, {}).status !=
        owo::model::ModelStatus::timeout) return 5;
    std::stop_source cancelled;
    cancelled.request_stop();
    if (slow.rank({5, std::string(slow.id()), "你", {"你好"}, 100ms}, cancelled.get_token()).status !=
        owo::model::ModelStatus::cancelled) return 6;

    owo::model::InferenceBatch invalid{2, 4, {1, 2}, {1, 1}, {0, 0}};
    if (session->run(invalid, {}, 100ms).status != owo::model::ModelStatus::backend_error) return 7;
    if (argc != 2) return 8;
    auto loaded = owo::model::load_model_assets(argv[1]);
    if (!loaded.ok) return 9;
    owo::model::WordPieceTokenizer loaded_tokenizer(loaded.value.vocabulary);
    const auto loaded_first = loaded_tokenizer.encode_pair("你", "你好", 64);
    const auto loaded_second = loaded_tokenizer.encode_pair("你", "泥号", 64);
    if (!loaded_first.ok || loaded_first.value.input_ids !=
                                std::vector<std::int64_t>{2, 4, 3, 4, 5, 3}) return 11;
    if (!loaded_second.ok || loaded_second.value.input_ids !=
                                 std::vector<std::int64_t>{2, 4, 3, 6, 7, 3}) return 12;
    owo::model::AssetCandidateRanker loaded_ranker(
        std::move(loaded.value.manifest), std::move(loaded.value.vocabulary), session);
    const auto loaded_result = loaded_ranker.rank(
        {6, std::string(loaded_ranker.id()), "你", {"你好", "泥号"}, 100ms}, {});
    if (loaded_result.status != owo::model::ModelStatus::success ||
        loaded_result.candidates != std::vector<std::string>{"泥号", "你好"}) return 10;
    return 0;
}
