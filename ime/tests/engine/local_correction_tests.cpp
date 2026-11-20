#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <iostream>

int main() {
    const owo::engine::FullPinyinSchema schema;
    const owo::engine::MemoryLexicon lexicon({
        {{"gong", "neng", "kai", "guan"}, "功能开关", 5000},
        {{"gong", "neng"}, "功能", 10000}, {{"kai", "guan"}, "开馆", 1000000},
        {{"kai", "gan"}, "开干", 1000000}, {{"kai", "gua"}, "开挂", 1000000},
        {{"gong", "neng", "cai", "dan"}, "功能菜单", 5000},
        {{"cai", "dan"}, "菜单", 1000},
        {{"zhong", "wen", "neng", "li"}, "中文能力", 5000},
        {{"chang"}, "长", 5000}, {{"cheng"}, "成", 5000},
        {{"cha", "na", "ge"}, "刹那个", 1000000},
        {{"cha", "nei", "gui"}, "查内鬼", 1000000}
    });
    const owo::engine::CandidateGenerator generator(lexicon);
    struct Case {
        std::string input;
        std::string text;
        std::vector<std::string> source;
    };
    for (const auto& item : std::vector<Case>{
        {"chnag", "长", {"chnag"}},
        {"chneg", "成", {"chneg"}},
        {"gongnnegkaiguan", "功能开关", {"gong", "nneg", "kai", "guan"}},
        {"gongnegnkaiguan", "功能开关", {"gong", "negn", "kai", "guan"}},
        {"gongnnegcaidan", "功能菜单", {"gong", "nneg", "cai", "dan"}},
        {"zhongwennnegli", "中文能力", {"zhong", "wen", "nneg", "li"}}}) {
        for (const auto budget : {8U, 16U, 32U}) {
            const auto parsed = schema.parse(item.input, budget, true);
            const auto repair = std::find_if(parsed.paths.begin(), parsed.paths.end(), [&](const auto& path) {
                if (path.match_kind != owo::engine::InputMatchKind::corrected ||
                    path.edit_count != 1 || path.syllables.size() != item.source.size())
                    return false;
                for (std::size_t index = 0; index < item.source.size(); ++index) {
                    const auto& syllable = path.syllables[index];
                    if (parsed.normalized_input.substr(syllable.begin,
                            syllable.end - syllable.begin) != item.source[index]) return false;
                }
                return true;
            });
            if (repair == parsed.paths.end()) {
                std::cerr << "Missing local repair: " << item.input << " budget=" << budget << '\n';
                return 1;
            }
            for (const auto limit : {8U, 48U}) for (const bool model : {false, true}) {
                auto correction_only = parsed;
                std::erase_if(correction_only.paths, [](const auto& path) {
                    return path.match_kind != owo::engine::InputMatchKind::corrected;
                });
                const auto candidates = generator.generate(
                    correction_only, limit, false, {}, nullptr, {}, model, 7);
                if (candidates.empty() || candidates.front().text != item.text ||
                    candidates.front().source_segments != item.source ||
                    candidates.front().consumed_input_bytes != item.input.size()) {
                    std::cerr << "Bad local correction: " << item.input << " budget=" << budget << " limit=" << limit << '\n';
                    if (!candidates.empty()) std::cerr << candidates.front().text << '\n';
                    return 2;
                }
                if (std::any_of(candidates.begin(), candidates.end(), [](const auto& candidate) {
                        return candidate.match_kind != owo::engine::InputMatchKind::corrected;
                    })) {
                    std::cerr << "Correction-only generation leaked unrelated raw-input sources\n";
                    return 3;
                }
            }
        }
    }
    std::cout << "Local minimal-edit correction, suffix preservation and dictionary ranking tests passed\n";
}
