#include "owo/model/model_backend.h"
#include "owo/model/terminal_scoring.h"

#include <chrono>
#include <memory>
#include <thread>

using namespace std::chrono_literals;

int main() {
    using owo::model::calibrated_terminal_score;
    if (!owo::model::terminal_pair_flips(-9.15F, -9.89F, -11.61F, -9.89F) ||
        owo::model::terminal_pair_flips(-10.38F, -9.36F, -10.38F, -12.10F) ||
        owo::model::terminal_pair_flips(-9.0F, -9.0F, -9.0F, -12.0F)) return 4;
    if (calibrated_terminal_score(-9.0F, -12.0F) != -10.0F ||
        calibrated_terminal_score(-9.0F, -9.0F) != -9.0F) return 2;
    for (const auto whole : {-1.0F, -10.0F, -100.0F}) for (const auto loss : {0.0F, 1.0F, 50.0F}) {
        const auto split = whole - loss;
        const auto fused = calibrated_terminal_score(whole, split);
        if (fused < split || fused > whole) return 3;
    }
    const auto missing_libime =
        owo::model::load_libime_backend("missing-owo-libime-bridge.dll", "missing.zh_CN.lm");
    if (missing_libime || missing_libime.diagnostic.empty()) return 1;

    auto backend = std::make_shared<owo::model::MockModelBackend>();
    owo::model::AsyncModelScheduler scheduler(backend);
    auto success = scheduler.submit({1, "owo.mock.rank.v1", "sensitive-input", {"泥号", "你好"}, 100ms});
    const auto ranked = success.get();
    if (ranked.status != owo::model::ModelStatus::success ||
        ranked.candidates != std::vector<std::string>{"你好", "泥号"}) return 1;

    auto slow = std::make_shared<owo::model::MockModelBackend>(
        owo::model::MockBackendOptions{100ms, false});
    owo::model::AsyncModelScheduler cancellable(slow);
    auto cancelled = cancellable.submit({2, "owo.mock.rank.v1", {}, {"b", "a"}, 500ms});
    std::this_thread::sleep_for(5ms);
    if (!cancellable.cancel(2) || cancelled.get().status != owo::model::ModelStatus::cancelled) return 1;

    owo::model::AsyncModelScheduler expiring(slow);
    auto timeout = expiring.submit({3, "owo.mock.rank.v1", {}, {"b", "a"}, 5ms});
    if (timeout.get().status != owo::model::ModelStatus::timeout) return 1;

    auto failing_backend = std::make_shared<owo::model::MockModelBackend>(
        owo::model::MockBackendOptions{0ms, true});
    owo::model::AsyncModelScheduler failing(failing_backend);
    if (failing.submit({4, "owo.mock.rank.v1", {}, {"b", "a"}, 100ms}).get().status !=
        owo::model::ModelStatus::backend_error) return 1;
    return 0;
}
