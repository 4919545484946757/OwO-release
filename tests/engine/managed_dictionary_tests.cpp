#include "owo/engine/managed_dictionary.h"
#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"
#include <algorithm>
#include <filesystem>
#include <fstream>
#include <iostream>

int main(int argc, char** argv) {
    if (argc != 2) return 1;
    const std::filesystem::path root(argv[1]);
    if (!root.is_absolute() || root.filename() != "managed-dictionary-test-fixture") return 98;
    std::error_code error;
    std::filesystem::remove_all(root, error); std::filesystem::create_directories(root);
    const auto publish = [&](const std::string& id, const std::string& text) {
        std::filesystem::create_directories(root / "snapshots" / id);
        { std::ofstream file(root / "snapshots" / id / "active.tsv", std::ios::binary); file << text; }
        { std::ofstream file(root / "current", std::ios::binary); file << id; }
    };
    owo::engine::UserFrequencyStore learned;
    if (!learned.load(root / "user-frequency.owuf").success) return 2;
    owo::engine::ManagedDictionaryModel model(&learned, root);
    if (model.refresh(true) || !model.lookup("ceshiciku", 10).empty()) return 3;
    const std::string a(32, 'a'), b(32, 'b'), c(32, 'c');
    publish(a, "OWO_DICTIONARIES_V1\nceshiciku\t测试词库\t32\nceshiciku\t词典测试\t2\nceshifangfa\t测试方法\t12\ncs\t测试词库\t16\niirc\tif I remember correctly\t10\n");
    if (!model.refresh(true) || !model.diagnostic().empty()) return 4;
    const auto words = model.lookup("ce'shi'ci'ku", 7);
    if (words.size() != 2 || words[0].text != "测试词库" || words[0].preferred || learned.count("测试词库") != 0) return 5;
    if (model.lookup_prefix("ceshi", 10).size() != 3) return 6;
    const owo::engine::MemoryLexicon base({{{"ce", "shi"}, "测试", 500}});
    const owo::engine::FullPinyinSchema schema;
    const auto candidates = owo::engine::CandidateGenerator(base, nullptr, &model).generate(schema.parse("ceshiciku"), 20);
    if (std::none_of(candidates.begin(), candidates.end(), [](const auto& candidate) { return candidate.text == "测试词库"; })) return 7;
    const auto aliases = owo::engine::CandidateGenerator(base, nullptr, &model).generate(schema.parse("cs"), 20);
    if (std::none_of(aliases.begin(), aliases.end(), [](const auto& candidate) { return candidate.text == "测试词库" && candidate.consumed_input_bytes == 2; })) return 13;
    const auto english = owo::engine::CandidateGenerator(base, nullptr, &model).generate(schema.parse("iirc"), 20);
    if (std::none_of(english.begin(), english.end(), [](const auto& candidate) { return candidate.text == "if I remember correctly"; })) return 14;
    model.set_enabled(false);
    if (!model.lookup("ceshiciku", 7).empty()) return 8;
    model.set_enabled(true);
    learned.record("ceshiciku", "测试词库", 5);
    if (model.lookup_related("csc", 8).empty()) return 18;
    if (model.lookup("ceshiciku", 7).size() != 2 || model.score("测试词库") != learned.score("测试词库")) return 9;
    publish(b, "OWO_DICTIONARIES_V1\n");
    if (!model.refresh(true) || model.lookup("ceshiciku", 10).size() != 1 || learned.count("测试词库") == 0) return 10;
    const auto sources = root / "snapshots" / b / "sources.tsv";
    { std::ofstream file(sources, std::ios::binary); file << "OWO_DICTIONARY_SOURCES_V1\nbase\t0\nuser\t0\n"; }
    if (!model.refresh(true) || model.base_enabled() || model.user_enabled() ||
        !model.lookup("ceshiciku", 10).empty() || model.score("测试词库") ||
        model.contextual_score("ceshiciku", "测试词库") || model.preferred("ceshiciku", "测试词库") ||
        !model.lookup_related("csc", 8).empty()) return 15;
    { std::ofstream file(sources, std::ios::binary); file << "OWO_DICTIONARY_SOURCES_V1\nbase\t1\nuser\t1\n"; }
    if (!model.refresh(true) || !model.base_enabled() || !model.user_enabled() ||
        model.lookup("ceshiciku", 10).size() != 1) return 16;
    { std::ofstream file(sources, std::ios::binary); file << "OWO_DICTIONARY_SOURCES_V1\nbase\t0\nuser\tbad\n"; }
    model.refresh(true);
    if (model.diagnostic().empty() || !model.base_enabled() || !model.user_enabled()) return 17;
    publish(c, "OWO_DICTIONARIES_V1\nbad1\t非法编码\t1\n");
    if (!model.refresh(true) || model.diagnostic().empty() || model.lookup("ceshiciku", 10).size() != 1) return 11;
    { std::ofstream pointer(root / "current"); pointer << "../../outside"; }
    if (!model.refresh(true) || model.diagnostic().empty()) return 12;
    std::filesystem::remove_all(root, error);
    std::cout << "Managed dictionaries: independent lookup, live snapshots, candidates, disable/delete, learned data and fail-closed validation passed.\n";
}
