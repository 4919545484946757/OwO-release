#pragma once

#include <algorithm>
#include <cstddef>
#include <limits>
#include <optional>

namespace owo::tsf {

// 选中页与可视窗口分离；行内编号始终映射到选中页，而不是滚动窗口的第一行。
struct ExpandedCandidatePages {
    std::size_t selected{};
    std::size_t first_visible{};

    void reset(std::size_t page = 0) noexcept { selected = first_visible = page; }

    static std::size_t page_count(std::size_t count, std::size_t size) noexcept {
        size = std::max<std::size_t>(1, size);
        return count / size + (count % size != 0);
    }

    // 插件每批候选按整页请求，避免 256 项的批边界切开 7 项一页的键位映射。
    static std::size_t block_size(std::size_t maximum, std::size_t size) noexcept {
        maximum = std::max<std::size_t>(1, maximum);
        size = std::min(maximum, std::max<std::size_t>(1, size));
        return maximum / size * size;
    }

    void ensure_visible(std::size_t count, std::size_t size, std::size_t rows) noexcept {
        const auto pages = page_count(count, size);
        rows = std::max<std::size_t>(1, rows);
        selected = std::min(selected, pages ? pages - 1 : 0);
        first_visible = std::min(first_visible, pages > rows ? pages - rows : 0);
        if (selected < first_visible) first_visible = selected;
        else if (selected - first_visible >= rows) first_visible = selected - rows + 1;
    }

    void move_requested(int direction) noexcept {
        const auto magnitude = direction < 0 ? static_cast<std::size_t>(-static_cast<long long>(direction))
                                             : static_cast<std::size_t>(direction);
        if (direction < 0) selected -= std::min(selected, magnitude);
        else selected += std::min(magnitude, std::numeric_limits<std::size_t>::max() - selected);
    }

    void move(int direction, std::size_t count, std::size_t size, std::size_t rows) noexcept {
        move_requested(direction);
        ensure_visible(count, size, rows);
    }

    void scroll(int direction, std::size_t count, std::size_t size, std::size_t rows) noexcept {
        const auto old_selected = selected;
        selected = first_visible;
        move_requested(direction);
        first_visible = selected;
        const auto pages = page_count(count, size);
        rows = std::max<std::size_t>(1, rows);
        first_visible = std::min(first_visible, pages > rows ? pages - rows : 0);
        selected = std::clamp(old_selected, first_visible,
            first_visible + std::min(rows, pages ? pages : 1) - 1);
        ensure_visible(count, size, rows);
    }

    [[nodiscard]] std::optional<std::size_t> index(std::size_t slot, std::size_t count,
                                                  std::size_t size) const noexcept {
        size = std::max<std::size_t>(1, size);
        if (slot >= size || selected >= page_count(count, size)) return {};
        const auto begin = selected * size; // selected < ceil(count / size)，乘积不会溢出。
        return slot < count - begin ? std::optional{begin + slot} : std::nullopt;
    }
};

} // namespace owo::tsf
