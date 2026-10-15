#include "owo/model/model_backend.h"

#include <algorithm>
#include <mutex>
#include <thread>
#include <unordered_map>

namespace owo::model {

ModelResult MockModelBackend::rank(const ModelRequest& request, const std::stop_token stop) {
    const auto started = std::chrono::steady_clock::now();
    const auto deadline = started + request.timeout;
    while (std::chrono::steady_clock::now() - started < options_.latency) {
        if (stop.stop_requested()) return {request.request_id, ModelStatus::cancelled, {}, "cancelled"};
        if (std::chrono::steady_clock::now() >= deadline)
            return {request.request_id, ModelStatus::timeout, {}, "deadline exceeded"};
        std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }
    if (stop.stop_requested()) return {request.request_id, ModelStatus::cancelled, {}, "cancelled"};
    if (options_.fail)
        return {request.request_id, ModelStatus::backend_error, {}, "injected backend failure"};
    auto ranked = request.candidates;
    std::stable_sort(ranked.begin(), ranked.end());
    return {request.request_id, ModelStatus::success, std::move(ranked), {}};
}

struct AsyncModelScheduler::State {
    std::mutex mutex;
    std::unordered_map<std::uint64_t, std::stop_source> requests;
};

AsyncModelScheduler::AsyncModelScheduler(std::shared_ptr<IModelBackend> backend)
    : backend_(std::move(backend)), state_(std::make_shared<State>()) {}

std::future<ModelResult> AsyncModelScheduler::submit(ModelRequest request) {
    std::stop_source source;
    {
        std::lock_guard lock(state_->mutex);
        if (!state_->requests.emplace(request.request_id, source).second) {
            return std::async(std::launch::deferred, [id = request.request_id] {
                return ModelResult{id, ModelStatus::backend_error, {}, "duplicate request id"};
            });
        }
    }
    auto backend = backend_;
    auto state = state_;
    return std::async(std::launch::async,
                      [backend = std::move(backend), state = std::move(state),
                       source = std::move(source), request = std::move(request)]() mutable {
        auto result = backend->rank(request, source.get_token());
        std::lock_guard lock(state->mutex);
        state->requests.erase(request.request_id);
        return result;
    });
}

bool AsyncModelScheduler::cancel(const std::uint64_t request_id) {
    std::lock_guard lock(state_->mutex);
    const auto found = state_->requests.find(request_id);
    return found != state_->requests.end() && found->second.request_stop();
}

}  // namespace owo::model
