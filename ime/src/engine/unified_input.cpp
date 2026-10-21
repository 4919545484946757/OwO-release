#include "owo/engine/unified_input.h"

#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/smart_candidates.h"

#include <algorithm>
#include <cstddef>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace owo::engine {
namespace {

enum class RawTokenKind : std::uint8_t {
    letters,
    tone_digit,
    number,
    symbol,
    separator,
};

struct RawToken {
    RawTokenKind kind{RawTokenKind::letters};
    std::size_t begin{};
    std::size_t end{};
};

bool ascii_letter(const char value) {
    return (value >= 'a' && value <= 'z') || (value >= 'A' && value <= 'Z');
}

bool ascii_digit(const char value) { return value >= '0' && value <= '9'; }

bool tone_digit_character(const char value) {
    return value >= '1' && value <= '5';
}

bool whitespace_character(const char value) {
    return value == ' ' || value == '\t' || value == '\r' || value == '\n';
}

std::string lowercase_ascii(const std::string_view text) {
    std::string lowered;
    lowered.reserve(text.size());
    for (const char character : text)
        lowered.push_back(character >= 'A' && character <= 'Z'
                              ? static_cast<char>(character - 'A' + 'a')
                              : character);
    return lowered;
}

std::vector<RawToken> tokenize(const std::string_view text) {    std::vector<RawToken> tokens;
    for (std::size_t position = 0; position < text.size();) {
        const char current = text[position];
        if (ascii_letter(current) || current == '\'') {
            const auto begin = position;
            while (position < text.size() &&
                   (ascii_letter(text[position]) || text[position] == '\''))
                ++position;
            tokens.push_back({RawTokenKind::letters, begin, position});
            continue;
        }
        if (ascii_digit(current)) {
            const bool previous_letters =
                !tokens.empty() && tokens.back().kind == RawTokenKind::letters;
            const bool followed_by_letters =
                position + 1 < text.size() && ascii_letter(text[position + 1]);
            const bool at_end = position + 1 == text.size();
            if (previous_letters && tone_digit_character(current) &&
                (followed_by_letters || at_end)) {
                const auto begin = position++;
                tokens.push_back({RawTokenKind::tone_digit, begin, position});
                continue;
            }
            const auto begin = position;
            while (position < text.size()) {
                if (ascii_digit(text[position])) {
                    ++position;
                    continue;
                }
                if (text[position] == '.' && position + 1 < text.size() &&
                    ascii_digit(text[position + 1])) {
                    ++position;
                    continue;
                }
                break;
            }
            if (position < text.size() && text[position] == '%') ++position;
            tokens.push_back({RawTokenKind::number, begin, position});
            continue;
        }
        if (whitespace_character(current)) {
            const auto begin = position;
            while (position < text.size() && whitespace_character(text[position]))
                ++position;
            tokens.push_back({RawTokenKind::separator, begin, position});
            continue;
        }
        const auto begin = position;
        while (position < text.size() && !ascii_letter(text[position]) &&
               !ascii_digit(text[position]) && !whitespace_character(text[position]) &&
               text[position] != '\'')
            ++position;
        tokens.push_back({RawTokenKind::symbol, begin, position});
    }
    return tokens;
}

UnifiedSegment make_segment(const UnifiedSegmentKind kind, const std::string_view raw,
                            const std::size_t begin, const std::size_t end) {
    UnifiedSegment segment;
    segment.kind = kind;
    segment.begin = begin;
    segment.end = end;
    segment.text = std::string(raw.substr(begin, end - begin));
    return segment;
}

void insert_candidate_by_score(std::vector<Candidate>& target, Candidate candidate) {
    const bool duplicate = std::any_of(
        target.begin(), target.end(), [&candidate](const Candidate& existing) {
            return existing.text == candidate.text;
        });
    if (duplicate) return;
    const auto position = std::find_if(
        target.begin(), target.end(), [&candidate](const Candidate& existing) {
            return !existing.model_only && existing.score < candidate.score;
        });
    target.insert(position, std::move(candidate));
}

Candidate literal_candidate(const std::string_view raw, const std::int64_t score) {
    Candidate candidate;
    candidate.text = std::string(raw);
    candidate.score = score;
    candidate.match_kind = InputMatchKind::exact;
    candidate.consumed_input_bytes = raw.size();
    candidate.segment_count = 1;
    return candidate;
}

// A reading only counts as pinyin when at least one path is exact or an
// unfinished trailing syllable. Corrected and abbreviated paths are recovery
// paths; treating them as evidence would let "hello" masquerade as pinyin.
bool strict_pinyin_parsed(const ParseResult& parsed) {
    return parsed.valid &&
           std::any_of(parsed.paths.begin(), parsed.paths.end(),
                       [](const ParsePath& path) {
                           return path.match_kind == InputMatchKind::exact ||
                                  path.match_kind ==
                                      InputMatchKind::incomplete_completion;
                       });
}

// The unified layer never needs typo recovery: the runtime already owns that
// path, and a correction-enabled parse is two orders of magnitude more
// expensive than a plain one. It also asks for a cheap plausibility verdict
// first, so a Latin word never reaches the abbreviation search
// (see benchmarks/unified_candidate_benchmark.cpp).
bool plausible_reading(const InputSchema& schema, const std::string_view reading) {
    if (const auto* full_pinyin =
            dynamic_cast<const FullPinyinSchema*>(&schema))
        return full_pinyin->is_plausible_reading(reading);
    return schema.parse(reading, 2).valid;
}

ParseResult parse_without_correction(const InputSchema& schema,
                                     const std::string_view reading,
                                     const std::size_t max_paths) {
    if (const auto* full_pinyin =
            dynamic_cast<const FullPinyinSchema*>(&schema))
        return full_pinyin->parse(reading, max_paths, false);
    return schema.parse(reading, max_paths);
}

}  // namespace

