#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <iostream>

namespace {
class LearnedPhrase final : public owo::engine::UserFrequencyModel {
public:
    std::int64_t score(std::string_view text) const override {
        return text == "案由先读" ? 2000 : 0;
    }
    std::vector<owo::engine::UserLexiconEntry> lookup(std::string_view input, std::size_t) const override {
        return input == "anyouxiandu" ? std::vector<owo::engine::UserLexiconEntry>{{"案由先读", 1, "anyouxiandu"}}
                                      : std::vector<owo::engine::UserLexiconEntry>{};
    }
};
std::vector<owo::engine::LexiconEntry> fixture(bool bridge, bool whole_word = false) {
    std::vector<owo::engine::LexiconEntry> entries{
        {{"an"}, "按", 3000000}, {{"gao"}, "高", 3000000}, {{"di"}, "低", 3000000},
        {{"an", "you"}, "案由", 20000}, {{"gao", "you"}, "高由", 20000},
        {{"di", "you"}, "低由", 20000},
        {{"xian", "du"}, "先读", 5000}, {{"xian", "du"}, "限度", 5000},
        {{"you", "xian", "du"}, "有限度", 8000},
        {{"an", "you", "xian"}, "按优先", 800},
        {{"gao", "you", "xian"}, "高优先", 800},
        {{"di", "you", "xian"}, "低优先", 800},
        {{"du"}, "度", 2000000}, {{"du"}, "读", 1000000}};
    if (bridge) entries.push_back({{"you", "xian", "du"}, "优先度", 100});
    if (whole_word) entries.push_back({{"an", "you", "xian", "du"}, "案由先读", 1});
    return entries;
}

auto find_text(const std::vector<owo::engine::Candidate>& candidates, std::string_view text) {
    return std::find_if(candidates.begin(), candidates.end(), [text](const auto& candidate) {
        return candidate.text == text;
    });
}
}  // namespace

int main() {
    const owo::engine::FullPinyinSchema schema;
    const owo::engine::MemoryLexicon lexicon(fixture(true));
    const owo::engine::CandidateGenerator generator(lexicon);
    for (const auto& [head, text] : {std::pair{"an", "按"}, {"gao", "高"}, {"di", "低"}}) {
        const auto input = std::string(head) + "youxiandu";
        const auto expected = std::string(text) + "优先度";
        const auto prefix = std::string(text) + "优先";
        for (const bool explicit_boundary : {false, true}) {
            const auto source = explicit_boundary ? std::string(head) + "'you'xian'du" : input;
            const auto parsed = schema.parse(source, 32, false);
            owo::engine::FullPinyinIncrementalState state;
            for (std::size_t size = 1; size <= source.size(); ++size)
                static_cast<void>(schema.parse_incremental(std::string_view(source).substr(0, size), 32, state));
            for (const bool incremental : {false, true})
            for (const auto limit : {8u, 18u, 48u, 128u})
            for (const auto page : {0u, 5u, 7u, 9u})
            for (const bool model : {false, true}) {
                const auto candidates = generator.generate(incremental ? state.result : parsed,
                    limit, false, {}, nullptr, {}, model, page);
                const auto short_word = find_text(candidates, prefix);
                if (candidates.empty() || candidates.front().text != expected ||
                    candidates.front().consumed_input_bytes != source.size() ||
                    candidates.front().coherence_score < 24000 ||
                    short_word == candidates.end() ||
                    short_word->consumed_input_bytes != source.size() - (explicit_boundary ? 3 : 2) ||
                    find_text(candidates, text) == candidates.end()) {
                    std::cerr << "dictionary bridge lost: " << source << " budget=" << limit
                              << " page=" << page << " model=" << model << '\n';
                    if (!candidates.empty()) std::cerr << "actual=" << candidates.front().text << '\n';
                    return 1;
                }
                // This request has exactly one source reading. Repairing the
                // score must not expose a Cartesian list of visible sentences.
                if (page != 0 && std::count_if(candidates.begin(), candidates.end(), [&](const auto& candidate) {
                        return !candidate.model_only && candidate.consumed_input_bytes == source.size() &&
                               candidate.segment_count > 1;
                    }) > 2) return 2;
            }
        }
    }
    const auto parsed = schema.parse("anyouxiandu", 32, false);
    const auto candidates = generator.generate(parsed, 128, false, {}, nullptr, {}, true, 7);
    const auto unsupported = find_text(candidates, "按有限度");
    if (unsupported == candidates.end() || unsupported->coherence_score != 0) {
        std::cerr << "unmatched spans still bypass uncovered-character costs\n";
        return 3;
    }
    const owo::engine::MemoryLexicon no_bridge(fixture(false));
    const auto without = owo::engine::CandidateGenerator(no_bridge).generate(
        parsed, 128, false, {}, nullptr, {}, true, 7);
    const auto unbridged = find_text(without, "按优先度");
    if (unbridged == without.end() || unbridged->coherence_score != 0) return 4;
    const owo::engine::MemoryLexicon whole(fixture(true, true));
    const auto whole_candidates = owo::engine::CandidateGenerator(whole).generate(
        parsed, 48, false, {}, nullptr, {}, true, 0);
    if (whole_candidates.empty() || whole_candidates.front().text != "案由先读" ||
        whole_candidates.front().segment_count != 1) {
        std::cerr << "legacy full-word precedence was displaced\n";
        return 5;
    }
    const auto partitioned_whole = owo::engine::CandidateGenerator(whole).generate(
        parsed, 48, false, {}, nullptr, {}, true, 7);
    if (find_text(partitioned_whole, "案由先读") == partitioned_whole.end()) return 6;
    const LearnedPhrase learned;
    const owo::engine::CandidateGenerator personalized(lexicon, nullptr, &learned);
    for (const auto limit : {8u, 18u, 48u, 128u}) for (const bool model : {false, true}) {
        const auto result = personalized.generate(parsed, limit, true, {}, nullptr, {}, model, 7);
        const auto old_word = find_text(result, "案由先读");
        const auto alternative = find_text(result, "按优先度");
        if (old_word == result.end() || !old_word->user_lexicon || alternative == result.end() ||
            alternative->model_only || alternative->user_lexicon) {
            std::cerr << "learned dictionary source suppressed the bridged sentence source\n";
            return 7;
        }
    }
    std::cout << "Dictionary bridges, unmatched edges, quotas and prefix commits passed\n";
}
