#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <iostream>
#include <set>

int main() {
    using namespace owo::engine;
    FullPinyinSchema schema;
    std::size_t checked = 0, missing = 0;
    for (const std::string syllable : {"xia", "xie", "xue", "qie", "qian", "jiang", "neng", "chang", "shuang"}) {
        std::set<std::string> mutations;
        for (std::size_t index = 0; index < syllable.size(); ++index) {
            auto deleted = syllable;
            deleted.erase(index, 1);
            mutations.insert(deleted);
            auto repeated = syllable;
            repeated.insert(index, 1, syllable[index]);
            mutations.insert(repeated);
            if (index + 1 < syllable.size()) {
                auto swapped = syllable;
                std::swap(swapped[index], swapped[index + 1]);
                mutations.insert(swapped);
            }
        }
        for (const auto& typo : mutations) {
            if (typo == syllable) continue;
            for (const auto context : {0, 1, 2}) {
                // A repeated final letter can be the next syllable's initial;
                // do not require automatic deletion of that intentional input.
                if (context == 2 && typo == syllable + syllable.back()) continue;
                const auto prefix = context == 0 ? std::string{} : std::string{"ceshi"};
                const auto suffix = context == 2 ? std::string{} : std::string{"jixu"};
                const auto input = prefix + typo + suffix;
                const auto strict = schema.parse(input, 32, false);
                // A valid exact reading is intentional, not proof of a typo.
                if (std::any_of(strict.paths.begin(), strict.paths.end(), [](const auto& path) {
                        return path.match_kind == InputMatchKind::exact &&
                            std::all_of(path.syllables.begin(), path.syllables.end(),
                                [](const auto& token) { return token.complete; });
                    })) continue;
                std::vector<std::string> expected;
                if (!prefix.empty()) expected = {"ce", "shi"};
                expected.push_back(syllable);
                if (!suffix.empty()) { expected.push_back("ji"); expected.push_back("xu"); }
                for (const auto budget : {8U, 16U, 32U}) {
                    // Missing/changed initials can have more than eight valid
                    // repairs; require coverage at the normal public budget,
                    // not every ambiguous reading in the smallest preview pool.
                    if (budget < 32 && typo.front() != syllable.front()) continue;
                    const auto parsed = schema.parse(input, budget, true);
                    ++checked;
                    if (std::none_of(parsed.paths.begin(), parsed.paths.end(), [&](const auto& path) {
                            return path.match_kind == InputMatchKind::corrected && path.edit_count == 1 &&
                                path.syllables.size() == expected.size() &&
                                std::equal(path.syllables.begin(), path.syllables.end(), expected.begin(),
                                    [](const auto& token, const auto& text) { return token.text == text; });
                        })) {
                        if (missing < 24) std::cerr << "Missing " << input << " -> " << syllable << " budget=" << budget << '\n';
                        ++missing;
                    }
                }
            }
        }
    }
    for (const std::string_view input : {"mingd", "xingb", "ceshigongnengg", "ceshishuangg",
                                        "ce'shi'jie'mai'n"}) {
        const auto parsed = schema.parse(input, 32, true);
        const auto strict = schema.parse(input, 32, false);
        // Optional repairs may coexist with raw input. They must not replace
        // the original mixed-pinyin interpretation or cross manual boundaries.
        if (!parsed.valid || !strict.valid || parsed.paths.front() != strict.paths.front() ||
            (input.find('\'') != std::string_view::npos &&
             std::any_of(parsed.paths.begin(), parsed.paths.end(), [](const auto& path) {
                 return path.match_kind == InputMatchKind::corrected;
             }))) {
            std::cerr << "Intentional trailing initial or explicit boundary was replaced: " << input << '\n';
            return 2;
        }
    }
    std::cout << "One-edit local correction matrix: " << checked << " checks, " << missing << " missing.\n";
    return missing ? 1 : 0;
}
