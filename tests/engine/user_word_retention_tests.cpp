#include "owo/engine/binary_lexicon.h"
#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/user_frequency.h"

#include <algorithm>
#include <iostream>
#include <tuple>
#include <cstdlib>

int main(int argc, char **argv) {
    using namespace owo::engine;
    const MemoryLexicon fixture({{{"dong"}, "动", 1000000},
                                 {{"dong"}, "东", 900000},
                                 {{"dong"}, "洞", 800000},
                                 {{"dong"}, "冬", 700000},
                                 {{"dong"}, "董", 10},
                                 {{"ke"}, "柯", 10},
                                 {{"yi"}, "伊", 10},
                                 {{"ke", "yi"}, "可以", 1000000},
                                 {{"dong", "ke"}, "冬科", 100000},
                                 {{"dong", "ke"}, "鮗科", 90000}});
    BinaryLexicon actual;
    if (argc > 1 && !actual.load(argv[1]).success)
        return 2;
    const auto &lexicon =
        argc > 1 ? static_cast<const Lexicon &>(actual) : static_cast<const Lexicon &>(fixture);
    std::uint64_t now = 1'000'000;
    UserFrequencyStore frequency([&] { return now; });
    frequency.record("dongkeyi", "董柯伊");
    now += 61;
    const FullPinyinSchema schema;
    CandidateGenerator generator(lexicon, nullptr, &frequency);
    bool ok = true;
    for (const bool explicit_edit : {false, true}) {
        if (explicit_edit) {
            if (!frequency.edit_words({{"董柯伊", 1, "dongkeyi"}}).empty())
                return 3;
            frequency.record("ceshi", "测试");
            now += 86400;
            if (!frequency.preferred("dongkeyi", "董柯伊"))
                return 4;
        }
        for (const bool model : {false, true}) {
            for (const auto page_size : {0U, 7U}) {
                for (const auto limit : {8U, 18U, 48U, 256U}) {
                    FullPinyinIncrementalState state;
                    for (const auto input :
                         {"d", "do", "don", "dong", "dongk", "dongke", "dongkey", "dongkeyi"}) {
                        const auto parsed = schema.parse_incremental(input, 32, state);
                        const auto candidates = generator.generate(parsed, limit, true, {}, nullptr,
                                                                   {}, model, page_size, true, 0);
                        if (std::string_view(input) != "dongkey" &&
                            std::string_view(input) != "dongkeyi")
                            continue;
                        const auto found = std::find_if(
                            candidates.begin(), candidates.end(), [](const auto &candidate) {
                                return candidate.text == "董柯伊" && !candidate.model_only;
                            });
                        const bool complete = std::string_view(input) == "dongkeyi";
                        if (found == candidates.end() || found->segment_count != 1 ||
                            !found->user_lexicon ||
                            (complete && explicit_edit &&
                             (candidates.empty() || candidates.front().text != "董柯伊"))) {
                            ok = false;
                            if (!candidates.empty()) std::cerr << "first=" << candidates.front().text << '\n';
                            std::cerr << "Lost user-word identity/visibility: input=" << input
                                      << " limit=" << limit << " page=" << page_size
                                      << " model=" << model << " edited=" << explicit_edit << '\n';
                            if (found != candidates.end())
                                std::cerr << "segments=" << found->segment_count
                                          << " user=" << found->user_lexicon
                                          << " provisional=" << found->provisional_user_word
                                          << '\n';
                        }
                    }
                }
            }
        }
    }
    UserFrequencyStore shared([&] { return now; });
    shared.record("ejirenyuan", "俄籍人员", 4);
    shared.record("shenyuci", "沈昱慈", 4); // Private word absent from the base dictionary.
    shared.record("moyu", "莫玉", 4);
    const CandidateGenerator transferred(fixture, nullptr, &shared);
    for (const auto& [full, text, variants] : std::vector<std::tuple<std::string, std::string, std::vector<std::string>>>{
             {"ejirenyuan", "俄籍人员", {"ejry", "e'j'r'y", "ej'ry", "ejiren'y", "ejireny", "ejirenyuan"}},
             {"shenyuci", "沈昱慈", {"syc", "shyci", "shen'yu'c", "shenyuci"}},
             {"moyu", "莫玉", {"my", "moy", "moyu"}}}) {
        for (const auto limit : {8U, 18U, 48U}) for (const bool model : {false, true}) {
            std::int64_t reference = 0;
            const auto original = transferred.generate(schema.parse(full, 32, false), limit, false, {}, nullptr, {}, model, 7, false, 0);
            for (const auto& candidate : original) if (candidate.text == text) reference = candidate.score;
            for (const auto& input : variants) {
                const auto result = transferred.generate(schema.parse(input, 32, false), limit, false, {}, nullptr, {}, model, 7, false, 0);
                if (result.empty() || result.front().text != text || !result.front().user_preferred() ||
                    result.front().consumed_input_bytes != input.size() ||
                    std::abs(result.front().score - reference) > 1000) {
                    std::cerr << "Learned boost did not transfer: " << input << '\n'; return 6;
                }
            }
        }
    }
    for (const auto input : {"ejr", "ejiren'yu'an", "egry"}) {
        const auto result = transferred.generate(schema.parse(input, 32, false), 18, false, {}, nullptr, {}, true, 7, false, 0);
        if (std::any_of(result.begin(), result.end(), [](const auto& candidate) { return candidate.text == "俄籍人员"; })) return 7;
    }
    if (shared.contextual_count("ejry", "俄籍人员") != 0 || shared.count("俄籍人员") != 4) return 8;
    shared.record("moyuer", "莫玉珥");
    if (shared.lookup_related("myer", 8).empty() || !shared.lookup_related("myer", 8).front().preferred) return 10;
    now += 61;
    if (shared.lookup_related("myer", 8).empty() || shared.lookup_related("myer", 8).front().preferred) return 11;
    shared.clear_learning();
    if (!shared.lookup_related("ejry", 8).empty()) return 9;
    return ok ? 0 : 5;
}
