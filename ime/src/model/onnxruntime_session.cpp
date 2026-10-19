#include "owo/model/onnxruntime_session.h"

#include <onnxruntime_cxx_api.h>

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>

#include <array>
#include <chrono>
#include <limits>

namespace owo::model {
namespace {

std::string element_type_name(const ONNXTensorElementDataType type) {
    if (type == ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64) return "int64";
    if (type == ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT) return "float32";
    return "unsupported";
}

void request_termination(OrtRunOptions* options) noexcept {
    if (auto* status = Ort::GetApi().RunOptionsSetTerminate(options); status != nullptr)
        Ort::GetApi().ReleaseStatus(status);
}

void CALLBACK timeout_callback(PTP_CALLBACK_INSTANCE, void* context, PTP_TIMER) noexcept {
    request_termination(static_cast<OrtRunOptions*>(context));
}

class DeadlineTimer final {
public:
    DeadlineTimer(OrtRunOptions* options, const std::chrono::milliseconds timeout)
        : timer_(CreateThreadpoolTimer(timeout_callback, options, nullptr)) {
        if (timer_ == nullptr) return;
        const auto relative_100ns = -static_cast<LONGLONG>(timeout.count()) * 10'000;
        ULARGE_INTEGER due_value{};
        due_value.QuadPart = static_cast<ULONGLONG>(relative_100ns);
        FILETIME due_time{due_value.LowPart, due_value.HighPart};
        SetThreadpoolTimer(timer_, &due_time, 0, 0);
    }
    DeadlineTimer(const DeadlineTimer&) = delete;
    DeadlineTimer& operator=(const DeadlineTimer&) = delete;
    ~DeadlineTimer() {
        if (timer_ == nullptr) return;
        SetThreadpoolTimer(timer_, nullptr, 0, 0);
        WaitForThreadpoolTimerCallbacks(timer_, TRUE);
        CloseThreadpoolTimer(timer_);
    }
    [[nodiscard]] explicit operator bool() const noexcept { return timer_ != nullptr; }

private:
    PTP_TIMER timer_{};
};

class OrtCpuSession final : public IInferenceSession {
public:
    OrtCpuSession(const std::filesystem::path& model_path, const ModelManifest& manifest)
        : env_(ORT_LOGGING_LEVEL_WARNING, "OwO.ModelHost"), manifest_(manifest) {
        options_.SetIntraOpNumThreads(1);
        options_.SetInterOpNumThreads(1);
        options_.SetExecutionMode(ExecutionMode::ORT_SEQUENTIAL);
        options_.SetGraphOptimizationLevel(GraphOptimizationLevel::ORT_ENABLE_EXTENDED);
        session_ = std::make_unique<Ort::Session>(env_, model_path.c_str(), options_);
    }

    [[nodiscard]] OnnxModelMetadata metadata(const std::uint32_t opset) const {
        Ort::AllocatorWithDefaultOptions allocator;
        OnnxModelMetadata result;
        result.opset = opset;
        for (std::size_t index = 0; index < session_->GetInputCount(); ++index) {
            auto name = session_->GetInputNameAllocated(index, allocator);
            const auto info = session_->GetInputTypeInfo(index).GetTensorTypeAndShapeInfo();
            result.inputs.push_back({name.get(), element_type_name(info.GetElementType()),
                                     info.GetShape()});
        }
        for (std::size_t index = 0; index < session_->GetOutputCount(); ++index) {
            auto name = session_->GetOutputNameAllocated(index, allocator);
            const auto info = session_->GetOutputTypeInfo(index).GetTensorTypeAndShapeInfo();
            result.outputs.push_back({name.get(), element_type_name(info.GetElementType()),
                                      info.GetShape()});
        }
        return result;
    }

