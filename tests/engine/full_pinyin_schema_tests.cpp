#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <chrono>
#include <iostream>
#include <string_view>

namespace {

bool contains_path(const owo::engine::ParseResult& result,
                   const std::initializer_list<std::string_view> expected) {
    return std::any_of(result.paths.begin(), result.paths.end(), [&](const auto& path) {
        if (path.syllables.size() != expected.size()) return false;
        return std::equal(path.syllables.begin(), path.syllables.end(), expected.begin(), expected.end(),
                          [](const auto& syllable, const auto text) { return syllable.text == text; });
    });
}

bool contains_path(const owo::engine::ParseResult& result,
                   const std::initializer_list<std::string_view> expected,
                   const owo::engine::InputMatchKind match_kind) {
    return std::any_of(result.paths.begin(), result.paths.end(), [&](const auto& path) {
        if (path.match_kind != match_kind || path.syllables.size() != expected.size()) return false;
        return std::equal(path.syllables.begin(), path.syllables.end(), expected.begin(), expected.end(),
                          [](const auto& syllable, const auto text) { return syllable.text == text; });
    });
}

bool preferred_source_is(const owo::engine::ParseResult& result,
                         const std::initializer_list<std::string_view> expected) {
    const auto path = std::find_if(result.paths.begin(), result.paths.end(), [](const auto& value) {
        return value.match_kind == owo::engine::InputMatchKind::exact &&
               !value.syllables.empty();
    });
    if (path == result.paths.end() || path->syllables.size() != expected.size()) return false;
    return std::equal(path->syllables.begin(), path->syllables.end(), expected.begin(),
                      expected.end(), [&](const auto& syllable, const auto source) {
        return syllable.begin < syllable.end &&
               syllable.end <= result.normalized_input.size() &&
               std::string_view(result.normalized_input).substr(
                   syllable.begin, syllable.end - syllable.begin) == source;
    });
}

int fail(const std::string_view message) {
    std::cerr << message << '\n';
    return 1;
}

}  // namespace

