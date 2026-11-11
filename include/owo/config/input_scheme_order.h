#pragma once

#include <algorithm>
#include <span>
#include <string>
#include <string_view>

namespace owo::config {

// Returns the next enabled/usable provider; an empty current ID selects the
// first usable provider. Emergency English is never persisted by this lookup.
template<class IsAvailable>
std::string next_enabled_input_scheme(std::span<const std::string> order,
                                      std::string_view current, IsAvailable is_available) {
    const auto position = std::ranges::find(order, current);
    const auto first = position == order.end() ? std::size_t{0}
        : (static_cast<std::size_t>(position - order.begin()) + 1) % order.size();
    for (std::size_t index = 0; index < order.size(); ++index) {
        const auto& id = order[(first + index) % order.size()];
        if (!id.starts_with('!') && is_available(id)) return id;
    }
    return "english";
}

}  // namespace owo::config
