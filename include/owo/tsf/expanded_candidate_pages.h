#pragma once

#include "owo/protocol/candidate_pages.h"

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

    void set_layout(std::size_t count, std::size_t size,
                    std::span<const std::uint64_t> groups = {}) {
        boundaries_ = protocol::candidate_page_boundaries(count, size, groups);
        layout_count_ = count; layout_size_ = size;
    }

    std::size_t count_pages(std::size_t count, std::size_t size) const noexcept {
        return has_layout(count, size) ? boundaries_.size() - 1 : page_count(count, size);
    }

    std::size_t begin(std::size_t page, std::size_t count, std::size_t size) const noexcept {
        if (page >= count_pages(count, size)) return count;
        return has_layout(count, size) ? boundaries_[page] : page * std::max<std::size_t>(1, size);
    }

    std::size_t end(std::size_t page, std::size_t count, std::size_t size) const noexcept {
        const auto start = begin(page, count, size);
        return has_layout(count, size) && page < count_pages(count, size)
            ? boundaries_[page + 1] : start + std::min(count - start, std::max<std::size_t>(1, size));
    }

    std::size_t page_for(std::size_t index, std::size_t count, std::size_t size) const noexcept {
        if (has_layout(count, size))
            return static_cast<std::size_t>(std::upper_bound(boundaries_.begin(), boundaries_.end(), index) - boundaries_.begin() - 1);
        return index / std::max<std::size_t>(1, size);
    }

    // 插件每批候选按整页请求，避免 256 项的批边界切开 7 项一页的键位映射。
    static std::size_t block_size(std::size_t maximum, std::size_t size) noexcept {
        maximum = std::max<std::size_t>(1, maximum);
        size = std::min(maximum, std::max<std::size_t>(1, size));
        return maximum / size * size;
    }

    void ensure_visible(std::size_t count, std::size_t size, std::size_t rows) noexcept {
        const auto pages = count_pages(count, size);
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
        const auto pages = count_pages(count, size);
        rows = std::max<std::size_t>(1, rows);
        first_visible = std::min(first_visible, pages > rows ? pages - rows : 0);
        selected = std::clamp(old_selected, first_visible,
            first_visible + std::min(rows, pages ? pages : 1) - 1);
        ensure_visible(count, size, rows);
    }

    [[nodiscard]] std::optional<std::size_t> index(std::size_t slot, std::size_t count,
                                                  std::size_t size) const noexcept {
        size = std::max<std::size_t>(1, size);
        if (slot >= size || selected >= count_pages(count, size)) return {};
        const auto start = begin(selected, count, size);
        return slot < end(selected, count, size) - start ? std::optional{start + slot} : std::nullopt;
    }

private:
    bool has_layout(std::size_t count, std::size_t size) const noexcept {
        return !boundaries_.empty() && layout_count_ == count && layout_size_ == size;
    }
    std::vector<std::size_t> boundaries_;
    std::size_t layout_count_{}, layout_size_{};
};

} // namespace owo::tsf
