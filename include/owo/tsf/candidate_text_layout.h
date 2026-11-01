#pragma once

#include <algorithm>
#include <string>
#include <string_view>
#include <span>
#include <vector>

namespace owo::tsf {
inline bool candidate_symbol_break(const wchar_t value) noexcept {
    return std::wstring_view(L";:!?；：！？、\r\n").find(value) != std::wstring_view::npos;
}
inline std::size_t candidate_scalar_size(std::wstring_view text, const std::size_t at) noexcept {
    return text[at] >= 0xd800 && text[at] <= 0xdbff && at + 1 < text.size() &&
        text[at + 1] >= 0xdc00 && text[at + 1] <= 0xdfff ? 2 : 1;
}
inline std::size_t candidate_scalar_count(std::wstring_view text) noexcept {
    std::size_t count = 0;
    for (std::size_t at = 0; at < text.size(); at += candidate_scalar_size(text, at)) ++count;
    return count;
}

// Ordinary per-candidate wrapping remains configurable and independent from
// the physical placement of symbol-separated candidate blocks.
inline std::wstring wrapped_candidate_text(std::wstring_view text, std::size_t wrap_length) {
    wrap_length = std::max<std::size_t>(1, wrap_length);
    std::wstring result;
    std::size_t line_length = 0;
    for (std::size_t at = 0; at < text.size();) {
        if (text[at] == L'\r' || text[at] == L'\n') {
            if (text[at++] == L'\r' && at < text.size() && text[at] == L'\n') ++at;
            result += L'\n'; line_length = 0; continue;
        }
        if (line_length == wrap_length) { result += L'\n'; line_length = 0; }
        const auto size = candidate_scalar_size(text, at);
        result.append(text.substr(at, size)); at += size; ++line_length;
    }
    return result;
}

struct SymbolCandidateBlock {
    std::size_t maximum_characters{};
    bool mandatory_break_after{true};
};
inline std::vector<std::vector<std::size_t>> symbol_candidate_block_rows(
    const std::span<const SymbolCandidateBlock> blocks) {
    std::vector<std::vector<std::size_t>> rows;
    std::size_t line_characters = 0;
    for (std::size_t index = 0; index < blocks.size(); ++index) {
        const auto length = blocks[index].maximum_characters;
        if (rows.empty() || blocks[index - 1].mandatory_break_after ||
            (line_characters + length > 6 && length > 6)) {
            rows.emplace_back(); line_characters = 0;
        }
        rows.back().push_back(index);
        line_characters += length;
    }
    return rows;
}
} // namespace owo::tsf
