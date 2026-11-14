#pragma once

#include <algorithm>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace owo::config {

inline std::vector<std::string> legacy_input_scheme_order(std::string_view preferred) {
    if (preferred.empty()) preferred = "full_pinyin";
    std::vector<std::string> order{std::string(preferred)};
    if (preferred != "english") order.emplace_back("english");
    return order;
}

template<class IsAvailable>
std::vector<std::string> enabled_input_schemes(std::span<const std::string> order,
                                               IsAvailable is_available) {
    std::vector<std::string> enabled;
    enabled.reserve(order.size());
    for (const auto& id : order)
        if (!id.starts_with('!') && is_available(id)) enabled.push_back(id);
    return enabled;
}

// Returns the next enabled/usable provider; an empty current ID selects the
// first usable provider. Emergency English is never persisted by this lookup.
template<class IsAvailable>
std::string next_enabled_input_scheme(std::span<const std::string> order,
                                      std::string_view current, IsAvailable is_available) {
    const auto enabled = enabled_input_schemes(order, is_available);
    if (enabled.empty()) return "english";
    const auto position = std::ranges::find(enabled, current);
    if (position == enabled.end()) return enabled.front();
    const auto next = position + 1;
    return next == enabled.end() ? enabled.front() : *next;
}

}  // namespace owo::config
