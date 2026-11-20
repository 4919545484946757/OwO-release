#include "owo/engine/binary_lexicon.h"
#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <iostream>
#include <string_view>
#include <utility>
#include <vector>

int main(int argc, char **argv) {
    using namespace owo::engine;
    if (argc != 1 && argc != 2)
        return 2;
    const FullPinyinSchema schema;
    if (argc == 2) {
        BinaryLexicon lexicon;
        if (!lexicon.load(argv[1]).success)
            return 2;
        for (const auto &reading : std::vector<std::vector<std::string_view>>{
                 {"zi", "dong"}, {"zi", "dong", "zhan"}, {"zhan", "kai"}}) {
            for (const auto &entry : lexicon.lookup_texts(reading))
                std::cout << entry.text << " frequency=" << entry.frequency << '\n';
        }
        const auto result = CandidateGenerator(lexicon).generate(
            schema.parse("zidongzhankai", 16, false), 48, false, {}, nullptr, {}, true, 7);
        for (const auto &item : result) {
            if (item.consumed_input_bytes != 13)
                continue;
            std::cout << item.text << " score=" << item.score
                      << " coherence=" << item.coherence_score << " model_only=" << item.model_only
                      << " segments=";
            for (const auto length : item.segment_lengths)
                std::cout << length << ',';
            std::cout << '\n';
        }
        return result.empty() || result.front().text != "自动展开" ? 1 : 0;
    }
    const MemoryLexicon lexicon({{{"zi", "dong"}, "自动", 50000},
                                 {{"zi", "dong", "zhan"}, "自动站", 100},
                                 {{"bei", "dong"}, "被动", 50000},
                                 {{"bei", "dong", "zhan"}, "被动站", 100},
                                 {{"zhan", "kai"}, "展开", 50000},
                                 {{"zhan", "kai"}, "站开", 5000},
                                 {{"kai"}, "开", 1000000},
                                 {{"zi"}, "自", 100000},
                                 {{"bei"}, "被", 100000}});
    for (const auto &[input, expected] :
         {std::pair{"zidongzhankai", "自动展开"}, {"beidongzhankai", "被动展开"},
          {"zi'dong'zhan'kai", "自动展开"}, {"bei'dong'zhan'kai", "被动展开"}}) {
        for (const auto limit : {8U, 18U, 48U, 128U})
            for (const auto page : {0U, 5U, 7U, 9U})
                for (const bool model : {false, true}) {
                    const auto result = CandidateGenerator(lexicon).generate(
                        schema.parse(input, 16, false), limit, false, {}, nullptr, {}, model, page);
                    if (result.empty() || result.front().text != expected ||
                        result.front().consumed_input_bytes != std::string_view(input).size() ||
                        result.front().segment_lengths != std::vector<std::size_t>{2, 2}) {
                        std::cerr << "Dictionary phrase chain lost: " << input << " limit=" << limit
                                  << " page=" << page << " model=" << model << '\n';
                        if (!result.empty())
                            std::cerr << result.front().text << '\n';
                        return 1;
                    }
                }
    }
    return 0;
}
