#pragma once
#include <charconv>
#include <cstdint>
#include <optional>
#include <string_view>

namespace owo::config {
struct SchemePresentation {
    bool inline_preview{};
    std::uint32_t page_size{7}, wrap_length{12};
    bool operator==(const SchemePresentation&) const = default;
};

// Strict package/override contract. An invalid override never partially applies.
inline std::optional<SchemePresentation> parse_scheme_presentation(std::string_view text) {
    const auto next = [&]() {
        const auto lf = text.find('\n');
        auto line = text.substr(0, lf);
        text.remove_prefix(lf == text.npos ? text.size() : lf + 1);
        if (line.ends_with('\r')) line.remove_suffix(1);
        return line;
    };
    if (next() != "OWO_SCHEME_PRESENTATION_V1") return {};
    SchemePresentation out;
    unsigned seen{};
    while (!text.empty()) {
        const auto line = next();
        const auto split = line.find('=');
        if (split == line.npos) return {};
        const auto key = line.substr(0, split), value = line.substr(split + 1);
        if (key == "preview" && !(seen & 1) && (value == "input" || value == "candidate")) {
            out.inline_preview = value == "input"; seen |= 1; continue;
        }
        std::uint32_t n{};
        const auto parsed = std::from_chars(value.data(), value.data() + value.size(), n);
        if (parsed.ec != std::errc{} || parsed.ptr != value.data() + value.size()) return {};
        if (key == "page_size" && !(seen & 2) && n >= 1 && n <= 9) { out.page_size = n; seen |= 2; }
        else if (key == "wrap_length" && !(seen & 4) && n >= 4 && n <= 64) { out.wrap_length = n; seen |= 4; }
        else return {};
    }
    return seen == 7 ? std::optional(out) : std::nullopt;
}
}
