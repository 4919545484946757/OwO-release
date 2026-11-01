#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <span>
#include <vector>

namespace owo::protocol {

// The final boundary is the item count. Sparse partitioned pages may end
// early rather than pull a third sentence into a configured-length row.
inline std::vector<std::size_t> candidate_page_boundaries(
    const std::size_t count, std::size_t size,
    const std::span<const std::uint64_t> groups = {}) {
    size = std::max<std::size_t>(1, size);
    const bool partitioned = size >= 4 && groups.size() == count;
    std::vector<std::size_t> boundaries{0};
    std::size_t sentences = 0;
    for (std::size_t index = 0; index < count; ++index) {
        const bool sentence = partitioned && groups[index] == 0;
        if (index - boundaries.back() >= size || (sentence && sentences == 2)) {
            boundaries.push_back(index); sentences = 0;
        }
        sentences += sentence;
    }
    if (count != 0) boundaries.push_back(count);
    return boundaries;
}

} // namespace owo::protocol
