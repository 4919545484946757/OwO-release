#include "owo/engine/binary_lexicon.h"
#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/user_frequency.h"

#include <algorithm>
#include <iostream>

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
    return ok ? 0 : 5;
}
