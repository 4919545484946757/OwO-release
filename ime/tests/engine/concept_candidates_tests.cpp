#include "owo/engine/concept_candidates.h"

#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <string_view>

namespace {

int fail(const std::string_view message) {
    std::cerr << message << '\n';
    return 1;
}

std::filesystem::path temporary_table_path(const std::string_view tag) {
    const auto unique = std::chrono::steady_clock::now().time_since_epoch().count();
    return std::filesystem::temp_directory_path() /
           ("owo-concept-" + std::string(tag) + "-" + std::to_string(unique) + ".tsv");
}

}  // namespace

int main() {
    if (owo::engine::concept_lookup_key("Chong Fu'Zhi-xing 123") !=
        "chongfuzhixing")
        return fail("concept key normalization failed");

    const auto& builtin = owo::engine::builtin_concept_table();
    if (builtin.empty()) return fail("builtin concept table is empty");
    if (builtin.lookup("").size() != 0)
        return fail("empty key matched a concept");
    {
        const auto matches = builtin.lookup("chongfuzhixing");
        if (matches.size() != 1 || matches.front()->term != "幂等")
            return fail("builtin concept lookup failed");
    }

    {
        const auto candidates =
            owo::engine::generate_concept_candidates("chongfuzhixing");
        if (candidates.size() != 1 ||
            candidates.front().text != "幂等" ||
            candidates.front().score != owo::engine::kConceptCandidateScore ||
            candidates.front().consumed_input_bytes != 14 ||
            candidates.front().model_only)
            return fail("concept candidate fields are wrong");
    }
    if (!owo::engine::generate_concept_candidates("nihao").empty())
        return fail("unrelated pinyin produced a concept candidate");
    // Digits and symbols make the preedit a different token, even when the
    // letter part happens to equal a cue.
    if (!owo::engine::generate_concept_candidates("chongfuzhixing123").empty())
        return fail("version suffix matched a concept cue");
    if (!owo::engine::generate_concept_candidates("chongfuzhixing!").empty())
        return fail("trailing symbol matched a concept cue");
    if (owo::engine::generate_concept_candidates("chong fu zhi xing").size() != 1)
        return fail("spaced cue did not match");
    if (owo::engine::generate_concept_candidates("chong'fu'zhi'xing").size() != 1)
        return fail("apostrophe-separated cue did not match");
    {
        // Typing a description shows the term once the prefix is long enough.
        const auto candidates =
            owo::engine::generate_concept_candidates("chongfuzh");
        if (candidates.size() != 1 || candidates.front().text != "幂等" ||
            candidates.front().score != owo::engine::kConceptPrefixScore)
            return fail("concept prefix hint failed");
    }
    if (!owo::engine::generate_concept_candidates("chongfu").empty())
        return fail("short prefix produced a concept hint");
    {
        const auto candidates =
            owo::engine::generate_concept_candidates("chongfuzhixingjieguobubian");
        if (candidates.size() != 1 ||
            candidates.front().score != owo::engine::kConceptCandidateScore)
            return fail("exact cue must outrank the prefix hint");
    }
    if (!owo::engine::builtin_concept_table()
             .lookup_prefix("chongfu", 8)
             .empty())
        return fail("lookup_prefix ignored the minimum length");
    {
        const auto candidates =
            owo::engine::generate_concept_candidates("chongfuzhixing", builtin, 0);
        if (!candidates.empty()) return fail("zero concept limit was ignored");
    }
    {
        const auto candidates = owo::engine::generate_concept_candidates(
            "xianghudengdai", builtin, 1);
        if (candidates.size() != 1 || candidates.front().text != "死锁")
            return fail("second cue reading did not resolve");
    }

    const auto table_path = temporary_table_path("valid");
    {
        std::ofstream output(table_path, std::ios::binary | std::ios::trunc);
        output << "# project glossary\n"
               << "自定义术语\tshuyu\tzidingyicue,customcue\t项目内的说法\n"
               << "\n"
               << "第二个词\tdierge\t dier cue , extra \t\n";
        output.close();
    }
    const auto loaded = owo::engine::load_concept_table(table_path);
    std::error_code ignored;
    std::filesystem::remove(table_path, ignored);
    if (!loaded.table)
        return fail("valid concept table failed to load: " + loaded.error);
    {
        const auto matches = loaded.table->lookup("zidingyicue");
        if (matches.size() != 1 || matches.front()->term != "自定义术语" ||
            matches.front()->cue_text != "项目内的说法")
            return fail("loaded concept entry is wrong");
    }
    {
        const auto candidates = owo::engine::generate_concept_candidates(
            "di'ercue", *loaded.table);
        if (candidates.size() != 1 || candidates.front().text != "第二个词")
            return fail("loaded cue with spacing/separators did not match");
    }

    const auto broken_path = temporary_table_path("broken");
    {
        std::ofstream output(broken_path, std::ios::binary | std::ios::trunc);
        output << "只有两列\treadings\n";
        output.close();
    }
    const auto broken = owo::engine::load_concept_table(broken_path);
    std::filesystem::remove(broken_path, ignored);
    if (broken.table) return fail("malformed concept table was accepted");
    if (broken.error.empty()) return fail("malformed concept table had no error");

    const auto missing = owo::engine::load_concept_table(temporary_table_path("missing"));
    if (missing.table || missing.error.empty())
        return fail("missing concept table was not reported");

    {
        // A project glossary can be large; the cue indexes must stay correct
        // (and duplicate cues must resolve to every owner).
        std::vector<owo::engine::ConceptEntry> many;
        many.reserve(2002);
        for (int index = 0; index < 2000; ++index) {
            owo::engine::ConceptEntry entry;
            entry.term = "术语" + std::to_string(index);
            entry.cue_readings = {"cue" + std::to_string(index)};
            many.push_back(std::move(entry));
        }
        owo::engine::ConceptEntry shared_a;
        shared_a.term = "甲";
        shared_a.cue_readings = {"shared"};
        many.push_back(std::move(shared_a));
        owo::engine::ConceptEntry shared_b;
        shared_b.term = "乙";
        shared_b.cue_readings = {"shared"};
        many.push_back(std::move(shared_b));
        const owo::engine::ConceptTable table(std::move(many));
        const auto exact = table.lookup("cue1999");
        if (exact.size() != 1 || exact.front()->term != "术语1999")
            return fail("exact index missed a large-table entry");
        if (!table.lookup("cue2000").empty())
            return fail("exact index matched a missing cue");
        const auto prefixed = table.lookup_prefix("cue199", 5);
        if (prefixed.size() != 10)
            return fail("prefix index returned the wrong range");
        const auto shared = table.lookup("shared");
        if (shared.size() != 2 || shared[0]->term != "甲" ||
            shared[1]->term != "乙")
            return fail("duplicate cue did not resolve to both owners");
        // Exact keys are not prefix hits.
        if (!table.lookup_prefix("cue1999", 5).empty())
            return fail("exact cue leaked into the prefix range");
    }

    std::cout << "concept candidate tests passed\n";
    return 0;
}
