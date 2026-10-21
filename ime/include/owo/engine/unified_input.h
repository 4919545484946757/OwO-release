#pragma once

#include "owo/engine/candidate_generator.h"
#include "owo/engine/concept_candidates.h"
#include "owo/engine/expressive_candidates.h"
#include "owo/engine/input_schema.h"
#include "owo/engine/lexicon.h"

#include <cstddef>
#include <cstdint>
#include <limits>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {

// Modeless unified input: one preedit may mix pinyin, Latin words, digits, and
// symbols. The classifier never rewrites the user's text; it only labels spans
// so candidate sources can be attached without toggling an input mode. The
// pinyin parser keeps owning pure-pinyin input; this layer exists for the
// mixed content it cannot cover today.
inline constexpr std::size_t kUnifiedInputMaximumBytes = 128;
// Decoding a long pinyin span inside a mixed preedit duplicates work the
// runtime already does for pure pinyin, and its cost grows with the span.
// Beyond this length the literal candidate is the useful offer.
inline constexpr std::size_t kUnifiedDecodeMaximumReadingBytes = 32;
inline constexpr std::size_t kNoUnifiedSegment =
    (std::numeric_limits<std::size_t>::max)();
// Literal candidates rank below dictionary words on purpose: an explicit
// "keep it as typed" choice should stay reachable without displacing a strong
// Chinese decoding.
inline constexpr std::int64_t kUnifiedLiteralScore = 450;
// Uppercase input is an explicit Latin signal (API, HelloWorld): the literal
// candidate leads instead of trailing the dictionary.
inline constexpr std::int64_t kUnifiedLatinIntentScore = 5'000;

enum class UnifiedSegmentKind : std::uint8_t {
    pinyin,    // letters (with apostrophes and tone digits) that parse
    latin,     // letters that do not form a pinyin reading
    number,    // digits, decimal point, percent
    symbol,    // code and math punctuation
    separator  // whitespace
};

struct UnifiedSegment {
    UnifiedSegmentKind kind{UnifiedSegmentKind::latin};
    // Exact bytes from the preedit, including tone digits.
    std::string text;
    // Pinyin bytes with tone digits removed; lowercase for Latin segments.
    std::string reading;
    std::size_t begin{};
    std::size_t end{};
    bool tone_marked{};
    // A strict reading (exact or unfinished trailing syllable). Recovery-only
    // readings (typo correction, abbreviation) keep the span pinyin-owned but
    // are not strong enough to attach mixed-input decoding.
    bool strict{};

    bool operator==(const UnifiedSegment&) const = default;
};

struct UnifiedInputPlan {
    std::string raw;
    std::vector<UnifiedSegment> segments;
    std::size_t primary_pinyin_segment{kNoUnifiedSegment};
    bool has_pinyin{};
    // At least one Latin, number, or symbol span is present. Whitespace alone
    // does not make an otherwise pure pinyin reading mixed.
    bool mixed{};
    // The primary pinyin span parsed strictly.
    bool decodable{};
    // A 1-5 tone digit was consumed into a reading.
    bool tone_normalized{};
    // The preedit contains material outside the lowercase pinyin alphabet
    // (digits, symbols, whitespace, CJK, uppercase). Letter-only pinyin input,
    // including typo recovery and abbreviation, stays with the pinyin engine
    // and never receives literal or structural extras.
    bool foreign_input{};
    // At least one ASCII uppercase letter: the user signalled explicit Latin
    // intent (acronym, camelCase, English word), so the literal candidate
    // ranks above the literal-tail default.
    bool explicit_latin{};

    [[nodiscard]] bool pure_pinyin() const noexcept {
        return decodable && !mixed && !tone_normalized && !foreign_input;
    }
};

[[nodiscard]] UnifiedInputPlan plan_unified_input(const InputSchema& schema,
                                                  std::string_view raw);

// Counts what the unified entry point actually added. The core service logs
// this so the field can tell which candidate families are firing without
// reproducing a session.
struct UnifiedCandidateBreakdown {
    std::size_t decoded{};
    std::size_t literal{};
    std::size_t rules{};
    std::size_t expressive{};
    std::size_t combinations{};
    std::size_t concepts{};

    [[nodiscard]] std::size_t total() const noexcept {
        return decoded + literal + rules + expressive + combinations + concepts;
    }
};

struct UnifiedCandidateOptions {
    std::size_t limit{12};
    std::size_t decode_limit{8};
    std::size_t decode_path_limit{16};
    std::size_t rule_limit{4};
    std::size_t expressive_limit{3};
    std::int64_t literal_score{kUnifiedLiteralScore};
    bool include_literal{true};
    bool include_rule_candidates{true};
    bool include_expressive{true};
    bool include_combinations{true};
    bool include_concepts{true};
    const ExpressiveAssetSource* expressive_source{};
    // Null selects the built-in starter glossary.
    const ConceptTable* concept_table{};
    std::size_t concept_limit{2};
};

// Whole-input candidates for modeless preedit content. Dictionary decoding is
// attempted for the primary pinyin span when the input is mixed or uses tone
// digits; the remaining spans stay literal. Literal, structural, expressive,
// and concept candidates are inserted by score so ordinary pinyin ranking is
// not disturbed.
[[nodiscard]] std::vector<Candidate> generate_unified_candidates(
    const InputSchema& schema, const Lexicon& lexicon, std::string_view raw,
    const BigramModel* bigram = nullptr,
    const UserFrequencyModel* user_frequency = nullptr,
    const UnifiedCandidateOptions& options = {},
    UnifiedCandidateBreakdown* breakdown = nullptr);

// Adds unified extras to an already generated candidate list. Existing order
// is preserved except that each extra is inserted before the first candidate
// with a lower score; duplicate texts are dropped.
void merge_unified_candidates(std::vector<Candidate>& target,
                              std::vector<Candidate>&& extras);

}  // namespace owo::engine
