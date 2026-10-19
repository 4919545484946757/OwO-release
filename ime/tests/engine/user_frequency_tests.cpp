#include "owo/engine/user_frequency.h"

#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <span>
#include <vector>

namespace {

void append_u32(std::vector<unsigned char>& bytes, const std::uint32_t value) {
    for (unsigned shift = 0; shift < 32; shift += 8)
        bytes.push_back(static_cast<unsigned char>(value >> shift));
}

void append_u64(std::vector<unsigned char>& bytes, const std::uint64_t value) {
    for (unsigned shift = 0; shift < 64; shift += 8)
        bytes.push_back(static_cast<unsigned char>(value >> shift));
}

std::uint64_t checksum(const std::span<const unsigned char> bytes) {
    std::uint64_t value = 14695981039346656037ULL;
    for (const auto byte : bytes) { value ^= byte; value *= 1099511628211ULL; }
    return value;
}

void write_v1(const std::filesystem::path& path) {
    constexpr std::string_view text = "legacy";
    std::vector<unsigned char> payload;
    append_u32(payload, static_cast<std::uint32_t>(text.size()));
    payload.insert(payload.end(), text.begin(), text.end());
    append_u32(payload, 4);
    std::vector<unsigned char> bytes{'O', 'W', 'U', 'F'};
    append_u32(bytes, 1);
    append_u32(bytes, 1);
    append_u64(bytes, checksum(payload));
    bytes.insert(bytes.end(), payload.begin(), payload.end());
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    output.write(reinterpret_cast<const char*>(bytes.data()),
                 static_cast<std::streamsize>(bytes.size()));
}

void write_v2(const std::filesystem::path& path) {
    constexpr std::string_view input = "nihao";
    constexpr std::string_view text = "legacy-context";
    std::vector<unsigned char> payload;
    append_u32(payload, static_cast<std::uint32_t>(input.size()));
    append_u32(payload, static_cast<std::uint32_t>(text.size()));
    payload.insert(payload.end(), input.begin(), input.end());
    payload.insert(payload.end(), text.begin(), text.end());
    append_u32(payload, 3);
    std::vector<unsigned char> bytes{'O', 'W', 'U', 'F'};
    append_u32(bytes, 2);
    append_u32(bytes, 1);
    append_u64(bytes, checksum(payload));
    bytes.insert(bytes.end(), payload.begin(), payload.end());
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    output.write(reinterpret_cast<const char*>(bytes.data()),
                 static_cast<std::streamsize>(bytes.size()));
}

}  // namespace