UnifiedInputPlan plan_unified_input(const InputSchema& schema,
                                    const std::string_view raw) {
    UnifiedInputPlan plan;
    plan.raw = std::string(raw);
    if (raw.empty()) return plan;
    plan.foreign_input =
        std::any_of(raw.begin(), raw.end(), [](const char value) {
            return !(value >= 'a' && value <= 'z') && value != '\'';
        });
    plan.explicit_latin =
        std::any_of(raw.begin(), raw.end(), [](const char value) {
            return value >= 'A' && value <= 'Z';
        });
    const auto tokens = tokenize(raw);
    for (std::size_t index = 0; index < tokens.size();) {
        const auto& token = tokens[index];
        if (token.kind == RawTokenKind::letters) {
            std::string reading(raw.substr(token.begin, token.end - token.begin));
            // A capitalised or all-caps run is an explicit Latin word or
            // acronym. Pinyin input is lowercase, so the case carries the
            // user's intent without a mode switch.
            const bool explicit_latin =
                std::any_of(reading.begin(), reading.end(), [](const char value) {
                    return value >= 'A' && value <= 'Z';
                });
            std::size_t last = index;
            bool tone_marked = false;
            for (;;) {
                if (last + 2 < tokens.size() &&
                    tokens[last + 1].kind == RawTokenKind::tone_digit &&
                    tokens[last + 2].kind == RawTokenKind::letters) {
                    const auto& next = tokens[last + 2];
                    reading +=
                        std::string(raw.substr(next.begin, next.end - next.begin));
                    tone_marked = true;
                    last += 2;
                    continue;
                }
                if (last + 1 < tokens.size() &&
                    tokens[last + 1].kind == RawTokenKind::tone_digit) {
                    tone_marked = true;
                    ++last;
                }
                break;
            }
            const bool plausible =
                !explicit_latin && plausible_reading(schema, reading);
            if (plausible) {
                auto segment = make_segment(UnifiedSegmentKind::pinyin, raw,
                                            token.begin, tokens[last].end);
                segment.reading = std::move(reading);
                segment.tone_marked = tone_marked;
                segment.strict = true;
                plan.segments.push_back(std::move(segment));
                index = last + 1;
                continue;
            }
            for (std::size_t primitive = index; primitive <= last; ++primitive) {
                const auto& current = tokens[primitive];
                auto segment = make_segment(
                    current.kind == RawTokenKind::letters
                        ? UnifiedSegmentKind::latin
                        : UnifiedSegmentKind::number,
                    raw, current.begin, current.end);
                if (current.kind == RawTokenKind::letters)
                    segment.reading = lowercase_ascii(segment.text);
                plan.segments.push_back(std::move(segment));
            }
            index = last + 1;
            continue;
        }
        const auto kind = token.kind == RawTokenKind::number
                              ? UnifiedSegmentKind::number
                          : token.kind == RawTokenKind::symbol
                              ? UnifiedSegmentKind::symbol
                              : UnifiedSegmentKind::separator;
        plan.segments.push_back(make_segment(kind, raw, token.begin, token.end));
        ++index;
    }

    for (std::size_t index = 0; index < plan.segments.size(); ++index) {
        const auto& segment = plan.segments[index];
        switch (segment.kind) {
            case UnifiedSegmentKind::pinyin:
                plan.has_pinyin = true;
                if (segment.tone_marked) plan.tone_normalized = true;
                if (plan.primary_pinyin_segment == kNoUnifiedSegment ||
                    segment.reading.size() >
                        plan.segments[plan.primary_pinyin_segment].reading.size())
                    plan.primary_pinyin_segment = index;
                break;
            case UnifiedSegmentKind::latin:
            case UnifiedSegmentKind::number:
            case UnifiedSegmentKind::symbol:
                plan.mixed = true;
                break;
            case UnifiedSegmentKind::separator: break;
        }
    }
    plan.decodable = plan.has_pinyin &&
                     plan.primary_pinyin_segment != kNoUnifiedSegment &&
                     plan.segments[plan.primary_pinyin_segment].strict;
    return plan;
}

