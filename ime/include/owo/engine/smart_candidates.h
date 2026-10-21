#pragma once

#include "owo/engine/candidate_generator.h"

#include <cstddef>
#include <cstdint>
#include <string_view>
#include <vector>

namespace owo::engine {

// Intelligent candidates extend, rather than replace, the dictionary ranking.
// Every rule below is deterministic, local, and bounded: it inspects the
// current preedit only, never the surrounding document, and never leaves the
// machine.

inline constexpr std::int64_t kSmartMathResultScore = 5'200;
inline constexpr std::int64_t kSmartCorrectionScore = 3'000;
inline constexpr std::int64_t kSmartOrderScore = 2'850;
inline constexpr std::int64_t kSmartNumberScore = 2'650;
inline constexpr std::int64_t kSmartNumberFormalScore = 2'550;
inline constexpr std::int64_t kSmartArabicNumberScore = 2'600;
inline constexpr std::int64_t kSmartStructureScore = 2'700;
inline constexpr std::int64_t kSmartWeekdayScore = 2'600;
inline constexpr std::int64_t kSmartShapeScore = 2'400;
inline constexpr std::int64_t kSmartDurationScore = 2'450;
inline constexpr std::int64_t kSmartWidthScore = 2'300;
inline constexpr std::int64_t kSmartSymbolScore = 2'500;
inline constexpr std::int64_t kSmartSpacingScore = 2'200;
inline constexpr std::size_t kSmartMaximumCandidates = 6;
// Expressions and linted text stay short so a pathological paste cannot turn
// candidate generation into unbounded work.
inline constexpr std::size_t kSmartMaximumInputBytes = 64;
// Chinese numeral conversion covers ordinary amounts; longer digit runs are
// left alone rather than risk a wrong grouping.
inline constexpr std::size_t kSmartMaximumNumberDigits = 16;

// Structure candidate: evaluates one ASCII arithmetic expression that covers
// the whole input. Digits, + - * / % ^ ( ) . and spaces are accepted. A
// trailing '=' commits "expression=result"; otherwise the candidate is the
// bare result. Division by zero and malformed input produce nothing.
[[nodiscard]] std::vector<Candidate> generate_math_result_candidates(
    std::string_view raw);

// Value and format lint: impossible calendar dates are clamped to the nearest
// valid day, a reversed date range is swapped, repeated punctuation collapses,
// unbalanced quotes/brackets are closed, ASCII punctuation next to Chinese text
// becomes full-width, missing CJK/Latin spacing is inserted, and full-width
// ASCII collapses to half-width. Each triggered rule contributes at most one
// whole-text candidate.
[[nodiscard]] std::vector<Candidate> generate_text_lint_candidates(
    std::string_view raw);

// Shape candidates for short literal input: split camelCase or separator
// words, join space-separated ASCII words, reverse a comma-separated list, and
// fill a trailing "__" gap marker with an explicit placeholder.
[[nodiscard]] std::vector<Candidate> generate_text_shape_candidates(
    std::string_view raw);

// Number reading candidates: a whole-input ASCII decimal (optionally with a
// trailing '%') offers the Chinese lowercase reading and, for integer amounts,
// the formal financial reading. "12.5" -> 十二点五, "12%" -> 百分之十二,
// "1234" -> 一千二百三十四 / 壹仟贰佰叁拾肆.
[[nodiscard]] std::vector<Candidate> generate_number_reading_candidates(
    std::string_view raw);

// Arabic reading for a Chinese numeral: 一千二百三十四 -> 1234,
// 壹仟贰佰叁拾肆元整 -> 1234, 十二点五 -> 12.5, 两百 -> 200. Consecutive
// digits, repeated units and values beyond 1e16 are rejected.
[[nodiscard]] std::vector<Candidate> generate_arabic_number_candidates(
    std::string_view raw);

// Structured date/time results: a whole-input date also offers the localized
// form and the weekday ("2026-10-19" -> 2026年10月19日 / 2026年10月19日 星期一),
// a Chinese date offers ISO form, and a loosely padded time is normalized
// ("9:5" -> 09:05). Invalid dates and times produce nothing.
[[nodiscard]] std::vector<Candidate> generate_date_structure_candidates(
    std::string_view raw);

// Symbol encodings: ASCII arrows and comparisons offer their Unicode form
// ("a->b" -> a→b, "x!=y" -> x≠y, "a<=b" -> a≤b, "a...b" -> a…b). Command-line
// flags such as "--help" are left alone.
[[nodiscard]] std::vector<Candidate> generate_symbol_candidates(
    std::string_view raw);

// Duration reading for notes: a whole-input decimal plus `h`, `min`, or `s`
// offers the Chinese form ("90min" -> 1小时30分, "1.5h" -> 1小时30分,
// "75s" -> 1分15秒). Only the exact unit suffixes are accepted so ordinary
// identifiers are left alone.
[[nodiscard]] std::vector<Candidate> generate_duration_candidates(
    std::string_view raw);

// Merged, deduplicated, bounded view used by the unified entry point.
[[nodiscard]] std::vector<Candidate> generate_smart_candidates(
    std::string_view raw, std::size_t limit = 4);

}  // namespace owo::engine
