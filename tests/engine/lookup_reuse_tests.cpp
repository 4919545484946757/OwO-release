#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"
#include "../../src/engine/lexicon_match.h"

#include <algorithm>
#include <iostream>
#include <map>
#include <memory>
#include <random>

namespace {
using namespace owo::engine;
class CountingLexicon final : public Lexicon {
public:
    std::unique_ptr<MemoryLexicon> data;
    mutable std::map<std::vector<std::string>, unsigned> rows, maxima;
    auto own(std::span<const std::string_view> reading) const {
        std::vector<std::string> result;
        for (auto s : reading) result.emplace_back(s);
        return result;
    }
    std::vector<LexiconEntry> lookup(std::span<const std::string_view> r) const override { return data->lookup(r); }
    std::vector<LexiconTextEntry> lookup_texts(std::span<const std::string_view> r) const override {
        ++rows[own(r)]; return data->lookup_texts(r);
    }
    std::uint32_t lookup_max_frequency(std::span<const std::string_view> r) const override {
        ++maxima[own(r)]; return data->lookup_max_frequency(r);
    }
    std::vector<LexiconEntry> lookup_initial(char c, std::size_t n) const override { return data->lookup_initial(c,n); }
    std::vector<InitialLexiconTextEntry> lookup_initial_texts(char c, std::size_t n) const override { return data->lookup_initial_texts(c,n); }
    std::vector<AbbreviatedLexiconMatch> lookup_mixed_abbreviation(std::string_view s, std::size_t n) const override { return data->lookup_mixed_abbreviation(s,n); }
    std::vector<AbbreviatedLexiconTextMatch> lookup_mixed_abbreviation_texts(std::string_view s, std::size_t n) const override { return data->lookup_mixed_abbreviation_texts(s,n); }
    std::size_t maximum_reading_length() const noexcept override { return data->maximum_reading_length(); }
};

// Keep the original exhaustive DFS as an independent equivalence oracle.
std::optional<std::vector<std::string>> reference(
    std::span<const std::string_view> syllables, std::string_view input) {
    if (syllables.size() < 2 || input.size() < 2 || input.find('\'') != input.npos) return {};
    std::vector<std::string> segments;
    const auto visit = [&](auto&& self, std::size_t index, std::size_t offset, bool abbreviated) -> bool {
        if (index == syllables.size()) return offset == input.size() && abbreviated;
        if (input.size() - offset < syllables.size() - index) return false;
        const auto maximum = std::min(syllables[index].size(),
            input.size() - offset - (syllables.size() - index - 1));
        for (auto n = maximum; n > 0; --n) {
            const auto source = input.substr(offset, n);
            if (!syllables[index].starts_with(source)) continue;
            segments.emplace_back(source);
            if (self(self, index + 1, offset + n, abbreviated || n < syllables[index].size())) return true;
            segments.pop_back();
        }
        return false;
    };
    return visit(visit, 0, 0, false) ? std::optional{segments} : std::nullopt;
}
}

int main() {
    CountingLexicon lexicon;
    const CandidateGenerator generator(lexicon);
    const auto parsed = FullPinyinSchema{}.parse("ni'hao'shi'jie", 32, false);
    for (const auto word : {"你好", "泥号"}) {
        lexicon.data = std::make_unique<MemoryLexicon>(std::vector<LexiconEntry>{
            {{"ni", "hao"}, word, 1000}, {{"shi", "jie"}, "世界", 900},
            {{"ni"}, "你", 800}, {{"hao"}, "好", 800}, {{"shi"}, "是", 800}, {{"jie"}, "界", 800}});
        lexicon.rows.clear(); lexicon.maxima.clear();
        CandidateGenerationMetrics metrics;
        const auto result = generator.generate(parsed, 48, false, {}, &metrics, {}, true, 7);
        const auto repeated = [](const auto& table) {
            return std::any_of(table.begin(), table.end(), [](const auto& row) { return row.second > 1; });
        };
        if (result.empty() || result.front().text != std::string(word) + "世界" ||
            metrics.exact_lookup_cache_hits == 0 || repeated(lexicon.rows) || repeated(lexicon.maxima)) {
            std::cerr << "Repeated dictionary lookup, trimmed row, or cross-request stale cache\n";
            return 1;
        }
    }
    // Distinct syllable boundaries must not alias even when concatenation is equal.
    const MemoryLexicon codes({{{"a", "bc"}, "甲乙", 1000}, {{"ab", "c"}, "丙丁", 900}});
    ParseResult code;
    code.normalized_input = "abc"; code.valid = true; code.phonetic_assistance = false;
    code.paths = {{{{"a",0,1,true},{"bc",1,3,true}}}, {{{"ab",0,2,true},{"c",2,3,true}}}};
    const auto code_candidates = CandidateGenerator(codes).generate(code,48,false,{},nullptr,{},false,7);
    for (const auto expected : {"甲乙", "丙丁"})
        if (std::none_of(code_candidates.begin(), code_candidates.end(), [&](const auto& c) { return c.text == expected; })) return 2;

    std::mt19937 random(17391);
    constexpr std::string_view syllables[]{"a", "aa", "ab", "aba", "b", "ba", "baa", "bb", ""};
    for (unsigned sample = 0; sample < 6000; ++sample) {
        std::vector<std::string_view> reading;
        for (unsigned n = 2 + random() % 5; n > 0; --n) reading.push_back(syllables[random() % 9]);
        std::string input;
        for (unsigned n = random() % 13; n > 0; --n) input += random() % 2 ? 'a' : 'b';
        if (detail::mixed_abbreviation_segments(reading,input) != reference(reading,input)) {
            std::cerr << "Memoized mixed reading differs from exhaustive DFS\n"; return 3;
        }
    }
    std::vector<std::string_view> repeated(18, "aaaa");
    const auto pathological = std::string(37,'a') + 'b';
    detail::MixedMatchMetrics work;
    if (detail::mixed_abbreviation_segments(repeated,pathological,&work) ||
        work.memo_hits == 0 || work.visits > 18 * 39 * 8) return 4;
    std::vector<std::string_view> large(400,"aaaa");
    const std::string large_input(1200,'a');
    if (detail::mixed_abbreviation_segments(large,large_input) != reference(large,large_input)) return 5;
    std::cout << "Request lookup reuse, dictionary refresh, structured keys and memoized matching passed\n";
}
