#include "owo/engine/symbol_composition.h"
#include <algorithm>

namespace owo::engine {
namespace {
std::vector<std::size_t> boundaries(std::string_view text) {
    std::vector<std::size_t> result{0};
    for (std::size_t i = 0; i < text.size();) {
        const auto c = static_cast<unsigned char>(text[i]);
        const auto length = c < 0x80 ? 1U : (c & 0xe0) == 0xc0 ? 2U : (c & 0xf0) == 0xe0 ? 3U : (c & 0xf8) == 0xf0 ? 4U : 0U;
        if (!length || i + length > text.size()) return {};
        for (unsigned j = 1; j < length; ++j)
            if ((static_cast<unsigned char>(text[i + j]) & 0xc0) != 0x80) return {};
        if ((length == 2 && c < 0xc2) || (length == 4 && c > 0xf4) ||
            (length == 3 && c == 0xe0 && static_cast<unsigned char>(text[i + 1]) < 0xa0) ||
            (length == 3 && c == 0xed && static_cast<unsigned char>(text[i + 1]) >= 0xa0) ||
            (length == 4 && c == 0xf0 && static_cast<unsigned char>(text[i + 1]) < 0x90) ||
            (length == 4 && c == 0xf4 && static_cast<unsigned char>(text[i + 1]) >= 0x90)) return {};
        i += length; result.push_back(i);
    }
    return result;
}
bool letter(char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'); }
std::size_t letters(std::string_view value) { return std::count_if(value.begin(), value.end(), letter); }
}

CompositionSymbolKind composition_symbol_kind(std::string_view value) {
    for (const auto strong : {",", ".", ";", ":", "!", "?", "，", "。", "；", "：", "！", "？", "、", "\n", "\r"})
        if (value == strong) return CompositionSymbolKind::strong;
    for (const auto weak : {"\"", "'", "‘", "’", "“", "”"})
        if (value == weak) return CompositionSymbolKind::weak;
    return CompositionSymbolKind::literal;
}

SymbolComposition split_symbol_composition(std::string_view source,
    const bool apostrophe_is_separator, const std::span<const std::uint64_t> manual_breaks) {
    SymbolComposition result;
    result.source = source;
    const auto points = boundaries(source);
    if (points.empty() || source.size() > 4096) return result;
    for (const auto offset : manual_breaks)
        if (offset >= source.size() || source[static_cast<std::size_t>(offset)] != '\'' ||
            !std::binary_search(points.begin(), points.end(), static_cast<std::size_t>(offset))) return result;
    result.segments.emplace_back();
    std::vector<std::pair<std::string, std::size_t>> quote_stack;
    bool separate = false;
    for (std::size_t index = 0; index + 1 < points.size(); ++index) {
        const auto begin = points[index], end = points[index + 1];
        const auto token = source.substr(begin, end - begin);
        if (!result.segments.back().delimiter.empty() && composition_symbol_kind(token) != CompositionSymbolKind::strong) {
            result.segments.emplace_back(); result.segments.back().source_begin = begin;
            result.segments.back().source_ends[0] = begin;
            quote_stack.clear(); separate = false;
        }
        auto& segment = result.segments.back(); segment.source_end = end;
        if (token.size() == 1 && (letter(token.front()) || (token == "'" &&
            (apostrophe_is_separator || std::find(manual_breaks.begin(), manual_breaks.end(), begin) != manual_breaks.end())))) {
            if (separate && !segment.query.empty() && segment.query.back() != '\'' && token != "'") {
                segment.query += '\''; segment.source_ends.push_back(begin);
            }
            segment.query += token; segment.source_ends.push_back(end);
            segment.letters += letter(token.front()); separate = false;
            continue;
        }
        const auto kind = composition_symbol_kind(token);
        if (kind == CompositionSymbolKind::strong) { segment.delimiter += token; continue; }
        bool opening = false;
        if (kind == CompositionSymbolKind::weak) {
            const bool explicit_open = token == "‘" || token == "“";
            const bool explicit_close = token == "’" || token == "”";
            const auto family = (token == "‘" || token == "’") ? std::string("'") :
                (token == "“" || token == "”") ? std::string("\"") : std::string(token);
            const bool closes = !quote_stack.empty() && quote_stack.back().first == family && !explicit_open;
            if (closes) { segment.weak_spans.push_back({quote_stack.back().second, segment.letters}); quote_stack.pop_back(); }
            else if (!explicit_close) { opening = true; quote_stack.emplace_back(family, segment.letters); }
        }
        segment.symbols.push_back({std::string(token), segment.letters, begin, end, opening});
        separate = true;
    }
    result.valid = true;
    return result;
}

SymbolProjection project_symbol_candidate(const SymbolSegment& segment,
    std::string_view text, std::span<const std::string> syllables, const std::size_t query_consumed) {
    SymbolProjection result;
    if (query_consumed > segment.query.size()) return result;
    const auto consumed_letters = letters(std::string_view(segment.query).substr(0, query_consumed));
    const auto glyphs = boundaries(text);
    if (glyphs.empty()) return result;
    std::vector<std::string> insertions(glyphs.size());
    result.consumed = query_consumed < segment.source_ends.size() ? segment.source_ends[query_consumed] : segment.source_begin;
    for (const auto& symbol : segment.symbols) {
        if (symbol.letters > consumed_letters || (symbol.opening && symbol.letters == consumed_letters && query_consumed < segment.query.size())) continue;
        std::size_t position = 0, source_letters = 0;
        for (const auto& syllable : syllables) {
            if (source_letters >= symbol.letters) break;
            source_letters += letters(syllable); ++position;
        }
        if (syllables.size() != glyphs.size() - 1)
            position = consumed_letters == 0 ? 0 : symbol.letters * (glyphs.size() - 1) / consumed_letters;
        insertions[std::min(position, glyphs.size() - 1)] += symbol.text;
        result.consumed = std::max(result.consumed, symbol.source_end);
    }
    for (std::size_t index = 0; index < glyphs.size(); ++index) {
        result.text += insertions[index];
        if (index + 1 < glyphs.size()) result.text += text.substr(glyphs[index], glyphs[index + 1] - glyphs[index]);
    }
    if (query_consumed == segment.query.size()) {
        result.text += segment.delimiter;
        result.consumed = segment.source_end;
    }
    return result;
}

std::string symbol_segment_preview(const SymbolSegment& segment, std::span<const std::string> syllables) {
    // Build letter-boundary positions from the source reading, preserving case.
    std::vector<std::size_t> cuts;
    std::size_t count = 0;
    for (const auto& syllable : syllables) { count += letters(syllable); cuts.push_back(count); }
    std::string result;
    std::size_t current = 0;
    for (std::size_t index = 0; index < segment.query.size(); ++index) {
        for (const auto& symbol : segment.symbols)
            if (symbol.letters == current && (index == 0 || segment.query[index - 1] != '\'')) result += symbol.text;
        const auto c = segment.query[index];
        if (c == '\'') {
            const bool literal_boundary = std::any_of(segment.symbols.begin(), segment.symbols.end(),
                [current](const auto& symbol) { return symbol.letters == current; });
            if (!literal_boundary) result += c;
        } else {
            if (index != 0 && segment.query[index - 1] != '\'' &&
                std::find(cuts.begin(), cuts.end(), current) != cuts.end()) result += '\'';
            result += c; ++current;
        }
    }
    for (const auto& symbol : segment.symbols) if (symbol.letters == current) result += symbol.text;
    return result + segment.delimiter;
}

void refine_weak_symbol_candidates(const SymbolSegment& segment, std::vector<Candidate>& candidates,
    const std::function<std::vector<Candidate>(std::string_view)>& lookup, const std::size_t maximum) {
    if (segment.weak_spans.empty() || candidates.empty() || maximum == 0) return;
    std::string compact;
    for (const auto c : segment.query) if (letter(c)) compact += c;
    std::vector<Candidate> additions;
    for (const auto& span : segment.weak_spans) {
        if (span.begin_letters >= span.end_letters || span.end_letters > compact.size()) continue;
        const auto query = std::string_view(compact).substr(span.begin_letters, span.end_letters - span.begin_letters);
        const auto local = lookup(query);
        // Only a bounded number of complete base sentences are refined.
        std::size_t seeds = 0;
        for (const auto& base : candidates) {
            if (base.model_only || base.consumed_input_bytes != segment.query.size()) continue;
            const auto points = boundaries(base.text);
            if (points.empty() || points.size() - 1 != base.source_segments.size()) continue;
            std::vector<std::size_t> cuts{0};
            for (const auto& part : base.source_segments) cuts.push_back(cuts.back() + letters(part));
            const auto first = std::find(cuts.begin(), cuts.end(), span.begin_letters);
            const auto last = std::find(cuts.begin(), cuts.end(), span.end_letters);
            if (first == cuts.end() || last == cuts.end()) continue;
            const auto a = static_cast<std::size_t>(first - cuts.begin());
            const auto b = static_cast<std::size_t>(last - cuts.begin());
            for (const auto& choice : local) {
                const auto chars = boundaries(choice.text);
                if (choice.model_only || choice.consumed_input_bytes != query.size() ||
                    chars.empty() || chars.size() - 1 != b - a) continue;
                auto changed = base;
                changed.text.replace(points[a], points[b] - points[a], choice.text);
                if (std::any_of(candidates.begin(), candidates.end(), [&](const auto& c) { return c.text == changed.text; }) ||
                    std::any_of(additions.begin(), additions.end(), [&](const auto& c) { return c.text == changed.text; })) continue;
                // Whole-input evidence remains the baseline; local lookup only
                // supplies bounded alternatives, never an unconditional pin.
                changed.score = base.score - static_cast<std::int64_t>(additions.size() + 1);
                additions.push_back(std::move(changed));
                if (additions.size() >= maximum) break;
            }
            if (++seeds >= 2 || additions.size() >= maximum) break;
        }
        if (additions.size() >= maximum) break;
    }
    candidates.insert(candidates.begin() + 1, std::make_move_iterator(additions.begin()),
        std::make_move_iterator(additions.end()));
}
} // namespace owo::engine
