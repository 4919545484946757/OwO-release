#include "owo/engine/candidate_generator.h"
#include "owo/engine/candidate_scoring.h"
#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <iostream>
#include <vector>

namespace {
struct State { std::string text, previous; std::int64_t score{}; };
bool better(const State& a, const State& b) {
    return a.score != b.score ? a.score > b.score : a.text < b.text;
}
}

int main() {
    using namespace owo::engine;
    std::vector<LexiconEntry> entries;
    const std::vector<std::string> readings{"ba", "pa", "ma", "fa"};
    for (std::size_t r = 0; r < readings.size(); ++r)
        for (unsigned i = 0; i < 48; ++i) {
            const auto code = 0x4e00U + static_cast<unsigned>(r) * 64 + i;
            const std::string text{static_cast<char>(0xe0U | (code >> 12)),
                static_cast<char>(0x80U | ((code >> 6) & 63)),
                static_cast<char>(0x80U | (code & 63))};
            entries.push_back({{readings[r]}, text, 100000U - i * 1000U});
        }
    const MemoryLexicon lexicon(std::move(entries));
    MemoryBigramModel bigram;
    const auto first = lexicon.lookup_texts(std::vector<std::string_view>{"ba"});
    const auto second = lexicon.lookup_texts(std::vector<std::string_view>{"pa"});
    bigram.set(std::string(first.back().text), std::string(second.back().text), 100000);
    const FullPinyinSchema schema;
    const auto parsed = schema.parse("ba'pa'ma'fa", 32, false);
    for (const bool contextual : {false, true}) {
        const BigramModel* model = contextual ? &bigram : nullptr;
        CandidateGenerator generator(lexicon, model);
        // Deliberately materialize and sort every transition as an independent
        // oracle for the old beam. No timing threshold depending on the CPU.
        std::vector<State> reference(1);
        if (std::count_if(parsed.paths.begin(), parsed.paths.end(), [](const auto& path) {
                return path.match_kind == InputMatchKind::exact;
            }) > 1)
            reference.front().score = kCandidatePreferredExactSegmentationBonus;
        for (const auto& reading : readings) {
            const std::vector<std::string_view> key{reading};
            const auto matches = lexicon.lookup_texts(key);
            std::vector<State> expanded;
            for (const auto& state : reference) for (const auto& entry : matches) {
                auto score = state.score + candidate_unigram_score(entry.frequency);
                if (!state.previous.empty()) {
                    score -= kCandidateAdditionalSegmentPenalty;
                    if (model) score += model->score(state.previous, entry.text);
                }
                expanded.push_back({state.text + std::string(entry.text),
                    std::string(entry.text), score});
            }
            std::sort(expanded.begin(), expanded.end(), better);
            if (expanded.size() > 48) expanded.resize(48);
            reference = std::move(expanded);
        }
        CandidateGenerationMetrics metrics;
        const auto candidates = generator.generate(parsed, 12, false, {}, &metrics);
        if (candidates.empty() || candidates.front().text != reference.front().text ||
            candidates.front().score != reference.front().score ||
            metrics.beam_transitions < 6000 ||
            metrics.beam_materialized * 3 >= metrics.beam_transitions) {
            std::cerr << "Bounded beam differs from full expansion or copied losing states\n";
            if (!candidates.empty()) std::cerr << "actual=" << candidates.front().text
                << " score=" << candidates.front().score << " expected=" << reference.front().text
                << " score=" << reference.front().score << '\n';
            std::cerr << "transitions=" << metrics.beam_transitions
                << " materialized=" << metrics.beam_materialized << '\n';
            return 1;
        }
        auto initials = parsed;
        initials.normalized_input = "bpmf";
        initials.paths.clear(); // Exercise the dedicated initial-sequence beam.
        for (const auto* source : {&parsed, static_cast<const ParseResult*>(&initials)}) {
            CandidateGenerationMetrics interrupted;
            const auto cancelled = generator.generate(*source, 12, false, {}, &interrupted,
                [&] { return interrupted.beam_transitions >= 1000; });
            if (!cancelled.empty() || interrupted.beam_transitions < 1000 ||
                interrupted.beam_transitions > 1100) {
                std::cerr << "Search did not stop inside the active beam expansion\n";
                return 2;
            }
        }
    }
    std::cout << "Bounded beam matches exhaustive expansion, with early rejection and cancellation\n";
}
