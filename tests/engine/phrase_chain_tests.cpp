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
    if (argc != 1 && argc != 2 && argc != 3)
        return 2;
    const FullPinyinSchema schema;
    if (argc >= 2) {
        BinaryLexicon lexicon;
        if (!lexicon.load(argv[1]).success)
            return 2;
        if (argc == 2) for (const auto &reading : std::vector<std::vector<std::string_view>>{
                 {"zi", "dong"}, {"zi", "dong", "zhan"}, {"zhan", "kai"}}) {
            for (const auto &entry : lexicon.lookup_texts(reading))
                std::cout << entry.text << " frequency=" << entry.frequency << '\n';
        }
        const auto input = argc == 3 ? std::string_view(argv[2]) : std::string_view("zidongzhankai");
        if (argc == 3) {
            const auto parsed = schema.parse(input, 32, false);
            if (!parsed.valid || parsed.paths.empty() || parsed.paths.front().syllables.empty()) return 2;
            const std::vector<std::string_view> tail{parsed.paths.front().syllables.back().text};
            auto entries = lexicon.lookup_texts(tail);
            std::sort(entries.begin(), entries.end(), [](const auto& a, const auto& b) { return a.frequency > b.frequency; });
            for (std::size_t i = 0; i < std::min<std::size_t>(10, entries.size()); ++i)
                std::cout << entries[i].text << " tail_frequency=" << entries[i].frequency << '\n';
        }
        const auto result = CandidateGenerator(lexicon).generate(
            schema.parse(input, 32, false), 48, false, {}, nullptr, {}, true, 7);
        for (const auto &item : result) {
            if (item.consumed_input_bytes != input.size())
                continue;
            std::cout << item.text << " score=" << item.score
                      << " coherence=" << item.coherence_score
                      << " shortest_bonus=" << item.shortest_segmentation_bonus << " model_only=" << item.model_only
                      << " segments=";
            for (const auto length : item.segment_lengths)
                std::cout << length << ',';
            std::cout << '\n';
        }
        return result.empty() || (argc == 2 && result.front().text != "自动展开") ? 1 : 0;
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
    // Same reading, different lexical boundaries: an intact name + verb
    // must not erase a common character + adverb + verb interpretation.
    // Dedup may retain the latter text as character + character + verb;
    // overlapping dictionary evidence must still recall it.
    const MemoryLexicon boundary_lexicon({
        {{"ni"}, "你", 1422192}, {{"ta"}, "他", 1400000},
        {{"ping"}, "平", 1439957}, {{"chang"}, "常", 1623697},
        {{"yong"}, "用", 3170468}, {{"yong"}, "永", 399313},
        {{"ni", "ping"}, "倪萍", 15000}, {{"ta", "ping"}, "踏平", 15000},
        {{"ping", "chang"}, "平常", 20000}, {{"chang", "yong"}, "常用", 500000}});
    for (const auto& [input, expected] : {
             std::pair{"nipingchangyong", "你平常用"}, {"tapingchangyong", "他平常用"},
             {"ni'ping'chang'yong", "你平常用"}})
        for (const auto limit : {8U, 18U, 48U})
            for (const auto page : {5U, 7U, 9U})
                for (const bool model : {false, true}) {
                    const auto result = CandidateGenerator(boundary_lexicon).generate(
                        schema.parse(input, 32, false), limit, false, {}, nullptr, {}, model, page);
                    const auto match = std::find_if(result.begin(), result.end(), [&](const auto& candidate) {
                        return candidate.text == expected && !candidate.model_only &&
                               candidate.consumed_input_bytes == std::string_view(input).size();
                    });
                    const auto sentences = std::count_if(result.begin(), result.end(), [&](const auto& candidate) {
                        return !candidate.model_only && candidate.consumed_input_bytes == std::string_view(input).size();
                    });
                    if (match == result.end() || sentences > 2) {
                        std::cerr << "Boundary interpretation lost or sentence quota enlarged: " << input << '\n';
                        return 3;
                    }
                }
    const MemoryLexicon ending_lexicon({
        {{"you"}, "有", 1000000}, {{"you", "lei", "si"}, "有类似", 50000},
        {{"lei", "si", "de"}, "类似的", 50000}, {{"zhuan", "li"}, "专利", 500000},
        {{"li", "ma"}, "利马", 50000}, {{"ma"}, "吗", 9000000}, {{"ma"}, "马", 1700000},
        {{"ma"}, "妈", 900000}, {{"jia", "zhi"}, "价值", 100000},
        {{"da", "ma"}, "大妈", 15000}, {{"da", "ma"}, "大码", 14000}, {{"da", "ma"}, "大吗", 13000}});
    const CandidateGenerator ending_generator(ending_lexicon);
    for (const auto limit : {8U, 18U, 48U}) for (const auto page : {0U, 5U, 7U, 9U})
        for (const bool model : {false, true}) {
            const auto question = ending_generator.generate(schema.parse("youleisidezhuanlima", 32, false),
                limit, false, {}, nullptr, {}, model, page);
            if (question.empty() || question.front().text != "有类似的专利吗") return 4;
            const auto alternatives = ending_generator.generate(schema.parse("jiazhidama", 32, false),
                limit, false, {}, nullptr, {}, model, page);
            if (page != 0 && (alternatives.size() < 2 || alternatives.front().text != "价值大妈" ||
                alternatives[1].text != "价值大吗" || alternatives[1].model_only)) return 5;
        }
    // Coverage from an extra syllable split must not erase an independently
    // backed character + word reading from the visible sentence sources.
    const MemoryLexicon leading_word_lexicon({
        {{"yao"}, "要", 2600000}, {{"yao"}, "摇", 200000},
        {{"mai"}, "买", 2400000}, {{"qu"}, "去", 3000000},
        {{"an"}, "按", 2000000}, {{"bu"}, "不", 3000000},
        {{"yao", "qu"}, "要去", 200000}, {{"mai", "qu"}, "买去", 200000},
        {{"an", "bu"}, "按不", 200000}, {{"an", "bu"}, "按部", 180000},
        {{"quan", "bu"}, "全部", 200000}, {{"quan", "bu"}, "全不", 5000}});
    for (const auto& [input, expected] : {
             std::pair{"yaoquanbu", "要全部"}, {"maiquanbu", "买全部"},
             {"yao'quan'bu", "要全部"}, {"yaoquan'bu", "要全部"}})
        for (const auto limit : {8U, 18U, 48U, 128U}) for (const auto page : {0U, 5U, 7U, 9U})
            for (const bool model : {false, true}) for (const bool correction : {false, true}) {
                const auto result = CandidateGenerator(leading_word_lexicon).generate(
                    schema.parse(input, 32, correction), limit, false, {}, nullptr, {}, model, page);
                const auto match = std::find_if(result.begin(), result.end(), [&](const auto& value) {
                    return value.text == expected && !value.model_only && value.qualified_sentence_reading &&
                        value.consumed_input_bytes == std::string_view(input).size();
                });
                if (match == result.end()) {
                    std::cerr << "Leading-character word reading lost: " << input << " limit=" << limit
                              << " page=" << page << " model=" << model << '\n';
                    return 6;
                }
                for (const auto& value : result) {
                    if (value.model_only || value.consumed_input_bytes != std::string_view(input).size()) continue;
                    const auto same_reading = std::count_if(result.begin(), result.end(), [&](const auto& other) {
                        return !other.model_only && other.consumed_input_bytes == value.consumed_input_bytes &&
                            other.source_segments == value.source_segments;
                    });
                    if (same_reading > 2) return 7;
                }
            }
    const auto explicit_other = CandidateGenerator(leading_word_lexicon).generate(
        schema.parse("yao'qu'an'bu", 32, true), 48, false, {}, nullptr, {}, true, 7);
    if (std::any_of(explicit_other.begin(), explicit_other.end(), [](const auto& value) {
            return value.text == "要全部";
        })) return 8;
    auto non_phonetic = schema.parse("yaoquanbu", 32, false);
    non_phonetic.phonetic_assistance = false;
    const auto non_phonetic_values = CandidateGenerator(leading_word_lexicon).generate(
        non_phonetic, 48, false, {}, nullptr, {}, true, 7);
    if (non_phonetic_values.empty() || std::any_of(non_phonetic_values.begin(), non_phonetic_values.end(),
            [](const auto& value) { return value.shortest_segmentation_bonus != 0; })) return 9;
    return 0;
}
