#include "owo/engine/candidate_generator.h"
#include "owo/engine/exact_code_schema.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/user_lexicon_alignment.h"

#include <algorithm>
#include <iostream>

namespace {
bool contains(const std::vector<owo::engine::Candidate>& candidates,
              const std::string_view text) {
    return std::any_of(candidates.begin(), candidates.end(), [&](const auto& candidate) {
        return candidate.text == text;
    });
}
}

int main() {
    using namespace owo::engine;
    const FullPinyinSchema pinyin;
    const ExactCodeSchema codes;
    const MemoryLexicon lexicon({{{"xiu", "fu"}, "修复", 1000}});
    UserFrequencyStore learned;
    learned.record("xiufugaiwenti", "修复该问题", 1);
    learned.record("xiufu", "修复该问题", 1); // Legacy polluted alias.
    learned.record("xian", "西安", 1);
    learned.record("zidingyici", "自定义词", 1);
    learned.record("nh", "你好世界", 1); // A valid table-code phrase binding.
    const CandidateGenerator generator(lexicon, nullptr, &learned);

    for (const auto page_size : {0U, 7U}) {
        for (const auto limit : {8U, 48U}) {
            for (const bool model : {false, true}) {
                const auto get = [&](const std::string_view input, const bool enabled,
                                     const std::size_t completion) {
                    return generator.generate(pinyin.parse(input, 32, false), limit,
                        false, {}, nullptr, {}, model, page_size, enabled, completion);
                };
                if (contains(get("xiufu", true, 0), "修复该问题") ||
                    contains(get("xiufu", false, 0), "修复该问题")) {
                    std::cerr << "Legacy long alias bypassed completion limits\n";
                    return 1;
                }
                if (!contains(get("xiufugaiwenti", false, 0), "修复该问题") ||
                    !contains(get("xiufugaiwent", true, 0), "修复该问题") ||
                    !contains(get("xiufu", true, 3), "修复该问题") ||
                    contains(get("xiufu", true, 2), "修复该问题")) {
                    std::cerr << "Valid learned word or configured completion range changed\n";
                    return 2;
                }
                const auto ambiguous = get("xian", false, 0);
                const auto city = std::find_if(ambiguous.begin(), ambiguous.end(), [](const auto& candidate) {
                    return candidate.text == "西安";
                });
                if (city == ambiguous.end() || city->syllables != std::vector<std::string>{"xi", "an"} ||
                    city->source_segments != std::vector<std::string>{"xi", "an"} ||
                    !contains(get("zidingyic", true, 0), "自定义词")) {
                    std::cerr << "Ambiguous or incomplete user spelling was rejected\n";
                    return 3;
                }
            }
        }
    }
    const auto source = pinyin.parse("xiufu", 32, false);
    if (std::any_of(source.paths.begin(), source.paths.end(), [&](const auto& path) {
            return user_word_matches_source_path(source, path, "修复该问题");
        })) return 4;
    const auto code_source = codes.parse("nh");
    if (!user_word_matches_source_path(code_source, code_source.paths.front(), "你好世界") ||
        !contains(generator.generate(code_source), "你好世界")) return 5;
    // Recall filtering must not silently erase old user data.
    if (learned.contextual_count("xiufu", "修复该问题") != 1) return 6;
    std::cout << "User-word source alignment and completion isolation tests passed\n";
}
