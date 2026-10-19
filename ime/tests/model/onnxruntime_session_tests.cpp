#include "owo/model/model_assets.h"
#include "owo/model/model_inference.h"
#include "owo/model/onnxruntime_session.h"

#include <chrono>
#include <stop_token>

using namespace std::chrono_literals;

int main(const int argc, char** argv) {
    if (argc != 2) return 1;
    auto assets = owo::model::load_model_assets(argv[1]);
    if (!assets.ok) return 2;
    const auto created = owo::model::create_onnxruntime_cpu_session(
        assets.value.manifest, assets.value.model_path);
    if (!created) return 3;
    owo::model::AssetCandidateRanker ranker(
        assets.value.manifest, assets.value.vocabulary, created.session);
    const auto result = ranker.rank(
        {1, std::string(ranker.id()), "你", {"你好", "泥号"}, 100ms}, {});
    if (result.status != owo::model::ModelStatus::success ||
        result.candidates != std::vector<std::string>{"泥号", "你好"}) return 4;
    const auto opset = owo::model::read_onnx_default_opset(assets.value.model_path);
    if (!opset.ok || opset.opset != 17) return 5;
    owo::model::InferenceBatch malformed{1, 64, {1}, {1}, {1}};
    if (created.session->run(malformed, {}, 100ms).status !=
        owo::model::ModelStatus::backend_error) return 6;
    owo::model::InferenceBatch batch{1, 64, std::vector<std::int64_t>(64),
                                    std::vector<std::int64_t>(64),
                                    std::vector<std::int64_t>(64)};
    if (created.session->run(batch, {}, 0ms).status != owo::model::ModelStatus::timeout) return 7;
    std::stop_source cancelled;
    cancelled.request_stop();
    if (created.session->run(batch, cancelled.get_token(), 100ms).status !=
        owo::model::ModelStatus::cancelled) return 8;
    return 0;
}