    [[nodiscard]] InferenceResult run(const InferenceBatch& batch, const std::stop_token stop,
                                      const std::chrono::milliseconds timeout) override {
        if (timeout.count() <= 0) return {ModelStatus::timeout, {}, "deadline exceeded"};
        if (batch.batch_size == 0 || batch.sequence_length == 0 ||
            batch.batch_size > std::numeric_limits<std::size_t>::max() / batch.sequence_length) {
            return {ModelStatus::backend_error, {}, "invalid inference batch shape"};
        }
        const auto element_count = batch.batch_size * batch.sequence_length;
        if (batch.input_ids.size() != element_count ||
            batch.attention_mask.size() != element_count ||
            batch.token_type_ids.size() != element_count) {
            return {ModelStatus::backend_error, {}, "inference tensor size mismatch"};
        }
        const auto deadline = std::chrono::steady_clock::now() + timeout;
        try {
            const std::array<std::int64_t, 2> shape{
                static_cast<std::int64_t>(batch.batch_size),
                static_cast<std::int64_t>(batch.sequence_length)};
            auto memory = Ort::MemoryInfo::CreateCpu(OrtArenaAllocator, OrtMemTypeDefault);
            std::array<Ort::Value, 3> inputs{
                Ort::Value::CreateTensor<std::int64_t>(
                    memory, const_cast<std::int64_t*>(batch.input_ids.data()), batch.input_ids.size(),
                    shape.data(), shape.size()),
                Ort::Value::CreateTensor<std::int64_t>(
                    memory, const_cast<std::int64_t*>(batch.attention_mask.data()),
                    batch.attention_mask.size(), shape.data(), shape.size()),
                Ort::Value::CreateTensor<std::int64_t>(
                    memory, const_cast<std::int64_t*>(batch.token_type_ids.data()),
                    batch.token_type_ids.size(), shape.data(), shape.size())};
            std::array<const char*, 3> input_names{
                manifest_.input_ids_name.c_str(), manifest_.attention_mask_name.c_str(),
                manifest_.token_type_ids_name.c_str()};
            const std::array<const char*, 1> output_names{manifest_.output_name.c_str()};
            Ort::RunOptions run_options;
            DeadlineTimer timer(run_options, timeout);
            if (!timer)
                return {ModelStatus::backend_error, {}, "cannot create inference deadline timer"};
            std::stop_callback cancellation(stop, [&] { request_termination(run_options); });
            auto outputs = session_->Run(run_options, input_names.data(), inputs.data(), inputs.size(),
                                         output_names.data(), output_names.size());
            if (stop.stop_requested()) return {ModelStatus::cancelled, {}, "cancelled"};
            if (std::chrono::steady_clock::now() >= deadline)
                return {ModelStatus::timeout, {}, "deadline exceeded"};
            if (outputs.size() != 1 || !outputs.front().IsTensor())
                return {ModelStatus::backend_error, {}, "ORT output is not one tensor"};
            const auto output_info = outputs.front().GetTensorTypeAndShapeInfo();
            if (output_info.GetElementType() != ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT ||
                output_info.GetElementCount() != batch.batch_size)
                return {ModelStatus::backend_error, {}, "ORT output tensor shape or type changed"};
            const auto* scores = outputs.front().GetTensorData<float>();
            return {ModelStatus::success, std::vector<float>(scores, scores + batch.batch_size), {}};
        } catch (const Ort::Exception& error) {
            if (stop.stop_requested()) return {ModelStatus::cancelled, {}, "cancelled"};
            if (std::chrono::steady_clock::now() >= deadline)
                return {ModelStatus::timeout, {}, "deadline exceeded"};
            return {ModelStatus::backend_error, {}, std::string("ORT run failed: ") + error.what()};
        }
    }

private:
    Ort::Env env_;
    Ort::SessionOptions options_;
    ModelManifest manifest_;
    std::unique_ptr<Ort::Session> session_;
};

}  // namespace

InferenceSessionCreateResult create_onnxruntime_cpu_session(
    const ModelManifest& manifest, const std::filesystem::path& model_path) {
    const auto opset = read_onnx_default_opset(model_path);
    if (!opset.ok) return {{}, {}, opset.diagnostic};
    try {
        auto session = std::make_shared<OrtCpuSession>(model_path, manifest);
        auto metadata = session->metadata(opset.opset);
        const auto validation = validate_onnx_metadata(manifest, metadata);
        if (!validation.ok) return {{}, std::move(metadata), validation.diagnostic};
        return {std::move(session), std::move(metadata), {}};
    } catch (const Ort::Exception& error) {
        return {{}, {}, std::string("ORT session creation failed: ") + error.what()};
    }
}

}  // namespace owo::model
