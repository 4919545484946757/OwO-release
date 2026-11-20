#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <iostream>

namespace {
class LearnedPhrase final : public owo::engine::UserFrequencyModel {
public:
    explicit LearnedPhrase(std::string input = "anyouxiandu", std::string text = "案由先读")
        : input_(std::move(input)), text_(std::move(text)) {}
    std::int64_t score(std::string_view text) const override {
        return text == text_ ? 2000 : 0;
    }
    std::vector<owo::engine::UserLexiconEntry> lookup(std::string_view input, std::size_t) const override {
        return input == input_ ? std::vector<owo::engine::UserLexiconEntry>{{text_, 1, input_}}
                                      : std::vector<owo::engine::UserLexiconEntry>{};
    }
private:
    std::string input_, text_;
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
    const owo::engine::MemoryLexicon suffix_lexicon({
        {{"rang"}, "让", 3000000}, {{"shi"}, "使", 3000000}, {{"jiang"}, "将", 3000000},
        {{"xue", "xi"}, "学习", 100000}, {{"ji", "lu"}, "记录", 100000},
        {{"xue", "xi", "ji", "lu"}, "学习记录", 500},
        {{"xue", "xi", "ji"}, "学习记", 20},
        {{"ji"}, "记", 1000000}, {{"lu"}, "路", 1000000}, {{"lu"}, "录", 1000}});
    const owo::engine::CandidateGenerator suffix_generator(suffix_lexicon);
    for (const auto& [head, text] : {std::pair{"rang", "让"}, {"shi", "使"}, {"jiang", "将"}}) {
        const auto input = std::string(head) + "xuexijilu";
        const auto expected = std::string(text) + "学习记录";
        const auto unsupported_text = std::string(text) + "学习记路";
        const LearnedPhrase old_selection(input, unsupported_text);
        const owo::engine::CandidateGenerator personalized_suffix(suffix_lexicon, nullptr, &old_selection);
        owo::engine::FullPinyinIncrementalState state;
        for (std::size_t size = 1; size <= input.size(); ++size)
            static_cast<void>(schema.parse_incremental(std::string_view(input).substr(0, size), 32, state));
        for (const auto limit : {8u, 18u, 48u, 128u}) for (const auto page : {0u, 5u, 7u, 9u})
        for (const bool model : {false, true}) for (const bool incremental : {false, true}) {
            const auto parsed_suffix = incremental ? state.result : schema.parse(input, 32, false);
            const auto result = suffix_generator.generate(parsed_suffix, limit, false, {}, nullptr, {}, model, page);
            if (result.empty() || result.front().text != expected ||
                result.front().segment_count != 2 || result.front().coherence_score <= 0 ||
                result.front().consumed_input_bytes != input.size() || find_text(result, text) == result.end()) {
                std::cerr << "long dictionary suffix displaced by local bridge: " << input << '\n';
                return 8;
            }
            const auto local = find_text(result, unsupported_text);
            if (local != result.end() && local->coherence_score != 0) return 9;
            if (page != 0) {
                const auto personalized_result = personalized_suffix.generate(
                    parsed_suffix, limit, false, {}, nullptr, {}, model, page);
                const auto retained = find_text(personalized_result, expected);
                if (retained == personalized_result.end() || retained->model_only ||
                    find_text(personalized_result, unsupported_text) == personalized_result.end()) return 10;
            }
        }
        const auto explicit_input = std::string(head) + "'xue'xi'ji'lu";
        const auto separated = suffix_generator.generate(schema.parse(explicit_input, 32, false),
            18, false, {}, nullptr, {}, false, 7);
        if (separated.empty() || separated.front().text != expected ||
            separated.front().consumed_input_bytes != explicit_input.size()) return 11;
    }
    // Hidden model seeds must not undo the visible phrase-chain decision by
    // reintroducing weak, more fragmented homophone permutations. No phrase
    // blacklist: the same structural rule applies to multiple leading words.
    const owo::engine::MemoryLexicon model_seed_lexicon({
        {{"xiu", "fu"}, "修复", 50000}, {{"hui", "fu"}, "恢复", 50000},
        {{"lei", "si", "wen", "ti"}, "类似问题", 50000},
        {{"lei", "si"}, "类似", 40000}, {{"lei", "si"}, "累死", 500},
        {{"wen", "ti"}, "问题", 40000},
        {{"xiu"}, "修", 50000}, {{"hui"}, "恢", 50000}});
    for (const auto& [head, text] : {std::pair{"xiufu", "修复"}, {"huifu", "恢复"}}) {
        const auto input = std::string(head) + "leisiwenti";
        const auto expected = std::string(text) + "类似问题";
        const auto weak = std::string(text) + "累死问题";
        const auto request = schema.parse(input, 32, false);
        for (const auto limit : {18u, 48u, 128u}) for (const auto page : {0u, 5u, 7u, 9u}) {
            const auto result = owo::engine::CandidateGenerator(model_seed_lexicon).generate(
                request, limit, false, {}, nullptr, {}, true, page);
            if (result.empty() || result.front().text != expected ||
                find_text(result, weak) != result.end()) {
                std::cerr << "weak fragmented model seed survived: " << input << '\n';
                return 12;
            }
            const LearnedPhrase selected(input, weak);
            const auto learned_result = owo::engine::CandidateGenerator(
                model_seed_lexicon, nullptr, &selected).generate(
                    request, limit, false, {}, nullptr, {}, true, page);
            if (find_text(learned_result, weak) == learned_result.end()) return 13;
        }
    }
    std::cout << "Dictionary bridges, unmatched edges, quotas and prefix commits passed\n";
}