int main() {
    auto path = std::filesystem::temp_directory_path() / "owo-user-frequency-test.bin";
    std::error_code ignored;
    std::filesystem::remove(path, ignored);
    std::filesystem::remove(path.wstring() + L".bak", ignored);

    owo::engine::UserFrequencyStore store;
    if (!store.load(path).success) return 1;
    store.record("你好", 2);
    store.record("nihao", "你好", 2);
    if (!store.flush().success) return 1;
    store.record("你好", 3);
    store.record("西安");
    store.record("我喜欢", "pingguo", "苹果", 2);
    if (!store.flush().success) return 1;

    owo::engine::UserFrequencyStore loaded;
    if (!loaded.load(path).success || loaded.count("你好") != 7 ||
        loaded.count("西安") != 1 || loaded.score("你好") <= loaded.score("西安"))
        return 1;
    if (loaded.contextual_count("nihao", "你好") != 2 ||
        loaded.contextual_count("xian", "你好") != 0) return 2;
    const auto persisted_learned = loaded.lookup("nihao", 4);
    if (persisted_learned.size() != 1 ||
        persisted_learned[0].text != "你好" ||
        persisted_learned[0].count != 2) return 8;
    loaded.record("xi'an", "西安", 2);
    const auto compact_boundary = loaded.lookup("xian", 4);
    if (compact_boundary.size() != 1 || compact_boundary[0].text != "西安")
        return 9;
    loaded.record("zidingyici", "自定义词", 3);
    loaded.record("zidingyici", "自订义词", 1);
    const auto learned = loaded.lookup("zidingyici", 1);
    if (learned.size() != 1 || learned[0].text != "自定义词" ||
        learned[0].count != 3 || !loaded.lookup("missing", 4).empty()) return 7;
    const auto learned_prefix = loaded.lookup_prefix("zidingyic", 4);
    if (learned_prefix.size() != 2 ||
        learned_prefix[0].text != "自定义词" ||
        learned_prefix[0].input != "zidingyici" ||
        !loaded.lookup_prefix("z", 4).empty()) return 10;
    const auto exact_preference =
        loaded.language_context_score("我喜欢", "pingguo", "苹果");
    const auto generalized_preference =
        loaded.language_context_score("他喜欢", "pingguo", "苹果");
    if (exact_preference <= generalized_preference || generalized_preference <= 0)
        return 5;
    if (loaded.language_context_score("昨天我喜欢", "pingguo", "苹果") <= 0 ||
        loaded.language_context_score("昨天我讨厌", "pingguo", "苹果") != 0)
        return 15;
    loaded.record("我不喜欢", "pingguo", "香蕉", 2);
    const auto apple_after_like =
        loaded.language_context_score("昨天我喜欢", "pingguo", "苹果");
    const auto banana_after_like =
        loaded.language_context_score("昨天我喜欢", "pingguo", "香蕉");
    const auto banana_after_dislike =
        loaded.language_context_score("大家都不喜欢", "pingguo", "香蕉");
    const auto apple_after_dislike =
        loaded.language_context_score("大家都不喜欢", "pingguo", "苹果");
    if (apple_after_like <= banana_after_like ||
        banana_after_dislike <= apple_after_dislike) {
        std::cerr << "context specificity failed: "
                  << apple_after_like << ',' << banana_after_like << ','
                  << banana_after_dislike << ',' << apple_after_dislike << '\n';
        return 16;
    }
    loaded.record("旧句。现在", "ceshi", "跨句隔离");
    if (loaded.language_context_score("别的句子。现在", "ceshi", "跨句隔离") <= 0 ||
        loaded.language_context_score("别的句子。", "ceshi", "跨句隔离") != 0)
        return 17;
    loaded.set_sensitivity(10);
    const auto sensitive = loaded.score("你好");
    loaded.set_sensitivity(1);
    if (loaded.score("你好") >= sensitive ||
        loaded.contextual_score("nihao", "你好") <= 0) return 3;

    owo::engine::UserFrequencyStore ageing;
    if (!ageing.load(path.wstring() + L".ageing").success) return 11;
    ageing.set_sensitivity(10);
    ageing.record("旧词");
    ageing.record("热点", (std::numeric_limits<std::uint32_t>::max)());
    const auto maximum_global_score = ageing.score("热点");
    if (ageing.count("旧词") != 0 || ageing.count("热点") > 256 ||
        maximum_global_score <= 0) return 11;
    ageing.record("热点");
    if (ageing.count("热点") >= 256 ||
        ageing.score("热点") >= maximum_global_score) return 12;

    ageing.record("ceshi", "旧测试");
    ageing.record("ceshi", "新测试",
                  (std::numeric_limits<std::uint32_t>::max)());
    const auto aged_input_entries = ageing.lookup("ceshi", 4);
    if (aged_input_entries.size() != 1 ||
        aged_input_entries.front().text != "新测试" ||
        aged_input_entries.front().count > 128) return 13;
    ageing.record("同一上下文", "ceshi", "旧上下文");
    ageing.record("同一上下文", "ceshi", "新上下文",
                  (std::numeric_limits<std::uint32_t>::max)());
    if (ageing.language_context_score("同一上下文", "ceshi", "旧上下文") != 0 ||
        ageing.language_context_score("同一上下文", "ceshi", "新上下文") <= 0)
        return 14;
    std::filesystem::remove(path.wstring() + L".ageing", ignored);

    { std::ofstream corrupt(path, std::ios::binary | std::ios::trunc); corrupt << "broken"; }
    owo::engine::UserFrequencyStore recovered;
    const auto recovery = recovered.load(path);
    if (!recovery.success || !recovery.recovered_from_backup ||
        recovered.count("你好") != 4 ||
        recovered.contextual_count("nihao", "你好") != 2) {
        std::cerr << "backup recovery failed\n";
        return 1;
    }
    const auto legacy_path = path.wstring() + L".v1";
    write_v1(legacy_path);
    owo::engine::UserFrequencyStore legacy;
    if (!legacy.load(legacy_path).success || legacy.count("legacy") != 4 ||
        legacy.contextual_count("legacy", "legacy") != 0) return 4;
    std::filesystem::remove(legacy_path, ignored);
    const auto version_two_path = path.wstring() + L".v2";
    write_v2(version_two_path);
    owo::engine::UserFrequencyStore version_two;
    if (!version_two.load(version_two_path).success ||
        version_two.contextual_count("nihao", "legacy-context") != 3 ||
        version_two.language_context_score("context", "nihao", "legacy-context") != 0)
        return 6;
    std::filesystem::remove(version_two_path, ignored);
    std::filesystem::remove(path, ignored);
    std::filesystem::remove(path.wstring() + L".bak", ignored);
    return 0;
}