int main() {
    const owo::engine::FullPinyinSchema schema;

    for (const auto& [input, expected] : {
             std::pair{"chnag", "chang"}, {"shnag", "shang"},
             {"zhogn", "zhong"}, {"chneg", "cheng"}, {"jnig", "jing"}}) {
        if (!schema.is_single_syllable_transposition(input))
            return fail("single-syllable transposition preflight missed a typo");
        for (const auto budget : {8u, 16u, 32u}) {
            if (!contains_path(schema.parse(input, budget, true), {expected},
                               owo::engine::InputMatchKind::corrected))
                return fail("single-syllable transposition correction was pruned");
            const auto strict = schema.parse(input, budget, false);
            if (std::any_of(strict.paths.begin(), strict.paths.end(), [](const auto& path) {
                    return path.match_kind == owo::engine::InputMatchKind::corrected;
                })) return fail("disabled correction produced a transposition repair");
        }
    }
    for (const auto input : {"chang", "xian", "wsm", "bugd", "shqiyishi",
                              "ch'nag", "chnag'", "CHNAG", "", "abcdefgh"}) {
        if (schema.is_single_syllable_transposition(input))
            return fail("transposition preflight widened beyond normalized single syllables");
    }

    const auto nihao = schema.parse("NiHao");
    if (!nihao.valid || nihao.normalized_input != "nihao" ||
        !contains_path(nihao, {"ni", "hao"})) return fail("nihao parse failed");

    const auto ambiguous = schema.parse("xian");
    if (!contains_path(ambiguous, {"xian"}) ||
        !contains_path(ambiguous, {"xi", "an"}) ||
        ambiguous.paths.empty() ||
        ambiguous.paths.front().syllables.size() != 1 ||
        ambiguous.paths.front().syllables.front().text != "xian")
        return fail("unseparated xian did not retain its dictionary-backed boundary");

    const auto separated = schema.parse("xi'an");
    if (!separated.valid || !contains_path(separated, {"xi", "an"}) ||
        contains_path(separated, {"xian"})) return fail("apostrophe boundary failed");

    const auto separated_abbreviation = schema.parse("wdt'a", 32, false);
    const auto has_source_aligned_abbreviation = [](const auto& parsed,
                                                     const std::size_t count) {
        return std::any_of(parsed.paths.begin(), parsed.paths.end(), [&](const auto& path) {
            if (path.match_kind != owo::engine::InputMatchKind::abbreviated_completion ||
                path.syllables.size() != count) return false;
            constexpr std::string_view sources = "wdta";
            for (std::size_t index = 0; index < count; ++index) {
                const auto& syllable = path.syllables[index];
                if (syllable.begin >= parsed.normalized_input.size() ||
                    parsed.normalized_input[syllable.begin] != sources[index]) return false;
            }
            return true;
        });
    };
    if (!separated_abbreviation.valid ||
        !has_source_aligned_abbreviation(separated_abbreviation, 4))
        return fail("explicit boundary rejected an initial abbreviation chunk");
    const auto trailing_separator = schema.parse("wdt'", 32, false);
    if (!trailing_separator.valid || trailing_separator.normalized_input != "wdt'" ||
        !has_source_aligned_abbreviation(trailing_separator, 3))
        return fail("trailing explicit boundary rejected usable prefix candidates");

    const auto lan = schema.parse("lan");
    if (!contains_path(lan, {"lan"}) || contains_path(lan, {"la", "n"}))
        return fail("complete lan was over-segmented");
    const auto duo = schema.parse("duo");
    if (!contains_path(duo, {"duo"}) || contains_path(duo, {"du", "o"}))
        return fail("complete duo was over-segmented");
    const auto kua = schema.parse("kua");
    if (!contains_path(kua, {"kua"}) || !contains_path(kua, {"ku", "a"}))
        return fail("complete kua dropped its useful ku+a boundary");
    const auto lin = schema.parse("lin");
    if (!contains_path(lin, {"lin"}) ||
        contains_path(lin, {"ling"}, owo::engine::InputMatchKind::incomplete_completion))
        return fail("complete lin generated a ling completion");
    const auto bin = schema.parse("bin");
    if (!contains_path(bin, {"bin"}) ||
        contains_path(bin, {"bing"}, owo::engine::InputMatchKind::incomplete_completion))
        return fail("complete bin generated a bing completion");

    const auto incomplete = schema.parse("zhongg");
    if (!incomplete.valid || !incomplete.has_incomplete_syllable ||
        !contains_path(incomplete, {"zhong", "g"})) return fail("incomplete suffix failed");
    if (!contains_path(incomplete, {"zhong", "guo"},
                       owo::engine::InputMatchKind::incomplete_completion))
        return fail("incomplete suffix completion failed");

    if (!preferred_source_is(schema.parse("xingb"), {"xing", "b"}))
        return fail("xingb preferred segmentation failed");
    if (!contains_path(schema.parse("wodekuangaojiemingjiyuci", 16, false),
                       {"wo", "de", "kuang", "ao", "jie", "ming", "ji", "yu", "ci"},
                       owo::engine::InputMatchKind::exact))
        return fail("coherent long segmentation was starved by boundary variants");
    if (!preferred_source_is(schema.parse("xingbaf"), {"xing", "ba", "f"}))
        return fail("xingbaf preferred segmentation failed");
    if (!preferred_source_is(schema.parse("bingb"), {"bing", "b"}))
        return fail("bingb preferred segmentation failed");
    if (!preferred_source_is(schema.parse("lingm"), {"ling", "m"}))
        return fail("lingm preferred segmentation failed");
    if (!preferred_source_is(schema.parse("zhengcanf"), {"zheng", "can", "f"}))
        return fail("zhengcanf preferred segmentation failed");
    if (!preferred_source_is(schema.parse("mingd"), {"ming", "d"}))
        return fail("mingd preferred segmentation failed");
    if (!preferred_source_is(schema.parse("kenengd"), {"ke", "neng", "d"}))
        return fail("kenengd preferred segmentation failed");
    if (!preferred_source_is(schema.parse("wanan"), {"wan", "an"}) ||
        !preferred_source_is(schema.parse("nanan"), {"nan", "an"}))
        return fail("vowel-leading suffix preferred segmentation failed");
    if (!preferred_source_is(schema.parse("ceshiyixiav"),
                             {"ce", "shi", "yi", "xia", "v"}))
        return fail("standalone trailing v destabilized the valid pinyin prefix");
    if (!contains_path(schema.parse("mingan"), {"min", "gan"},
                       owo::engine::InputMatchKind::exact))
        return fail("valid alternative exact boundary was dropped");
    if (!contains_path(schema.parse("culue"), {"cu", "lve"},
                       owo::engine::InputMatchKind::exact) ||
        !contains_path(schema.parse("nunue"), {"nu", "nve"},
                       owo::engine::InputMatchKind::exact))
        return fail("ASCII u spelling for lue/nue was not normalized");
    if (!preferred_source_is(schema.parse("nengd"), {"neng", "d"}) ||
        !preferred_source_is(schema.parse("pengd"), {"peng", "d"}) ||
        !preferred_source_is(schema.parse("shengx"), {"sheng", "x"}) ||
        !preferred_source_is(schema.parse("kenengx"), {"ke", "neng", "x"}) ||
        !preferred_source_is(schema.parse("zenengd"), {"ze", "neng", "d"}))
        return fail("complete-final plus trailing-initial segmentation failed");

    owo::engine::FullPinyinParseMetrics long_typo_metrics;
    const auto long_typo_started = std::chrono::steady_clock::now();
    const auto long_typo = schema.parse("muqianshizheyanh", 16, true, &long_typo_metrics);
    const auto long_typo_elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::steady_clock::now() - long_typo_started);
    if (!preferred_source_is(long_typo, {"mu", "qian", "shi", "zhe", "yan", "h"}) ||
        long_typo_elapsed > std::chrono::milliseconds(100))
        return fail("long incomplete pinyin parsing was unstable or too slow");

    const auto initial = schema.parse("b");
    if (!initial.valid || !initial.has_incomplete_syllable ||
        !contains_path(initial, {"ba"}, owo::engine::InputMatchKind::incomplete_completion) ||
        !contains_path(initial, {"bu"}, owo::engine::InputMatchKind::incomplete_completion))
        return fail("single-letter completion failed");

    if (!contains_path(schema.parse("wq"), {"wo", "qu"},
                       owo::engine::InputMatchKind::abbreviated_completion) ||
        !contains_path(schema.parse("ky"), {"ke", "yi"},
                       owo::engine::InputMatchKind::abbreviated_completion))
        return fail("multi-syllable abbreviation completion failed");
    const auto mixed_abbreviation = schema.parse("gom");
    if (!contains_path(mixed_abbreviation, {"gou", "mai"},
                       owo::engine::InputMatchKind::abbreviated_completion) ||
        !contains_path(mixed_abbreviation, {"gou", "ma"},
                       owo::engine::InputMatchKind::abbreviated_completion))
        return fail("mixed abbreviation completion failed");

    const auto diverse_completion = schema.parse("shqiyishi", 32);
    if (!contains_path(diverse_completion, {"sheng", "qi", "yi", "shi"},
                       owo::engine::InputMatchKind::abbreviated_completion))
        return fail("completion pruning starved a distinct syllable family");

    if (!contains_path(schema.parse("niaho"), {"ni", "hao"},
                       owo::engine::InputMatchKind::corrected))
        return fail("transposed-letter correction failed");
    if (!contains_path(schema.parse("gongnneg", 16), {"gong", "neng"},
                       owo::engine::InputMatchKind::corrected))
        return fail("second-syllable transposition correction failed");
    for (const auto input : {"gongnnegkaiguan", "gongnegnkaiguan", "gongnnegkaiguna"}) {
        for (const auto budget : {8U, 16U, 32U}) {
            if (!contains_path(schema.parse(input, budget), {"gong", "neng", "kai", "guan"},
                               owo::engine::InputMatchKind::corrected)) {
                std::cerr << input << " budget=" << budget << '\n';
                return fail("local sentence repair was crowded out by extra-edit variants");
            }
            const auto strict = schema.parse(input, budget, false);
            if (std::any_of(strict.paths.begin(), strict.paths.end(), [](const auto& path) {
                return path.match_kind == owo::engine::InputMatchKind::corrected;
            })) return fail("disabled sentence correction produced repaired paths");
        }
    }
    if (contains_path(schema.parse("niaho", 32, false), {"ni", "hao"},
                      owo::engine::InputMatchKind::corrected))
        return fail("disabled correction still produced a corrected path");
    const auto separated_typo_without_correction = schema.parse("quan'loi", 32, false);
    if (separated_typo_without_correction.valid ||
        !separated_typo_without_correction.paths.empty())
        return fail("explicit invalid chunk fell back to single-letter readings");
    if (!contains_path(schema.parse("nihap"), {"ni", "hao"},
                       owo::engine::InputMatchKind::corrected))
        return fail("adjacent-key correction failed");
    if (!contains_path(schema.parse("niho"), {"ni", "hao"},
                       owo::engine::InputMatchKind::corrected))
        return fail("missing-letter correction failed");
    if (!contains_path(schema.parse("nihhao"), {"ni", "hao"},
                       owo::engine::InputMatchKind::corrected))
        return fail("extra-letter correction failed");
    if (contains_path(schema.parse("nihax"), {"ni", "hao"},
                      owo::engine::InputMatchKind::corrected))
        return fail("non-adjacent substitution was corrected");
    if (contains_path(schema.parse("nxhap"), {"ni", "hao"},
                      owo::engine::InputMatchKind::corrected))
        return fail("two edits were corrected");
    const auto long_local_correction = schema.parse(
        "wangwangshituobukuaigauanxi", 16);
    if (!contains_path(long_local_correction,
                       {"wang", "wang", "shi", "tuo", "bu", "kai", "guan", "xi"},
                       owo::engine::InputMatchKind::corrected))
        return fail("local correction in long input was not retained");
    if (!contains_path(schema.parse("zhonggu"), {"zhong", "guo"},
                       owo::engine::InputMatchKind::incomplete_completion))
        return fail("complete-prefix completion failed");

    const auto capped_assisted = schema.parse("nihap", 8);
    if (capped_assisted.paths.size() > 8 ||
        !contains_path(capped_assisted, {"ni", "hao"},
                       owo::engine::InputMatchKind::corrected))
        return fail("assisted path budget starved correction");

    if (schema.parse("ni hao").valid || schema.parse("'ni").valid ||
        schema.parse("ni''hao").valid || schema.parse("").valid)
        return fail("invalid input was accepted");

    const auto limited = schema.parse("xian", 1);
    if (!limited.valid || limited.paths.size() != 1) return fail("path cap failed");

    std::string long_input;
    for (int index = 0; index < 80; ++index) {
        if (!long_input.empty()) long_input.push_back('\'');
        long_input += "ni";
    }
    const auto long_result = schema.parse(long_input);
    if (!long_result.valid || long_result.paths.empty() ||
        long_result.paths.front().syllables.size() != 80)
        return fail("long pinyin input was truncated");

    const auto long_initials = schema.parse("ffffffffff");
    if (!long_initials.valid ||
        std::none_of(long_initials.paths.begin(), long_initials.paths.end(), [](const auto& path) {
            return path.match_kind == owo::engine::InputMatchKind::abbreviated_completion;
        })) return fail("long initial sequence was not segmented");

    const auto separated_initials = schema.parse("f'f'f'f'f'f'f'f'f'f");
    if (!separated_initials.valid || separated_initials.paths.empty())
        return fail("separated incomplete initials were rejected");

    const auto long_mixed_abbreviation = schema.parse("sefsefsegsegsegsefddsgv");
    if (!long_mixed_abbreviation.valid ||
        std::none_of(long_mixed_abbreviation.paths.begin(),
                     long_mixed_abbreviation.paths.end(), [](const auto& path) {
            return path.match_kind == owo::engine::InputMatchKind::abbreviated_completion &&
                   !path.syllables.empty();
        })) return fail("long mixed abbreviation was rejected");

    owo::engine::FullPinyinParseMetrics cancelled_metrics;
    const auto cancelled_parse = schema.parse(
        "sefsefsegsegsegsefddsgv", 32, true, &cancelled_metrics,
        [] { return true; });
    if (cancelled_parse.valid || !cancelled_parse.paths.empty())
        return fail("cancelled parse continued producing paths");

    owo::engine::FullPinyinIncrementalState incremental_state;
    bool reused = false;
    const auto incremental_prefix = schema.parse_incremental(
        "niha", 16, incremental_state, nullptr, {}, &reused);
    if (!incremental_prefix.valid || reused)
        return fail("incremental parser incorrectly reused its initial request");
    const auto incremental_nihao = schema.parse_incremental(
        "nihao", 16, incremental_state, nullptr, {}, &reused);
    const auto complete_nihao = schema.parse("nihao", 16, false);
    if (!reused || !incremental_nihao.valid ||
        incremental_nihao.normalized_input != complete_nihao.normalized_input ||
        incremental_nihao.paths != complete_nihao.paths)
        return fail("niha to nihao incremental parse diverged from full parse");

    for (const std::string_view input : {
             "heng", "cheng", "sheng", "zheng", "jiaodian", "shubiao",
             "qingkuang", "ceshiyixiav"}) {
        owo::engine::FullPinyinIncrementalState typing_state;
        owo::engine::ParseResult typed;
        for (std::size_t length = 1; length <= input.size(); ++length)
            typed = schema.parse_incremental(input.substr(0, length), 32,
                                             typing_state);
        const auto direct = schema.parse(input, 32, false);
        if (typed.normalized_input != direct.normalized_input ||
            typed.paths != direct.paths) {
            std::cerr << "incremental divergence: " << input << '\n';
            return fail("focus-sensitive word diverged between typed and direct parse");
        }
    }

    return 0;
}
