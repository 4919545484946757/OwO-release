#include "owo/model/model_assets.h"
#include "owo/model/onnxruntime_session.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#include <Psapi.h>

#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <iostream>
#include <vector>

namespace {

double percentile(const std::vector<double>& sorted, const double fraction) {
    const auto index = static_cast<std::size_t>(fraction * static_cast<double>(sorted.size() - 1));
    return sorted[index];
}

std::size_t peak_working_set_bytes() {
    PROCESS_MEMORY_COUNTERS counters{sizeof(counters)};
    if (!GetProcessMemoryInfo(GetCurrentProcess(), &counters, sizeof(counters))) return 0;
    return counters.PeakWorkingSetSize;
}

owo::model::InferenceBatch make_batch(const std::size_t batch_size,
                                      const std::size_t sequence_length) {
    const auto count = batch_size * sequence_length;
    owo::model::InferenceBatch batch;
    batch.batch_size = batch_size;
    batch.sequence_length = sequence_length;
    batch.input_ids.assign(count, 1);
    batch.attention_mask.assign(count, 1);
    batch.token_type_ids.assign(count, 0);
    return batch;
}

}  // namespace

int main(const int argc, char** argv) {
    if (argc != 2) return 2;
    const auto process_start = std::chrono::steady_clock::now();
    const auto baseline_peak = peak_working_set_bytes();
    const auto assets = owo::model::load_model_assets(argv[1]);
    if (!assets.ok) return 3;
    const auto assets_ready = std::chrono::steady_clock::now();
    const auto created = owo::model::create_onnxruntime_cpu_session(
        assets.value.manifest, assets.value.model_path);
    if (!created) return 4;
    const auto session_ready = std::chrono::steady_clock::now();

    constexpr std::array<std::size_t, 4> batch_sizes{1, 2, 4, 8};
    constexpr std::size_t warmup_count = 20;
    constexpr std::size_t sample_count = 500;
    std::cout << "{\"benchmark\":\"owo.model.onnxruntime_cpu\",\"configuration\":\"Release\""
              << ",\"warmup\":" << warmup_count << ",\"samples_per_batch\":" << sample_count
              << ",\"asset_validation_ms\":"
              << std::chrono::duration<double, std::milli>(assets_ready - process_start).count()
              << ",\"session_creation_ms\":"
              << std::chrono::duration<double, std::milli>(session_ready - assets_ready).count()
              << ",\"baseline_peak_working_set_bytes\":" << baseline_peak << ",\"batches\":[";

    bool first = true;
    for (const auto batch_size : batch_sizes) {
        const auto batch = make_batch(batch_size, assets.value.manifest.maximum_sequence_length);
        for (std::size_t index = 0; index < warmup_count; ++index) {
            if (created.session->run(batch, {}, std::chrono::seconds(1)).status !=
                owo::model::ModelStatus::success) return 5;
        }
        std::vector<double> samples;
        samples.reserve(sample_count);
        for (std::size_t index = 0; index < sample_count; ++index) {
            const auto start = std::chrono::steady_clock::now();
            const auto result = created.session->run(batch, {}, std::chrono::seconds(1));
            if (result.status != owo::model::ModelStatus::success) return 6;
            samples.push_back(std::chrono::duration<double, std::micro>(
                                  std::chrono::steady_clock::now() - start).count());
        }
        std::sort(samples.begin(), samples.end());
        if (!first) std::cout << ',';
        first = false;
        std::cout << "{\"batch_size\":" << batch_size << ",\"latency_us\":{\"p50\":"
                  << percentile(samples, 0.50) << ",\"p95\":" << percentile(samples, 0.95)
                  << ",\"p99\":" << percentile(samples, 0.99)
                  << ",\"max\":" << samples.back() << "}}";
    }
    std::cout << "],\"peak_working_set_bytes\":" << peak_working_set_bytes() << "}\n";
    return 0;
}
