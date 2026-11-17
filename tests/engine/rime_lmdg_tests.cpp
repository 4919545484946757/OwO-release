#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/rime_lmdg.h"

#include <algorithm>
#include <array>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <vector>

namespace {
constexpr std::size_t kTable = 44;
constexpr std::uint32_t kUnits = 4096;
void put(std::vector<unsigned char>& bytes, const std::size_t pos, const std::uint32_t value) {
    for (unsigned i = 0; i < 4; ++i) bytes[pos + i] = static_cast<unsigned char>(value >> (8 * i));
}
void write(const std::filesystem::path& path, const std::vector<unsigned char>& bytes) {
    std::ofstream file(path, std::ios::binary | std::ios::trunc);
    file.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
}
}

int main(int argc, char** argv) {
    using namespace owo::engine;
    if (argc != 2) return 2;
    const auto directory = std::filesystem::absolute(argv[1]);
    std::filesystem::create_directories(directory);
    const auto path = directory / ("fixture-" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()) + ".gram");
    struct Cleanup { std::filesystem::path file; ~Cleanup() { std::error_code error; std::filesystem::remove(file, error); } } cleanup{path};
    auto bytes = std::vector<unsigned char>(kTable + 4 * kUnits);
    constexpr std::string_view version = "Rime::Grammar/1.0";
    std::copy(version.begin(), version.end(), bytes.begin());
    put(bytes, 36, kUnits);
    put(bytes, 40, 4);
    constexpr std::array<std::uint32_t, 8> key{0xb5, 0x32, 0x8e, 0x59, 0x8e, 0x19, 0x8e, 0x01}; // 甲乙丙丁
    std::uint32_t node = 0, label = 0;
    for (std::size_t i = 0; i < key.size(); ++i) {
        const auto child = static_cast<std::uint32_t>(256 * (i + 1));
        put(bytes, kTable + 4 * node, ((node ^ child ^ key[i]) << 10) | label);
        node = child;
        label = key[i];
    }
    constexpr std::uint32_t leaf = 4'000;
    put(bytes, kTable + 4 * node, ((node ^ leaf) << 10) | 0x100 | label);
    put(bytes, kTable + 4 * leaf, 0x80000000U | 175'000U);
    write(path, bytes);
    RimeLmdg model;
    if (!model.load(path).empty() || model.boundary_bonus("甲乙", "丙丁") != 5000 ||
        model.boundary_bonus("甲以", "丙丁") != 0 || model.boundary_bonus("甲，", "丙丁") != 0 ||
        model.boundary_bonus("甲", "乙丙丁") != 0) return 3;
    const MemoryLexicon lexicon({{{"jia", "yi"}, "甲乙", 700}, {{"jia", "yi"}, "甲以", 1600},
        {{"bing", "ding"}, "丙丁", 1800}});
    const CandidateGenerator generator(lexicon);
    const auto parsed = FullPinyinSchema{}.parse("jiayibingding", 16, false);
    CandidateGenerationMetrics metrics;
    const auto base = generator.generate(parsed, 16, false, {}, nullptr, {}, false, 7);
    const auto assisted = generator.generate(parsed, 16, false, {}, &metrics, {}, false, 7, true, 0, 0, false, false, &model);
    if (base.empty() || base.front().text != "甲以丙丁" || assisted.empty() || assisted.front().text != "甲乙丙丁" ||
        assisted.front().lmdg_bonus < RimeLmdg::kConfirmedSentenceBonus ||
        assisted.front().consumed_input_bytes != parsed.normalized_input.size() || !metrics.lmdg_hits ||
        metrics.lmdg_queries > RimeLmdg::kMaximumQueries) return 4;
    (void)model.load(directory / "intentionally-not-a-model.gram");
    if (model.ready() || model.boundary_bonus("甲乙", "丙丁")) return 5;
    // The header/root opens, but a malicious interior child reference cannot
    // turn into an out-of-range read or produce a candidate score.
    put(bytes, kTable + 4 * 256, (9000U << 10) | key.front());
    write(path, bytes);
    if (!model.load(path).empty() || model.boundary_bonus("甲乙", "丙丁")) return 6;
    (void)model.load(directory / "intentionally-not-a-model.gram");
    put(bytes, 36, 0xffffffffU);
    write(path, bytes);
    if (model.load(path).empty() || model.ready()) return 7;
    std::cout << "Rime grammar mapping, bounds, and in-beam score contract passed.\n";
}
