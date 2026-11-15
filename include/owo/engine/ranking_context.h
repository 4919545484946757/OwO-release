#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <type_traits>

namespace owo::engine {

namespace ranking_context_detail {
template <class Char>
std::uint32_t scalar(std::basic_string_view<Char> text, std::size_t& offset) noexcept {
    if constexpr (std::is_same_v<Char, char>) {
        const auto first = static_cast<unsigned char>(text[offset++]);
        if (first < 0xc2 || first > 0xf4) return first;
        const auto rest = first < 0xe0 ? 1U : first < 0xf0 ? 2U : 3U;
        if (text.size() - offset < rest) return first;
        auto value = static_cast<std::uint32_t>(first & (0x7fU >> (rest + 1)));
        for (unsigned i = 0; i < rest; ++i) {
            const auto byte = static_cast<unsigned char>(text[offset + i]);
            if ((byte & 0xc0U) != 0x80U) return first;
            value = (value << 6) | (byte & 0x3fU);
        }
        offset += rest;
        return value;
    } else {
        const auto first = static_cast<std::uint32_t>(text[offset++]);
        if constexpr (sizeof(Char) == 2) {
            if (first >= 0xd800 && first <= 0xdbff && offset < text.size()) {
                const auto second = static_cast<std::uint32_t>(text[offset]);
                if (second >= 0xdc00 && second <= 0xdfff) {
                    ++offset;
                    return 0x10000 + ((first - 0xd800) << 10) + second - 0xdc00;
                }
            }
        }
        return first;
    }
}

inline bool numeric_symbol(const std::uint32_t c) noexcept {
    if (c < 0x80) return c >= '0' && c <= '9';
    if (c >= 0x3400 && c <= 0x9fff) return false;
    // Unicode 16 decimal blocks plus common decorated and fractional numeric
    // symbols. Keep written Chinese numerals intact in actual words such as 一起.
    constexpr std::uint32_t decimal_zeros[]{0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6,
        0xb66, 0xbe6, 0xc66, 0xce6, 0xd66, 0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090,
        0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90, 0x1b50, 0x1bb0, 0x1c40, 0x1c50,
        0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0, 0xff10, 0x104a0, 0x10d30, 0x10d40,
        0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450, 0x114d0, 0x11650, 0x116c0, 0x116d0, 0x116da,
        0x11730, 0x118e0, 0x11950, 0x11bf0, 0x11c50, 0x11d50, 0x11da0, 0x11f50, 0x16130, 0x16a60, 0x16ac0,
        0x16b50, 0x16d70, 0x1ccf0, 0x1d7ce, 0x1d7d8, 0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0,
        0x1e4f0, 0x1e5f1, 0x1e950, 0x1fbf0};
    for (const auto first : decimal_zeros) {
        if (c < first) break;
        if (c - first < 10) return true;
    }
    return c == 0xb2 || c == 0xb3 || c == 0xb9 || (c >= 0xbc && c <= 0xbe) ||
        c == 0x2070 || (c >= 0x2074 && c <= 0x2079) || (c >= 0x2080 && c <= 0x2089) ||
        (c >= 0x2150 && c <= 0x2189) || (c >= 0x2460 && c <= 0x249b) ||
        (c >= 0x24ea && c <= 0x24ff) || (c >= 0x2776 && c <= 0x2793) || c == 0x3007 ||
        (c >= 0x3251 && c <= 0x325f) || (c >= 0x32b1 && c <= 0x32bf) ||
        (c >= 0x1f100 && c <= 0x1f10a);
}
}  // namespace ranking_context_detail

// Collapse runs, rather than preserving the digits or even how many were typed.
// This is only for ranking context, not visible text or the active composition.
template <class Char>
void redact_ranking_numbers(std::basic_string<Char>& text) noexcept {
    std::size_t written = 0;
    bool numeric_run = false;
    for (std::size_t offset = 0; offset < text.size();) {
        const auto start = offset;
        const auto c = ranking_context_detail::scalar(std::basic_string_view<Char>(text), offset);
        if (ranking_context_detail::numeric_symbol(c)) {
            if (!numeric_run) text[written++] = static_cast<Char>('0');
            numeric_run = true;
        } else {
            numeric_run = false;
            const auto size = offset - start;
            if (written != start) std::char_traits<Char>::move(text.data() + written, text.data() + start, size);
            written += size;
        }
    }
    text.resize(written);
}

// Readers see mostly already-canonical context; avoid copying it for every
// lookup of a learned candidate.
inline bool ranking_numbers_need_redaction(const std::string_view text) noexcept {
    bool previous_zero = false;
    for (std::size_t offset = 0; offset < text.size();) {
        const auto c = ranking_context_detail::scalar(text, offset);
        if ((c == '0' && previous_zero) || (c != '0' && ranking_context_detail::numeric_symbol(c))) return true;
        previous_zero = c == '0';
    }
    return false;
}

template <class Char>
bool ranking_context_ends_in_number(std::basic_string_view<Char> text) noexcept {
    if (text.empty()) return false;
    auto last = text.size() - 1;
    if constexpr (std::is_same_v<Char, char>) {
        while (last && (static_cast<unsigned char>(text[last]) & 0xc0U) == 0x80U) --last;
    } else if constexpr (sizeof(Char) == 2) {
        if (last && text[last] >= 0xdc00 && text[last] <= 0xdfff &&
            text[last - 1] >= 0xd800 && text[last - 1] <= 0xdbff) --last;
    }
    return ranking_context_detail::numeric_symbol(ranking_context_detail::scalar(text, last)) && last == text.size();
}

}  // namespace owo::engine
