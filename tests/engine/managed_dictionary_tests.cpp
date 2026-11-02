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
    const auto imported_lookup = [&](std::initializer_list<std::string_view> reading) {
        return model.phonetic_lexicon().lookup_texts({reading.begin(), reading.size()});
    };
    const auto words = imported_lookup({"ce", "shi", "ci", "ku"});
    if (words.size() != 2 || !model.lookup_explicit("ceshiciku", 7).empty() || learned.count("测试词库") != 0) return 5;
    if (!model.lookup_prefix("ceshi", 10).empty()) return 6;
    const owo::engine::MemoryLexicon base({{{"ce", "shi"}, "测试", 500}});
    const owo::engine::FullPinyinSchema schema;
    const owo::engine::CombinedLexicon combined(base, model.phonetic_lexicon());
    const auto candidates = owo::engine::CandidateGenerator(combined, nullptr, &model).generate(schema.parse("ceshiciku"), 20);
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
    if (model.lookup("ceshiciku", 7).size() != 1 || model.score("测试词库") != learned.score("测试词库")) return 9;
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
    // Mirror Core's disabled-base route with BOTH built-in sources off.
    // Imported entries must not accidentally depend on learned data or base
    // characters. Keep the user's real dictionary and config out of this test.
    publish(b, "OWO_DICTIONARIES_V1\nceshiciku\t测试词库\t32\nce'shi\t测试\t12\nci\t词\t4\nku\t库\t4\ncs\t测试词库\t16\nce'shi'fang'fa\t测试方法\t128\nxi'an\t西安\t12\nxian\t先\t8\nchu\t础\t128\n");
    { std::ofstream file(sources, std::ios::binary); file << "OWO_DICTIONARY_SOURCES_V1\nbase\t0\nuser\t0\n"; }
    if (!model.refresh(true) || model.base_enabled() || model.user_enabled()) return 20;
    const owo::engine::CandidateGenerator standalone(model.phonetic_lexicon(), nullptr, &model);
    const auto contains = [&](const std::string& input, const std::string& word, bool correction = true) {
        const auto found = standalone.generate(schema.parse(input, 32, correction), 40);
        return std::any_of(found.begin(), found.end(), [&](const auto& candidate) { return candidate.text == word && candidate.consumed_input_bytes == input.size(); });
    };
    if (!contains("ceshiciku", "测试词库") || !contains("ce'shi'ci'ku", "测试词库") ||
        !contains("ceshi", "测试") || !contains("ci", "词") || !contains("cs", "测试词库")) return 21;
    if (contains("nihao", "你好") || model.score("测试词库")) return 22;
    if (!contains("csck", "测试词库") || !contains("ceshick", "测试词库") ||
        !contains("ceshicik", "测试词库") || !contains("ciku", "词库")) return 24;
    const auto prefixes = standalone.generate(schema.parse("ceshiciku"), 40);
    if (std::none_of(prefixes.begin(), prefixes.end(), [](const auto& word) { return word.text == "测试" && word.consumed_input_bytes == 5; })) return 32;
    if (!contains("ceshifangaf", "测试方法", true) || contains("ceshifangaf", "测试方法", false)) return 25;
    if (!contains("cikuu", "词库", true) || contains("cikuu", "词库", false)) return 26;
    if (!contains("xian", "先") || !contains("xian", "西安") || !contains("xi'an", "西安") || contains("xi'an", "先")) return 27;
    if (imported_lookup({"xian"}).size() != 1 || imported_lookup({"xi", "an"}).size() != 1) return 28;
    const auto initials = model.phonetic_lexicon().lookup_initial_texts('c', 7);
    if (std::none_of(initials.begin(), initials.end(), [](const auto& word) { return word.text == "词"; })) return 29;
    const owo::engine::MemoryLexicon base_character({{{"ji"}, "基", 100}});
    const owo::engine::CombinedLexicon cross_source(base_character, model.phonetic_lexicon());
    const auto cross = owo::engine::CandidateGenerator(cross_source, nullptr, &model).generate(schema.parse("jichu"), 20);
    if (std::none_of(cross.begin(), cross.end(), [](const auto& word) { return word.text == "基础" && word.consumed_input_bytes == 5; })) return 30;
    const auto merged = combined.lookup_texts(std::array<std::string_view, 2>{"ce", "shi"});
    if (merged.size() != 1 || merged[0].frequency != 500) return 31;
    std::cout << "Imported-only: full/incomplete/initial/mixed/correction/composition, manual boundaries, cross-source merge passed.\n";
    { std::ofstream file(sources, std::ios::binary); file << "OWO_DICTIONARY_SOURCES_V1\nbase\t1\nuser\t1\n"; }
    if (!model.refresh(true) || !model.base_enabled() || !model.user_enabled()) return 23;
    publish(c, "OWO_DICTIONARIES_V1\nbad1\t非法编码\t1\n");
    if (!model.refresh(true) || model.diagnostic().empty() || model.lookup("ceshiciku", 10).size() != 1) return 11;
    { std::ofstream pointer(root / "current"); pointer << "../../outside"; }
    if (!model.refresh(true) || model.diagnostic().empty()) return 12;
    // A large same-reading bucket must still have indexed exact and initial
    // queries, and deterministic bounded mixed results (no full table sort).
    std::string indexed = "OWO_DICTIONARIES_V1\n";
    const auto chinese = [](unsigned value) {
        return std::string{char(0xe0 | (value >> 12)), char(0x80 | ((value >> 6) & 63)), char(0x80 | (value & 63))};
    };
    for (unsigned i = 0; i < 10000; ++i) indexed += "ce'shi\t测" + chinese(0x4e00 + i) + "\t1\n";
    indexed += "ce\t测\t1\n";
    publish(a, indexed);
    if (!model.refresh(true) || !model.diagnostic().empty() || imported_lookup({"ce", "shi"}).size() != 10000 ||
        model.phonetic_lexicon().lookup_initial_texts('c', 7).size() != 1 ||
        model.phonetic_lexicon().lookup_mixed_abbreviation_texts("cs", 7).size() != 7) return 33;
    // Exceed both old runtime limits: 100k rows and 24 MiB. This verifies
    // that settings-side imports are not silently rejected by the Core.
    std::string large = "OWO_DICTIONARIES_V1\n";
    std::string label;
    for (unsigned i = 0; i < 100; ++i) label += "测";
    for (unsigned i = 0; i < 100001; ++i)
        large += "kuorong\t" + label + std::to_string(i) + "\t1\n";
    publish(a, large);
    if (!model.refresh(true) || !model.diagnostic().empty() || model.lookup("kuorong", 1).empty()) return 19;
    std::filesystem::remove_all(root, error);
    std::cout << "Managed dictionaries: independent lookup, live snapshots, candidates, disable/delete, learned data and fail-closed validation passed.\n";
}
