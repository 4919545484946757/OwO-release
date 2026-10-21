#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/smart_candidates.h"
#include "owo/engine/unified_input.h"

#include <algorithm>
#include <iostream>
#include <string>
#include <string_view>
#include <vector>

namespace {

int fail(const std::string_view message) {
    std::cerr << message << '\n';
    return 1;
}

const owo::engine::Candidate* find_candidate(
    const std::vector<owo::engine::Candidate>& candidates,
    const std::string_view text) {
    const auto found = std::find_if(
        candidates.begin(), candidates.end(),
        [text](const owo::engine::Candidate& candidate) {
            return candidate.text == text;
        });
    return found == candidates.end() ? nullptr : &*found;
}

bool is_emoji_candidate(const std::string_view text) {
    return text.size() >= 4 &&
           static_cast<unsigned char>(text.front()) >= 0xF0;
}

}  // namespace

int main() {
    const owo::engine::FullPinyinSchema schema;

    {
        const auto plan = owo::engine::plan_unified_input(schema, "nihao");
        if (!plan.has_pinyin || plan.mixed || !plan.decodable ||
            !plan.pure_pinyin() || plan.tone_normalized || plan.foreign_input ||
            plan.segments.size() != 1 ||
            plan.segments.front().kind != owo::engine::UnifiedSegmentKind::pinyin)
            return fail("plain pinyin was not classified as pure pinyin");
    }
    {
        const auto plan = owo::engine::plan_unified_input(schema, "ni3hao3");
        if (plan.segments.size() != 1 || !plan.tone_normalized || plan.mixed ||
            !plan.foreign_input || !plan.segments.front().tone_marked ||
            plan.segments.front().reading != "nihao" ||
            plan.segments.front().text != "ni3hao3")
            return fail("tone digits were not folded into one pinyin span");
    }
    {
        // A word outside the pinyin syllable inventory is classified literal,
        // but literal extras stay gated on the preedit alphabet: pure
        // lowercase letters keep belonging to the pinyin engine.
        const auto plan = owo::engine::plan_unified_input(schema, "hello");
        if (plan.has_pinyin || !plan.mixed || plan.decodable ||
            plan.foreign_input ||
            plan.segments.size() != 1 ||
            plan.segments.front().kind != owo::engine::UnifiedSegmentKind::latin)
            return fail("Latin word was not classified as literal");
    }
    {
        const auto plan = owo::engine::plan_unified_input(schema, "API");
        if (plan.has_pinyin || !plan.mixed || !plan.foreign_input ||
            !plan.explicit_latin ||
            plan.segments.size() != 1 ||
            plan.segments.front().kind != owo::engine::UnifiedSegmentKind::latin)
            return fail("uppercase acronym was treated as pinyin");
    }
    {
        const auto plan = owo::engine::plan_unified_input(schema, "baidu.com");
        if (plan.explicit_latin)
            return fail("lowercase input was marked as explicit Latin");
    }
    {
        const auto plan = owo::engine::plan_unified_input(schema, "baidu.com");
        if (!plan.has_pinyin || !plan.mixed || !plan.foreign_input ||
            plan.segments.size() != 3 || plan.primary_pinyin_segment != 0 ||
            plan.segments[1].kind != owo::engine::UnifiedSegmentKind::symbol)
            return fail("domain-style mixed input was misclassified");
    }
    {
        const auto plan = owo::engine::plan_unified_input(schema, "woaini123");
        if (!plan.has_pinyin || !plan.mixed || !plan.foreign_input ||
            plan.segments.size() != 2 ||
            plan.segments[1].kind != owo::engine::UnifiedSegmentKind::number)
            return fail("trailing digits were not kept as a number span");
    }
    {
        const auto plan = owo::engine::plan_unified_input(schema, "3.14");
        if (plan.has_pinyin || !plan.mixed || !plan.foreign_input ||
            plan.segments.size() != 1 ||
            plan.segments.front().kind != owo::engine::UnifiedSegmentKind::number ||
            plan.segments.front().text != "3.14")
            return fail("decimal number was not classified as one span");
    }

    const owo::engine::MemoryLexicon lexicon({
        {{"ni", "hao"}, "你好", 1000},
        {{"ni", "hao"}, "你号", 50},
        {{"lei", "si"}, "累死", 300},
        {{"bai", "du"}, "百度", 2000},
    });

    {
        // Pure pinyin is owned by the dictionary path; the unified layer only
        // contributes extras, and a plain reading has none.
        const auto extras =
            owo::engine::generate_unified_candidates(schema, lexicon, "nihao");
        if (!extras.empty())
            return fail("pure pinyin produced unified extras");
    }
    for (const std::string_view recoverable : {"niaho", "wsm", "quan'loi",
                                               "gongnneg", "hello"}) {
        const auto extras = owo::engine::generate_unified_candidates(
            schema, lexicon, recoverable);
        if (!extras.empty())
            return fail("recoverable pinyin input produced unified extras");
    }
    {
        const auto candidates =
            owo::engine::generate_unified_candidates(schema, lexicon, "ni3hao3");
        const auto* decoded = find_candidate(candidates, "你好");
        const auto* literal = find_candidate(candidates, "ni3hao3");
        if (decoded == nullptr || decoded->consumed_input_bytes != 7)
            return fail("tone-digit pinyin did not decode to the whole input");
        if (literal == nullptr || literal->consumed_input_bytes != 7)
            return fail("toned literal fallback is missing");
        if (decoded->score <= literal->score)
            return fail("dictionary decoding must outrank the literal fallback");
    }
    {
        const auto candidates =
            owo::engine::generate_unified_candidates(schema, lexicon, "baidu.com");
        const auto* decoded = find_candidate(candidates, "百度.com");
        const auto* literal = find_candidate(candidates, "baidu.com");
        if (decoded == nullptr || decoded->consumed_input_bytes != 9)
            return fail("mixed pinyin span did not keep the literal suffix");
        if (literal == nullptr)
            return fail("mixed input lost its literal candidate");
        if (decoded->score <= literal->score)
            return fail("decoded span must outrank the literal fallback");
    }
    {
        const auto candidates =
            owo::engine::generate_unified_candidates(schema, lexicon, "hello123");
        if (candidates.size() != 1 || candidates.front().text != "hello123" ||
            candidates.front().consumed_input_bytes != 8)
            return fail("Latin-plus-digits input did not produce the literal");
    }
    {
        const auto candidates =
            owo::engine::generate_unified_candidates(schema, lexicon, "API");
        if (candidates.size() != 1 || candidates.front().text != "API" ||
            candidates.front().score != owo::engine::kUnifiedLatinIntentScore)
            return fail("uppercase acronym did not lead with the literal");
    }
    {
        // A merged dictionary candidate below the Latin-intent score keeps the
        // literal on top, so typing an acronym does not get rewritten.
        std::vector<owo::engine::Candidate> target;
        owo::engine::Candidate dictionary;
        dictionary.text = "阿皮";
        dictionary.score = 4'000;
        dictionary.consumed_input_bytes = 3;
        target.push_back(dictionary);
        auto extras = owo::engine::generate_unified_candidates(
            schema, lexicon, "API");
        owo::engine::merge_unified_candidates(target, std::move(extras));
        if (target.size() != 2 || target.front().text != "API")
            return fail("Latin-intent literal did not outrank a weak dictionary word");
    }
    {
        owo::engine::UnifiedCandidateOptions options;
        options.limit = 1;
        const auto candidates = owo::engine::generate_unified_candidates(
            schema, lexicon, "hello123", nullptr, nullptr, options);
        if (candidates.size() != 1)
            return fail("unified candidate limit was not applied");
    }
    {
        const auto candidates =
            owo::engine::generate_unified_candidates(schema, lexicon, "leisi");
        if (candidates.empty() || candidates.size() > 4)
            return fail("expressive candidate count is out of bounds");
        if (!is_emoji_candidate(candidates.front().text))
            return fail("expressive emoji candidate is missing");
        if (!find_candidate(candidates, "累死😩"))
            return fail("multimodal combination candidate is missing");
        for (const auto& candidate : candidates)
            if (candidate.consumed_input_bytes != 5)
                return fail("expressive candidate consumption is wrong");
    }
    {
        // Without a decoding lexicon entry there is nothing to combine.
        owo::engine::UnifiedCandidateOptions options;
        options.include_combinations = false;
        const auto candidates = owo::engine::generate_unified_candidates(
            schema, lexicon, "leisi", nullptr, nullptr, options);
        if (find_candidate(candidates, "累死😩"))
            return fail("combination candidate ignored the option");
    }
    {
        const auto candidates =
            owo::engine::generate_unified_candidates(schema, lexicon, "1234");
        if (!find_candidate(candidates, "1234") ||
            !find_candidate(candidates, "一千二百三十四") ||
            !find_candidate(candidates, "壹仟贰佰叁拾肆"))
            return fail("number readings are not part of unified extras");
    }

    {
        std::vector<owo::engine::Candidate> target;
        owo::engine::Candidate best;
        best.text = "累死";
        best.score = 6'000;
        best.consumed_input_bytes = 5;
        owo::engine::Candidate weak;
        weak.text = "雷死";
        weak.score = 300;
        weak.consumed_input_bytes = 5;
        target.push_back(best);
        target.push_back(weak);
        auto extras =
            owo::engine::generate_unified_candidates(schema, lexicon, "leisi");
        owo::engine::merge_unified_candidates(target, std::move(extras));
        if (target.size() != 5 || target[0].text != "累死" ||
            target[1].text.empty() || !is_emoji_candidate(target[1].text) ||
            target[2].text != "累死😩" || target[4].text != "雷死")
            return fail("unified extras were not inserted by score");
        owo::engine::merge_unified_candidates(target, {});
        if (target.size() != 5)
            return fail("empty merge changed the candidate list");
    }

    {
        // Hostile input must never escape the documented bounds or report a
        // consumption that does not match the preedit.
        const std::vector<std::string> hostile{
            std::string("你好").substr(0, 4),
            std::string("\x80\x80", 2),
            std::string("\xFF\xFE", 2),
            std::string(200, 'a'),
            "!!!???，，。",
            "A1!你,好",
            std::string("\xE3\x80\x80", 3),
            "1+",
        };
        for (const auto& input : hostile) {
            const auto unified = owo::engine::generate_unified_candidates(
                schema, lexicon, input);
            if (unified.size() > 12)
                return fail("unified candidates exceeded the limit");
            for (const auto& candidate : unified)
                if (candidate.consumed_input_bytes != input.size())
                    return fail("hostile input produced a wrong consumption");
            if (owo::engine::generate_smart_candidates(input, 4).size() > 6)
                return fail("smart candidates exceeded the limit");
            if (owo::engine::generate_expressive_candidates(input).size() > 3)
                return fail("expressive candidates exceeded the limit");
            if (owo::engine::generate_concept_candidates(input).size() > 2)
                return fail("concept candidates exceeded the limit");
        }
    }

    {
        // The breakdown is what the core service logs, so it must match the
        // candidate list the caller receives.
        owo::engine::UnifiedCandidateBreakdown breakdown;
        const auto candidates = owo::engine::generate_unified_candidates(
            schema, lexicon, "ni3hao3", nullptr, nullptr, {},
            &breakdown);
        if (breakdown.decoded != 2 || breakdown.literal != 1 ||
            breakdown.rules != 0 || breakdown.expressive != 0 ||
            breakdown.combinations != 0 || breakdown.concepts != 0)
            return fail("tone-digit breakdown is wrong");
        if (candidates.size() !=
            breakdown.decoded + breakdown.literal + breakdown.rules +
                breakdown.expressive + breakdown.combinations +
                breakdown.concepts)
            return fail("breakdown does not match the candidate list");
    }
    {
        owo::engine::UnifiedCandidateBreakdown breakdown;
        const auto expressive_candidates = owo::engine::generate_unified_candidates(
            schema, lexicon, "leisi", nullptr, nullptr, {}, &breakdown);
        if (expressive_candidates.size() !=
                breakdown.decoded + breakdown.literal + breakdown.rules +
                    breakdown.expressive + breakdown.combinations +
                    breakdown.concepts ||
            breakdown.expressive != 2 || breakdown.combinations != 1 ||
            breakdown.concepts != 0)
            return fail("expressive breakdown is wrong");
    }
    {
        owo::engine::UnifiedCandidateBreakdown breakdown;
        const auto concept_candidates = owo::engine::generate_unified_candidates(
            schema, lexicon, "chongfuzhixing", nullptr, nullptr, {}, &breakdown);
        if (concept_candidates.size() != breakdown.concepts ||
            breakdown.concepts != 1)
            return fail("concept breakdown is wrong");
    }
    {
        owo::engine::UnifiedCandidateBreakdown breakdown;
        const auto number_candidates = owo::engine::generate_unified_candidates(
            schema, lexicon, "1234", nullptr, nullptr, {}, &breakdown);
        if (number_candidates.size() !=
                breakdown.decoded + breakdown.literal + breakdown.rules +
                    breakdown.expressive + breakdown.combinations +
                    breakdown.concepts ||
            breakdown.rules < 2 || breakdown.literal != 1)
            return fail("number-reading breakdown is wrong");
    }

    {
        // Deterministic pseudo-random sweep: the unified entry point must stay
        // bounded, report the truth, and be stable across calls (the core
        // service caches these results).
        constexpr std::string_view kAlphabet =
            "abcdefghijklmnopqrstuvwxyz0123456789+-*/=%^().,;:!?'\"<>[]{}#@$&|~_ ABCXYZ\\";
        std::uint32_t state = 0x12345678U;
        const auto next = [&state]() {
            state = state * 1664525U + 1013904223U;
            return state;
        };
        for (int sample = 0; sample < 500; ++sample) {
            std::string input;
            const auto length = next() % 25U;
            for (std::uint32_t index = 0; index < length; ++index)
                input.push_back(kAlphabet[next() % kAlphabet.size()]);
            owo::engine::UnifiedCandidateBreakdown breakdown;
            const auto first = owo::engine::generate_unified_candidates(
                schema, lexicon, input, nullptr, nullptr, {}, &breakdown);
            const auto second = owo::engine::generate_unified_candidates(
                schema, lexicon, input);
            if (first.size() > owo::engine::UnifiedCandidateOptions{}.limit)
                return fail("random input exceeded the candidate limit");
            if (first.size() != breakdown.total())
                return fail("random input breakdown does not match");
            if (first.size() != second.size())
                return fail("random input is not deterministic");
            std::vector<std::string> seen;
            for (std::size_t index = 0; index < first.size(); ++index) {
                const auto& candidate = first[index];
                if (candidate.text.empty())
                    return fail("random input produced an empty candidate");
                if (candidate.consumed_input_bytes != input.size())
                    return fail("random input produced a wrong consumption");
                if (candidate.text != second[index].text ||
                    candidate.score != second[index].score)
                    return fail("random input ordering is not deterministic");
                if (std::find(seen.begin(), seen.end(), candidate.text) !=
                    seen.end())
                    return fail("random input produced a duplicate candidate");
                seen.push_back(candidate.text);
            }
        }
    }

    {
        // Every assistive source can be switched off independently; the
        // modeless literal/decoding still obey the same options.
        owo::engine::UnifiedCandidateOptions options;
        options.include_literal = false;
        options.include_rule_candidates = false;
        options.include_expressive = false;
        options.include_combinations = false;
        options.include_concepts = false;
        if (!owo::engine::generate_unified_candidates(
                 schema, lexicon, "hello123", nullptr, nullptr, options)
                 .empty())
            return fail("all-off options still produced candidates");
    }
    {
        owo::engine::UnifiedCandidateOptions options;
        options.limit = 0;
        if (!owo::engine::generate_unified_candidates(
                 schema, lexicon, "ni3hao3", nullptr, nullptr, options)
                 .empty())
            return fail("zero limit produced candidates");
    }

    std::cout << "unified input tests passed\n";
    return 0;
}
