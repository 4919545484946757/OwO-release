#pragma once
#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <string_view>
#include <vector>
#include <functional>
#include "owo/engine/candidate_generator.h"

namespace owo::engine {
enum class CompositionSymbolKind { strong, weak, literal };
struct CompositionSymbol {
    std::string text;
    std::size_t letters{}, source_begin{}, source_end{};
    bool opening{};
};
struct WeakSymbolSpan { std::size_t begin_letters{}, end_letters{}; };
struct SymbolSegment {
    std::string query;
    std::vector<std::size_t> source_ends{0};
    std::vector<CompositionSymbol> symbols;
    std::vector<WeakSymbolSpan> weak_spans;
    std::size_t source_begin{}, source_end{}, letters{};
    std::string delimiter;
};
struct SymbolComposition {
    bool valid{};
    std::string source;
    std::vector<SymbolSegment> segments;
};
struct SymbolProjection { std::string text; std::size_t consumed{}; };
[[nodiscard]] CompositionSymbolKind composition_symbol_kind(std::string_view);
// Offsets are UTF-8 source byte positions. Explicit manual breaks remain syntax,
// even when the physical apostrophe key is no longer bound to that action.
[[nodiscard]] SymbolComposition split_symbol_composition(std::string_view source,
    bool apostrophe_is_separator, std::span<const std::uint64_t> manual_breaks = {});
[[nodiscard]] SymbolProjection project_symbol_candidate(const SymbolSegment&,
    std::string_view text, std::span<const std::string> source_syllables,
    std::size_t query_consumed);
[[nodiscard]] std::string symbol_segment_preview(const SymbolSegment&,
    std::span<const std::string> source_syllables);
// Refine one quoted span at a time, retaining the rest of a complete candidate.
// Never forms a Cartesian product of alternatives across quotes.
void refine_weak_symbol_candidates(const SymbolSegment&, std::vector<Candidate>&,
    const std::function<std::vector<Candidate>(std::string_view)>& lookup,
    std::size_t maximum_alternatives);
} // namespace owo::engine
