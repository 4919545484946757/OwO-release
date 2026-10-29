#pragma once
#include <algorithm>
#include <string_view>

namespace owo::engine {
// Do not spend a second long-sentence slot on a small internal/leading
// homophone replacement with the same ending. Short word ambiguities and
// alternative sentence endings remain available.
inline bool redundant_sentence_variant(std::string_view first, std::string_view second) {
    const auto count = [](std::string_view text) {
        return static_cast<std::size_t>(std::count_if(text.begin(), text.end(),
            [](unsigned char byte) { return (byte & 0xc0U) != 0x80U; }));
    };
    const auto length = count(first);
    if (length < 4 || count(second) != length || first == second) return false;
    std::size_t prefix = 0, suffix = 0;
    while (prefix < std::min(first.size(), second.size()) && first[prefix] == second[prefix]) ++prefix;
    while (prefix != 0 && prefix < first.size() &&
           (static_cast<unsigned char>(first[prefix]) & 0xc0U) == 0x80U) --prefix;
    while (suffix < std::min(first.size(), second.size()) &&
           first[first.size() - suffix - 1] == second[second.size() - suffix - 1]) ++suffix;
    const auto tail = count(first.substr(first.size() - suffix));
    if (length == 4) return tail >= 2 && count(first.substr(0, prefix)) + tail == 3;
    return tail != 0 && count(first.substr(0, prefix)) + tail >= (length + 1) / 2;
}
}