std::vector<Candidate> generate_unified_candidates(
    const InputSchema& schema, const Lexicon& lexicon, const std::string_view raw,
    const BigramModel* bigram, const UserFrequencyModel* user_frequency,
    const UnifiedCandidateOptions& options,
    UnifiedCandidateBreakdown* breakdown) {
    std::vector<Candidate> candidates;
    if (raw.empty() || options.limit == 0 || raw.size() > kUnifiedInputMaximumBytes)
        return candidates;
    const auto plan = plan_unified_input(schema, raw);
    // Letter-only pinyin input, including typo recovery and abbreviation, is
    // owned by the pinyin engine. Unified extras only attach when the preedit
    // leaves the lowercase pinyin alphabet or carries tone digits.
    const bool unified_entry = plan.foreign_input || plan.tone_normalized;
    const auto& expressive_source = options.expressive_source != nullptr
                                        ? *options.expressive_source
                                        : builtin_expressive_source();
    auto expressive = options.include_expressive
                          ? generate_expressive_candidates(
                                raw, expressive_source, options.expressive_limit)
                          : std::vector<Candidate>{};
    const bool wants_combination =
        options.include_combinations && !expressive.empty();
    const bool decode_gate = plan.has_pinyin && plan.decodable &&
                             options.decode_limit != 0 &&
                             plan.segments[plan.primary_pinyin_segment]
                                     .reading.size() <=
                                 kUnifiedDecodeMaximumReadingBytes;
    std::string combination_source;
    if (decode_gate &&
        (plan.mixed || plan.tone_normalized || wants_combination)) {
        const auto& segment = plan.segments[plan.primary_pinyin_segment];
        const auto parsed = parse_without_correction(
            schema, segment.reading, options.decode_path_limit);
        if (strict_pinyin_parsed(parsed)) {
            const CandidateGenerator generator(lexicon, bigram, user_frequency);
            auto decoded = generator.generate(parsed, options.decode_limit);
            const std::string prefix(plan.raw.substr(0, segment.begin));
            const std::string suffix(plan.raw.substr(segment.end));
            const bool affixed = !prefix.empty() || !suffix.empty();
            // Decoding for a combination only needs the leading text; mixed or
            // tone-digit input additionally exposes the decoded candidates.
            const bool emit_decoded = plan.mixed || plan.tone_normalized;
            for (auto& candidate : decoded) {
                if (candidate.model_only) continue;
                if (candidate.match_kind == InputMatchKind::corrected ||
                    candidate.match_kind ==
                        InputMatchKind::abbreviated_completion)
                    continue;
                if (candidate.consumed_input_bytes !=
                    parsed.normalized_input.size())
                    continue;
                auto text = affixed ? prefix + candidate.text + suffix
                                    : candidate.text;
                if (!affixed && combination_source.empty())
                    combination_source = text;
                if (!emit_decoded) continue;
                candidate.text = std::move(text);
                candidate.consumed_input_bytes = plan.raw.size();
                candidate.source_segments.clear();
                candidate.segment_lengths.clear();
                candidate.segment_count = 1;
                candidates.push_back(std::move(candidate));
                if (breakdown != nullptr) ++breakdown->decoded;
                if (candidates.size() >= options.limit) break;
            }
        }
    }
    const auto count_added = [&candidates](const std::size_t before) {
        return candidates.size() - before;
    };
    if (options.include_literal && unified_entry) {
        const auto before = candidates.size();
        insert_candidate_by_score(
            candidates,
            literal_candidate(raw, plan.explicit_latin
                                       ? kUnifiedLatinIntentScore
                                       : options.literal_score));
        if (breakdown != nullptr) breakdown->literal = count_added(before);
    }
    if (options.include_rule_candidates && unified_entry) {
        const auto before = candidates.size();
        for (auto& candidate : generate_smart_candidates(raw, options.rule_limit)) {
            candidate.consumed_input_bytes = plan.raw.size();
            insert_candidate_by_score(candidates, std::move(candidate));
        }
        if (breakdown != nullptr) breakdown->rules = count_added(before);
    }
    if (options.include_expressive) {
        const std::string leading_asset =
            expressive.empty() ? std::string{} : expressive.front().text;
        const auto before = candidates.size();
        for (auto& candidate : expressive) {
            candidate.consumed_input_bytes = plan.raw.size();
            insert_candidate_by_score(candidates, std::move(candidate));
        }
        if (breakdown != nullptr) breakdown->expressive = count_added(before);
        if (wants_combination && !combination_source.empty() &&
            !leading_asset.empty()) {
            const auto before_combination = candidates.size();
            for (auto& candidate : generate_combination_candidates(
                     combination_source, leading_asset, plan.raw.size())) {
                insert_candidate_by_score(candidates, std::move(candidate));
            }
            if (breakdown != nullptr)
                breakdown->combinations = count_added(before_combination);
        }
    }
    // Concept lookup is keyed on the whole preedit, so it also serves pure
    // pinyin input without touching dictionary ranking.
    if (options.include_concepts) {
        const auto& table = options.concept_table != nullptr
                                ? *options.concept_table
                                : builtin_concept_table();
        const auto before = candidates.size();
        for (auto& candidate : generate_concept_candidates(
                 raw, table, options.concept_limit)) {
            candidate.consumed_input_bytes = plan.raw.size();
            insert_candidate_by_score(candidates, std::move(candidate));
        }
        if (breakdown != nullptr) breakdown->concepts = count_added(before);
    }
    if (candidates.size() > options.limit) candidates.resize(options.limit);
    return candidates;
}

void merge_unified_candidates(std::vector<Candidate>& target,
                              std::vector<Candidate>&& extras) {
    for (auto& extra : extras) {
        if (extra.model_only) continue;
        insert_candidate_by_score(target, std::move(extra));
    }
}

}  // namespace owo::engine
