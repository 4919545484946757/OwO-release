#pragma once

#include <algorithm>
#include <array>
#include <cstddef>
#include <numeric>
#include <span>
#include <string>
#include <vector>

namespace owo::engine {

// Candidates have already passed quality filtering. Keep each reading's
// internal rank, diversify each pair, then continue the preceding pair's
// reading order while those readings still have candidates.
inline std::vector<std::size_t> sentence_reading_page_order(
    const std::span<const std::string> readings) {
    std::vector<std::size_t> remaining(readings.size()), order;
    std::iota(remaining.begin(), remaining.end(), 0);
    order.reserve(readings.size());
    const auto missing = readings.size();
    std::array<std::size_t, 2> previous{missing, missing};
    while (!remaining.empty()) {
        std::array<std::size_t, 2> page{missing, missing};
        for (std::size_t slot = 0; slot < 2 && !remaining.empty(); ++slot) {
            auto chosen = remaining.end();
            const auto distinct = [&](std::size_t index) {
                return !readings[index].empty() &&
                    (slot == 0 || readings[index] != readings[page[0]]);
            };
            for (const auto previous_index : previous) {
                if (previous_index == missing) continue;
                const auto& reading = readings[previous_index];
                if (reading.empty() || (slot != 0 && reading == readings[page[0]])) continue;
                chosen = std::find_if(remaining.begin(), remaining.end(), [&](auto index) { return readings[index] == reading; });
                if (chosen != remaining.end()) break;
            }
            if (chosen == remaining.end() && slot != 0)
                chosen = std::find_if(remaining.begin(), remaining.end(), distinct);
            if (chosen == remaining.end()) chosen = remaining.begin();
            page[slot] = *chosen;
            order.push_back(*chosen);
            remaining.erase(chosen);
        }
        previous = page;
    }
    return order;
}

} // namespace owo::engine
