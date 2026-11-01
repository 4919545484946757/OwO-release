#pragma once

#include <algorithm>
#include <string>
#include <string_view>

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

// Display-only newlines: the source/candidate payload and consume ranges never
// change. Short symbol-separated fragments may share a line; a long fragment
// moves as a unit before being folded further if it exceeds the six-char cap.
inline std::wstring wrapped_candidate_text(std::wstring_view text, std::size_t wrap_length,
                                          const bool symbol_wrap = false) {
    constexpr std::size_t symbol_limit = 6;
    wrap_length = std::max<std::size_t>(1, wrap_length);
    const auto fragment_limit = std::min(symbol_limit, wrap_length);
    std::wstring staged;
    std::size_t line_length = 0;
    for (std::size_t begin = 0; begin < text.size();) {
        auto end = begin;
        while (end < text.size() && (!symbol_wrap || !candidate_symbol_break(text[end])) &&
               text[end] != L'\r' && text[end] != L'\n') end += candidate_scalar_size(text, end);
        const auto fragment = text.substr(begin, end - begin);
        const auto length = candidate_scalar_count(fragment);
        if (symbol_wrap && length > symbol_limit && line_length != 0 &&
            line_length + length > symbol_limit) { staged += L'\n'; line_length = 0; }
        std::size_t fragment_line = 0;
        for (std::size_t at = 0; at < fragment.size();) {
            if (symbol_wrap && length > symbol_limit && fragment_line == fragment_limit) {
                staged += L'\n'; line_length = fragment_line = 0;
            }
            const auto size = candidate_scalar_size(fragment, at);
            staged.append(fragment.substr(at, size)); at += size;
            ++line_length; ++fragment_line;
        }
        if (end == text.size()) break;
        const auto delimiter = text[end++];
        if (delimiter == L'\r' || delimiter == L'\n') {
            if (delimiter == L'\r' && end < text.size() && text[end] == L'\n') ++end;
            staged += L'\n'; line_length = 0;
        } else staged += delimiter;
        begin = end;
    }
    std::wstring result;
    line_length = 0;
    for (std::size_t at = 0; at < staged.size();) {
        if (staged[at] == L'\n') { result += L'\n'; ++at; line_length = 0; continue; }
        if (symbol_wrap && candidate_symbol_break(staged[at])) { result += staged[at++]; continue; }
        if (line_length == wrap_length) { result += L'\n'; line_length = 0; }
        const auto size = candidate_scalar_size(staged, at);
        result.append(std::wstring_view(staged).substr(at, size)); at += size; ++line_length;
    }
    return result;
}
} // namespace owo::tsf
