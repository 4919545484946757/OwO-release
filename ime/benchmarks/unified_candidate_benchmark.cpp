// Latency and output sanity for the modeless unified candidate entry point.
// The core service only calls this path on a candidate cache miss, so the
// numbers bound the extra work one keystroke can add.
#include "owo/engine/expressive_candidates.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/unified_input.h"

#include <algorithm>
#include <chrono>
#include <iostream>
#include <numeric>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace {

// Smoke bound, not a performance target: it exists to catch the class of
// regression where a correction-enabled or unbounded parse sneaks back into
// the unified entry point (that cost 60-260 ms in a Debug build).
constexpr std::uint64_t kLatencySmokeBoundNs = 200'000'000ULL;

struct Sample {
    std::string_view input;
    // Empty means the unified layer contributes nothing by design (pure
    // pinyin without an expressive or concept key).
    std::string_view expected_marker;
};

std::uint64_t percentile(const std::vector<std::uint64_t>& sorted,
                         const double ratio) {
    if (sorted.empty()) return 0;
    const auto index = static_cast<std::size_t>(
        ratio * static_cast<double>(sorted.size() - 1) + 0.5);
    return sorted[std::min(index, sorted.size() - 1)];
}

}  // namespace

int main() {
    const owo::engine::FullPinyinSchema schema;
    const owo::engine::MemoryLexicon lexicon({
        {{"ni", "hao"}, "你好", 1000},
        {{"ni", "hao"}, "你号", 50},
        {{"lei", "si"}, "累死", 300},
        {{"bai", "du"}, "百度", 2000},
        {{"chong", "fu"}, "重复", 900},
        {{"zhi", "xing"}, "执行", 800},
        {{"tong", "shi"}, "同时", 700},
    });
    const std::vector<Sample> samples{
        {"nihao", ""},
        {"ni3hao3", "你好"},
        {"baidu.com", "百度.com"},
        {"hello123", "hello123"},
        {"leisi", "😩"},
        {"chongfuzhixing", "幂等"},
        {"chongfuzh", "幂等"},
        {"1234", "一千二百三十四"},
        {"一千二百三十四", "1234"},
        {"2026-10-19", "2026年10月19日"},
        {"20261019", "2026-10-19"},
        {"2026-10-19~2026-10-25", "2026年10月19日 至 2026年10月25日"},
        {"90min", "1小时30分"},
        {"API", "API"},
        {"x!=y", "x≠y"},
        {"zhegehanshushixianlefuzadebingfakongzhiheziyuanguanli123",
         "zhegehanshushixianlefuzadebingfakongzhiheziyuanguanli123"},
    };
    constexpr int kWarmup = 200;
    constexpr int kIterations = 1000;
    const owo::engine::UnifiedCandidateOptions options;

    std::cout << "{\"benchmark\":\"owo.engine.unified_candidates\""
              << ",\"configuration\":\""
#ifdef NDEBUG
              << "Release"
#else
              << "Debug"
#endif
              << "\",\"warmup\":" << kWarmup << ",\"iterations\":" << kIterations
              << ",\"inputs\":[";
    bool first = true;
    for (const auto& sample : samples) {
        std::vector<std::uint64_t> timings;
        timings.reserve(kIterations);
        std::size_t candidate_count = 0;
        bool marker_found = false;
        for (int iteration = 0; iteration < kWarmup + kIterations; ++iteration) {
            const auto started = std::chrono::steady_clock::now();
            auto candidates = owo::engine::generate_unified_candidates(
                schema, lexicon, sample.input, nullptr, nullptr, options);
            const auto elapsed = std::chrono::steady_clock::now() - started;
            if (iteration < kWarmup) continue;
            timings.push_back(static_cast<std::uint64_t>(
                std::chrono::duration_cast<std::chrono::nanoseconds>(elapsed)
                    .count()));
            candidate_count = candidates.size();
            marker_found = sample.expected_marker.empty() ||
                           std::any_of(
                               candidates.begin(), candidates.end(),
                               [&sample](const owo::engine::Candidate& candidate) {
                                   return candidate.text == sample.expected_marker;
                               });
        }
        std::sort(timings.begin(), timings.end());
        const auto total = std::accumulate(timings.begin(), timings.end(),
                                           std::uint64_t{});
        const auto p95 = percentile(timings, 0.95);
        std::cout << (first ? "" : ",") << "{\"input\":\"" << sample.input
                  << "\",\"bytes\":" << sample.input.size()
                  << ",\"candidates\":" << candidate_count
                  << ",\"marker\":" << (marker_found ? "true" : "false")
                  << ",\"latency_ns\":{\"mean\":" << (total / timings.size())
                  << ",\"p95\":" << p95
                  << ",\"max\":" << timings.back() << "}}";
        first = false;
        if (!marker_found || p95 > kLatencySmokeBoundNs) return 3;
    }
    std::cout << "],\"latency_smoke_bound_ns\":" << kLatencySmokeBoundNs << "}\n";
    return 0;
}
