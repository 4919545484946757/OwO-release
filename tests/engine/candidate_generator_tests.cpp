#include "owo/engine/candidate_generator.h"
#include "owo/engine/compact_collocations.h"
#include "owo/engine/candidate_partition.h"
#include "owo/engine/candidate_scoring.h"
#include "owo/engine/exact_code_schema.h"
#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <iostream>
#include <string_view>

namespace {

int fail(const std::string_view message) {
    std::cerr << message << '\n';
    return 1;
}

std::size_t utf8_character_count(const std::string_view text) {
    return static_cast<std::size_t>(std::count_if(
        text.begin(), text.end(), [](const unsigned char byte) {
            return (byte & 0xc0U) != 0x80U;
        }));
}

}  // namespace

int main() {
    {
        using namespace owo::engine;
        const MemoryLexicon mixed({
            {{"zhe", "me"}, "这么", 10000}, {{"na", "me"}, "那么", 9000},
            {{"ni", "hao"}, "你好", 10000}, {{"zuo"}, "做", 10000}, {{"shuo"}, "说", 10000},
            {{"de", "hua"}, "的话", 10000}, {{"zhe"}, "这", 10000},
            {{"na"}, "那", 10000}, {{"n"}, "嗯", 10}});
        const CandidateGenerator generator(mixed);
        for (const auto& [input, expected] : std::vector<std::pair<std::string, std::string>>{
                {"zhemzuodehua", "这么做的话"}, {"namzuodehua", "那么做的话"},
                {"nhaodehua", "你好的话"}, {"zhemezuodehua", "这么做的话"},
                {"zhemshuodeh", "这么说的话"}, {"namshuodeh", "那么说的话"},
                {"zhemshuodehu", "这么说的话"}, {"zhemshuodehua", "这么说的话"},
                {"zhemzuodeh", "这么做的话"}, {"nhaodeh", "你好的话"}}) {
            for (const bool correction : {false, true}) for (const auto page : {0U, 5U, 7U, 9U}) {
                const auto parsed = FullPinyinSchema{}.parse(input, 16, correction);
                const auto candidates = generator.generate(parsed, 64, false, {}, nullptr, {}, false, page);
                if (std::none_of(candidates.begin(), candidates.end(), [&](const auto& value) {
                    return value.text == expected && value.consumed_input_bytes == input.size();
                })) return fail("dictionary-backed interior initial did not compose a full candidate");
            }
        }
        const auto strict = generator.generate(ExactCodeSchema{}.parse("zhemzuodehua"), 64);
        if (std::any_of(strict.begin(), strict.end(), [](const auto& c) { return c.text == "这么做的话"; }))
            return fail("mixed-pinyin recovery leaked into exact-code input");
    }
    {
        using namespace owo::engine;
        const auto candidate = [](std::size_t syllables, std::size_t consumed,
                                  InputMatchKind kind = InputMatchKind::exact) {
            Candidate value;
            for (std::size_t i = 0; i < syllables; ++i) value.text += "字";
            value.syllables.resize(syllables, "a");
            value.source_segments.resize(syllables, "a");
            value.consumed_input_bytes = consumed;
            value.match_kind = kind;
            value.score = 10000;
            return value;
        };
        std::vector<Candidate> values{candidate(3, 9), candidate(4, 9), candidate(6, 9),
            candidate(1, 3), candidate(1, 9, InputMatchKind::corrected),
            candidate(2, 9, InputMatchKind::incomplete_completion),
            candidate(2, 9, InputMatchKind::abbreviated_completion)};
        auto tied_shortest = values.front(); tied_shortest.segment_count = 3;
        values.push_back(tied_shortest);
        for (unsigned pass = 0; pass < 4; ++pass) {
            apply_shortest_segmentation_bonus(values, 9);
            for (std::size_t i = 0; i < values.size(); ++i) {
                const auto expected = (i == 0 || i == 7) ? kCandidateShortestSegmentationBonus : 0;
                if (values[i].shortest_segmentation_bonus != expected || values[i].score != 10000 + expected)
                    return fail("shortest exact reading reward stacked, depended on token count, or leaked to assisted/partial input");
            }
        }
        // Removing the competing reading removes the prior, rather than
        // carrying a stale preference to a different candidate pool.
        std::vector<Candidate> only_shortest{values.front(), values.back()};
        apply_shortest_segmentation_bonus(only_shortest, 9);
        for (const auto& value : only_shortest)
            if (value.shortest_segmentation_bonus != 0 || value.score != 10000)
                return fail("same-length readings received a shortest-reading reward");
        auto malformed = candidate(1, 9); malformed.source_segments.clear();
        values.push_back(malformed);
        apply_shortest_segmentation_bonus(values, 9);
        if (values.front().shortest_segmentation_bonus != kCandidateShortestSegmentationBonus ||
            values.back().shortest_segmentation_bonus != 0)
            return fail("invalid reading changed the shortest-reading prior");
        apply_shortest_segmentation_bonus(values, 0);
        for (const auto& value : values)
            if (value.shortest_segmentation_bonus != 0 || value.score != 10000)
                return fail("empty input retained a segmentation reward");
    }
    {
        const owo::engine::MemoryLexicon lexicon({
            {{"bu", "tian", "jia"}, "不添加", 100},
            {{"bu", "ti"}, "不提", 50000}, {{"an", "jia"}, "安家", 50000},
            {{"bu"}, "不", 1000000}, {{"tian"}, "天", 1000000}, {{"jia"}, "家", 1000000}});
        const owo::engine::CandidateGenerator generator(lexicon);
        const owo::engine::FullPinyinSchema schema;
        for (const auto page : {0U, 5U, 7U, 9U})
            for (const auto limit : {8U, 18U, 48U})
                for (const bool model : {false, true}) {
                    const auto values = generator.generate(schema.parse("butianjia"), limit,
                        false, {}, nullptr, {}, model, page);
                    if (values.empty() || values.front().text != "不添加" ||
                        values.front().source_segments != std::vector<std::string>{"bu", "tian", "jia"} ||
                        values.front().consumed_input_bytes != 9 || values.front().model_only)
                        return fail("a longer exact reading bypassed the higher-scored complete reading");
                    if (page != 0 && std::none_of(values.begin(), values.end(), [](const auto& candidate) {
                        return candidate.text == "不提安家" && candidate.consumed_input_bytes == 9;
                    })) return fail("alternative full-input reading was erased instead of ranked");
                    const auto explicit_values = generator.generate(schema.parse("bu'ti'an'jia", 32, false),
                        limit, false, {}, nullptr, {}, model, page);
                    if (explicit_values.empty() || explicit_values.front().text != "不提安家")
                        return fail("score comparison crossed explicit pinyin boundaries");
                }
    }
    // Intentional initials must not favor short pinyin, and a beam duplicate
    // must not remove a direct dictionary word's normal lexical evidence.
    const owo::engine::MemoryLexicon initials_quality_lexicon({
        {{"ru", "guo", "shuo"}, "如果说", 100},
        {{"ru", "gu", "sui"}, "入骨髓", 100},
        {{"ren", "sheng"}, "人生", 100},
        {{"re", "shui"}, "热水", 100}});
    const owo::engine::CandidateGenerator initials_quality_generator(initials_quality_lexicon);
    std::vector<owo::engine::LexiconEntry> prefix_entries{
        {{"ru", "guo"}, "如果", 1000}, {{"ren", "gong"}, "人工", 900},
        {{"ru"}, "如", 100000}, {{"ren"}, "人", 100000},
        {{"guo"}, "果", 100000}, {{"shi"}, "是", 100000},
        {{"wei", "shen"}, "为甚", 1000}, {{"wei"}, "为", 100000},
        {{"shen"}, "甚", 100000}, {{"me"}, "么", 100000}};
    for (unsigned index = 0; index < 40; ++index) {
        const auto text = std::string("全串") + std::to_string(index);
        prefix_entries.push_back({{"ru", "gu", "si"}, text, 100000 - index});
        prefix_entries.push_back({{"wei", "shi", "ma"}, text + "词", 100000 - index});
    }
    const owo::engine::MemoryLexicon prefix_lexicon(std::move(prefix_entries));
    const owo::engine::CandidateGenerator prefix_generator(prefix_lexicon);
    for (const auto page : {5U, 7U, 9U})
        for (const bool partition : {false, true})
            for (const bool model : {false, true})
                for (const auto limit : {8U, 18U, 128U})
                    for (const auto& [input, prefix] : {std::pair{"rgs", "如果"}, std::pair{"wsm", "为甚"}}) {
                        const auto result = prefix_generator.generate(
                            owo::engine::FullPinyinSchema{}.parse(input), limit, false, {}, nullptr, {},
                            model, partition ? page : 0, true, 0, page);
                        if (std::none_of(result.begin(), result.end(), [&](const auto& candidate) {
                            return candidate.text == prefix && !candidate.model_only &&
                                candidate.consumed_input_bytes == 2 && candidate.segment_count == 1 &&
                                candidate.source_segments.size() == 2;
                        })) return fail("whole abbreviation words erased the leading dictionary-word source");
                        if (std::count_if(result.begin(), result.end(), owo::engine::is_non_single_candidate) > page * 2)
                            return fail("retained prefix words escaped the shared non-single quota");
                        if (std::string_view(input) == "rgs" && limit >= 18 &&
                            std::none_of(result.begin(), result.end(), [](const auto& candidate) {
                                return candidate.text == "人工" && !candidate.model_only &&
                                    candidate.consumed_input_bytes == 2;
                            })) return fail("only one of the bounded leading dictionary alternatives survived");
                    }
    const owo::engine::MemoryLexicon composition_lexicon({
        {{"ni"}, "你", 1000000}, {{"nian"}, "年", 900000}, {{"ne"}, "呢", 800000},
        {{"ma"}, "吗", 1000000}, {{"men"}, "们", 900000}, {{"mei"}, "每", 800000},
        {{"ni", "men"}, "你们", 1000000}});
    for (const auto page : {4U, 5U, 7U, 9U})
        for (const bool partition : {false, true})
            for (const bool model : {false, true}) {
                const auto result = owo::engine::CandidateGenerator(composition_lexicon).generate(
                    owo::engine::FullPinyinSchema{}.parse("nmnmnm"), 128, false, {}, nullptr, {},
                    model, partition ? page : 0, true, 0, page);
                const auto count = std::count_if(result.begin(), result.end(), owo::engine::is_non_single_candidate);
                if (count == 0 || count > page * 2)
                    return fail("incomplete non-single candidates exceeded the shared configured page quota");
                if (std::none_of(result.begin(), result.end(), [](const auto& candidate) {
                    return !owo::engine::is_non_single_candidate(candidate);
                })) {
                    std::cerr << "page=" << page << " partition=" << partition << " model=" << model << '\n';
                    for (const auto& candidate : result)
                        std::cerr << candidate.text << ':' << candidate.consumed_input_bytes << ':' << candidate.segment_count << ' ';
                    std::cerr << '\n';
                    return fail("non-single quota removed the single-character area");
                }
            }
    for (const auto limit : {8U, 18U, 48U, 128U})
        for (const auto page : {0U, 5U, 7U, 9U}) {
            const auto candidates = initials_quality_generator.generate(
                owo::engine::FullPinyinSchema{}.parse("rgs"), limit, false, {}, nullptr, {}, false, page);
            const auto first = std::find_if(candidates.begin(), candidates.end(), [](const auto& value) {
                return value.text == "如果说";
            });
            const auto second = std::find_if(candidates.begin(), candidates.end(), [](const auto& value) {
                return value.text == "入骨髓";
            });
            if (first == candidates.end() || second == candidates.end() || first->score != second->score ||
                first->segment_count != 1 || second->segment_count != 1)
                return fail("initial syllable length or beam budget changed dictionary quality");
        }
    // A deliberately small, repository-owned fixture. It validates engine
    // semantics and is not presented as an imported Rime Ice lexicon.
    const owo::engine::MemoryLexicon lexicon({
        {{"ni", "hao"}, "你好", 1000},
        {{"ni", "hao"}, "你号", 50},
        {{"xian"}, "先", 800},
        {{"xian"}, "线", 700},
        {{"xi", "an"}, "西安", 900},
        {{"zhong", "gu"}, "中古", 200},
        {{"zhong", "guo"}, "中国", 2000},
        {{"ba"}, "把", 1200},
        {{"bai"}, "白", 1000},
        {{"niao"}, "鸟", 5000},
        {{"wo", "qu"}, "我去", 1000},
        {{"wei", "qu"}, "委屈", 1200},
        {{"ke", "yi"}, "可以", 1500},
        {{"gou", "mai"}, "购买", 1600},
        {{"gou", "ma"}, "够吗", 1200},
        {{"gong", "neng"}, "功能", 1800},
        {{"nian", "hou"}, "年后", 10000},
        {{"lan"}, "LAN", 100},
        {{"la", "n"}, "LA-N", 100000},
        {{"duo"}, "DUO", 100},
        {{"du", "o"}, "DU-O", 100000},
        {{"lin"}, "LIN", 1},
        {{"ling"}, "LING", 1},
        {{"bin"}, "BIN", 1},
        {{"bing"}, "BING", 1},
        {{"ming", "an"}, "MING_AN", 1000},
        {{"min", "gan"}, "敏感", 200000},
        {{"cu", "lve"}, "粗略", 100000},
    });
    const owo::engine::FullPinyinSchema schema;
    const owo::engine::CandidateGenerator generator(lexicon);
    const owo::engine::ExactCodeSchema exact_code_schema;
    const auto exact_code_parse = exact_code_schema.parse("NI'HAO");
    const auto exact_code_candidates = generator.generate(exact_code_parse);
    if (!exact_code_parse.valid ||
        exact_code_parse.scheme != owo::engine::InputSchemeKind::exact_code ||
        exact_code_parse.normalized_input != "ni'hao" ||
        exact_code_candidates.empty() ||
        exact_code_candidates.front().text != "你好")
        return fail("exact-code input scheme did not perform an exact lookup");
    const auto unsegmented_exact_code = generator.generate(
        exact_code_schema.parse("nh"));
    if (!unsegmented_exact_code.empty())
        return fail("exact-code input unexpectedly used pinyin abbreviation rules");

    owo::engine::UserFrequencyStore learned_words;
    learned_words.record("zidingyici", "自定义词", 2);
    const owo::engine::CandidateGenerator learned_generator(
        lexicon, nullptr, &learned_words);
    const auto learned_candidates = learned_generator.generate(
        schema.parse("zidingyici"));
    const auto learned_word = std::find_if(
        learned_candidates.begin(), learned_candidates.end(), [](const auto& value) {
            return value.text == "自定义词";
        });
    if (learned_word == learned_candidates.end() || learned_word != learned_candidates.begin() ||
        !learned_word->user_lexicon ||
        learned_word->consumed_input_bytes != std::string_view("zidingyici").size())
        return fail("learned user word was not recalled as a full candidate");
    const auto learned_incomplete_candidates = learned_generator.generate(
        schema.parse("zidingyic"));
    if (learned_incomplete_candidates.empty() ||
        learned_incomplete_candidates.front().text != "自定义词" ||
        learned_incomplete_candidates.front().match_kind !=
            owo::engine::InputMatchKind::incomplete_completion ||
        !learned_incomplete_candidates.front().user_lexicon)
        return fail("incomplete pinyin did not recall a learned user word");
    const auto learned_missing_syllable_candidates = learned_generator.generate(
        schema.parse("zidingyi"), 32, false, {}, nullptr, {}, false, 0, true, 1);
    if (learned_missing_syllable_candidates.empty() ||
        learned_missing_syllable_candidates.front().text != "自定义词" ||
        learned_missing_syllable_candidates.front().match_kind !=
            owo::engine::InputMatchKind::incomplete_completion ||
        !learned_missing_syllable_candidates.front().user_lexicon)
        return fail("a complete shorter reading displaced an incomplete learned word");
    const auto default_completion_candidates = learned_generator.generate(schema.parse("zidingyi"));
    if (std::any_of(default_completion_candidates.begin(), default_completion_candidates.end(),
            [](const auto& candidate) { return candidate.text == "自定义词"; }))
        return fail("default zero-syllable completion predicted an extra learned syllable");

    learned_words.record("tianjiagengduofuhao", "添加更多符号", 2);
    const auto learned_long_word = learned_generator.generate(
        schema.parse("tianjiagengduofuhao"));
    if (std::none_of(learned_long_word.begin(), learned_long_word.end(),
                     [](const auto& value) { return value.text == "添加更多符号"; }))
        return fail("full learned phrase was not recalled");
    const auto learned_near_complete = learned_generator.generate(
        schema.parse("tianjiagengduofu"), 32, false, {}, nullptr, {}, false, 0, true, 1);
    if (std::none_of(learned_near_complete.begin(), learned_near_complete.end(),
                     [](const auto& value) { return value.text == "添加更多符号"; }))
        return fail("one missing syllable no longer recalls a learned phrase");
    const auto learned_short_prefix = learned_generator.generate(
        schema.parse("tianjiage"));
    if (std::any_of(learned_short_prefix.begin(), learned_short_prefix.end(),
                    [](const auto& value) { return value.text == "添加更多符号"; }))
        return fail("short prefix predicted an unrelated long learned phrase");
    const auto contains_learned_phrase = [](const auto& candidates) {
        return std::any_of(candidates.begin(), candidates.end(),
                           [](const auto& candidate) {
                               return candidate.text == "添加更多符号";
                           });
    };
    if (contains_learned_phrase(learned_generator.generate(
            schema.parse("tianjiagengduofu"), 32, false, {}, nullptr, {},
            false, 0, false, 1)))
        return fail("disabled user completion still recalled a prefix");
    if (!contains_learned_phrase(learned_generator.generate(
            schema.parse("tianjiagengduofuhao"), 32, false, {}, nullptr, {},
            false, 0, false, 1)))
        return fail("disabled user completion also disabled exact learned words");
    if (contains_learned_phrase(learned_generator.generate(
            schema.parse("tianjiagengduofu"), 32, false, {}, nullptr, {},
            false, 0, true, 0)))
        return fail("zero following syllables still recalled a longer phrase");
    if (contains_learned_phrase(learned_generator.generate(
            schema.parse("tianjiage"), 32, false, {}, nullptr, {},
            false, 0, true, 2)) ||
        !contains_learned_phrase(learned_generator.generate(
            schema.parse("tianjiage"), 32, false, {}, nullptr, {},
            false, 0, true, 3)))
        return fail("user completion syllable range was not applied");

    const auto nihao = generator.generate(schema.parse("nihao"));
    if (nihao.size() != 2 || nihao[0].text != "你好" || nihao[1].text != "你号")
        return fail("real Chinese nihao candidates failed");
    const auto uppercase_nihao_parse = schema.parse("NiHao");
    const auto uppercase_nihao = generator.generate(uppercase_nihao_parse);
    if (uppercase_nihao_parse.normalized_input != "nihao" ||
        uppercase_nihao != nihao)
        return fail("uppercase pinyin normalization failed");
    const auto mingan = generator.generate(schema.parse("mingan"));
    const auto sensitive = std::find_if(mingan.begin(), mingan.end(), [](const auto& value) {
        return value.text == "敏感";
    });
    if (sensitive == mingan.end() ||
        static_cast<std::size_t>(sensitive - mingan.begin()) > 2)
        return fail("dictionary-backed alternative exact boundary was hidden");
    const auto culue = generator.generate(schema.parse("culue"));
    if (culue.empty() || culue.front().text != "粗略")
        return fail("lue spelling did not reach the canonical lve dictionary entry");

    const auto xian = generator.generate(schema.parse("xian"));
    const auto xian_place = std::find_if(xian.begin(), xian.end(), [](const auto& value) {
        return value.text == "西安" &&
               value.source_segments == std::vector<std::string>{"xi", "an"};
    });
    const auto xian_character = std::find_if(
        xian.begin(), xian.end(), [](const auto& value) { return value.text == "先"; });
    if (xian_place == xian.end() || xian_character == xian.end() ||
        static_cast<std::size_t>(xian_place - xian.begin()) > 2) {
        for (std::size_t index = 0; index < xian.size(); ++index) {
            std::cerr << "xian[" << index << "]=" << xian[index].text << " segments=";
            for (const auto& segment : xian[index].source_segments)
                std::cerr << segment << '/';
            std::cerr << " score=" << xian[index].score << '\n';
        }
        return fail("unseparated xian did not expose dictionary-backed 西安");
    }

    const auto separated = generator.generate(schema.parse("xi'an"));
    if (separated.size() != 1 || separated[0].text != "西安")
        return fail("explicit syllable boundary candidate failed");

    const auto lan = generator.generate(schema.parse("lan"));
    if (lan.empty() || lan[0].text != "LAN" ||
        lan[0].source_segments != std::vector<std::string>{"lan"} ||
        std::any_of(lan.begin(), lan.end(), [](const auto& value) {
            return value.text == "LA-N";
        })) return fail("lan candidate used la+n segmentation");
    const auto duo = generator.generate(schema.parse("duo"));
    if (duo.empty() || duo[0].text != "DUO" ||
        duo[0].source_segments != std::vector<std::string>{"duo"} ||
        std::any_of(duo.begin(), duo.end(), [](const auto& value) {
            return value.text == "DU-O";
        })) return fail("duo candidate used du+o segmentation");
    const auto lin_candidates = generator.generate(schema.parse("lin"));
    if (lin_candidates.empty() || lin_candidates[0].text != "LIN" ||
        std::any_of(lin_candidates.begin(), lin_candidates.end(), [](const auto& value) {
            return value.text == "LING";
        })) return fail("lin candidates included ling completion");
    const auto bin_candidates = generator.generate(schema.parse("bin"));
    if (bin_candidates.empty() || bin_candidates[0].text != "BIN" ||
        std::any_of(bin_candidates.begin(), bin_candidates.end(), [](const auto& value) {
            return value.text == "BING";
        })) return fail("bin candidates included bing completion");

    const auto incomplete = generator.generate(schema.parse("zhongg"));
    if (incomplete.empty() || incomplete[0].text != "中国" ||
        incomplete[0].match_kind != owo::engine::InputMatchKind::incomplete_completion)
        return fail("incomplete suffix did not generate a candidate");

    const auto mixed_incomplete = generator.generate(schema.parse("nih"));
    if (mixed_incomplete.empty() || mixed_incomplete[0].text != "你好" ||
        mixed_incomplete[0].match_kind != owo::engine::InputMatchKind::incomplete_completion) {
        for (const auto& candidate : mixed_incomplete)
            std::cerr << "nih candidate=" << candidate.text << " kind="
                      << static_cast<unsigned>(candidate.match_kind) << '\n';
        return fail("complete and incomplete syllables did not generate a candidate");
    }

    const owo::engine::MemoryLexicon evidence_ordered_completion_lexicon({
        {{"wei", "shen", "me"}, "为什么", 500000},
        {{"wei"}, "为", 3000000},
        {{"shen"}, "神", 2000000},
        {{"ma"}, "吗", 2500000},
    });
    const owo::engine::CandidateGenerator evidence_ordered_completion_generator(
        evidence_ordered_completion_lexicon);
    const auto evidence_ordered_completion =
        evidence_ordered_completion_generator.generate(schema.parse("weishenm", 8));
    if (evidence_ordered_completion.empty() ||
        evidence_ordered_completion.front().text != "为什么" ||
        evidence_ordered_completion.front().syllables !=
            std::vector<std::string>{"wei", "shen", "me"})
        return fail("dictionary-backed incomplete completion was starved");

    const owo::engine::MemoryLexicon phrase_covered_completion_lexicon({
        {{"sheng", "qi", "yi", "shi"}, "升旗仪式", 9380},
        {{"sheng", "qi"}, "升旗", 5000},
        {{"yi", "shi"}, "仪式", 5000},
        {{"shi"}, "是", 100000}, {{"qi"}, "其", 100000},
        {{"yi"}, "一", 100000}});
    const owo::engine::CandidateGenerator phrase_covered_completion_generator(
        phrase_covered_completion_lexicon);
    const auto phrase_covered_completion =
        phrase_covered_completion_generator.generate(schema.parse("shqiyishi", 32), 32);
    const auto phrase_covered_result = std::find_if(
        phrase_covered_completion.begin(), phrase_covered_completion.end(),
        [](const auto& candidate) { return candidate.text == "升旗仪式"; });
    if (phrase_covered_result == phrase_covered_completion.end() ||
        phrase_covered_result->syllables !=
            std::vector<std::string>{"sheng", "qi", "yi", "shi"} ||
        phrase_covered_result - phrase_covered_completion.begin() >= 2) {
        return fail("dictionary phrase-covered incomplete completion was starved");
    }

    const auto single_initial = generator.generate(schema.parse("b"));
    if (single_initial.empty() || single_initial[0].text != "把")
        return fail("single initial did not generate prefix candidates");

    const owo::engine::MemoryLexicon varied_initial_lexicon({
        {{"da"}, "答", 100},   {{"dai"}, "带", 700},
        {{"dan"}, "但", 600},  {{"dao"}, "到", 650},
        {{"de"}, "的", 1000},  {{"deng"}, "等", 800},
        {{"di"}, "地", 500},   {{"dou"}, "都", 950},
        {{"dui"}, "对", 900},  {{"da"}, "生僻", 100000},
    });
    const owo::engine::CandidateGenerator varied_initial_generator(
        varied_initial_lexicon);
    const auto varied_initial = varied_initial_generator.generate(schema.parse("d"));
    const std::vector<std::string> expected_initial{
        "的", "都", "对", "等", "到", "但", "带", "地", "答"};
    if (varied_initial.size() != expected_initial.size() ||
        !std::equal(varied_initial.begin(), varied_initial.end(),
                    expected_initial.begin(),
                    [](const auto& candidate, const auto& expected) {
                        return candidate.text == expected;
                    }))
        return fail("single initial was grouped by parser completion order");

    const auto abbreviated = generator.generate(schema.parse("wq"));
    if (abbreviated.empty() || abbreviated[0].text != "我去" ||
        abbreviated[0].match_kind != owo::engine::InputMatchKind::abbreviated_completion)
        return fail("multi-syllable abbreviation did not generate candidates");

    const auto initial_abbreviation = generator.generate(schema.parse("ky"));
    if (initial_abbreviation.empty() || initial_abbreviation[0].text != "可以")
        return fail("initial abbreviation did not generate candidates");

    const auto mixed_abbreviation = generator.generate(schema.parse("gom"));
    if (std::none_of(mixed_abbreviation.begin(), mixed_abbreviation.end(), [](const auto& value) {
            return value.text == "购买";
        }) ||
        std::none_of(mixed_abbreviation.begin(), mixed_abbreviation.end(), [](const auto& value) {
            return value.text == "够吗";
        })) return fail("mixed abbreviation did not generate expected candidates");

    const owo::engine::MemoryLexicon shorthand_lexicon({
        {{"wei", "shen", "me"}, "为什么", 9000},
        {{"wan", "shen", "me"}, "玩什么", 3000},
        {{"ni", "jiu"}, "你就", 8000},
        {{"na", "jiu"}, "那就", 4000},
        {{"ren", "wei"}, "认为", 9000},
        {{"ke", "yi"}, "可以", 9000},
        {{"yong"}, "用", 9000},
        {{"ji", "zhe"}, "记者", 9000},
        {{"jie", "zhe"}, "接着", 8000},
        {{"wo"}, "我", 7000},
        {{"shi"}, "是", 7000},
        {{"ma"}, "吗", 7000},
        {{"ni"}, "你", 7000},
        {{"ren"}, "人", 7000},
        {{"ji"}, "记", 7000},
    });
    const owo::engine::CandidateGenerator shorthand_generator(shorthand_lexicon);
    const auto wsm = shorthand_generator.generate(
        schema.parse("wsm"), 16, false, {}, nullptr, {}, false, 7);
    const auto nji = shorthand_generator.generate(
        schema.parse("nji"), 16, false, {}, nullptr, {}, false, 7);
    const auto rwkyy = shorthand_generator.generate(
        schema.parse("rwkyy"), 16, false, {}, nullptr, {}, false, 7);
    const auto jzhe = shorthand_generator.generate(
        schema.parse("jzhe"), 16, false, {}, nullptr, {}, false, 7);
    const auto composed_phrase = std::find_if(
        rwkyy.begin(), rwkyy.end(), [](const auto& candidate) {
            return candidate.text == "认为可以用" && candidate.segment_count > 1;
        });
    const auto has_single_character = [](const auto& candidates) {
        return std::any_of(candidates.begin(), candidates.end(), [](const auto& candidate) {
            return candidate.text.size() == 3 && candidate.consumed_input_bytes == 1;
        });
    };
    const auto has_dictionary_word = [](const auto& candidates) {
        return std::any_of(candidates.begin(), candidates.end(), [](const auto& candidate) {
            return candidate.text.size() >= 6 && candidate.segment_count == 1;
        });
    };
    const auto has_sentence = [](const auto& candidates) {
        return std::any_of(candidates.begin(), candidates.end(), [](const auto& candidate) {
            return candidate.syllables.size() >= 3 && candidate.segment_count > 1;
        });
    };
    const auto why = std::find_if(wsm.begin(), wsm.end(), [](const auto& candidate) {
        return candidate.text == "为什么" && candidate.segment_count == 1;
    });
    if (wsm.empty() || why == wsm.end() ||
        nji.empty() || nji.front().text != "你就" ||
        composed_phrase == rwkyy.end() ||
        jzhe.size() < 2 || jzhe[0].text != "记者" ||
        jzhe[1].text != "接着" || jzhe[0].segment_count != 1 ||
        jzhe[1].segment_count != 1 ||
        !has_sentence(wsm) || !has_dictionary_word(wsm) ||
        !has_single_character(wsm) ||
        !has_dictionary_word(nji) || !has_single_character(nji) ||
        !has_sentence(rwkyy) || !has_dictionary_word(rwkyy) ||
        !has_single_character(rwkyy) ||
        !has_dictionary_word(jzhe) || !has_single_character(jzhe)) {
        const auto dump = [](const std::string_view label, const auto& candidates) {
            for (const auto& candidate : candidates)
                std::cerr << label << '=' << candidate.text << " segments="
                          << candidate.segment_count << '\n';
        };
        dump("wsm", wsm);
        dump("nji", nji);
        dump("rwkyy", rwkyy);
        dump("jzhe", jzhe);
        return fail("mixed shorthand candidates or partition metadata failed");
    }

    const auto corrected = generator.generate(schema.parse("niaho"));
    if (corrected.empty() ||
        corrected[0].match_kind != owo::engine::InputMatchKind::corrected ||
        std::none_of(corrected.begin(), corrected.end(), [](const auto& value) {
            return value.text == "你好" &&
                   value.match_kind == owo::engine::InputMatchKind::corrected;
        }))
        return fail("transposed input did not generate corrected candidates");
    const auto corrected_second_syllable =
        generator.generate(schema.parse("gongnneg", 16));
    if (corrected_second_syllable.empty() ||
        corrected_second_syllable.front().text != "功能" ||
        corrected_second_syllable.front().match_kind !=
            owo::engine::InputMatchKind::corrected)
        return fail("second-syllable transposition did not generate corrected candidates");

    const auto exact_and_completed = generator.generate(schema.parse("zhonggu"));
    if (exact_and_completed.size() < 2 || exact_and_completed[0].text != "中古" ||
        exact_and_completed[0].match_kind != owo::engine::InputMatchKind::exact ||
        std::none_of(exact_and_completed.begin(), exact_and_completed.end(), [](const auto& value) {
            return value.text == "中国" &&
                   value.match_kind == owo::engine::InputMatchKind::incomplete_completion;
        })) return fail("exact candidate priority or completion candidate failed");

    const auto limited = generator.generate(schema.parse("nihao"), 1);
    if (limited.size() != 1 || limited[0].text != "你好") return fail("candidate limit failed");

    const auto exact_without_correction = generator.generate(schema.parse("nihao"));
    if (std::any_of(exact_without_correction.begin(), exact_without_correction.end(),
                    [](const auto& value) {
                        return value.match_kind == owo::engine::InputMatchKind::corrected;
                    })) return fail("correction was mixed into an exact dictionary match");

    const owo::engine::MemoryLexicon ranged_lexicon({
        {{"ni", "hao"}, "你好", 3000},
        {{"ni", "ma"}, "你吗", 2600},
        {{"ni"}, "你", 2500},
        {{"ma"}, "吗", 2400},
        {{"shi", "jie"}, "世界", 2800},
    });
    const owo::engine::CandidateGenerator ranged_generator(ranged_lexicon);
    const auto initial_sequence = ranged_generator.generate(schema.parse("nm"));
    if (initial_sequence.size() < 2 || initial_sequence[0].text != "你吗" ||
        initial_sequence[0].source_segments != std::vector<std::string>{"n", "m"} ||
        initial_sequence[0].consumed_input_bytes != 2 ||
        initial_sequence[1].text != "你" || initial_sequence[1].consumed_input_bytes != 1)
        return fail("initial sequence did not split into source-aligned characters");

    const owo::engine::MemoryLexicon ambiguous_boundary_lexicon({
        {{"wan", "an"}, "WAN_AN", 100},
        {{"wa", "nan"}, "WA_NAN", 1000000},
        {{"nan", "an"}, "NAN_AN", 100},
        {{"na", "nan"}, "NA_NAN", 1000000},
    });
    const owo::engine::CandidateGenerator ambiguous_boundary_generator(
        ambiguous_boundary_lexicon);
    const auto wanan_candidates =
        ambiguous_boundary_generator.generate(schema.parse("wanan"));
    const auto nanan_candidates =
        ambiguous_boundary_generator.generate(schema.parse("nanan"));
    if (wanan_candidates.empty() || wanan_candidates.front().text != "WAN_AN" ||
        nanan_candidates.empty() || nanan_candidates.front().text != "NAN_AN")
        return fail("candidate segmentation diverged from the pinyin preview");

    const owo::engine::MemoryLexicon double_initial_lexicon({
        {{"ga"}, "A", 1000}, {{"ge"}, "B", 900},
        {{"gou"}, "E", 800}, {{"gu"}, "F", 700},
        {{"da"}, "C", 1000}, {{"de"}, "D", 900},
        {{"ga", "da"}, "XY", 2000},
        {{"ge", "de"}, "UV", 1800},
        {{"gou", "dong"}, "MN", 1600},
        {{"gu", "dian"}, "OP", 1400},
    });
    const owo::engine::CandidateGenerator double_initial_generator(
        double_initial_lexicon);
    const auto double_initial_candidates =
        double_initial_generator.generate(schema.parse("gd"), 5);
    if (double_initial_candidates.size() != 5 ||
        double_initial_candidates[0].consumed_input_bytes != 2 ||
        double_initial_candidates[1].consumed_input_bytes != 1 ||
        double_initial_candidates[2].consumed_input_bytes != 2 ||
        double_initial_candidates[3].consumed_input_bytes != 1 ||
        double_initial_candidates[4].consumed_input_bytes != 2 ||
        std::any_of(double_initial_candidates.begin(),
                    double_initial_candidates.end(), [](const auto& value) {
            return value.text == "AC" || value.text == "AD" ||
                   value.text == "BC" || value.text == "BD";
        }))
        return fail("double-initial dictionary/prefix candidates were not interleaved");
    for (std::size_t dynamic_limit = 2; dynamic_limit <= 7; ++dynamic_limit) {
        const auto dynamic_candidates = double_initial_generator.generate(
            schema.parse("gd"), dynamic_limit);
        if (dynamic_candidates.size() != dynamic_limit)
            return fail("double-initial dynamic candidate limit was not filled");
        for (std::size_t index = 0; index < dynamic_candidates.size(); ++index) {
            const auto expected_consumed = index % 2 == 0 ? 2U : 1U;
            if (dynamic_candidates[index].consumed_input_bytes != expected_consumed)
                return fail("double-initial ratio depended on a fixed page size");
        }
    }

    const auto unmodified_initial = ranged_generator.generate(schema.parse("n"));
    if (unmodified_initial.empty() ||
        unmodified_initial[0].source_segments != std::vector<std::string>{"n"} ||
        unmodified_initial[0].syllables != std::vector<std::string>{"ni"})
        return fail("incomplete preview source was replaced by dictionary reading");

    const auto ranged = ranged_generator.generate(schema.parse("nihaoshijie"));
    if (ranged.size() < 2 || ranged[0].text != "你好世界" ||
        ranged[0].consumed_input_bytes != 11 || ranged[1].text != "你好" ||
        ranged[1].consumed_input_bytes != 5 ||
        ranged[0].source_segments !=
            std::vector<std::string>{"ni", "hao", "shi", "jie"})
        return fail("whole sentence and leading-range candidates were not interleaved");

    const owo::engine::MemoryLexicon segmentation({
        {{"ni", "hao"}, "你好", 332885},
        {{"ni"}, "你", 20000000}, {{"hao"}, "好", 18000000},
        {{"ha"}, "哈", 16000000}, {{"o"}, "哦", 15000000},
    });
    const owo::engine::CandidateGenerator segmentation_generator(segmentation);
    const auto segmented = segmentation_generator.generate(schema.parse("nihao"));
    if (segmented.empty() || segmented[0].text != "你好")
        return fail("over-segmentation outranked whole word");

    const owo::engine::MemoryLexicon boundary_coherence_lexicon({
        {{"yi"}, "一", 1000000},
        {{"yi"}, "以", 100},
        {{"wo"}, "我", 1000},
        {{"de"}, "的", 1000},
        {{"shi", "jiao"}, "视角", 1000},
        {{"yi", "wo", "de"}, "以我的", 1000},
    });
    const owo::engine::CandidateGenerator boundary_coherence_generator(
        boundary_coherence_lexicon);
    const auto coherent = boundary_coherence_generator.generate(
        schema.parse("yiwodeshijiao"));
    const auto coherent_sentences = std::count_if(
        coherent.begin(), coherent.end(), [&coherent](const auto& candidate) {
            return candidate.consumed_input_bytes ==
                   coherent.front().consumed_input_bytes;
        });
    if (coherent.size() < 2 || coherent.front().text != "以我的视角" ||
        coherent[1].text != "一" || coherent_sentences != 1)
        return fail("cross-boundary dictionary coherence was ignored");

    const owo::engine::MemoryLexicon ambiguous_sentence_lexicon({
        {{"yi", "kuan"}, "一款", 5000},
        {{"gao", "du"}, "高度", 5000},
        {{"zi", "ding", "yi"}, "自定义", 5000},
        {{"yi"}, "一", 10000000},
        {{"kuan"}, "宽", 10000000},
        {{"kuang", "ao"}, "狂傲", 10000000},
        {{"du", "zi"}, "独自", 10000000},
        {{"ding", "yi"}, "定义", 10000000},
    });
    const owo::engine::CandidateGenerator ambiguous_sentence_generator(
        ambiguous_sentence_lexicon);
    const auto ambiguous_sentence_parse =
        schema.parse("yikuangaoduzidingyi", 8);
    const auto ambiguous_sentences =
        ambiguous_sentence_generator.generate(ambiguous_sentence_parse);
    const auto long_sentence_count = std::count_if(
        ambiguous_sentences.begin(), ambiguous_sentences.end(),
        [&ambiguous_sentence_parse](const auto& candidate) {
            return candidate.consumed_input_bytes ==
                   ambiguous_sentence_parse.normalized_input.size();
        });
    if (ambiguous_sentences.size() < 2 ||
        ambiguous_sentences[0].text != "一款高度自定义" ||
        long_sentence_count != 1) {
        for (const auto& value : ambiguous_sentences)
            if (value.consumed_input_bytes == ambiguous_sentence_parse.normalized_input.size())
                std::cerr << value.text << ':' << value.score << ':' << value.coherence_score << ':' << value.qualified_sentence_reading << '\n';
        return fail("weak alternate split bypassed the long sentence quality gate");
    }

    const owo::engine::MemoryLexicon bridge_alternative_lexicon({
        {{"wo", "de"}, "AB", 5000},
        {{"kuang", "ao"}, "CD", 5000},
        {{"jie"}, "E", 5000},
        {{"jie"}, "F", 1000},
        {{"jie"}, "K", 500},
        {{"ming", "ji"}, "GH", 5000},
        {{"yu", "ci"}, "IJ", 5000},
    });
    const owo::engine::CandidateGenerator bridge_alternative_generator(
        bridge_alternative_lexicon);
    const auto bridge_alternatives = bridge_alternative_generator.generate(
        schema.parse("wodekuangaojiemingjiyuci", 16), 6, false, {}, nullptr, {},
        true);
    const auto first_model_only = std::find_if(
        bridge_alternatives.begin(), bridge_alternatives.end(),
        [](const auto& candidate) { return candidate.model_only; });
    if (first_model_only == bridge_alternatives.end() ||
        static_cast<std::size_t>(first_model_only - bridge_alternatives.begin()) > 6 ||
        std::none_of(first_model_only, bridge_alternatives.end(),
                     [](const auto& candidate) {
                         return candidate.text == "ABCDKGHIJ" &&
                                candidate.model_only;
                     }))
        return fail("model-only bridge alternatives were not retained");

    const owo::engine::MemoryLexicon stable_trailing_lexicon({
        {{"zhe", "kuan"}, "这款", 5000},
        {{"gao", "du"}, "高度", 5000},
        {{"zi", "ding"}, "自定", 5000},
        {{"zi", "ding"}, "自订", 4800},
        {{"zhe"}, "这", 10000000},
        {{"kuang", "ao"}, "狂傲", 10000000},
        {{"du", "zi"}, "独自", 10000000},
        {{"ding"}, "定", 10000000},
    });
    const owo::engine::CandidateGenerator stable_trailing_generator(
        stable_trailing_lexicon);
    const auto completed_trailing = stable_trailing_generator.generate(
        schema.parse("zhekuangaoduziding", 8));
    const auto incomplete_trailing = stable_trailing_generator.generate(
        schema.parse("zhekuangaoduzidin", 32));
    if (completed_trailing.size() < 2 || incomplete_trailing.size() < 2 ||
        completed_trailing[0].text != "这款高度自定" ||
        completed_trailing[1].text != "这款高度自订" ||
        incomplete_trailing[0].text != completed_trailing[0].text ||
        incomplete_trailing[1].text != completed_trailing[1].text)
        return fail("finishing a trailing syllable destabilized long candidates");

    const owo::engine::MemoryLexicon leading_prefix_lexicon({
        {{"zhe", "kuan"}, "这款", 5000},
        {{"gao", "du"}, "高度", 5000},
        {{"zi", "ding", "yi"}, "自定义", 5000},
        {{"zhe"}, "这", 1000},
        {{"zhe"}, "着", 100000},
        {{"zhe"}, "者", 90000},
    });
    const owo::engine::CandidateGenerator leading_prefix_generator(
        leading_prefix_lexicon);
    const auto leading_prefixes = leading_prefix_generator.generate(
        schema.parse("zhe'kuan'gao'du'zi'ding'yi"));
    if (leading_prefixes.size() < 4 ||
        leading_prefixes[0].text != "这款高度自定义" ||
        leading_prefixes[1].text != "这款" ||
        leading_prefixes[2].text != "这" ||
        leading_prefixes[3].text != "着")
        return fail("winning sentence prefixes were buried by alternatives");

    const owo::engine::MemoryLexicon leading_character_lexicon({
        {{"shi", "zhe", "ge"}, "ABC", 5000},
        {{"shi"}, "A", 1000},
        {{"shi"}, "B", 100000},
    });
    const owo::engine::CandidateGenerator leading_character_generator(
        leading_character_lexicon);
    const auto leading_characters = leading_character_generator.generate(
        schema.parse("shi'zhe'ge"));
    if (leading_characters.size() < 3 ||
        leading_characters[0].text != "ABC" ||
        leading_characters[1].text != "B" ||
        leading_characters[2].text != "A")
        return fail("winning sentence leading character was not retained");

    const owo::engine::MemoryLexicon compositional({
        {{"ni"}, "你", 1000}, {{"ni"}, "泥", 950},
        {{"hao"}, "好", 1000}, {{"hao"}, "号", 950},
    });
    owo::engine::MemoryBigramModel bigram;
    bigram.set("你", "好", 5000);
    bigram.set("泥", "号", -5000);
    const owo::engine::CandidateGenerator beam_generator(compositional, &bigram);
    const auto composed = beam_generator.generate(schema.parse("nihao"));
    if (composed.empty() || composed[0].text != "你好" ||
        composed[0].syllables != std::vector<std::string>{"ni", "hao"} ||
        std::count_if(composed.begin(), composed.end(), [](const auto& candidate) {
            return candidate.syllables.size() == 2 && candidate.segment_count > 1;
        }) > 1)
        return fail("beam search or bigram ranking failed");

    const owo::engine::MemoryLexicon sentence_alternative_lexicon({
        {{"jin"}, "进", 2706350},
        {{"jin"}, "仅", 466676},
        {{"bao", "liu"}, "保留", 500457},
    });
    const owo::engine::CandidateGenerator sentence_alternative_generator(
        sentence_alternative_lexicon);
    const auto sentence_alternatives = sentence_alternative_generator.generate(
        schema.parse("jinbaoliu"), 12, false, {}, nullptr, {}, true);
    const auto model_alternative = std::find_if(
        sentence_alternatives.begin(), sentence_alternatives.end(),
        [](const auto& candidate) {
            return candidate.text == "仅保留" && candidate.model_only;
        });
    if (sentence_alternatives.empty() ||
        sentence_alternatives.front().text != "进保留" ||
        model_alternative == sentence_alternatives.end())
        return fail("coherent short sentence alternative was not retained for the model");

    owo::engine::UserFrequencyStore user_frequency;
    user_frequency.record("泥号", 20);
    const owo::engine::CandidateGenerator personalized(compositional, nullptr, &user_frequency);
    const auto learned = personalized.generate(schema.parse("nihao"));
    if (learned.empty() || learned[0].text != "泥号")
        return fail("user frequency ranking failed");

    owo::engine::UserFrequencyStore language_frequency;
    language_frequency.set_sensitivity(10);
    language_frequency.record("我说", "nihao", "泥号");
    const owo::engine::CandidateGenerator language_personalized(
        compositional, nullptr, &language_frequency);
    const auto language_learned = language_personalized.generate(
        schema.parse("nihao"), 32, true, "我说");
    if (language_learned.empty() || language_learned[0].text != "泥号")
        return fail("language context ranking failed");

    owo::engine::UserFrequencyStore generalized_language_frequency;
    generalized_language_frequency.set_sensitivity(10);
    generalized_language_frequency.record("会议中我说", "nihao", "泥号");
    generalized_language_frequency.record("会议中他说", "nihao", "你好");
    const owo::engine::CandidateGenerator generalized_language_personalized(
        compositional, nullptr, &generalized_language_frequency);
    const auto generalized_first = generalized_language_personalized.generate(
        schema.parse("nihao"), 32, true, "昨天我说");
    const auto generalized_second = generalized_language_personalized.generate(
        schema.parse("nihao"), 32, true, "昨天他说");
    if (generalized_first.empty() || generalized_first.front().text != "泥号" ||
        generalized_second.empty() || generalized_second.front().text != "你好")
        return fail("language context suffix generalization failed");

    const owo::engine::MemoryLexicon long_lexicon({
        {{"ni"}, "N", 1000},
        {{"ni", "hao"}, "NH", 2000},
    });
    const owo::engine::CandidateGenerator long_generator(long_lexicon);
    std::string long_input;
    for (int index = 0; index < 100; ++index) long_input += "ni";
    const auto long_candidates = long_generator.generate(schema.parse(long_input));
    if (long_candidates.empty() || long_candidates.front().text != std::string(100, 'N') ||
        long_candidates.front().consumed_input_bytes != long_input.size())
        return fail("long pinyin candidate generation failed");

    const owo::engine::MemoryLexicon initial_lexicon({{{"fa"}, "F", 1000}});
    const owo::engine::CandidateGenerator initial_generator(initial_lexicon);
    const auto initial_candidates = initial_generator.generate(schema.parse("ffffffffff"));
    if (initial_candidates.empty() || initial_candidates.front().text != std::string(10, 'F') ||
        initial_candidates.front().source_segments !=
            std::vector<std::string>(10, "f"))
        return fail("long initial candidates failed");
    const auto trailing_initial_candidates =
        initial_generator.generate(schema.parse("ffffffffff'"));
    if (trailing_initial_candidates.empty() ||
        trailing_initial_candidates.front().text != std::string(10, 'F') ||
        trailing_initial_candidates.front().consumed_input_bytes != 11)
        return fail("trailing separator did not preserve initial candidates");

    const owo::engine::MemoryLexicon mixed_fallback_lexicon({
        {{"shi"}, "S", 1000}, {{"e"}, "E", 1000}, {{"fa"}, "F", 1000},
        {{"ge"}, "G", 1000},  {{"de"}, "D", 1000}, {{"yu"}, "V", 1000},
    });
    const owo::engine::CandidateGenerator mixed_fallback_generator(mixed_fallback_lexicon);
    const std::string mixed_fallback_input = "sefsefsegsegsegsefddsgv";
    const auto mixed_fallback_candidates =
        mixed_fallback_generator.generate(schema.parse(mixed_fallback_input));
    if (mixed_fallback_candidates.empty() ||
        mixed_fallback_candidates.front().text != "SEFSEFSEGSEGSEGSEFDDSGV" ||
        mixed_fallback_candidates.front().consumed_input_bytes != mixed_fallback_input.size())
        return fail("long mixed fallback candidates failed");

    const owo::engine::MemoryLexicon phrase_abbreviation_lexicon({
        {{"bu", "gan", "dang"}, "BGD", 1000},
        {{"bu", "ge"}, "BG", 1000},
    });
    const owo::engine::CandidateGenerator phrase_abbreviation_generator(
        phrase_abbreviation_lexicon);
    const auto phrase_abbreviation =
        phrase_abbreviation_generator.generate(schema.parse("bugd"));
    if (phrase_abbreviation.empty() || phrase_abbreviation.front().text != "BGD" ||
        phrase_abbreviation.front().syllables !=
            std::vector<std::string>{"bu", "gan", "dang"} ||
        phrase_abbreviation.front().source_segments !=
            std::vector<std::string>{"bu", "g", "d"})
        return fail("lexicon-aware mixed abbreviation failed");

    const owo::engine::MemoryLexicon word_priority_lexicon({
        {{"shi"}, "中国", 10000}, {{"shi"}, "世界", 9000},
        {{"shi"}, "可以", 8000},  {{"shi"}, "我们", 7000},
        {{"shi"}, "你们", 6000},  {{"shi"}, "是", 1000000},
        {{"shi"}, "时", 900000},  {{"shi"}, "生僻", 1},
    });
    const owo::engine::CandidateGenerator word_priority_generator(word_priority_lexicon);
    const auto word_priority = word_priority_generator.generate(schema.parse("shi"));
    const std::vector<std::string> expected_priority{
        "中国", "世界", "可以", "我们", "你们", "是", "时", "生僻"};
    if (word_priority.size() != expected_priority.size() ||
        !std::equal(word_priority.begin(), word_priority.end(), expected_priority.begin(),
                    [](const auto& candidate, const auto& expected) {
                        return candidate.text == expected;
                    }))
        return fail("two-character word priority bands failed");

    const owo::engine::MemoryLexicon bounded_composition_lexicon({
        {{"chan"}, "产", 1000000}, {{"chan"}, "禅", 900000},
        {{"wu"}, "无", 1000000},   {{"wu"}, "五", 900000},
        {{"chan", "wu"}, "产物", 1000},
        {{"chan", "wu"}, "禅悟", 900},
        {{"chan", "wu"}, "产无", 100},
        {{"ni"}, "你", 1000000},  {{"ni"}, "拟", 900000},
        {{"jian"}, "见", 1000000}, {{"jian"}, "间", 900000},
        {{"ni", "jian"}, "拟建", 800},
    });
    const owo::engine::CandidateGenerator bounded_composition_generator(
        bounded_composition_lexicon);
    for (const auto input : {"chanwu", "nijian"}) {
        const auto bounded = bounded_composition_generator.generate(schema.parse(input), 16);
        const auto dictionary_words = std::count_if(
            bounded.begin(), bounded.end(), [](const auto& candidate) {
                return candidate.syllables.size() == 2 && candidate.segment_count == 1;
            });
        if (bounded.empty() || dictionary_words > 2 || std::any_of(
                bounded.begin(), bounded.end(), [](const auto& candidate) {
                    return candidate.syllables.size() == 2 &&
                           candidate.segment_count > 1;
                }))
            return fail("two-syllable permutations displaced dictionary words");
    }

    const owo::engine::MemoryLexicon fallback_composition_lexicon({
        {{"ni"}, "你", 1000}, {{"ni"}, "拟", 900},
        {{"jian"}, "见", 1000}, {{"jian"}, "间", 900},
    });
    const owo::engine::CandidateGenerator fallback_composition_generator(
        fallback_composition_lexicon);
    const auto fallback_compositions = fallback_composition_generator.generate(
        schema.parse("nijian"), 16);
    if (std::count_if(fallback_compositions.begin(), fallback_compositions.end(),
                      [](const auto& candidate) {
                          return candidate.syllables.size() == 2 &&
                                 candidate.segment_count > 1;
                      }) > 1)
        return fail("two-syllable fallback permutations were not bounded");

    const owo::engine::MemoryLexicon partitioned_composition_lexicon({
        {{"wo"}, "我", 4000}, {{"wo"}, "握", 3000},
        {{"wo"}, "窝", 2000}, {{"wo"}, "卧", 1000},
        {{"ka"}, "卡", 4000}, {{"ka"}, "咖", 3000},
        {{"ka"}, "喀", 2000}, {{"ka"}, "咔", 1000},
    });
    const owo::engine::CandidateGenerator partitioned_composition_generator(
        partitioned_composition_lexicon);
    const auto partitioned_compositions = partitioned_composition_generator.generate(
        schema.parse("woka"), 32, false, {}, nullptr, {}, false, 5);
    const auto composition_count = std::count_if(
        partitioned_compositions.begin(), partitioned_compositions.end(),
        [](const auto& candidate) {
            return candidate.syllables.size() == 2 && candidate.segment_count > 1;
        });
    const auto single_count = std::count_if(
        partitioned_compositions.begin(), partitioned_compositions.end(),
        [](const auto& candidate) {
            return candidate.text.size() == 3 && candidate.segment_count == 1;
        });
    if (composition_count != 10 || single_count == 0)
        return fail("partitioned composition quota displaced single characters");

    const owo::engine::MemoryLexicon dictionary_quota_lexicon({
        {{"wo"}, "我", 4000}, {{"wo"}, "握", 3000},
        {{"wo"}, "窝", 2000}, {{"wo"}, "卧", 1000},
        {{"ka"}, "卡", 4000}, {{"ka"}, "咖", 3000},
        {{"ka"}, "喀", 2000}, {{"ka"}, "咔", 1000},
        {{"wo", "ka"}, "词一", 8000},
        {{"wo", "ka"}, "词二", 7000},
        {{"wo", "ka"}, "词三", 6000},
    });
    const owo::engine::CandidateGenerator dictionary_quota_generator(
        dictionary_quota_lexicon);
    const auto dictionary_quota = dictionary_quota_generator.generate(
        schema.parse("woka"), 32, false, {}, nullptr, {}, false, 5);
    const auto real_dictionary_count = std::count_if(
        dictionary_quota.begin(), dictionary_quota.end(), [](const auto& candidate) {
            return candidate.syllables.size() == 2 && candidate.segment_count == 1;
        });
    const auto bounded_composition_count = std::count_if(
        dictionary_quota.begin(), dictionary_quota.end(), [](const auto& candidate) {
            return candidate.syllables.size() == 2 && candidate.segment_count > 1;
        });
    if (real_dictionary_count != 3 || bounded_composition_count != 7)
        return fail("real dictionary count did not reduce the composition quota");

    const owo::engine::MemoryLexicon mixed_weight_lexicon({
        {{"wo"}, "我", 100000}, {{"de"}, "的", 100000},
        {{"wo", "de"}, "低频整词", 1},
    });
    const owo::engine::CandidateGenerator mixed_weight_generator(
        mixed_weight_lexicon);
    const auto mixed_weight = mixed_weight_generator.generate(
        schema.parse("wode"), 16, false, {}, nullptr, {}, false, 5);
    if (mixed_weight.empty() || mixed_weight.front().text != "我的")
        return fail("low-frequency dictionary word unconditionally outranked composition");

    const owo::engine::MemoryLexicon alternative_boundary_lexicon({
        {{"kuang", "ao"}, "狂傲", 10000000},
        {{"kuang", "ao"}, "词甲", 9000000},
        {{"kuang", "ao"}, "词乙", 8000000},
        {{"kuang", "ao"}, "词丙", 7000000},
        {{"kuang", "ao"}, "词丁", 6000000},
        {{"kuan"}, "宽", 100000000},
        {{"gao"}, "高", 100000000},
        {{"kuang"}, "狂", 100000000},
        {{"ao"}, "傲", 100000000},
    });
    const owo::engine::CandidateGenerator alternative_boundary_generator(
        alternative_boundary_lexicon);
    const auto alternative_boundary = alternative_boundary_generator.generate(
        schema.parse("kuangao"), 32, false, {}, nullptr, {}, false, 7);
    const auto wide_high = std::find_if(
        alternative_boundary.begin(), alternative_boundary.end(), [](const auto& candidate) {
            return candidate.text == "宽高" &&
                   candidate.source_segments ==
                       std::vector<std::string>{"kuan", "gao"};
        });
    if (wide_high == alternative_boundary.end() ||
        static_cast<std::size_t>(wide_high - alternative_boundary.begin()) > 2) {
        for (const auto& candidate : alternative_boundary)
            std::cerr << candidate.text << ':' << candidate.score << ':'
                      << candidate.segment_count << '\n';
        return fail("strong alternative exact segmentation composition was buried");
    }

    const owo::engine::MemoryLexicon single_syllable_boundary_lexicon({
        {{"kua"}, "跨", 10000000},
        {{"ku"}, "酷", 100000000},
        {{"a"}, "啊", 100000000},
    });
    const owo::engine::CandidateGenerator single_syllable_boundary_generator(
        single_syllable_boundary_lexicon);
    const auto single_syllable_boundary = single_syllable_boundary_generator.generate(
        schema.parse("kua"), 16);
    const auto ku_a = std::find_if(
        single_syllable_boundary.begin(), single_syllable_boundary.end(),
        [](const auto& candidate) {
            return candidate.text == "酷啊" &&
                   candidate.source_segments ==
                       std::vector<std::string>{"ku", "a"};
        });
    if (ku_a == single_syllable_boundary.end())
        return fail("kua candidates dropped the exact ku+a segmentation");

    const owo::engine::MemoryLexicon full_span_word_lexicon({
        {{"yi", "fu", "zi"}, "遗腹子", 100},
        {{"yi", "fu"}, "衣服", 10000000},
        {{"yi", "fu"}, "依附", 9000000},
        {{"yi"}, "遗", 1000}, {{"yi"}, "衣", 1000000},
        {{"fu"}, "腹", 1000}, {{"fu"}, "服", 1000000},
        {{"zi"}, "子", 1000000},
    });
    const owo::engine::CandidateGenerator full_span_word_generator(full_span_word_lexicon);
    const auto full_span_word = full_span_word_generator.generate(
        schema.parse("yifuzi"), 32, false, {}, nullptr, {}, false, 7);
    const auto exact_full_word = std::find_if(
        full_span_word.begin(), full_span_word.end(), [](const auto& candidate) {
            return candidate.text == "遗腹子";
        });
    const auto first_two_character = std::find_if(
        full_span_word.begin(), full_span_word.end(), [](const auto& candidate) {
            return utf8_character_count(candidate.text) == 2;
        });
    if (exact_full_word == full_span_word.end() ||
        (first_two_character != full_span_word.end() &&
         exact_full_word > first_two_character)) {
        for (const auto& candidate : full_span_word)
            std::cerr << candidate.text << ':' << candidate.score << ':'
                      << candidate.segment_count << ':'
                      << candidate.consumed_input_bytes << '\n';
        return fail("exact full-span word was ranked behind a shorter prefix");
    }

    const owo::engine::MemoryLexicon full_head_lexicon({
        {{"zhang", "zi", "hao"}, "长子号", 10000000},
        {{"zhang", "zi", "hao"}, "张子豪", 1},
        {{"zhang"}, "长", 1000000}, {{"zhang"}, "章", 900000},
        {{"zhang"}, "涨", 800000}, {{"zhang"}, "张", 100},
        {{"zi"}, "子", 1000000}, {{"hao"}, "好", 1000000},
    });
    const owo::engine::CandidateGenerator full_head_generator(full_head_lexicon);
    const auto full_heads = full_head_generator.generate(
        schema.parse("zhangzihao"), 32, false, {}, nullptr, {}, false, 7);
    std::vector<std::string> single_heads;
    for (const auto& candidate : full_heads) {
        if (candidate.syllables.size() == 1 &&
            utf8_character_count(candidate.text) == 1)
            single_heads.push_back(candidate.text);
    }
    const auto zhang = std::find(single_heads.begin(), single_heads.end(), "张");
    if (zhang == single_heads.end() ||
        static_cast<std::size_t>(zhang - single_heads.begin()) > 1)
        return fail("leading character of a full-span candidate was buried");

    // A suffix must not reward re-tokenizing the same reading into more
    // short dictionary words. Compare equal covered lengths, not word counts.
    // Before length normalization the 2+2+2 path earned three capped bonuses,
    // beating the stronger 2+4 path solely on coherence (36k versus 24k).
    const owo::engine::MemoryLexicon suffix_lexicon({
        {{"bian", "de"}, "变得", 500000},
        {{"you", "yi", "si"}, "有意思", 100000},
        {{"you", "yi", "si", "ma"}, "有意思吗", 50000},
        {{"you", "yi", "si", "ba"}, "有意思吧", 50000},
        {{"you", "yi"}, "有一", 200000},
        {{"si", "ma"}, "司马", 50000},
        {{"si", "ba"}, "四把", 50000},
        {{"ma"}, "吗", 100000}, {{"ba"}, "吧", 100000},
        {{"ne"}, "呢", 100000},
        {{"bi", "an"}, "彼岸", 100000},
        {{"bi", "an", "de"}, "彼岸的", 100000},
    });
    const owo::engine::CandidateGenerator suffix_generator(suffix_lexicon);
    for (const auto page_size : {std::size_t{0}, std::size_t{7}}) {
        for (const auto limit : {std::size_t{8}, std::size_t{16}, std::size_t{32}}) {
            for (const auto& [input, expected] : {
                     std::pair{"biandeyouyisi", "变得有意思"},
                     std::pair{"biandeyouyisima", "变得有意思吗"},
                     std::pair{"biandeyouyisiba", "变得有意思吧"},
                     std::pair{"biandeyouyisine", "变得有意思呢"},
                     std::pair{"bian'de'you'yi'si'ma", "变得有意思吗"}}) {
                const auto values = suffix_generator.generate(
                    schema.parse(input), limit, false, {}, nullptr, {}, true, page_size);
                if (values.empty() || values.front().text != expected ||
                    values.front().consumed_input_bytes != std::string_view(input).size())
                    return fail("suffix caused short-word coherence to overwhelm an intact phrase");
                if (std::string_view(input).ends_with("ma") &&
                    values.front().coherence_score != 36000)
                    return fail("equal covered syllables did not retain the two-syllable evidence scale");
                const auto visible_sentences = std::count_if(
                    values.begin(), values.end(), [&](const auto& value) {
                        return !value.model_only && value.segment_count > 1 &&
                               value.consumed_input_bytes == std::string_view(input).size();
                    });
                if (visible_sentences > 2)
                    return fail("suffix correction expanded the visible sentence budget");
            }
        }
    }

    const owo::engine::MemoryLexicon incomplete_word_lexicon({
        {{"bu", "yan", "er", "yu"}, "不言而喻", 30000},
        {{"bu", "yan"}, "不言", 30000},
        {{"er", "yi"}, "而已", 100000},
        {{"bu"}, "不", 100000}, {{"yan"}, "言", 100000},
        {{"er"}, "而", 100000}, {{"yu"}, "喻", 100000},
        {{"bu", "ke", "si", "yi", "de"}, "不可思议的", 30000},
        {{"bu", "ke"}, "不可", 30000},
        {{"si", "yi"}, "思议", 100000}, {{"de"}, "的", 100000},
    });
    const owo::engine::CandidateGenerator incomplete_word_generator(incomplete_word_lexicon);
    for (const auto page_size : {std::size_t{0}, std::size_t{7}}) {
        for (const auto limit : {std::size_t{8}, std::size_t{32}}) {
            for (const auto& [input, expected] : {
                     std::pair{"buyanery", "不言而喻"},
                     std::pair{"buyaneryu", "不言而喻"},
                     std::pair{"bu'yan'er'y", "不言而喻"},
                     std::pair{"bukesiyid", "不可思议的"}}) {
                const auto values = incomplete_word_generator.generate(
                    schema.parse(input), limit, false, {}, nullptr, {}, true, page_size);
                if (values.empty() || values.front().text != expected ||
                    values.front().consumed_input_bytes != std::string_view(input).size() ||
                    values.front().segment_count != 1 || values.front().model_only)
                    return fail("incomplete whole dictionary word lost to fragment coverage bonuses");
                if (values.front().coherence_score !=
                    static_cast<std::int64_t>(values.front().syllables.size()) * 6000)
                    return fail("whole dictionary coverage was truncated to substring search length");
            }
        }
    }

    const owo::engine::MemoryLexicon complete_character_lexicon({
        {{"qian"}, "前", 10000}, {{"qian"}, "钱", 9000},
        {{"qi", "an"}, "奇案", 1000000}, {{"qi", "an"}, "起岸", 900000},
        {{"qi"}, "奇", 100000}, {{"qi"}, "起", 90000}, {{"an"}, "案", 100000},
    });
    const owo::engine::CandidateGenerator complete_character_generator(complete_character_lexicon);
    for (const auto limit : {std::size_t{8}, std::size_t{32}}) {
        const auto values = complete_character_generator.generate(
            schema.parse("qian"), limit, false, {}, nullptr, {}, true, 7);
        bool saw_prefix = false, saw_complete = false, saw_alternative = false;
        for (const auto& value : values) {
            if (value.model_only) continue;
            saw_alternative |= value.text == "奇案";
            if (utf8_character_count(value.text) != 1) continue;
            if (value.consumed_input_bytes == 2) saw_prefix = true;
            if (value.consumed_input_bytes == 4) {
                if (saw_prefix) return fail("phrase head displaced an exact whole-input character");
                saw_complete = true;
            }
        }
        if (!saw_complete || !saw_alternative)
            return fail("complete character protection dropped exact or alternate readings");
    }

    const owo::engine::MemoryLexicon manual_boundary_lexicon({
        {{"bin"}, "宾", 1000}, {{"ge", "wai"}, "格外", 100000},
        {{"bing"}, "并", 1000}, {{"e", "wai"}, "额外", 1000},
    });
    owo::engine::UserFrequencyStore ambiguous_learning;
    ambiguous_learning.record("bingewai", "并额外", 20);
    const owo::engine::CandidateGenerator manual_boundary_generator(manual_boundary_lexicon, nullptr, &ambiguous_learning);
    const auto explicit_bin = manual_boundary_generator.generate(schema.parse("bin'gewai"), 16, false, {}, nullptr, {}, true, 7);
    if (std::any_of(explicit_bin.begin(), explicit_bin.end(), [](const auto& value) { return value.text == "并额外"; }))
        return fail("compact learned key crossed an explicit source boundary");
    const auto explicit_incomplete = manual_boundary_generator.generate(schema.parse("bin'gewa"), 16, false, {}, nullptr, {}, true, 7);
    if (std::any_of(explicit_incomplete.begin(), explicit_incomplete.end(), [](const auto& value) { return value.text == "并额外"; }))
        return fail("learned prefix completion crossed an explicit source boundary");
    const auto compact_learned = manual_boundary_generator.generate(schema.parse("bingewai"), 16, false, {}, nullptr, {}, true, 7);
    if (compact_learned.empty() || compact_learned.front().text != "并额外" ||
        compact_learned.front().syllables != std::vector<std::string>{"bing", "e", "wai"})
        return fail("ambiguous learned word inherited a conflicting reading");
    const owo::engine::CandidateGenerator unlearned_boundary_generator(manual_boundary_lexicon);
    const auto diverse = unlearned_boundary_generator.generate(schema.parse("bingewai"), 8, false, {}, nullptr, {}, true, 7);
    if (std::none_of(diverse.begin(), diverse.end(), [](const auto& value) { return value.text == "并额外"; }))
        return fail("homophones of one exact split starved the model's alternate reading");

    const owo::engine::MemoryLexicon supported_boundary_lexicon({
        {{"hou", "xu"}, "后续", 100000}, {{"an", "zhi"}, "安置", 100000},
        {{"hou", "xuan"}, "候选", 100000}, {{"hou"}, "后", 1000000},
        {{"zhi"}, "只", 5000000}, {{"zhi"}, "值", 900000},
        {{"xu"}, "续", 100000}, {{"an"}, "安", 100000},
        {{"cuo", "bie", "zi"}, "错别字", 100000},
        {{"cuo"}, "错", 1000000}, {{"bie"}, "别", 1000000},
        {{"zi", "mu"}, "字母", 100000}, {{"zi"}, "字", 1000000}, {{"mu"}, "母", 1000000}});
    const owo::engine::CandidateGenerator supported_boundary_generator(supported_boundary_lexicon);
    for (const auto page : {5U, 7U, 9U})
        for (const auto limit : {8U, 18U, 48U, 128U})
            for (const bool model : {false, true}) {
                const auto words = supported_boundary_generator.generate(schema.parse("houxuanzhi"),
                    limit, false, {}, nullptr, {}, model, page);
                if (std::none_of(words.begin(), words.end(), [](const auto& word) {
                    return word.text == "后续安置" && !word.model_only && word.consumed_input_bytes == 10 &&
                        word.source_segments == std::vector<std::string>{"hou", "xu", "an", "zhi"};
                })) return fail("dictionary-backed extra vowel boundary was discarded");
                const auto explicit_words = supported_boundary_generator.generate(schema.parse("hou'xu'an'zhi", 32, false),
                    limit, false, {}, nullptr, {}, model, page);
                if (std::any_of(explicit_words.begin(), explicit_words.end(), [](const auto& word) {
                    return word.source_segments == std::vector<std::string>{"hou", "xuan", "zhi"};
                })) return fail("recovered vowel boundary crossed an explicit apostrophe");
                const auto fragments = supported_boundary_generator.generate(schema.parse("cuobiezimu"),
                    limit, false, {}, nullptr, {}, model, page);
                if (std::none_of(fragments.begin(), fragments.end(), [](const auto& word) {
                    return word.text == "错别" && !word.model_only && word.segment_count == 2 &&
                        word.consumed_input_bytes == 6 && word.source_segments == std::vector<std::string>{"cuo", "bie"};
                })) return fail("missing short dictionary entry erased a supported longer-word fragment");
                if (fragments.empty() || fragments.front().text != "错别字母")
                    return fail("verified terminal word was ignored after a longer dictionary prefix");
            }

    const owo::engine::MemoryLexicon reading_quota_lexicon({
        {{"xi", "an"}, "西安", 1000000}, {{"jie", "dao"}, "街道", 1000000},
        {{"jie", "dao"}, "接到", 800000}, {{"xian", "jie"}, "衔接", 1000000},
        {{"xian", "jie"}, "仙界", 100000}, {{"xian"}, "先", 500000},
        {{"xi"}, "西", 1000000}, {{"dao"}, "到", 11000000}});
    const owo::engine::CandidateGenerator reading_quota_generator(reading_quota_lexicon);
    for (const auto page : {5U, 7U, 9U}) for (const bool partition : {false, true})
        for (const bool model : {false, true}) for (const auto limit : {8U, 18U, 128U}) {
            const auto values = reading_quota_generator.generate(schema.parse("xianjiedao"), limit,
                false, {}, nullptr, {}, model, partition ? page : 0);
            if (owo::engine::sentence_candidate_limit(values, 10) != 4 ||
                std::none_of(values.begin(), values.end(), [](const auto& value) {
                    return value.text == "衔接到" && !value.model_only && value.consumed_input_bytes == 10;
                })) return fail("independent word-backed reading lost its two-sentence quota");
            std::vector<std::string> readings;
            for (const auto& value : values) {
                if (value.model_only || value.segment_count <= 1 || value.consumed_input_bytes != 10 || value.syllables.size() < 3) continue;
                const auto key = owo::engine::sentence_reading_key(value);
                readings.push_back(key);
                if (std::count(readings.begin(), readings.end(), key) > 2 || readings.size() > 4)
                    return fail("homophones created extra segmentation sentence slots");
            }
        }

    owo::engine::CompactCollocations collocation_index;
    int collocation_probes = 0;
    const auto evidence = [&] { ++collocation_probes; return true; };
    if (collocation_index.score("left", "right", evidence) != 1000 ||
        collocation_index.score("left", "right", evidence) != 1000 || collocation_probes != 1 ||
        collocation_index.score("left", "absent", [] { return false; }) != 0)
        return fail("compact collocation index repeated or inverted evidence");
    for (unsigned i = 0; i < 400; ++i)
        (void)collocation_index.score(std::to_string(i), "tail", evidence);
    if (collocation_probes != 255) return fail("compact collocation evidence exceeded its bounded budget");
    const owo::engine::MemoryLexicon mixed_tail_lexicon({
        {{"xiu","fu"},"修复",1000000}, {{"lei","si"},"类似",1000000},
        {{"wen","ti"},"问题",100000}, {{"wu","ti"},"物体",1000000},
        {{"lei","si","wen","ti"},"类似问题",1000000},
        {{"xiu"},"修",100000}, {{"fu"},"复",100000}, {{"lei"},"类",100000}, {{"si"},"似",100000}});
    for (const auto page : {5U,7U,9U}) for (const bool partition : {false,true}) {
        const auto values = owo::engine::CandidateGenerator(mixed_tail_lexicon).generate(
            schema.parse("xiufuleisiwt"), 64, false, {}, nullptr, {}, false, partition ? page : 0, false, 0, page);
        const auto target = std::find_if(values.begin(), values.end(), [](const auto& value) { return value.text == "修复类似问题"; });
        if (target == values.end() || target->model_only || target->consumed_input_bytes != 12 ||
            target->source_segments != std::vector<std::string>{"xiu","fu","lei","si","w","t"})
            return fail("word-backed consonant tail was pruned before sentence composition");
    }
    const auto cancelled_generation = word_priority_generator.generate(
        schema.parse("shi"), 32, false, {}, nullptr, [] { return true; });
    if (!cancelled_generation.empty())
        return fail("cancelled candidate generation continued producing candidates");
    return 0;
}
